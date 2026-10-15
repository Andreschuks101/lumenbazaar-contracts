import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const docPath = join(repoRoot, "docs", "audit-readiness.md");
const freezePath = join(
  repoRoot,
  "artifacts",
  "audit-readiness",
  "interface-freeze.json",
);

const doc = readFileSync(docPath, "utf8");
const freeze = JSON.parse(readFileSync(freezePath, "utf8"));

for (const section of [
  "Public Interface Freeze",
  "Known Limitations",
  "Threat Model",
  "Test Coverage",
  "Deployment Procedure",
]) {
  if (!doc.includes(`## ${section}`)) {
    throw new Error(`Audit readiness doc is missing ${section}`);
  }
}

for (const artifact of [
  "docs/security-checklist.md",
  "artifacts/resource-usage/upto-session.md",
  "docs/backend-integration-handoff.md",
  "artifacts/interface/upto-session-v3.json",
  "deployments/testnet.example.json",
]) {
  if (!existsSync(join(repoRoot, artifact))) {
    throw new Error(`Missing audit artifact ${artifact}`);
  }
}

for (const file of freeze.files) {
  const path = join(repoRoot, file.path);
  if (!existsSync(path)) {
    throw new Error(`Frozen file is missing: ${file.path}`);
  }
  const actual = sha256(readNormalizedText(path));
  if (actual !== file.sha256) {
    throw new Error(`Frozen file hash changed: ${file.path}`);
  }
}

console.log("audit readiness check passed");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function readNormalizedText(path) {
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}
