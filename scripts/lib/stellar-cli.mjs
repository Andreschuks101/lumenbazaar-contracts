import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

export function hasFlag(flag) {
  return process.argv.includes(flag);
}

export function sourceAccount(defaultValue = "local-deployer") {
  if (hasFlag("--dry-run")) {
    return (
      process.env.LUMENBAZAAR_SOURCE_ACCOUNT ||
      process.env.STELLAR_ACCOUNT ||
      defaultValue
    );
  }

  const source = process.env.LUMENBAZAAR_SOURCE_ACCOUNT || process.env.STELLAR_ACCOUNT;
  if (!source) {
    throw new Error("Set STELLAR_ACCOUNT or LUMENBAZAAR_SOURCE_ACCOUNT");
  }
  return source;
}

export function networkArgs(defaultNetwork) {
  const network = process.env.STELLAR_NETWORK || defaultNetwork;
  return network ? ["--network", network] : [];
}

export function runStellar(args, { capture = false, dryRun = false } = {}) {
  if (dryRun) {
    console.log(formatCommand("stellar", args));
    return "";
  }

  const output = execFileSync("stellar", args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });

  return typeof output === "string" ? output.trim() : "";
}

export function buildContracts({ dryRun = false } = {}) {
  runStellar(["contract", "build", "--locked"], { dryRun });
}

export function deployContract({
  name,
  wasm,
  alias,
  source,
  network = "local",
  dryRun = false,
}) {
  const args = [
    "contract",
    "deploy",
    "--wasm",
    wasm,
    "--source-account",
    source,
    ...networkArgs(network),
    "--alias",
    alias,
  ];
  const output = runStellar(args, { capture: true, dryRun });

  return {
    name,
    alias,
    network: process.env.STELLAR_NETWORK || network,
    wasm,
    contractId: output ? findContractId(output) || output : "dry-run",
  };
}

export function invokeContract({
  contractId,
  source,
  network = "local",
  args,
  capture = false,
  dryRun = false,
}) {
  return runStellar(
    [
      "contract",
      "invoke",
      "--id",
      contractId,
      "--source-account",
      source,
      ...networkArgs(network),
      "--",
      ...args,
    ],
    { capture, dryRun },
  );
}

export function readManifest(path) {
  if (!existsSync(path)) {
    return { contracts: {} };
  }
  return JSON.parse(readFileSync(path, "utf8"));
}

export function writeManifest(path, manifest, { dryRun = false } = {}) {
  const value = {
    ...manifest,
    generatedAt: new Date().toISOString(),
  };

  if (dryRun) {
    console.log(`write ${relative(repoRoot, path)}`);
    console.log(JSON.stringify(value, null, 2));
    return;
  }

  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

export function mergeContracts(manifestPath, contractEntries, { dryRun = false } = {}) {
  const manifest = readManifest(manifestPath);
  const contracts = { ...(manifest.contracts || {}) };
  for (const entry of contractEntries) {
    contracts[entry.name] = entry;
  }

  writeManifest(
    manifestPath,
    {
      ...manifest,
      contracts,
    },
    { dryRun },
  );
}

function findContractId(output) {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .reverse()
    .find((line) => /^C[A-Z2-7]{55}$/.test(line));
}

function formatCommand(command, args) {
  return [command, ...args].map((part) => quoteArg(part)).join(" ");
}

function quoteArg(value) {
  if (/^[A-Za-z0-9_./:=@-]+$/.test(value)) {
    return value;
  }
  return JSON.stringify(value);
}
