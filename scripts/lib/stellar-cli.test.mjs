import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  SUPPORTED_NETWORKS,
  deployContract,
  findContractId,
  formatCommand,
  getContractEntry,
  invokeContract,
  mergeContracts,
  networkArgs,
  quoteArg,
  readManifest,
  runStellar,
  validateNetwork,
  writeManifest,
} from "./stellar-cli.mjs";

test("command arguments are asserted as distinct arrays without shell interpolation", () => {
  const recorded = [];
  const mockExec = (cmd, args, opts) => {
    recorded.push({ cmd, args, opts });
    return "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM";
  };

  const wasmPath = "path with spaces/target/release/contract.wasm";
  const source = "GCYEX7MPJL64ZJ7ABZSPRC7YEBSI7OMC62FFEVFHCZFREBOYJPQDUCYJ";

  deployContract({
    name: "upto-session",
    wasm: wasmPath,
    alias: "alias-with-spaces and special chars",
    source,
    network: "testnet",
    exec: mockExec,
  });

  assert.equal(recorded.length, 1);
  assert.equal(recorded[0].cmd, "stellar");
  assert.deepEqual(recorded[0].args, [
    "contract",
    "deploy",
    "--wasm",
    wasmPath,
    "--source-account",
    source,
    "--network",
    "testnet",
    "--alias",
    "alias-with-spaces and special chars",
  ]);

  // Argument containing space must be intact as single array element
  assert.equal(recorded[0].args[3], wasmPath);
});

test("invokeContract constructs exact arguments array with parameters", () => {
  const recorded = [];
  const mockExec = (cmd, args) => {
    recorded.push({ cmd, args });
    return "success";
  };

  invokeContract({
    contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
    source: "local-deployer",
    network: "local",
    args: ["initialize", "--admin", "local-deployer", "--cap", "100"],
    exec: mockExec,
  });

  assert.equal(recorded.length, 1);
  assert.deepEqual(recorded[0].args, [
    "contract",
    "invoke",
    "--id",
    "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
    "--source-account",
    "local-deployer",
    "--network",
    "local",
    "--",
    "initialize",
    "--admin",
    "local-deployer",
    "--cap",
    "100",
  ]);
});

test("nonzero exits produce actionable distinct failures preserving stderr", () => {
  const mockFailExec = () => {
    const error = new Error("Command failed: stellar contract deploy");
    error.status = 127;
    error.stderr = "error: transaction submission failed: host error";
    error.stdout = "tx_id: 123";
    throw error;
  };

  assert.throws(
    () => runStellar(["contract", "deploy"], { exec: mockFailExec }),
    (err) => {
      assert.equal(err.status, 127);
      assert.equal(err.stderr, "error: transaction submission failed: host error");
      assert.match(err.message, /exit code 127/);
      assert.match(err.message, /transaction submission failed: host error/);
      return true;
    },
  );
});

test("malformed deployment output without contract ID fails distinctly", () => {
  const mockExec = () => "Transaction processed successfully but no ID printed\nRandom info";

  assert.throws(
    () =>
      deployContract({
        name: "test-token",
        wasm: "token.wasm",
        alias: "token-alias",
        source: "deployer",
        network: "local",
        exec: mockExec,
      }),
    /Failed to extract valid contract ID from deployment output for test-token/,
  );
});

test("findContractId correctly parses last valid C-address from multi-line output", () => {
  const output = `
    Simulating transaction...
    Result: Ok
    CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM
    Done!
  `;
  assert.equal(
    findContractId(output),
    "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
  );
  assert.equal(findContractId(""), undefined);
  assert.equal(findContractId("No contract here"), undefined);
});

