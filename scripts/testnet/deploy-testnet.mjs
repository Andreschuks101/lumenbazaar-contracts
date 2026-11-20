import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { recordReceipt, requireTestnet } from "../lib/network-receipts.mjs";
import { buildSimulateAndSubmit } from "../lib/soroban-transactions.mjs";
import {
  checkLedgerFreshness,
  createDeploymentPlan,
  formatPreflightReport,
  runPreflight,
  expectedWasmHashes,
  verifyExpectedArtifacts,
  writeDeploymentPlan,
} from "../lib/deployment-preflight.mjs";
import {
  buildContracts,
  deployUploadedContract,
  hasFlag,
  invokeContractTransaction,
  publicKey,
  repoRoot,
  runStellar,
  sourceAccount,
  uploadContract,
  writeManifest,
} from "../lib/stellar-cli.mjs";

const dryRun = hasFlag("--dry-run");
const manifestPath = join(repoRoot, "deployments", "testnet.json");
const planPath = join(repoRoot, "deployments", "testnet-plan.json");

const available = [
  {
    name: "upto-session",
    alias: "lumenbazaar-upto-session-testnet",
    wasm: "target/wasm32v1-none/release/upto_session.wasm",
  },
  {
    name: "test-token",
    alias: "lumenbazaar-test-token-testnet",
    wasm: "target/wasm32v1-none/release/test_token.wasm",
  },
  {
    name: "policy-wallet-example",
    alias: "lumenbazaar-policy-wallet-testnet",
    wasm: "target/wasm32v1-none/release/policy_wallet_example.wasm",
  },
];
const selected = selectContracts();

// Fail early, before any build or deployment work, with a category-specific report.
const preflight = await runPreflight({ dryRun, contracts: selected });
if (!preflight.success) {
  console.error(formatPreflightReport(preflight));
  process.exit(1);
}
const reviewedHashes = expectedWasmHashes();

if (!dryRun) {
  requireTestnet();
  requireCleanContractSources();
}

buildContracts({ dryRun });
const plannedContracts = dryRun
  ? selected.map((contract) => ({ ...contract, artifactSha256: reviewedHashes[contract.name], hashSource: "LUMENBAZAAR_EXPECTED_WASM_HASHES" }))
  : verifyExpectedArtifacts(selected, reviewedHashes, repoRoot);
for (const contract of plannedContracts) {
  if (!contract.artifactSha256) throw new Error(`Missing reviewed WASM hash for ${contract.name}`);
}
const rpcUrl = process.env.LUMENBAZAAR_RPC_URL || "https://soroban-testnet.stellar.org";
const ledgerGuard = dryRun ? {
  primary: rpcUrl,
  trusted: process.env.LUMENBAZAAR_TRUSTED_RPC_URL || "required-before-deployment",
  maxLag: Number(process.env.LUMENBAZAAR_MAX_RPC_LEDGER_LAG || 2),
} : await checkLedgerFreshness({
  primaryUrl: rpcUrl,
  trustedUrl: process.env.LUMENBAZAAR_TRUSTED_RPC_URL,
  maxLag: Number(process.env.LUMENBAZAAR_MAX_RPC_LEDGER_LAG || 2),
});
const plan = createDeploymentPlan({
  contracts: plannedContracts,
  deployer: process.env.LUMENBAZAAR_DEPLOYER_PUBLIC_KEY || "testnet-deployer",
  initialization: ["test-token", "upto-session", "policy-wallet-example"],
  expectedWrites: ["deployments/testnet.json", "network transaction receipts"],
  ledgerGuard,
});
writeDeploymentPlan(planPath, plan);
console.log(`deployment plan ${plan.planSha256}: ${planPath}`);
if (dryRun) process.exit(0);

const source = sourceAccount("testnet-deployer");

