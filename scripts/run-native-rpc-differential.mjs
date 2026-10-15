import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const fixturePath = join(
  repoRoot,
  "artifacts",
  "resource-usage",
  "native-rpc-differential-fixtures.json",
);
const fixtures = JSON.parse(readFileSync(fixturePath, "utf8"));
const nativeOnly = process.argv.includes("--native-only");
const outputOption = optionValue("--output");
const reportPath = outputOption
  ? isAbsolute(outputOption)
    ? outputOption
    : resolve(repoRoot, outputOption)
  : null;

validateFixtureDocument(fixtures);
const native = runNativeFixtures();
const report = {
  schemaVersion: 1,
  generatedAt: new Date().toISOString(),
  mode: nativeOnly ? "native-only" : "native-and-rpc",
  status: "running",
  fixtureSource: "artifacts/resource-usage/native-rpc-differential-fixtures.json",
  native,
  rpc: null,
  comparisons: [],
};

try {
  validateNativeResults(native);
  if (nativeOnly) {
    report.status = "passed";
  } else {
    report.rpc = await runRpcFixtures();
    report.comparisons = compareResults(native, report.rpc);
    const regressions = report.comparisons.flatMap((entry) =>
      entry.differences.filter((difference) => difference.classification === "regression"),
    );
    report.status = regressions.length === 0 ? "passed" : "regression";
    if (regressions.length > 0) {
      throw new Error(`${regressions.length} native/RPC semantic difference(s) require review.`);
    }
  }
} catch (error) {
  if (error instanceof RpcUnavailableError) {
    report.status = "outage";
    report.outage = error.message;
    writeReport();
    console.error(`RPC_OUTAGE: ${error.message}`);
    process.exit(2);
  }
  report.status = report.status === "regression" ? report.status : "failed";
  report.failure = error instanceof Error ? error.message : String(error);
  writeReport();
  throw error;
}

writeReport();
for (const entry of native) {
  console.log(
    `native ${entry.name}: ${entry.outcome}/${entry.result}, auth=${entry.authorizationEntries}, ` +
      `events=${entry.contractEventCount}, instructions=${entry.resources.instructions}`,
  );
}
if (report.rpc) {
  for (const entry of report.rpc.fixtures) {
    console.log(
      `rpc ${entry.name}: ${entry.outcome}/${entry.result}, auth=${entry.authorizationEntries}, ` +
        `events=${entry.contractEventCount}, instructions=${entry.resources?.instructions ?? "unavailable"}`,
    );
  }
}
console.log(`native/RPC differential ${report.status}`);

function runNativeFixtures() {
  const output = execFileSync(
    "cargo",
    [
      "test",
      "-p",
      "upto-session",
      "native_rpc_differential_",
      "--",
      "--nocapture",
      "--test-threads=1",
    ],
    {
      cwd: repoRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      maxBuffer: 16 * 1024 * 1024,
    },
  );
  return output
    .split(/\r?\n/)
    .map((line) => line.match(/NATIVE_RPC_DIFFERENTIAL_JSON (\{.*\})/))
    .filter(Boolean)
    .map((match) => JSON.parse(match[1]))
    .sort((left, right) => left.name.localeCompare(right.name));
}

function validateNativeResults(nativeResults) {
  if (nativeResults.length !== fixtures.fixtures.length) {
    throw new Error(
      `Expected ${fixtures.fixtures.length} native fixtures, received ${nativeResults.length}.`,
    );
  }
  for (const fixture of fixtures.fixtures) {
    const actual = nativeResults.find((entry) => entry.name === fixture.name);
    compareSemantic(fixture.name, fixture.expected, actual, "native");
    for (const [name, value] of Object.entries(actual.resources)) {
      if (!Number.isFinite(value) || value < 0) {
        throw new Error(`Native fixture ${fixture.name} has invalid ${name}.`);
      }
    }
  }
}

