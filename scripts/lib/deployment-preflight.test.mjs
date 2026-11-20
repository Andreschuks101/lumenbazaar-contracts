import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { formatPreflightReport, runPreflight } from "./deployment-preflight.mjs";

const PASSPHRASE = "Test SDF Network ; September 2015";
const ADDRESS = `G${"B".repeat(55)}`;
const PRIMARY = "https://primary.example";
const TRUSTED = "https://trusted.example";
const WASM = Buffer.from("fake wasm");
const HASH = createHash("sha256").update(WASM).digest("hex");

function fixture({ writeWasm = true, content = WASM } = {}) {
  const repoRoot = mkdtempSync(join(tmpdir(), "preflight-"));
  const contracts = [{ name: "demo", wasm: "target/wasm32v1-none/release/demo.wasm" }];
  if (writeWasm) {
    mkdirSync(join(repoRoot, "target/wasm32v1-none/release"), { recursive: true });
    writeFileSync(join(repoRoot, contracts[0].wasm), content);
  }
  return { repoRoot, contracts };
}

const baseEnv = () => ({
  STELLAR_NETWORK: "testnet",
  LUMENBAZAAR_SOURCE_ACCOUNT: ADDRESS,
  LUMENBAZAAR_RPC_URL: PRIMARY,
  LUMENBAZAAR_TRUSTED_RPC_URL: TRUSTED,
  LUMENBAZAAR_EXPECTED_WASM_HASHES: JSON.stringify({ demo: HASH }),
});

function mockFetch({ primary = 5000000, trusted = 5000000, account = "100.0000000", down = false, status } = {}) {
  return async (url, init) => {
    const json = (body, code = 200) => ({ ok: code < 400, status: code, json: async () => body });
    if (url.startsWith(PRIMARY) && down) throw new Error("connect ECONNREFUSED");
    if (url.includes("/accounts/")) {
      if (status) return json({}, status);
      return json({ balances: [{ asset_type: "native", balance: account }] });
    }
    const method = JSON.parse(init.body).method;
    const ledger = url.startsWith(PRIMARY) ? primary : trusted;
    if (method === "getNetwork") return json({ result: { passphrase: PASSPHRASE } });
    return json({ result: { sequence: ledger } });
  };
}

const categories = (report) => report.failures.map((f) => f.category);

test("passes when every prerequisite is satisfied", async () => {
  const { repoRoot, contracts } = fixture();
  const report = await runPreflight({ env: baseEnv(), repoRoot, contracts, fetchImpl: mockFetch() });
  assert.equal(report.success, true, JSON.stringify(report.failures));
  assert.equal(report.synthetic, false);
});

test("configuration failures are categorized", async () => {
  const { repoRoot, contracts } = fixture();
  for (const [name, env] of [
    ["missing source", { ...baseEnv(), LUMENBAZAAR_SOURCE_ACCOUNT: undefined }],
    ["wrong network", { ...baseEnv(), STELLAR_NETWORK: "mainnet" }],
    ["invalid hash json", { ...baseEnv(), LUMENBAZAAR_EXPECTED_WASM_HASHES: "{nope" }],
    ["bad hash value", { ...baseEnv(), LUMENBAZAAR_EXPECTED_WASM_HASHES: JSON.stringify({ demo: "ABC" }) }],
    ["missing trusted rpc", { ...baseEnv(), LUMENBAZAAR_TRUSTED_RPC_URL: undefined }],
    ["trusted equals primary", { ...baseEnv(), LUMENBAZAAR_TRUSTED_RPC_URL: PRIMARY }],
  ]) {
    const report = await runPreflight({ env: env, repoRoot, contracts, fetchImpl: mockFetch() });
    assert.equal(report.success, false, name);
    assert.deepEqual([...new Set(categories(report))], ["configuration"], name);
  }
});

test("network failures are categorized", async () => {
  const { repoRoot, contracts } = fixture();
  const down = await runPreflight({ env: baseEnv(), repoRoot, contracts, fetchImpl: mockFetch({ down: true }) });
  assert.deepEqual(categories(down), ["network"]);
  const lag = await runPreflight({ env: baseEnv(), repoRoot, contracts, fetchImpl: mockFetch({ primary: 4999990 }) });
  assert.deepEqual(categories(lag), ["network"]);
  assert.match(lag.failures[0].message, /behind trusted/);
});

