import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  classifyAbi,
  extractAbi,
  normalizeSpecText,
  sha256,
  validateBreakingRelease,
} from "./lib/abi-compatibility.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const specPath = join(repoRoot, "artifacts", "spec", "upto-session.json");
const baselinePath = join(
  repoRoot,
  "artifacts",
  "interface",
  "upto-session-release-baseline.json",
);
const contractPath = join(repoRoot, "contracts", "upto-session", "src", "lib.rs");
const storagePath = join(repoRoot, "contracts", "upto-session", "src", "storage.rs");
const specText = normalizeSpecText(readFileSync(specPath, "utf8"));
const spec = JSON.parse(specText);
const interfaceVersion = readRustVersion(contractPath, "INTERFACE_VERSION");
const storageLayoutVersion = readRustVersion(storagePath, "STORAGE_LAYOUT_VERSION");

if (process.argv.includes("--write-baseline")) {
  const releaseTag = argumentValue("--release-tag");
  if (!releaseTag) {
    throw new Error("--write-baseline requires --release-tag <tag>");
  }
  const baseline = {
    releaseTag,
    sourceSpecSha256: sha256(specText),
    ...extractAbi(spec, interfaceVersion, storageLayoutVersion),
  };
  writeFileSync(baselinePath, `${JSON.stringify(baseline, null, 2)}\n`, "ascii");
  console.log(`wrote ABI baseline for ${releaseTag}`);
  process.exit(0);
}

const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
const current = extractAbi(spec, interfaceVersion, storageLayoutVersion);
const result = classifyAbi(baseline, current);

console.log(`ABI baseline: ${baseline.releaseTag}`);
console.log(`Baseline spec SHA-256: ${baseline.sourceSpecSha256}`);
for (const classification of ["additive", "compatible", "breaking"]) {
  console.log(`${classification}: ${result[classification].length}`);
  for (const change of result[classification]) {
    console.log(`  - ${change}`);
  }
}

if (result.breaking.length > 0) {
  const migrationPath = join(
    repoRoot,
    "artifacts",
    "interface",
    "migrations",
    `v${baseline.interfaceVersion}-to-v${current.interfaceVersion}.json`,
  );
  const migration = existsSync(migrationPath)
    ? JSON.parse(readFileSync(migrationPath, "utf8"))
    : null;
  const failure = validateBreakingRelease(baseline, current, migration);
  if (failure) {
    throw new Error(failure);
  }
  console.log(`breaking release evidence: ${migrationPath}`);
}

function readRustVersion(path, name) {
  const source = readFileSync(path, "utf8");
  const match = source.match(new RegExp(`pub const ${name}: u32 = (\\d+);`));
  if (!match) {
    throw new Error(`Unable to read ${name} from ${path}`);
  }
  return Number(match[1]);
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? null : process.argv[index + 1];
}