async function runRpcFixtures() {
  const latestLedger = await rpcCall("getLatestLedger");
  if (!Number.isInteger(latestLedger.sequence)) {
    throw new RpcUnavailableError("getLatestLedger returned no integer sequence.");
  }
  const results = [];
  for (const fixture of fixtures.fixtures) {
    const args = fixture.args.map((value) =>
      value === "${LATEST_PLUS_100}" ? String(latestLedger.sequence + 100) : value,
    );
    const transaction = buildUnsignedTransaction(fixture, args);
    const simulation = await rpcCall("simulateTransaction", { transaction });
    results.push(decodeSimulation(fixture, simulation));
  }
  return {
    network: fixtures.rpc.network,
    rpcUrl: fixtures.rpc.rpcUrl,
    latestLedger: latestLedger.sequence,
    simulationOnly: true,
    signingSecretUsed: false,
    fixtures: results.sort((left, right) => left.name.localeCompare(right.name)),
  };
}

function buildUnsignedTransaction(fixture, args) {
  try {
    return execFileSync(
      "stellar",
      [
        "contract",
        "invoke",
        "--id",
        fixtures.rpc.contractId,
        "--source-account",
        fixture.sourceAccount,
        "--network",
        fixtures.rpc.network,
        "--build-only",
        "--",
        fixture.method,
        ...args,
      ],
      {
        cwd: repoRoot,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      },
    ).trim();
  } catch (error) {
    throw new Error(`Could not build unsigned ${fixture.name} transaction: ${commandError(error)}`);
  }
}

