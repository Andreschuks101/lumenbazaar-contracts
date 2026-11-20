// Reproduces the WASM recorded in a testnet deployment manifest and verifies
// every SHA-256. The release workflow runs this script; maintainers and
// reviewers run the same command on a clean Linux machine.
//
//   node scripts/reproduce-testnet-wasm.mjs --manifest deployments/testnet-2026-09-06.json
//   node scripts/reproduce-testnet-wasm.mjs --tag testnet-wasm-2026-09-06 --out-dir <dir>
//   node scripts/reproduce-testnet-wasm.mjs --manifest <path> --dry-run
//   node scripts/reproduce-testnet-wasm.mjs --manifest <path> --artifacts-dir <dir>
//   node scripts/reproduce-testnet-wasm.mjs --manifest <path> --plan
//
// The build runs in a clean `git archive` export of the recorded source
// commit with its committed Cargo.lock (`--locked`), the recorded Rust
// version (RUSTUP_TOOLCHAIN) and the recorded Stellar CLI version. Nothing is
// written unless every artifact matches or has a reviewed explanation.
// `Dockerfile.reproducible` wraps this pinned toolchain in a credential-free
// image and compares two clean WASM/spec builds.
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BUILD_COMMAND,
  MANIFEST_PATH_PATTERN,
  WASM_TARGET,
  buildChecksums,
  manifestPathForTag,
  parseReviewedMismatches,
  releaseExpectations,
  renderReleaseNotes,
  sha256Hex,
  stellarCliPin,
  verifyArtifacts,
} from "./lib/wasm-release.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs(process.argv.slice(2));

try {
  await main();
} catch (error) {
  console.error(`reproduce-testnet-wasm: ${error.message}`);
  process.exit(1);
}

async function main() {
  if (args.tag !== undefined && args.manifest !== undefined && manifestPathForTag(args.tag) !== args.manifest) {
    throw new Error(`--tag ${args.tag} maps to ${manifestPathForTag(args.tag)}, not ${args.manifest}`);
  }
  const manifestPath = args.manifest ?? (args.tag !== undefined ? manifestPathForTag(args.tag) : undefined);
  if (manifestPath === undefined) {
    throw new Error("pass --manifest deployments/testnet-YYYY-MM-DD.json or --tag testnet-wasm-YYYY-MM-DD");
  }
  if (!MANIFEST_PATH_PATTERN.test(manifestPath)) {
    throw new Error(`${JSON.stringify(manifestPath)} is not a deployments/testnet-YYYY-MM-DD.json manifest`);
  }
  const manifestBytes = readFileSync(join(repoRoot, manifestPath));
  const expectations = releaseExpectations(JSON.parse(manifestBytes.toString("utf8")), manifestPath);
  const cliPin = stellarCliPin(expectations.stellarCli);

  if (args.plan) {
    console.log(
      JSON.stringify({
        manifest: manifestPath,
        sourceCommit: expectations.sourceCommit,
        rustc: expectations.rustc,
        stellarCli: expectations.stellarCli,
        stellarCliUrl: cliPin.url,
        stellarCliSha256: cliPin.sha256,
      }),
    );
    return;
  }

  const reviewedPath = args.reviewedMismatches ?? join("deployments", "reviewed-mismatches", basename(manifestPath));
  const reviewed = existsSync(join(repoRoot, reviewedPath))
    ? parseReviewedMismatches(
        JSON.parse(readFileSync(join(repoRoot, reviewedPath), "utf8")),
        manifestPath,
        reviewedPath.replaceAll("\\", "/"),
      )
    : [];

  git(["cat-file", "-e", `${expectations.sourceCommit}^{commit}`], "source commit is not in this clone (fetch full history)");
  git(
    ["merge-base", "--is-ancestor", expectations.sourceCommit, "HEAD"],
    "source commit is not an ancestor of the checked-out commit; only reviewed history is released",
  );
  const cargoLockSha256 = sha256Hex(
    execFileSync("git", ["show", `${expectations.sourceCommit}:Cargo.lock`], { cwd: repoRoot, maxBuffer: 64 * 1024 * 1024 }),
  );
  const sourceCommitTimestamp = git(["show", "-s", "--format=%cI", expectations.sourceCommit]).trim();
  const tagCommit = git(["rev-parse", "HEAD"]).trim();

  console.log(`Manifest:        ${manifestPath}`);
  console.log(`Source commit:   ${expectations.sourceCommit} (${sourceCommitTimestamp})`);
  console.log(`Cargo.lock:      ${cargoLockSha256}`);
  console.log(`Rust:            ${expectations.rustc} (${WASM_TARGET})`);
  console.log(`Stellar CLI:     ${expectations.stellarCli} (archive ${cliPin.sha256})`);
  console.log(`Build command:   ${BUILD_COMMAND.join(" ")}`);
  console.log(`Recorded:        ${Object.entries(expectations.artifacts).map(([name, a]) => `${name}=${a.sha256}`).join(", ")}`);
  console.log(`Reviewed file:   ${reviewed.length > 0 ? reviewedPath : "none"}`);

  if (args.artifactsDir !== undefined) {
    const verification = verifyArtifacts(expectations, collectWasm(resolve(args.artifactsDir)), reviewed);
    report(verification);
    if (!verification.ok) {
      process.exit(1);
    }
    console.log("Prebuilt artifacts verified. No release files were written (toolchain was not exercised).");
    return;
  }

  const rustcVersion = checkTool(
    "rustc",
    ["--version"],
    { RUSTUP_TOOLCHAIN: args.rustToolchain ?? expectations.rustc },
    `rustc ${expectations.rustc} `,
  );
  const stellarCliVersion = checkTool(args.stellarBin, ["version"], {}, `stellar ${expectations.stellarCli} `);

  if (args.dryRun) {
    console.log("Dry run: toolchain and manifest checks passed; nothing was built or written.");
    return;
  }

  const workDir = mkdtempSync(join(tmpdir(), "lumenbazaar-wasm-"));
  try {
    const sourceDir = join(workDir, "source");
    mkdirSync(sourceDir);
    const archive = join(workDir, "source.tar");
    execFileSync("git", ["archive", "--format=tar", "-o", archive, expectations.sourceCommit], { cwd: repoRoot });
    execFileSync("tar", ["-xf", archive, "-C", sourceDir]);
    // BUILD_COMMAND is `stellar contract build --locked`; only the binary path is configurable.
    execFileSync(args.stellarBin, BUILD_COMMAND.slice(1), {
      cwd: sourceDir,
      stdio: "inherit",
      env: {
        ...process.env,
        RUSTUP_TOOLCHAIN: args.rustToolchain ?? expectations.rustc,
        SOURCE_DATE_EPOCH: String(Math.floor(Date.parse(sourceCommitTimestamp) / 1000)),
      },
    });
    const releaseDir = join(sourceDir, "target", WASM_TARGET, "release");
    const built = collectWasm(releaseDir);
    const verification = verifyArtifacts(expectations, built, reviewed);
    report(verification);
    if (!verification.ok) {
      process.exit(1);
    }

    const outDir = resolve(args.outDir ?? join(repoRoot, "target", "testnet-wasm-release", basename(manifestPath, ".json")));
    const wasmDir = join(outDir, "wasm");
    mkdirSync(wasmDir, { recursive: true });
    for (const artifact of verification.published) {
      copyFileSync(join(releaseDir, artifact.name), join(wasmDir, artifact.name));
    }
    const checksums = buildChecksums({
      tag: args.tag,
      tagCommit,
      manifestPath,
      manifestSha256: sha256Hex(manifestBytes),
      expectations,
      sourceCommitTimestamp,
      cargoLockSha256,
      rustcVersion,
      stellarCliVersion,
      verification,
    });
    writeFileSync(join(outDir, "checksums.json"), checksums.json);
    writeFileSync(join(outDir, "SHA256SUMS"), checksums.sums);
    writeFileSync(join(outDir, "release-notes.md"), renderReleaseNotes(checksums));
    console.log(`Wrote ${outDir}: wasm/, checksums.json, SHA256SUMS, release-notes.md`);
  } finally {
    rmSync(workDir, { recursive: true, force: true });
  }
}

