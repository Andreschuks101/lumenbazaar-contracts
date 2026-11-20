import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  artifactNameForWasm,
  readLockfile,
  readToolchain,
  sha256Hex,
  verifyProvenance,
} from "./lib/provenance.mjs";
import { verifyReleaseSignature } from "./lib/release-signature.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmDir = join(repoRoot, argumentValue("--wasm-dir") ?? join("target", "wasm32v1-none", "release"));
const outDir = join(repoRoot, argumentValue("--out-dir") ?? join("artifacts", "provenance"));
const provenancePath = join(outDir, "build-provenance.json");
const sbomPath = join(outDir, "sbom.cdx.json");

for (const [label, path] of [
  ["provenance", provenancePath],
  ["SBOM", sbomPath],
]) {
  if (!existsSync(path)) {
    throw new Error(`Missing ${label} artifact at ${relative(repoRoot, path)}; run scripts/generate-provenance.mjs first.`);
  }
}

const provenance = JSON.parse(readFileSync(provenancePath, "utf8"));
const sbom = JSON.parse(readFileSync(sbomPath, "utf8"));
const lockfile = readLockfile(repoRoot);
const toolchain = readToolchain(repoRoot);

const wasmFiles = existsSync(wasmDir)
  ? readdirSync(wasmDir).filter((fileName) => fileName.endsWith(".wasm"))
  : [];
const artifacts = wasmFiles.map((fileName) => {
  const bytes = readFileSync(join(wasmDir, fileName));
  return { name: artifactNameForWasm(fileName), sha256: sha256Hex(bytes), sizeBytes: bytes.length };
});
const expectedWasmFiles = wasmFiles.map(artifactNameForWasm);

const failures = verifyProvenance({
  provenance,
  sbom,
  packages: lockfile.packages,
  lockfileSha256: lockfile.sha256,
  toolchain,
  artifacts,
  expectedWasmFiles,
});

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`provenance check failed: ${failure}`);
  }
  process.exitCode = 1;
} else {
  const provenanceSignature = argumentValue("--provenance-signature");
  const sbomSignature = argumentValue("--sbom-signature");
  if (provenanceSignature || sbomSignature) {
    if (!provenanceSignature || !sbomSignature) {
      throw new Error("Both provenance and SBOM detached signatures are required");
    }
    const signersPath = join(repoRoot, "release-signers.json");
    verifyReleaseSignature({ artifactPath: provenancePath, signaturePath: join(repoRoot, provenanceSignature), signersPath });
    verifyReleaseSignature({ artifactPath: sbomPath, signaturePath: join(repoRoot, sbomSignature), signersPath });
  }
  console.log(
    `provenance and SBOM verified for ${artifacts.length} WASM artifact(s) and ${lockfile.packages.length} crate(s)`,
  );
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? null : process.argv[index + 1];
}
