import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const packageRoot = resolve(repoRoot, process.argv[2] || "bindings/upto-session");
const packageJson = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8"));
const files = ["README.md", "package.json", ...walk(join(packageRoot, "src")).map((file) => relative(packageRoot, file).replaceAll("\\", "/"))];
const attestation = {
  schemaVersion: 1,
  package: { name: packageJson.name, version: packageJson.version },
  sourceRevision: execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim(),
  contractSpec: {
    path: "artifacts/spec/upto-session.json",
    sha256: sha256(readFileSync(join(repoRoot, "artifacts/spec/upto-session.json"))),
  },
  toolchain: { node: process.version, stellarSdk: packageJson.dependencies["@stellar/stellar-sdk"] },
  files: Object.fromEntries(files.sort().map((file) => [file, sha256(readFileSync(join(packageRoot, file)))])),
};
writeFileSync(join(packageRoot, "PROVENANCE.json"), `${JSON.stringify(attestation, null, 2)}\n`);

function walk(directory) {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}