function collectWasm(dir) {
  if (!existsSync(dir)) {
    throw new Error(`${dir} does not exist`);
  }
  return readdirSync(dir)
    .filter((name) => name.endsWith(".wasm") && statSync(join(dir, name)).isFile())
    .sort()
    .map((name) => {
      const bytes = readFileSync(join(dir, name));
      return { name, sha256: sha256Hex(bytes), size: bytes.length };
    });
}

function report(verification) {
  for (const result of verification.results) {
    console.log(`  ${result.status.padEnd(18)} ${result.name} ${result.sha256 ?? "-"}${result.recordedSha256 && result.recordedSha256 !== result.sha256 ? ` (recorded ${result.recordedSha256})` : ""}`);
  }
  for (const error of verification.errors) {
    console.error(`reproduce-testnet-wasm: ${error}`);
  }
  console.log(`Verification: ${verification.verification}`);
}

function checkTool(command, toolArgs, env, expectedPrefix) {
  let output;
  try {
    output = execFileSync(command, toolArgs, { encoding: "utf8", env: { ...process.env, ...env } });
  } catch (error) {
    throw new Error(`cannot run ${command} ${toolArgs.join(" ")}: ${error.message}`);
  }
  const firstLine = output.split(/\r?\n/)[0];
  if (!firstLine.startsWith(expectedPrefix)) {
    throw new Error(
      `${command} reports "${firstLine}", expected "${expectedPrefix.trim()}"; a different toolchain would produce ` +
        "differently built artifacts, which are never published",
    );
  }
  return firstLine;
}

function git(gitArgs, failure) {
  try {
    return execFileSync("git", gitArgs, { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  } catch {
    throw new Error(failure ?? `git ${gitArgs.join(" ")} failed`);
  }
}

function parseArgs(argv) {
  const parsed = { stellarBin: "stellar", dryRun: false, plan: false };
  const valueFlags = {
    "--manifest": "manifest",
    "--tag": "tag",
    "--out-dir": "outDir",
    "--artifacts-dir": "artifactsDir",
    "--reviewed-mismatches": "reviewedMismatches",
    "--stellar-bin": "stellarBin",
    "--rust-toolchain": "rustToolchain",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--dry-run") {
      parsed.dryRun = true;
    } else if (flag === "--plan") {
      parsed.plan = true;
    } else if (valueFlags[flag] !== undefined) {
      const value = argv[index + 1];
      if (value === undefined || value.startsWith("--")) {
        throw new Error(`${flag} requires a value`);
      }
      parsed[valueFlags[flag]] = value;
      index += 1;
    } else {
      console.error(`reproduce-testnet-wasm: unknown argument ${flag}`);
      process.exit(1);
    }
  }
  if (parsed.manifest !== undefined) {
    parsed.manifest = parsed.manifest.replaceAll("\\", "/");
  }
  return parsed;
}