function decodeSimulation(fixture, simulation) {
  const contractEvents = (simulation.events || [])
    .map((event) => decodeXdr("DiagnosticEvent", event))
    .filter((event) => event.in_successful_contract_call && event.event?.type === "contract");
  const contractEventNames = contractEvents
    .map((event) => event.event.body?.v0?.topics?.[0]?.symbol)
    .filter(Boolean);
  const authorization = simulation.results?.[0]?.auth || [];
  const errorCode = simulation.error?.match(/Error\(Contract, #(\d+)\)/)?.[1];
  let outcome;
  let result;
  if (simulation.error) {
    outcome = "error";
    result = errorCode ? `ContractError#${errorCode}` : "RpcSimulationError";
  } else {
    outcome = "success";
    result = fixture.name === "create_session"
      ? "Created"
      : sessionStatus(decodeXdr("ScVal", simulation.results?.[0]?.xdr));
  }
  const transactionData = simulation.transactionData
    ? decodeXdr("SorobanTransactionData", simulation.transactionData)
    : null;
  const rpcResources = transactionData?.resources;
  return {
    name: fixture.name,
    outcome,
    result,
    authorizationEntries: authorization.length,
    authorizationXdrSha256: sha256Json(authorization),
    contractEventCount: contractEvents.length,
    contractEventNames,
    eventXdrSha256: sha256Json(simulation.events || []),
    latestLedger: simulation.latestLedger,
    resources: rpcResources
      ? {
          instructions: rpcResources.instructions,
          diskReadBytes: rpcResources.disk_read_bytes,
          writeBytes: rpcResources.write_bytes,
          readOnlyEntries: rpcResources.footprint?.read_only?.length || 0,
          readWriteEntries: rpcResources.footprint?.read_write?.length || 0,
          minResourceFeeStroops: Number(simulation.minResourceFee),
        }
      : null,
  };
}

function compareResults(nativeResults, rpcResults) {
  return fixtures.fixtures.map((fixture) => {
    const native = nativeResults.find((entry) => entry.name === fixture.name);
    const rpc = rpcResults.fixtures.find((entry) => entry.name === fixture.name);
    const differences = [];
    for (const field of ["outcome", "result", "authorizationEntries", "contractEventCount"]) {
      differences.push({
        field,
        native: native[field],
        rpc: rpc[field],
        classification: native[field] === rpc[field] ? "match" : "regression",
      });
    }
    differences.push({
      field: "contractEventNames",
      native: fixture.expected.contractEventNames,
      rpc: rpc.contractEventNames,
      classification:
        JSON.stringify(fixture.expected.contractEventNames) === JSON.stringify(rpc.contractEventNames)
          ? "match"
          : "regression",
    });
    if (rpc.resources) {
      differences.push({
        field: "resources.instructions",
        native: native.resources.instructions,
        rpc: rpc.resources.instructions,
        absoluteDelta: rpc.resources.instructions - native.resources.instructions,
        percentageDelta: percentageDelta(
          native.resources.instructions,
          rpc.resources.instructions,
        ),
        classification: "expected_environmental",
        reason: "Native SDK fixtures use fresh in-memory state; RPC uses public ledger state and restoration costs.",
      });
      differences.push({
        field: "resources.fee",
        native: native.resources.estimatedFeeStroops,
        rpc: rpc.resources.minResourceFeeStroops,
        absoluteDelta:
          rpc.resources.minResourceFeeStroops - native.resources.estimatedFeeStroops,
        percentageDelta: percentageDelta(
          native.resources.estimatedFeeStroops,
          rpc.resources.minResourceFeeStroops,
        ),
        classification: "expected_environmental",
        reason: "RPC fees include target-network ledger footprint and rent state.",
      });
    } else {
      differences.push({
        field: "resources",
        native: native.resources,
        rpc: null,
        classification: "expected_early_rejection",
        reason: "The RPC fixture rejects before a transaction resource envelope is produced.",
      });
    }
    return { name: fixture.name, differences };
  });
}

function compareSemantic(name, expected, actual, source) {
  if (!actual) throw new Error(`Missing ${source} fixture ${name}.`);
  for (const field of ["outcome", "result", "authorizationEntries", "contractEventCount"]) {
    if (actual[field] !== expected[field]) {
      throw new Error(
        `${source} fixture ${name} ${field}: expected ${expected[field]}, received ${actual[field]}.`,
      );
    }
  }
}

function sessionStatus(value) {
  const status = value?.map?.find((entry) => entry.key?.symbol === "status")?.val?.vec?.[0]?.symbol;
  if (!status) throw new Error("RPC get_session result did not contain a status.");
  return status;
}

function decodeXdr(type, value) {
  if (!value) throw new Error(`Missing ${type} XDR.`);
  try {
    return JSON.parse(
      execFileSync(
        "stellar",
        ["xdr", "decode", "--type", type, "--input", "single-base64", "--output", "json", value],
        { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
      ),
    );
  } catch (error) {
    throw new Error(`Could not decode ${type}: ${commandError(error)}`);
  }
}

async function rpcCall(method, params = undefined) {
  let response;
  try {
    response = await fetch(fixtures.rpc.rpcUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, ...(params ? { params } : {}) }),
      signal: AbortSignal.timeout(30_000),
    });
  } catch (error) {
    throw new RpcUnavailableError(`${method} request failed: ${error.message}`);
  }
  if (!response.ok) {
    throw new RpcUnavailableError(`${method} returned HTTP ${response.status}.`);
  }
  const payload = await response.json();
  if (payload.error || !payload.result) {
    throw new RpcUnavailableError(`${method} returned no result: ${JSON.stringify(payload.error)}`);
  }
  return payload.result;
}

function validateFixtureDocument(value) {
  if (
    value.schemaVersion !== 1 ||
    !value.rpc?.simulationOnly ||
    value.rpc.requiresSigningSecret !== false ||
    value.rpc.stellarCli !== "28.0.0" ||
    !Array.isArray(value.fixtures) ||
    value.fixtures.length !== 5
  ) {
    throw new Error("Invalid native/RPC fixture document.");
  }
  const names = new Set();
  for (const fixture of value.fixtures) {
    if (!fixture.name || names.has(fixture.name) || !fixture.expected) {
      throw new Error(`Invalid or duplicate differential fixture: ${fixture.name}`);
    }
    names.add(fixture.name);
  }
}

function writeReport() {
  if (!reportPath) return;
  mkdirSync(dirname(reportPath), { recursive: true });
  writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  console.log(`Differential report written to ${reportPath}`);
}

function percentageDelta(nativeValue, rpcValue) {
  if (nativeValue === 0) return null;
  return Number((((rpcValue - nativeValue) / nativeValue) * 100).toFixed(2));
}

function sha256Json(value) {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}

function commandError(error) {
  return error?.stderr?.toString().trim() || error?.message || String(error);
}

function optionValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
  return value;
}

class RpcUnavailableError extends Error {}
