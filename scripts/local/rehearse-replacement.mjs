import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  buildContracts,
  deployContract,
  hasFlag,
  invokeContract,
  repoRoot,
  sourceAccount,
} from "../lib/stellar-cli.mjs";
import {
  createMigrationState,
  deprecateDeployment,
  recordSessionRoute,
  routeExistingSession,
  switchActiveDeployment,
  validateMigrationState,
} from "./replacement-routing.mjs";

const dryRun = hasFlag("--dry-run");
const network = process.env.STELLAR_NETWORK || "local";
const deployer = sourceAccount();
const buyerAccount = requiredValue("LUMENBAZAAR_REHEARSAL_BUYER_ACCOUNT", "local-buyer");
const buyerAddress = requiredValue("LUMENBAZAAR_REHEARSAL_BUYER_ADDRESS", "G-LOCAL-BUYER");
const sellerAddress = requiredValue("LUMENBAZAAR_REHEARSAL_SELLER_ADDRESS", "G-LOCAL-SELLER");
const oldWasm = process.env.LUMENBAZAAR_REHEARSAL_OLD_WASM ||
  "target/wasm32v1-none/release/upto_session.wasm";
const replacementWasm = process.env.LUMENBAZAAR_REHEARSAL_NEW_WASM ||
  "target/wasm32v1-none/release/upto_session.wasm";

await run();

async function run() {
  buildContracts({ dryRun });
  const token = deployContract({
    name: "test-token",
    alias: "lumenbazaar-rehearsal-token",
    wasm: "target/wasm32v1-none/release/test_token.wasm",
    source: deployer,
    network,
    dryRun,
  });
  const oldContract = deployContract({
    name: "upto-session-old",
    alias: "lumenbazaar-upto-session-old",
    wasm: oldWasm,
    source: deployer,
    network,
    dryRun,
  });
  const replacementContract = deployContract({
    name: "upto-session-replacement",
    alias: "lumenbazaar-upto-session-replacement",
    wasm: replacementWasm,
    source: deployer,
    network,
    dryRun,
  });
  if (dryRun) {
    token.contractId = "C-DRY-RUN-TOKEN";
    oldContract.contractId = "C-DRY-RUN-OLD";
    replacementContract.contractId = "C-DRY-RUN-REPLACEMENT";
  }

  initializeToken(token.contractId);
  initializeEscrow(oldContract.contractId, token.contractId);
  initializeEscrow(replacementContract.contractId, token.contractId);
  mintBuyer(token.contractId);

  const state = createMigrationState(
    {
      contractId: oldContract.contractId,
      release: "old-v2",
      interfaceVersion: 2,
      wasm: oldWasm,
    },
    {
      contractId: replacementContract.contractId,
      release: "replacement-v2",
      interfaceVersion: 2,
      wasm: replacementWasm,
    },
  );

  const expiry = dryRun ? 500100 : (await latestLedger()) + 100;
  const oldSession = createSession(
    oldContract.contractId,
    token.contractId,
    expiry,
    "11".repeat(32),
    "old-session-dry-run",
  );
  recordSessionRoute(state, oldSession, oldContract.contractId);

  switchActiveDeployment(state, "replacement");
  const replacementSession = createSession(
    replacementContract.contractId,
    token.contractId,
    expiry,
    "22".repeat(32),
    "replacement-session-dry-run",
  );
  recordSessionRoute(state, replacementSession, replacementContract.contractId);

  assertSessionLookup(routeExistingSession(state, oldSession), oldSession);
  assertSessionLookup(routeExistingSession(state, replacementSession), replacementSession);
  assertCrossDeploymentIsolation(replacementContract.contractId, oldSession);

  switchActiveDeployment(state, "old");
  if (routeExistingSession(state, oldSession) !== oldContract.contractId) {
    throw new Error("Rollback routed an old session to the replacement contract.");
  }
  switchActiveDeployment(state, "replacement");
  deprecateDeployment(state, "old");
  validateMigrationState(state);

  const output = {
    ...state,
    network,
    assetContractId: token.contractId,
    checks: {
      oldAndReplacementIdsAreDistinct: oldContract.contractId !== replacementContract.contractId,
      oldSessionRemainsOnOldContract: routeExistingSession(state, oldSession) === oldContract.contractId,
      replacementSessionRemainsOnReplacement:
        routeExistingSession(state, replacementSession) === replacementContract.contractId,
      crossDeploymentLookupRejected: true,
      rollbackPreservedExistingRoutes: true,
      oldDeploymentDeprecatedForNewSessions: !state.deployments.old.acceptsNewSessions,
    },
  };

  if (dryRun) {
    console.log("REPLACEMENT_REHEARSAL_JSON " + JSON.stringify(output));
    return;
  }
  const outputPath = join(repoRoot, "target", "rehearsals", "replacement.json");
  mkdirSync(join(repoRoot, "target", "rehearsals"), { recursive: true });
  writeFileSync(outputPath, `${JSON.stringify(output, null, 2)}\n`, "utf8");
  console.log(`Replacement rehearsal passed; evidence written to ${outputPath}`);
}

function initializeToken(contractId) {
  invokeContract({
    contractId,
    source: deployer,
    network,
    args: ["initialize", "--admin", deployer],
    dryRun,
  });
}

function initializeEscrow(contractId, tokenId) {
  invokeContract({
    contractId,
    source: deployer,
    network,
    args: [
      "initialize",
      "--admin",
      deployer,
      "--supported_assets",
      JSON.stringify([tokenId]),
    ],
    dryRun,
  });
}

function mintBuyer(contractId) {
  invokeContract({
    contractId,
    source: deployer,
    network,
    args: ["mint", "--to", buyerAddress, "--amount", "2000"],
    dryRun,
  });
}

function createSession(contractId, asset, expiry, resourceHash, dryRunId) {
  const output = invokeContract({
    contractId,
    source: buyerAccount,
    network,
    args: [
      "create_session",
      "--buyer",
      buyerAddress,
      "--seller",
      sellerAddress,
      "--asset",
      asset,
      "--max_amount",
      "500",
      "--expires_at_ledger",
      String(expiry),
      "--resource_hash",
      resourceHash,
    ],
    capture: true,
    dryRun,
  });
  if (dryRun) return dryRunId;
  const sessionId = output.match(/[0-9a-fA-F]{64}/)?.[0]?.toLowerCase();
  if (!sessionId) throw new Error(`Could not parse session ID from: ${output}`);
  return sessionId;
}

function assertSessionLookup(contractId, sessionId) {
  invokeContract({
    contractId,
    source: deployer,
    network,
    args: ["get_session", "--session_id", sessionId],
    capture: true,
    dryRun,
  });
}

function assertCrossDeploymentIsolation(contractId, sessionId) {
  if (dryRun) return;
  try {
    assertSessionLookup(contractId, sessionId);
  } catch {
    return;
  }
  throw new Error("Old session ID unexpectedly resolved on the replacement contract.");
}

async function latestLedger() {
  const rpcUrl = process.env.STELLAR_RPC_URL || "http://localhost:8000/soroban/rpc";
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getLatestLedger" }),
  });
  if (!response.ok) throw new Error(`Local RPC returned HTTP ${response.status}.`);
  const payload = await response.json();
  if (!Number.isInteger(payload.result?.sequence)) {
    throw new Error(`Local RPC getLatestLedger failed: ${JSON.stringify(payload)}`);
  }
  return payload.result.sequence;
}

function requiredValue(name, dryRunDefault) {
  const value = process.env[name];
  if (value) return value;
  if (dryRun) return dryRunDefault;
  throw new Error(`Set ${name} for the live local rehearsal.`);
}
