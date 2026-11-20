import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { redactText } from "./deployment-logging.mjs";
import { repoRoot as defaultRepoRoot, runStellar } from "./stellar-cli.mjs";

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

export const PREFLIGHT_CATEGORIES = {
  CONFIGURATION: "configuration",
  NETWORK: "network",
  FUNDING: "funding",
  ARTIFACTS: "artifacts",
};

export const defaultContracts = [
  { name: "upto-session", alias: "lumenbazaar-upto-session-testnet", wasm: "target/wasm32v1-none/release/upto_session.wasm" },
  { name: "test-token", alias: "lumenbazaar-test-token-testnet", wasm: "target/wasm32v1-none/release/test_token.wasm" },
  { name: "policy-wallet-example", alias: "lumenbazaar-policy-wallet-testnet", wasm: "target/wasm32v1-none/release/policy_wallet_example.wasm" },
];

const TESTNET_PASSPHRASE = "Test SDF Network ; September 2015";
const DEFAULT_RPC = "https://soroban-testnet.stellar.org";
const DEFAULT_HORIZON = "https://horizon-testnet.stellar.org";
const DEFAULT_MIN_XLM = "10";
const ADDRESS = /^G[A-Z2-7]{55}$/;
const SYNTHETIC_ADDRESS = `G${"A".repeat(55)}`;
const STROOPS = 10_000_000n;

function parseXlm(value) {
  const text = String(value).trim();
  if (!/^\d+(\.\d{1,7})?$/.test(text)) throw new Error("must be a non-negative decimal XLM amount");
  const [whole, frac = ""] = text.split(".");
  return BigInt(whole) * STROOPS + BigInt(frac.padEnd(7, "0"));
}

function formatXlm(stroops) {
  return `${stroops / STROOPS}.${String(stroops % STROOPS).padStart(7, "0")}`;
}

