import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
verify("fuzz/seeds", "manifest.json", "seeds");
verify("fuzz/crashes", "manifest.json", "artifacts");

function verify(directory, manifestName, field) {
  const base = join(root, directory);
  const manifest = JSON.parse(readFileSync(join(base, manifestName), "utf8"));
  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest[field])) throw new Error(`${directory} manifest schema is invalid`);
  const files = walk(base).filter((path) => path.endsWith(".hex")).map((path) => relative(root, path).replaceAll("\\", "/")).sort();
  const listed = manifest[field].map((entry) => entry.path).sort();
  if (JSON.stringify(files) !== JSON.stringify(listed)) throw new Error(`${directory} files must be listed exactly once`);
  for (const entry of manifest[field]) {
    if (!entry.target || !entry.schema || !entry.purpose || !entry.expectedOutcome) throw new Error(`Incomplete fuzz provenance: ${entry.path}`);
    const bytes = readFileSync(join(root, entry.path));
    if (bytes.length > 4096 || createHash("sha256").update(bytes).digest("hex") !== entry.sha256) throw new Error(`Fuzz artifact drift: ${entry.path}`);
    if (!/^[\s0-9a-fA-F#:\-_.=>()]*$/.test(bytes.toString("utf8"))) throw new Error(`Fuzz artifact contains non-synthetic data: ${entry.path}`);
  }
}

function walk(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}
