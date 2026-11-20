import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const HASH = /^[0-9a-f]{64}$/;

export function expectedWasmHashes(environment = process.env) {
  let parsed;
  try {
    parsed = JSON.parse(environment.LUMENBAZAAR_EXPECTED_WASM_HASHES || "");
  } catch {
    throw new Error("LUMENBAZAAR_EXPECTED_WASM_HASHES must be a JSON object of reviewed SHA-256 hashes");
  }
  for (const [name, hash] of Object.entries(parsed || {})) {
    if (!HASH.test(hash)) throw new Error(`Expected WASM hash for ${name} is not lowercase SHA-256`);
  }
  return parsed;
}

export function verifyExpectedArtifacts(contracts, expected, repoRoot) {
  return contracts.map((contract) => {
    const reviewed = expected[contract.name];
    if (!reviewed) throw new Error(`Missing reviewed WASM hash for ${contract.name}`);
    const actual = createHash("sha256").update(readFileSync(join(repoRoot, contract.wasm))).digest("hex");
    if (actual !== reviewed) throw new Error(`WASM hash mismatch for ${contract.name}: expected ${reviewed}, got ${actual}`);
    return { ...contract, artifactSha256: actual, hashSource: "LUMENBAZAAR_EXPECTED_WASM_HASHES" };
  });
}

export function createDeploymentPlan({ contracts, deployer, initialization, expectedWrites, ledgerGuard }) {
  const plan = {
    schemaVersion: 1,
    network: "testnet",
    deployer,
    contracts: contracts.map(({ name, alias, wasm, artifactSha256, hashSource }) => ({
      name, alias, artifactPath: wasm, expectedWasmSha256: artifactSha256, hashSource,
    })),
    initialization,
    expectedWrites,
    ledgerGuard,
  };
  const canonical = canonicalJson(plan);
  return { ...plan, planSha256: createHash("sha256").update(canonical).digest("hex") };
}

export function writeDeploymentPlan(path, plan) {
  writeFileSync(path, `${JSON.stringify(plan, null, 2)}\n`, { mode: 0o600 });
}

export async function checkLedgerFreshness({ primaryUrl, trustedUrl, maxLag, fetchImpl = fetch }) {
  if (!trustedUrl || trustedUrl === primaryUrl) throw new Error("LUMENBAZAAR_TRUSTED_RPC_URL must be an independent HTTPS endpoint");
  if (!Number.isSafeInteger(maxLag) || maxLag < 0) throw new Error("LUMENBAZAAR_MAX_RPC_LEDGER_LAG must be a non-negative integer");
  const [primaryLedger, trustedLedger] = await Promise.all([
    latestLedger(primaryUrl, fetchImpl), latestLedger(trustedUrl, fetchImpl),
  ]);
  const lag = Math.max(0, trustedLedger - primaryLedger);
  if (lag > maxLag) throw new Error(`Deployment RPC is ${lag} ledgers behind trusted reference; maximum is ${maxLag}`);
  return { primaryLedger, trustedLedger, lag, maxLag };
}

async function latestLedger(url, fetchImpl) {
  if (!/^https:\/\//.test(url)) throw new Error("Ledger freshness endpoints must use HTTPS");
  const response = await fetchImpl(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getLatestLedger" }),
  });
  const payload = await response.json();
  const sequence = Number(payload?.result?.sequence);
  if (!response.ok || !Number.isSafeInteger(sequence) || sequence <= 0) throw new Error("RPC returned no valid latest ledger");
  return sequence;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}
