import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const packageRoot = resolve(repoRoot, process.argv[2] || "bindings/upto-session");
const path = join(packageRoot, "PROVENANCE.json");
if (!existsSync(path)) throw new Error("PROVENANCE.json is missing; regenerate the package attestation");
const attestation = JSON.parse(readFileSync(path, "utf8"));
for (const [file, expected] of Object.entries(attestation.files || {})) {
  const filePath = join(packageRoot, file);
  if (!existsSync(filePath) || sha256(readFileSync(filePath)) !== expected) {
    throw new Error(`Binding provenance checksum drift: ${file}`);
  }
}
if (!/^[0-9a-f]{40}$/.test(attestation.sourceRevision) || !/^[0-9a-f]{64}$/.test(attestation.contractSpec?.sha256)) {
  throw new Error("Binding provenance revision or contract spec digest is malformed");
}
console.log(`verified ${Object.keys(attestation.files).length} binding files for ${attestation.sourceRevision}`);

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}
