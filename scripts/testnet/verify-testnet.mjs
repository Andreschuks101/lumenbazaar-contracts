import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { hasFlag, readManifest, repoRoot, runStellar } from "../lib/stellar-cli.mjs";

const dryRun = hasFlag("--dry-run");
const passphrase = "Test SDF Network ; September 2015";
const manifestPath = process.env.LUMENBAZAAR_DEPLOYMENT_MANIFEST
  ? resolve(repoRoot, process.env.LUMENBAZAAR_DEPLOYMENT_MANIFEST)
  : join(repoRoot, "deployments", "testnet.json");
const manifest = readManifest(manifestPath);
const contracts = manifest.contracts || {};
const names = ["upto-session", "test-token", "policy-wallet-example"];
const deployed = dryRun ? names : names.filter((name) => contracts[name]);
const endpoints = parseEndpoints();
const quorum = Number(process.env.LUMENBAZAAR_VERIFY_RPC_QUORUM || endpoints.length);
if (!Number.isSafeInteger(quorum) || quorum < 1 || quorum > endpoints.length) {
  throw new Error("LUMENBAZAAR_VERIFY_RPC_QUORUM must be between one and the endpoint count");
}

const observations = [];
for (const endpoint of endpoints) {
  for (const name of deployed) {
    const contractId = contracts[name]?.contractId || `dry-run-${name}`;
    try {
      const codeHash = command(["contract", "info", "hash", "--id", contractId], endpoint);
      const interfaceJson = command(["contract", "info", "interface", "--contract-id", contractId, "--output", "json"], endpoint);
      const initialization = readInitialization(name, contractId, endpoint);
      observations.push({ endpoint, name, ok: true, codeHash, interfaceSha256: sha256(interfaceJson), initialization });
    } catch (error) {
      observations.push({ endpoint, name, ok: false, error: String(error?.message || error) });
    }
  }
}
const ledgers = dryRun ? endpoints.map((endpoint, index) => ({ endpoint, sequence: index + 1 }))
  : await Promise.all(endpoints.map(async (endpoint) => ({ endpoint, sequence: await latestLedger(endpoint) })));
const snapshot = {
  schemaVersion: 1,
  network: "testnet",
  quorum: { required: quorum, endpoints },
  ledgers,
  contracts: deployed.map((name) => ({
    name,
    contractId: contracts[name]?.contractId || `dry-run-${name}`,
    expectedCodeHash: contracts[name]?.wasmHash || contracts[name]?.artifactSha256 || "dry-run",
    observations: observations.filter((entry) => entry.name === name),
  })),
};
const snapshotPath = join(repoRoot, "artifacts", "testnet", "postcondition-snapshot.json");
mkdirSync(join(repoRoot, "artifacts", "testnet"), { recursive: true });
writeFileSync(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`);

const failures = [];
for (const contract of snapshot.contracts) {
  const matching = contract.observations.filter((entry) => entry.ok &&
    (dryRun || entry.codeHash === contract.expectedCodeHash) && entry.initialization !== null);
  if (matching.length < quorum) failures.push(`${contract.name}: ${matching.length}/${quorum} matching initialized endpoints`);
  const states = new Set(matching.map((entry) => JSON.stringify(entry.initialization)));
  if (states.size > 1) failures.push(`${contract.name}: initialized state conflicts across endpoints`);
}
if (failures.length) {
  throw new Error(`Multi-RPC verification disagreement; no redeploy attempted. Evidence: ${snapshotPath}\n${failures.join("\n")}`);
}
console.log(`verified ${deployed.length} contracts across ${endpoints.length} endpoints with quorum ${quorum}; ${snapshotPath}`);

function parseEndpoints() {
  if (dryRun) return ["https://rpc-a.example", "https://rpc-b.example"];
  const values = (process.env.LUMENBAZAAR_VERIFY_RPC_URLS || "").split(",").map((value) => value.trim()).filter(Boolean);
  if (values.length < 2 || values.length > 5 || values.some((value) => !/^https:\/\//.test(value))) {
    throw new Error("LUMENBAZAAR_VERIFY_RPC_URLS must contain two to five comma-separated HTTPS endpoints");
  }
  return [...new Set(values)];
}

function command(args, endpoint) {
  const separator = args.indexOf("--");
  const network = ["--rpc-url", endpoint, "--network-passphrase", passphrase];
  const commandArgs = separator === -1 ? [...args, ...network]
    : [...args.slice(0, separator), ...network, ...args.slice(separator)];
  return runStellar(commandArgs, { capture: true, dryRun });
}

function readInitialization(name, contractId, endpoint) {
  const methods = name === "upto-session" ? ["get_configuration"]
    : name === "test-token" ? ["symbol", "decimals"] : ["get_config"];
  return Object.fromEntries(methods.map((method) => [method, command([
    "contract", "invoke", "--id", contractId, "--", method,
  ], endpoint)]));
}

async function latestLedger(endpoint) {
  const response = await fetch(endpoint, {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getLatestLedger" }),
    signal: AbortSignal.timeout(10_000),
  });
  const body = await response.json();
  const sequence = Number(body?.result?.sequence);
  if (!response.ok || !Number.isSafeInteger(sequence)) throw new Error(`No ledger evidence from ${endpoint}`);
  return sequence;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}