export async function runPreflight({
  env = process.env,
  dryRun = false,
  repoRoot = defaultRepoRoot,
  contracts = defaultContracts,
  fetchImpl = fetch,
  runStellarImpl = runStellar,
} = {}) {
  const results = [];
  const failures = [];
  const secrets = [env.STELLAR_ACCOUNT, env.LUMENBAZAAR_SOURCE_ACCOUNT].filter(Boolean);
  const clean = (text) => redactText(text, secrets);
  const fail = (category, message) => failures.push({ category, message: clean(message) });
  const record = (category, ok, detail) => results.push({ category, ok, ...detail });

  // a. CONFIGURATION
  const cfgFailures = [];
  const cfgFail = (message) => { cfgFailures.push(message); fail(PREFLIGHT_CATEGORIES.CONFIGURATION, message); };
  const network = env.STELLAR_NETWORK || (dryRun ? "testnet" : "");
  if (network !== "testnet") cfgFail(`STELLAR_NETWORK must be "testnet" (got "${network || "unset"}")`);
  const source = env.LUMENBAZAAR_SOURCE_ACCOUNT || env.STELLAR_ACCOUNT || (dryRun ? SYNTHETIC_ADDRESS : "");
  if (!source) cfgFail("Set LUMENBAZAAR_SOURCE_ACCOUNT or STELLAR_ACCOUNT");

  let hashes = {};
  const rawHashes = env.LUMENBAZAAR_EXPECTED_WASM_HASHES;
  if (!rawHashes && dryRun) {
    hashes = Object.fromEntries(contracts.map((c) => [c.name, "0".repeat(64)]));
  } else {
    try {
      const parsed = JSON.parse(rawHashes || "");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("not an object");
      hashes = parsed;
      for (const contract of contracts) {
        if (!HASH.test(hashes[contract.name] ?? "")) {
          cfgFail(`LUMENBAZAAR_EXPECTED_WASM_HASHES needs a lowercase SHA-256 for ${contract.name}`);
        }
      }
    } catch {
      cfgFail("LUMENBAZAAR_EXPECTED_WASM_HASHES must be a JSON object of lowercase SHA-256 hashes");
    }
  }

  const primaryUrl = env.LUMENBAZAAR_RPC_URL || DEFAULT_RPC;
  const trustedUrl = env.LUMENBAZAAR_TRUSTED_RPC_URL || (dryRun ? "https://trusted-rpc.invalid" : "");
  if (!trustedUrl) cfgFail("LUMENBAZAAR_TRUSTED_RPC_URL must be configured");
  else if (!/^https:\/\//.test(trustedUrl)) cfgFail("LUMENBAZAAR_TRUSTED_RPC_URL must be an HTTPS endpoint");
  else if (trustedUrl === primaryUrl) cfgFail("LUMENBAZAAR_TRUSTED_RPC_URL must differ from the primary RPC URL");

  let minStroops = parseXlm(DEFAULT_MIN_XLM);
  if (env.LUMENBAZAAR_MIN_SOURCE_BALANCE_XLM) {
    try { minStroops = parseXlm(env.LUMENBAZAAR_MIN_SOURCE_BALANCE_XLM); }
    catch (err) { cfgFail(`LUMENBAZAAR_MIN_SOURCE_BALANCE_XLM ${err.message}`); }
  }
  const maxLag = Number(env.LUMENBAZAAR_MAX_RPC_LEDGER_LAG || 2);
  record(PREFLIGHT_CATEGORIES.CONFIGURATION, cfgFailures.length === 0, {
    detail: cfgFailures.length ? cfgFailures.map(clean) : ["configuration valid"],
    ...(dryRun && { synthetic: true }),
  });

  // b. NETWORK
  if (dryRun) {
    record(PREFLIGHT_CATEGORIES.NETWORK, true, {
      network: "testnet", passphrase: TESTNET_PASSPHRASE, primaryLedger: 5000000, trustedLedger: 5000000, lag: 0, synthetic: true,
    });
  } else if (cfgFailures.length === 0) {
    try {
      const networkInfo = await rpcCall(primaryUrl, "getNetwork", fetchImpl);
      if (networkInfo?.passphrase !== TESTNET_PASSPHRASE) throw new Error("RPC reports a network passphrase other than testnet");
      const freshness = await checkLedgerFreshness({ primaryUrl, trustedUrl, maxLag, fetchImpl });
      record(PREFLIGHT_CATEGORIES.NETWORK, true, { network: "testnet", passphrase: networkInfo.passphrase, ...freshness });
    } catch (err) {
      fail(PREFLIGHT_CATEGORIES.NETWORK, err.message);
      record(PREFLIGHT_CATEGORIES.NETWORK, false, { detail: [clean(err.message)] });
    }
  } else {
    record(PREFLIGHT_CATEGORIES.NETWORK, false, { skipped: true, detail: ["skipped: configuration invalid"] });
  }

  // c. FUNDING (read-only)
  if (dryRun) {
    record(PREFLIGHT_CATEGORIES.FUNDING, true, {
      sourceAccount: ADDRESS.test(source) ? source : SYNTHETIC_ADDRESS, funded: true, balanceXlm: "10000.0000000", synthetic: true,
    });
  } else if (cfgFailures.length === 0) {
    try {
      const address = ADDRESS.test(source)
        ? source
        : String(runStellarImpl(["keys", "public-key", source], { capture: true })).trim();
      if (!ADDRESS.test(address)) throw new Error("Could not resolve the source account to a public address");
      const horizon = env.LUMENBAZAAR_HORIZON_URL || DEFAULT_HORIZON;
      const response = await fetchImpl(`${horizon}/accounts/${address}`);
      if (response.status === 404) throw new Error(`Source account ${address} does not exist on testnet; fund it with friendbot first`);
      if (!response.ok) throw new Error(`Horizon account lookup failed with status ${response.status}`);
      const body = await response.json();
      const native = (body.balances || []).find((b) => b.asset_type === "native");
      if (!native) throw new Error(`Source account ${address} has no native XLM balance`);
      const balance = parseXlm(native.balance);
      if (balance < minStroops) {
        throw new Error(`Source account ${address} balance ${formatXlm(balance)} XLM is below minimum ${formatXlm(minStroops)} XLM`);
      }
      record(PREFLIGHT_CATEGORIES.FUNDING, true, { sourceAccount: address, funded: true, balanceXlm: formatXlm(balance) });
    } catch (err) {
      fail(PREFLIGHT_CATEGORIES.FUNDING, err.message);
      record(PREFLIGHT_CATEGORIES.FUNDING, false, { detail: [clean(err.message)] });
    }
  } else {
    record(PREFLIGHT_CATEGORIES.FUNDING, false, { skipped: true, detail: ["skipped: configuration invalid"] });
  }

  // d. ARTIFACTS
  if (dryRun) {
    record(PREFLIGHT_CATEGORIES.ARTIFACTS, true, {
      synthetic: true,
      artifacts: contracts.map((c) => ({ name: c.name, expectedSha256: hashes[c.name], verified: true })),
    });
  } else if (cfgFailures.length === 0) {
    const problems = [];
    const artifacts = [];
    for (const contract of contracts) {
      const path = join(repoRoot, contract.wasm);
      if (!existsSync(path)) { problems.push(`WASM artifact missing for ${contract.name}: ${contract.wasm}`); continue; }
      const actual = createHash("sha256").update(readFileSync(path)).digest("hex");
      if (actual !== hashes[contract.name]) {
        problems.push(`WASM hash mismatch for ${contract.name}: expected ${hashes[contract.name]}, got ${actual}`);
      } else artifacts.push({ name: contract.name, expectedSha256: actual, verified: true });
    }
    problems.forEach((p) => fail(PREFLIGHT_CATEGORIES.ARTIFACTS, p));
    record(PREFLIGHT_CATEGORIES.ARTIFACTS, problems.length === 0, { artifacts, ...(problems.length && { detail: problems.map(clean) }) });
  } else {
    record(PREFLIGHT_CATEGORIES.ARTIFACTS, false, { skipped: true, detail: ["skipped: configuration invalid"] });
  }

  return { success: failures.length === 0, results, failures, synthetic: dryRun };
}

async function rpcCall(url, method, fetchImpl) {
  if (!/^https:\/\//.test(url)) throw new Error("RPC endpoint must use HTTPS");
  let response;
  try {
    response = await fetchImpl(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method }),
    });
  } catch (err) {
    throw new Error(`Primary RPC unreachable: ${err.message}`);
  }
  const payload = await response.json().catch(() => ({}));
  if (!response.ok || !payload?.result) throw new Error(`Primary RPC ${method} returned no result (status ${response.status})`);
  return payload.result;
}

export function formatPreflightReport(report, { env = process.env } = {}) {
  const secrets = [env.STELLAR_ACCOUNT, env.LUMENBAZAAR_SOURCE_ACCOUNT].filter(Boolean);
  const lines = [`Deployment preflight${report.synthetic ? " (dry-run, synthetic results)" : ""}: ${report.success ? "PASSED" : "FAILED"}`];
  for (const result of report.results) {
    const { category, ok, detail, artifacts, ...rest } = result;
    lines.push(`[${category.toUpperCase()}] ${ok ? "ok" : result.skipped ? "skipped" : "FAILED"}`);
    for (const [key, value] of Object.entries(rest)) if (key !== "skipped") lines.push(`  ${key}: ${value}`);
    for (const a of artifacts || []) lines.push(`  artifact ${a.name}: ${a.expectedSha256}`);
    for (const d of detail || []) lines.push(`  ${d}`);
  }
  for (const f of report.failures) lines.push(`[${f.category.toUpperCase()}] error: ${f.message}`);
  return redactText(lines.join("\n"), secrets);
}