test("funding failures are categorized", async () => {
  const { repoRoot, contracts } = fixture();
  const missing = await runPreflight({ env: baseEnv(), repoRoot, contracts, fetchImpl: mockFetch({ status: 404 }) });
  assert.deepEqual(categories(missing), ["funding"]);
  const low = await runPreflight({ env: baseEnv(), repoRoot, contracts, fetchImpl: mockFetch({ account: "9.9999999" }) });
  assert.deepEqual(categories(low), ["funding"]);
  const custom = await runPreflight({
    env: { ...baseEnv(), LUMENBAZAAR_MIN_SOURCE_BALANCE_XLM: "5" }, repoRoot, contracts, fetchImpl: mockFetch({ account: "9.9999999" }),
  });
  assert.equal(custom.success, true);
});

test("funding resolves identity aliases through the stellar CLI without submitting", async () => {
  const { repoRoot, contracts } = fixture();
  const calls = [];
  const runStellarImpl = (args) => { calls.push(args); return `${ADDRESS}\n`; };
  const env = { ...baseEnv(), LUMENBAZAAR_SOURCE_ACCOUNT: "my-alias" };
  const report = await runPreflight({ env, repoRoot, contracts, fetchImpl: mockFetch(), runStellarImpl });
  assert.equal(report.success, true);
  assert.deepEqual(calls, [["keys", "public-key", "my-alias"]]);
});

test("artifact failures are categorized", async () => {
  const missing = fixture({ writeWasm: false });
  const r1 = await runPreflight({ env: baseEnv(), ...missing, fetchImpl: mockFetch() });
  assert.deepEqual(categories(r1), ["artifacts"]);
  const mismatch = fixture({ content: Buffer.from("tampered") });
  const r2 = await runPreflight({ env: baseEnv(), ...mismatch, fetchImpl: mockFetch() });
  assert.deepEqual(categories(r2), ["artifacts"]);
  assert.match(r2.failures[0].message, /mismatch/);
});

test("dry-run uses synthetic results without network or artifacts", async () => {
  const fetchImpl = async () => { throw new Error("network must not be used"); };
  const report = await runPreflight({
    env: {}, dryRun: true, repoRoot: join(tmpdir(), "does-not-exist"), fetchImpl,
    runStellarImpl: () => { throw new Error("cli must not be used"); },
  });
  assert.equal(report.success, true);
  assert.equal(report.synthetic, true);
  const byCat = Object.fromEntries(report.results.map((r) => [r.category, r]));
  assert.equal(byCat.network.primaryLedger, 5000000);
  assert.equal(byCat.network.lag, 0);
  assert.equal(byCat.network.passphrase, PASSPHRASE);
  assert.equal(byCat.funding.balanceXlm, "10000.0000000");
  assert.equal(byCat.funding.synthetic, true);
  assert.equal(byCat.artifacts.synthetic, true);
  assert.match(formatPreflightReport(report), /\[CONFIGURATION\][\s\S]*\[NETWORK\][\s\S]*\[FUNDING\][\s\S]*\[ARTIFACTS\]/);
});

test("secrets never appear in the report", async () => {
  const { repoRoot, contracts } = fixture();
  const secretSeed = `S${"A".repeat(55)}`;
  const env = {
    ...baseEnv(), LUMENBAZAAR_SOURCE_ACCOUNT: "super-secret-alias", STELLAR_PASSWORD: "hunter2-password",
    LUMENBAZAAR_EXPECTED_WASM_HASHES: `{bad ${secretSeed} super-secret-alias hunter2-password`,
  };
  const report = await runPreflight({ env, repoRoot, contracts, fetchImpl: mockFetch() });
  const text = `${formatPreflightReport(report, { env })}\n${JSON.stringify(report.failures)}`;
  assert.equal(report.success, false);
  for (const secret of [secretSeed, "super-secret-alias", "hunter2-password"]) assert.ok(!text.includes(secret), secret);

  const failing = await runPreflight({
    env: { ...baseEnv(), LUMENBAZAAR_SOURCE_ACCOUNT: "super-secret-alias" }, repoRoot, contracts,
    fetchImpl: mockFetch(), runStellarImpl: () => { throw new Error(`cannot read super-secret-alias ${secretSeed}`); },
  });
  const failText = formatPreflightReport(failing, { env: { LUMENBAZAAR_SOURCE_ACCOUNT: "super-secret-alias" } });
  assert.ok(!failText.includes("super-secret-alias") && !failText.includes(secretSeed));
  assert.match(failText, /<redacted>/);
});