const contracts = {};
for (const contract of plannedContracts) {
  const artifactSha256 = contract.artifactSha256;
  const interfaceJson = runStellar(
    ["contract", "info", "interface", "--wasm", contract.wasm, "--output", "json"],
    { capture: true, dryRun },
  );
  const upload = uploadContract({ ...contract, source, dryRun });
  if (upload.skipped) {
    // The CLI skips WASM that another account already installed. Public evidence needs an upload
    // signed by this deployer, so submit the install transaction explicitly.
    const receipt = await buildSimulateAndSubmit(
      ["contract", "upload", "--wasm", contract.wasm, "--source-account", source, "--network", "testnet", "--build-only"],
      source,
    );
    if (receipt.status !== "SUCCESS") {
      throw new Error(`Explicit upload of ${contract.name} failed on ledger.`);
    }
    upload.transactionHash = receipt.transactionHash;
    upload.wasmHash = artifactSha256;
    console.log(`${contract.name}: WASM was already installed; recorded an explicit upload by the deployer`);
  }
  if (!dryRun && upload.wasmHash !== artifactSha256) {
    throw new Error(`Uploaded WASM hash for ${contract.name} differs from the local artifact.`);
  }
  const deployed = deployUploadedContract({ ...contract, wasmHash: upload.wasmHash, source, dryRun });

  const entry = {
    name: contract.name,
    alias: contract.alias,
    network: "testnet",
    deploymentPlanSha256: plan.planSha256,
    ledgerGuard,
    wasm: contract.wasm,
    contractId: deployed.contractId,
    artifactSha256,
    wasmHash: upload.wasmHash,
    interfaceSha256: dryRun ? "dry-run" : sha256(interfaceJson),
    upload: { transactionHash: upload.transactionHash, wasmHash: upload.wasmHash },
    deploy: { transactionHash: deployed.transactionHash },
  };
  if (!dryRun) {
    await recordReceipt(upload.transactionHash);
    await recordReceipt(deployed.transactionHash);
    if (contract.name === "test-token") {
      entry.asset = readAssetMetadata(deployed.contractId);
    }
  }
  contracts[contract.name] = entry;
  console.log(`deployed ${contract.name}: ${deployed.contractId}`);
}

// A deployment run starts a new local manifest so evidence from older runs cannot be mixed in.
writeManifest(
  manifestPath,
  {
    network: "testnet",
    sourceCommit: dryRun ? "dry-run" : git(["rev-parse", "HEAD"]),
    deployer: dryRun ? source : publicKey(source),
    toolchain: dryRun ? {} : toolchainVersions(),
    contracts,
  },
  { dryRun },
);

function selectContracts() {
  const index = process.argv.indexOf("--contracts");
  if (index === -1) {
    return available;
  }
  const names = (process.argv[index + 1] ?? "").split(",").filter(Boolean);
  const unknown = names.filter((name) => !available.some((contract) => contract.name === name));
  if (names.length === 0 || unknown.length > 0) {
    throw new Error(`--contracts accepts a comma-separated subset of: ${available.map((c) => c.name).join(", ")}`);
  }
  return available.filter((contract) => names.includes(contract.name));
}

function requireCleanContractSources() {
  const changes = git(["status", "--porcelain", "--", "contracts", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]);
  if (changes) {
    throw new Error("Contract sources differ from HEAD; public evidence must be built from the recorded commit.");
  }
}

function toolchainVersions() {
  const stellar = runStellar(["--version"], { capture: true });
  const lock = readFileSync(join(repoRoot, "Cargo.lock"), "utf8");
  return {
    stellarCli: stellar.match(/^stellar (\S+)/m)?.[1],
    stellarXdr: stellar.match(/^stellar-xdr (\S+)/m)?.[1],
    rustc: run("rustc", ["--version"]).match(/^rustc (\S+)/)?.[1],
    sorobanSdk: lock.match(/name = "soroban-sdk"\r?\nversion = "([^"]+)"/)?.[1],
    target: "wasm32v1-none",
  };
}

function readAssetMetadata(contractId) {
  const read = (method) =>
    JSON.parse(
      invokeContractTransaction({
        contractId,
        source,
        network: "testnet",
        args: [method],
        send: "no",
      }).stdout,
    );
  return { code: read("symbol"), decimals: read("decimals"), purpose: "Custom test-only token; not USDC" };
}

function git(args) {
  return run("git", args);
}

function run(command, args) {
  return execFileSync(command, args, { cwd: repoRoot, encoding: "utf8" }).trim();
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}
