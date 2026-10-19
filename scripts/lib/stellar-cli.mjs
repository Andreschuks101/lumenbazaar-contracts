import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

export const SUPPORTED_NETWORKS = Object.freeze([
  "local",
  "testnet",
  "standalone",
  "futurenet",
  "mainnet",
]);

export function validateNetwork(network) {
  if (!network || typeof network !== "string") {
    throw new Error("Network must be a non-empty string");
  }
  if (!SUPPORTED_NETWORKS.includes(network)) {
    throw new Error(
      `Unsupported network "${network}". Supported networks are: ${SUPPORTED_NETWORKS.join(", ")}`,
    );
  }
  return network;
}

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
  if (network) {
    validateNetwork(network);
    return ["--network", network];
  }
  return [];
}

export function runStellar(
  args,
  { capture = false, dryRun = false, exec = execFileSync } = {},
) {
  if (dryRun) {
    console.log(formatCommand("stellar", args));
    return "";
  }

  try {
    const output = exec("stellar", args, {
      cwd: repoRoot,
      encoding: "utf8",
      stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
    });

    return typeof output === "string" ? output.trim() : "";
  } catch (err) {
    const status =
      err.status !== undefined
        ? err.status
        : err.code !== undefined
          ? err.code
          : 1;
    const stderr = err.stderr ? err.stderr.toString().trim() : "";
    const stdout = err.stdout ? err.stdout.toString().trim() : "";
    const detail = stderr || err.message;
    const error = new Error(
      `Stellar CLI command failed with exit code ${status}: ${detail}`,
    );
    error.status = status;
    error.stderr = stderr;
    error.stdout = stdout;
    error.args = args;
    throw error;
  }
}

export function buildContracts({ dryRun = false, exec = execFileSync } = {}) {
  runStellar(["contract", "build", "--locked"], { dryRun, exec });
}

export function deployContract({
  name,
  wasm,
  alias,
  source,
  network = "local",
  dryRun = false,
  exec = execFileSync,
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
  const output = runStellar(args, { capture: true, dryRun, exec });

  let contractId = "dry-run";
  if (!dryRun) {
    const found = findContractId(output);
    if (!found) {
      throw new Error(
        `Failed to extract valid contract ID from deployment output for ${name}: "${output}"`,
      );
    }
    contractId = found;
  }

  return {
    name,
    alias,
    network: process.env.STELLAR_NETWORK || network,
    wasm,
    contractId,
  };
}

export function invokeContract({
  contractId,
  source,
  network = "local",
  args,
  capture = false,
  dryRun = false,
  exec = execFileSync,
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
    { capture, dryRun, exec },
  );
}

export function readManifest(path) {
  if (!existsSync(path)) {
    return { contracts: {} };
  }
  let raw;
  try {
    raw = readFileSync(path, "utf8");
  } catch (err) {
    throw new Error(`Failed to read manifest file at ${path}: ${err.message}`);
  }
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    throw new Error(
      `Malformed JSON in deployment manifest at ${path}: ${err.message}`,
    );
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`Malformed manifest at ${path}: expected a JSON object`);
  }
  if (
    parsed.contracts !== undefined &&
    (typeof parsed.contracts !== "object" ||
      parsed.contracts === null ||
      Array.isArray(parsed.contracts))
  ) {
    throw new Error(
      `Malformed manifest at ${path}: "contracts" field must be an object`,
    );
  }
  return parsed;
}

export function getContractEntry(manifest, name) {
  if (!manifest || typeof manifest !== "object" || !manifest.contracts) {
    throw new Error('Invalid manifest structure: missing "contracts" object');
  }
  const entry = manifest.contracts[name];
  if (!entry) {
    throw new Error(`Contract "${name}" not found in manifest`);
  }
  if (!entry.contractId || !/^C[A-Z2-7]{55}$/.test(entry.contractId)) {
    throw new Error(
      `Contract "${name}" in manifest is missing a valid contractId`,
    );
  }
  return entry;
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

export function findContractId(output) {
  if (!output || typeof output !== "string") {
    return undefined;
  }
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .reverse()
    .find((line) => /^C[A-Z2-7]{55}$/.test(line));
}

export function formatCommand(command, args) {
  return [command, ...args].map((part) => quoteArg(part)).join(" ");
}

export function quoteArg(value) {
  if (/^[A-Za-z0-9_./:=@-]+$/.test(value)) {
    return value;
  }
  return JSON.stringify(value);
}