test("unsupported networks fail closed with actionable error", () => {
  assert.throws(
    () => validateNetwork("unknown-net"),
    /Unsupported network "unknown-net"/,
  );
  assert.throws(() => validateNetwork(""), /Network must be a non-empty string/);

  for (const net of SUPPORTED_NETWORKS) {
    assert.equal(validateNetwork(net), net);
  }

  // NetworkArgs validation
  const originalEnv = process.env.STELLAR_NETWORK;
  try {
    delete process.env.STELLAR_NETWORK;
    assert.throws(() => networkArgs("invalid_network"), /Unsupported network/);
    assert.deepEqual(networkArgs("testnet"), ["--network", "testnet"]);

    process.env.STELLAR_NETWORK = "bad-env-network";
    assert.throws(() => networkArgs("testnet"), /Unsupported network "bad-env-network"/);
  } finally {
    if (originalEnv !== undefined) {
      process.env.STELLAR_NETWORK = originalEnv;
    } else {
      delete process.env.STELLAR_NETWORK;
    }
  }
});

test("readManifest produces distinct errors on malformed JSON and structure", () => {
  const dir = mkdtempSync(join(tmpdir(), "stellar-cli-test-"));
  try {
    const missingPath = join(dir, "missing.json");
    assert.deepEqual(readManifest(missingPath), { contracts: {} });

    const malformedPath = join(dir, "malformed.json");
    writeFileSync(malformedPath, "{ invalid json", "utf8");
    assert.throws(
      () => readManifest(malformedPath),
      /Malformed JSON in deployment manifest at/,
    );

    const arrayPath = join(dir, "array.json");
    writeFileSync(arrayPath, "[]", "utf8");
    assert.throws(
      () => readManifest(arrayPath),
      /expected a JSON object/,
    );

    const badContractsPath = join(dir, "bad-contracts.json");
    writeFileSync(badContractsPath, JSON.stringify({ contracts: "not-an-object" }), "utf8");
    assert.throws(
      () => readManifest(badContractsPath),
      /"contracts" field must be an object/,
    );

    const validPath = join(dir, "valid.json");
    const manifest = {
      contracts: {
        "upto-session": {
          contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
        },
      },
    };
    writeFileSync(validPath, JSON.stringify(manifest), "utf8");
    const read = readManifest(validPath);
    assert.deepEqual(read, manifest);

    // Test getContractEntry
    const entry = getContractEntry(read, "upto-session");
    assert.equal(entry.contractId, "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM");

    assert.throws(
      () => getContractEntry(read, "missing-contract"),
      /Contract "missing-contract" not found in manifest/,
    );

    assert.throws(
      () =>
        getContractEntry(
          { contracts: { invalid: { contractId: "not-a-valid-id" } } },
          "invalid",
        ),
      /missing a valid contractId/,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("dry-run mode performs no process execution or filesystem mutation", () => {
  let executed = false;
  const mockExec = () => {
    executed = true;
    return "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM";
  };

  const output = runStellar(["contract", "build"], { dryRun: true, exec: mockExec });
  assert.equal(output, "");
  assert.equal(executed, false);

  const deployment = deployContract({
    name: "upto-session",
    wasm: "target/wasm.wasm",
    alias: "alias",
    source: "deployer",
    network: "local",
    dryRun: true,
    exec: mockExec,
  });
  assert.equal(deployment.contractId, "dry-run");
  assert.equal(executed, false);

  // Filesystem mutation check during dry run
  const dir = mkdtempSync(join(tmpdir(), "dryrun-test-"));
  try {
    const targetPath = join(dir, "manifest.json");
    writeManifest(targetPath, { contracts: {} }, { dryRun: true });
    assert.equal(existsSync(targetPath), false);

    mergeContracts(targetPath, [deployment], { dryRun: true });
    assert.equal(existsSync(targetPath), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("formatCommand correctly quotes paths containing spaces for dry-run inspection", () => {
  const formatted = formatCommand("stellar", [
    "contract",
    "deploy",
    "--wasm",
    "C:/Path With Spaces/my contract.wasm",
    "--network",
    "testnet",
  ]);
  assert.equal(
    formatted,
    'stellar contract deploy --wasm "C:/Path With Spaces/my contract.wasm" --network testnet',
  );

  assert.equal(quoteArg("normal_arg-123.wasm"), "normal_arg-123.wasm");
  assert.equal(quoteArg("spaced arg"), '"spaced arg"');
});
