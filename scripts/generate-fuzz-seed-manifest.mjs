import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const seedRoot = join(root, "fuzz", "seeds");
const entries = walk(seedRoot).filter((path) => path.endsWith(".hex")).map((path) => {
  const relativePath = relative(root, path).replaceAll("\\", "/");
  const [, , target, name] = relativePath.split("/");
  return {
    path: relativePath,
    sha256: createHash("sha256").update(readFileSync(path)).digest("hex"),
    target,
    schema: `${target}-input-v1`,
    purpose: name.replace(/^\d+-/, "").replace(/\.hex$/, ""),
    expectedOutcome: "defined-contract-result-without-host-failure",
    generator: "reviewed-boundary-seed-v1",
  };
});
writeFileSync(join(seedRoot, "manifest.json"), `${JSON.stringify({ schemaVersion: 1, seeds: entries }, null, 2)}\n`);

function walk(directory) {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}
