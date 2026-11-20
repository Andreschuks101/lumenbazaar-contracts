import { execFileSync } from "node:child_process";
import { readFileSync, existsSync, statSync } from "node:fs";
import { join } from "node:path";
import { repoRoot } from "./lib/stellar-cli.mjs";
import { checkChangelog } from "./lib/changelog-gate.mjs";
import { classifyAbi, extractAbi } from "./lib/abi-compatibility.mjs";

const index = process.argv.indexOf("--base");
const base = process.argv[index + 1];
if (index < 0 || !base || base.startsWith("-")) throw new Error("Usage: node scripts/check-changelog.mjs --base <commit-or-ref>");
const git = (args) => execFileSync("git", args, { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
// Resolve to a commit before using the value in any subsequent git operation.
const commit = git(["rev-parse", "--verify", `${base}^{commit}`]).trim();
const paths = git(["diff", "--no-renames", "--name-only", "-z", commit, "HEAD", "--"]).split("\0").filter(Boolean);
const before = (path) => {
  const present = git(["ls-tree", "--name-only", commit, "--", path]).trim();
  return present ? git(["show", `${commit}:${path}`]) : "";
};
const current = (path) => existsSync(join(repoRoot, path)) ? readFileSync(join(repoRoot, path), "utf8") : "";
const changes = paths.map((path) => ({ path, before: before(path), after: current(path) }));
const specPath = "artifacts/spec/upto-session.json";
let breaking = Boolean(before(specPath) && !current(specPath));
if (before(specPath) && current(specPath)) {
  const version = (text, name) => Number(text.match(new RegExp(`pub const ${name}: u32 = (\\d+);`))?.[1]);
  const lib = "contracts/upto-session/src/lib.rs";
  const storage = "contracts/upto-session/src/storage.rs";
  let oldAbi = null;
  try {
    oldAbi = extractAbi(JSON.parse(before(specPath)), version(before(lib), "INTERFACE_VERSION"), version(before(storage), "STORAGE_LAYOUT_VERSION"));
  } catch {
    oldAbi = { methods: [], events: [], errors: [], structs: [], enums: [], unions: [], types: [] };
  }
  const newAbi = extractAbi(JSON.parse(current(specPath)), version(current(lib), "INTERFACE_VERSION"), version(current(storage), "STORAGE_LAYOUT_VERSION"));
  breaking = classifyAbi(oldAbi, newAbi).breaking.length > 0;
}
const result = checkChangelog({ changes, beforeChangelog: before("CHANGELOG.md"), changelog: current("CHANGELOG.md"), breaking,
  exists: (path) => existsSync(join(repoRoot, path)) && statSync(join(repoRoot, path)).isFile() });
if (result.errors.length) throw new Error(`Changelog gate failed for ${result.paths.join(", ")}:\n- ${result.errors.join("\n- ")}`);
console.log(`Changelog gate passed (${result.paths.length} contract-facing files).`);
