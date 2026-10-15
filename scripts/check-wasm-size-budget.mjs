import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const budgetPath = join(repoRoot, "artifacts", "resource-usage", "wasm-size-budget.json");
const budget = JSON.parse(readFileSync(budgetPath, "utf8"));
const reviewPath = optionValue("--review");
const reportPath = optionValue("--report");

validateBudget(budget);
const rows = budget.contracts.map((entry) => measure(entry));
const failures = rows.filter((row) => !row.withinBudget);
let review = null;

if (failures.length > 0 && reviewPath) {
  review = validateReview(reviewPath, failures);
}

console.log(
  `WASM toolchain: rustc ${budget.build.rustc}, ${budget.build.target}, ` +
    `Soroban SDK ${budget.build.sorobanSdk}, Stellar CLI/optimizer ${budget.build.optimizer.version}`,
);
console.log("Contract | Bytes | Baseline | Delta bytes | Delta percent | Maximum | Result");
for (const row of rows) {
  const result = row.withinBudget ? "PASS" : review ? "REVIEWED OVERRIDE" : "FAIL";
  console.log(
    `${row.name} | ${row.actualBytes} | ${row.baselineBytes} | ${signed(row.deltaBytes)} | ` +
      `${signed(row.deltaPercent)}% | ${row.maxBytes} | ${result}`,
  );
}

const report = {
  schemaVersion: 1,
  sourceBudget: relative(repoRoot, budgetPath).replaceAll("\\", "/"),
  sourceCommit: budget.sourceCommit,
  build: budget.build,
  review,
  contracts: rows,
  passed: failures.length === 0 || review !== null,
};

if (reportPath) {
  const output = isAbsolute(reportPath) ? reportPath : resolve(repoRoot, reportPath);
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  console.log(`WASM size report written to ${output}`);
}

if (!report.passed) {
  throw new Error(
    `${failures.length} contract(s) exceed the reviewed WASM budget. ` +
      "Reduce the size, update the baseline through review, or pass --review docs/wasm-size-reviews/<record>.md.",
  );
}

function measure(entry) {
  const artifact = join(repoRoot, entry.artifact);
  if (!existsSync(artifact)) {
    throw new Error(`Missing optimized WASM artifact: ${entry.artifact}`);
  }
  const actualBytes = statSync(artifact).size;
  const deltaBytes = actualBytes - entry.baselineBytes;
  const deltaPercent = Number(((deltaBytes / entry.baselineBytes) * 100).toFixed(2));
  return {
    name: entry.name,
    artifact: entry.artifact,
    actualBytes,
    baselineBytes: entry.baselineBytes,
    deltaBytes,
    deltaPercent,
    maxBytes: entry.maxBytes,
    maxIncreasePercent: entry.maxIncreasePercent,
    withinBudget:
      actualBytes <= entry.maxBytes && deltaPercent <= entry.maxIncreasePercent,
  };
}

function validateBudget(value) {
  if (value.schemaVersion !== 1 || !value.build?.optimized || !value.build?.optimizer?.version) {
    throw new Error("WASM budget must record schema, optimized build, and optimizer version.");
  }
  if (!Array.isArray(value.contracts) || value.contracts.length !== 3) {
    throw new Error("WASM budget must contain all three release contracts.");
  }
  for (const entry of value.contracts) {
    if (
      !entry.name ||
      !entry.artifact ||
      !Number.isInteger(entry.baselineBytes) ||
      !Number.isInteger(entry.maxBytes) ||
      entry.baselineBytes <= 0 ||
      entry.maxBytes < entry.baselineBytes ||
      typeof entry.maxIncreasePercent !== "number" ||
      entry.maxIncreasePercent < 0
    ) {
      throw new Error(`Invalid WASM budget entry: ${JSON.stringify(entry)}`);
    }
  }
}

function validateReview(input, failures) {
  const absolute = resolve(repoRoot, input);
  const reviewRoot = join(repoRoot, "docs", "wasm-size-reviews");
  const relativeReview = relative(reviewRoot, absolute);
  if (relativeReview.startsWith("..") || isAbsolute(relativeReview) || !existsSync(absolute)) {
    throw new Error("Override review must be a committed file under docs/wasm-size-reviews/.");
  }
  const content = readFileSync(absolute, "utf8");
  for (const heading of ["## Reason", "## Security impact", "## Approval"]) {
    if (!content.includes(heading)) {
      throw new Error(`Override review is missing ${heading}.`);
    }
  }
  for (const failure of failures) {
    if (!content.includes(`\`${failure.name}\``)) {
      throw new Error(`Override review does not name \`${failure.name}\`.`);
    }
  }
  return {
    path: relative(repoRoot, absolute).replaceAll("\\", "/"),
    contracts: failures.map((failure) => failure.name),
  };
}

function optionValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${name} requires a value.`);
  }
  return value;
}

function signed(value) {
  return value > 0 ? `+${value}` : String(value);
}
