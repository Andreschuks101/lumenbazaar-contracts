import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { assertErrorMapMatchesFixture } from "./lib/contract-error-map.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const skipBuild = process.argv.includes("--skip-build");
const wasm = join(repoRoot, "target", "wasm32v1-none", "release", "upto_session.wasm");
const outputDir = join(repoRoot, "bindings", "upto-session");
const packageLockPath = join(outputDir, "package-lock.json");
const packageLock = existsSync(packageLockPath) ? readFileSync(packageLockPath, "utf8") : null;
const errorTemplate = join(repoRoot, "scripts", "templates", "contractErrors.ts");

// Fail before touching the output if the typed error map has drifted from the
// frozen interface fixture.
assertErrorMapMatchesFixture(
  errorTemplate,
  join(repoRoot, "artifacts", "interface", "upto-session-v3.json"),
);

if (!skipBuild) {
  execFileSync("stellar", ["contract", "build", "--locked"], {
    cwd: repoRoot,
    stdio: "inherit",
  });
}

const tempParent = mkdtempSync(join(tmpdir(), "bindings-"));
const tempDir = join(tempParent, "upto-session");
try {
  execFileSync(
    "stellar",
    [
      "contract",
      "bindings",
      "typescript",
      "--wasm",
      wasm,
      "--output-dir",
      tempDir,
      "--overwrite",
    ],
    {
      cwd: repoRoot,
      stdio: "inherit",
    },
  );
  cpSync(tempDir, outputDir, { recursive: true, force: true });
} finally {
  rmSync(tempParent, { recursive: true, force: true });
}

copyFileSync(
  join(repoRoot, "scripts", "templates", "usageReceipt.ts"),
  join(outputDir, "src", "usageReceipt.ts"),
);
copyFileSync(errorTemplate, join(outputDir, "src", "contractErrors.ts"));
copyFileSync(
  join(repoRoot, "scripts", "templates", "sessionId.ts"),
  join(outputDir, "src", "sessionId.ts"),
);
copyFileSync(
  join(repoRoot, "scripts", "templates", "transactionEvidence.ts"),
  join(outputDir, "src", "transactionEvidence.ts"),
);
copyFileSync(
  join(repoRoot, "scripts", "templates", "simulation.ts"),
  join(outputDir, "src", "simulation.ts"),
);
copyFileSync(
  join(repoRoot, "scripts", "templates", "clientValidation.ts"),
  join(outputDir, "src", "clientValidation.ts"),
);

const bindingPath = join(outputDir, "src", "index.ts");
const binding = readFileSync(bindingPath, "utf8");
const contractSpecPattern = /new ContractSpec\(\[\s*([\s\S]*?)\s*\]\)/;
const match = binding.match(contractSpecPattern);

if (match === null) {
  throw new Error("Generated binding does not contain a ContractSpec array.");
}

const encodedEntries = [...match[1].matchAll(/"([A-Za-z0-9+/=]+)"/g)].map((entry) => entry[1]);
const remainder = match[1].replaceAll(/"[A-Za-z0-9+/=]+"/g, "").replaceAll(/[\s,]/g, "");

if (encodedEntries.length === 0 || remainder.length > 0) {
  throw new Error("Generated ContractSpec array has an unexpected format.");
}

const canonicalContractSpec = `new ContractSpec([\n${encodedEntries
  .sort((left, right) => (left < right ? -1 : left > right ? 1 : 0))
  .map((entry) => `        ${JSON.stringify(entry)}`)
  .join(",\n")}\n      ])`;

let canonicalBinding = binding.replace(contractSpecPattern, canonicalContractSpec);
for (const templateExport of [
  'export * from "./usageReceipt.js";',
  'export * from "./contractErrors.js";',
]) {
  if (!canonicalBinding.includes(templateExport)) {
    canonicalBinding = `${canonicalBinding.trimEnd()}\n\n${templateExport}\n`;
  }
}
const sessionIdExport = 'export * from "./sessionId.js";';
if (!canonicalBinding.includes(sessionIdExport)) {
  canonicalBinding = `${canonicalBinding.trimEnd()}\n${sessionIdExport}\n`;
}
const transactionEvidenceExport = 'export * from "./transactionEvidence.js";';
if (!canonicalBinding.includes(transactionEvidenceExport)) {
  canonicalBinding = `${canonicalBinding.trimEnd()}\n${transactionEvidenceExport}\n`;
}
const simulationExport = 'export * from "./simulation.js";';
if (!canonicalBinding.includes(simulationExport)) {
  canonicalBinding = `${canonicalBinding.trimEnd()}\n${simulationExport}\n`;
}
const clientValidationExport = 'export * from "./clientValidation.js";';
if (!canonicalBinding.includes(clientValidationExport)) {
  canonicalBinding = `${canonicalBinding.trimEnd()}\n${clientValidationExport}\n`;
}
writeFileSync(bindingPath, canonicalBinding, "utf8");

// The CLI rewrites README.md; re-append the hand-written helper documentation.
const readmePath = join(outputDir, "README.md");
const readme = readFileSync(readmePath, "utf8");
const evidenceReadme = readFileSync(
  join(repoRoot, "scripts", "templates", "README.transactionEvidence.md"),
  "utf8",
);
if (!readme.includes("<!-- lumenbazaar:transaction-evidence")) {
  writeFileSync(readmePath, `${readme.trimEnd()}\n\n${evidenceReadme}`, "utf8");
}

const packagePath = join(outputDir, "package.json");
const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
packageJson.version = "3.0.0";
packageJson.dependencies["@stellar/stellar-sdk"] = "17.2.1";
packageJson.scripts.prepack = "node ../../scripts/generate-binding-attestation.mjs && node ../../scripts/verify-binding-attestation.mjs";
writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`, "utf8");

if (packageLock !== null) {
  writeFileSync(packageLockPath, packageLock, "utf8");
}
