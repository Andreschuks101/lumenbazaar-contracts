import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const outPath = join(repoRoot, "artifacts", "audit-readiness", "interface-freeze.json");

const files = [
  "contracts/upto-session/src/lib.rs",
  "contracts/upto-session/src/types.rs",
  "contracts/upto-session/src/errors.rs",
  "contracts/upto-session/src/events.rs",
  "artifacts/spec/upto-session.json",
  "bindings/upto-session/src/index.ts",
  "bindings/upto-session/package.json",
  "artifacts/interface/upto-session-v3.json",
];

const freeze = {
  reviewTarget: "upto-session",
  generatedBy: "node scripts/generate-audit-freeze.mjs",
  files: files.map((path) => ({
    path,
    sha256: sha256(readNormalizedText(join(repoRoot, path))),
  })),
};

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, `${JSON.stringify(freeze, null, 2)}\n`, "utf8");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function readNormalizedText(path) {
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}
