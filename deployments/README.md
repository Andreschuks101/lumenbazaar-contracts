# Deployments

Deployment manifests are written here by the repo scripts.

- `local.json` is ignored and belongs to a developer machine.
- `testnet.json` is ignored until the project chooses to publish a specific deployment.
- Example manifests may be committed when they document the expected shape.
- `testnet-2026-09-06.json` is the sanitized public evidence for the first testnet deployment. It
  contains no signing material and does not claim a live payment flow.
- `testnet-2026-10-07.json` is the immutable corrected v2 deployment evidence. It records the
  reviewed source commit, dedicated public admin, exact toolchain, WASM and interface hashes, and
  upload/deploy/initialize transactions without replacing the September record.
- `testnet-v2-supported-assets.json` is the public initialization input for the corrected escrow
  deployment.
- `testnet-2026-10-20.json` is the first generated manifest: the interface v3 `upto-session` and
  LBT `test-token` deployment used for the live rejection evidence in
  `artifacts/testnet/live-evidence-2026-10-20.json`. It is evidence infrastructure, not a release.

For testnet:

Deployment is fail closed. Set `LUMENBAZAAR_EXPECTED_WASM_HASHES` to a JSON
map of contract names to reviewed lowercase SHA-256 values. Set
`LUMENBAZAAR_TRUSTED_RPC_URL` to an independent HTTPS testnet RPC endpoint and
optionally `LUMENBAZAAR_MAX_RPC_LEDGER_LAG` (default `2`). A `--dry-run` writes
`testnet-plan.json` without signing or submitting. The canonical `planSha256`
is copied into the final manifest so reviewers can compare plan and evidence.

1. Copy `.env.testnet.example` to `.env.testnet` or export the same variables in your shell.
2. Run `node scripts/testnet/deploy-testnet.mjs` (optionally `--contracts upto-session,test-token`).
3. Run `node scripts/testnet/initialize-testnet.mjs`.
4. Run `node scripts/testnet/verify-testnet.mjs`.
5. Share the resulting `deployments/testnet.json` with backend operators through the agreed secure handoff channel.
6. Publish the sanitized record with `node scripts/testnet/generate-public-manifest.mjs`.

Validate committed public evidence with `node scripts/check-deployment-evidence.mjs`.

## Generating public manifests

The deploy and initialize scripts record every transaction hash in the ignored
`deployments/testnet.json` and save the network's own receipt for each transaction under the
ignored `deployments/receipts/testnet/<hash>.json`. A receipt holds only public fields from RPC
`getTransaction`: status, ledger, the ledger close time, and the source account. Public RPC keeps
transactions for a limited window, so receipts are captured at deployment time.

```bash
node scripts/testnet/generate-public-manifest.mjs                 # deployments/testnet-YYYY-MM-DD.json
node scripts/testnet/generate-public-manifest.mjs --kind evidence # artifacts/testnet/live-evidence-YYYY-MM-DD.json
node scripts/testnet/generate-public-manifest.mjs --dry-run       # validate only, write nothing
```

The generator copies an allowlist of public fields. Ledgers and timestamps come only from receipts,
never from local clocks or local manifest values, and the file date is the earliest receipt close
time. Keys are sorted and the output ends with a newline, so regenerating from the same inputs is
byte-identical; an existing file with different content is never overwritten.

It exits non-zero and writes nothing when:

- the input network is not `testnet`, or a receipt is not a testnet receipt for that transaction;
- any recorded transaction has no receipt, a deployment transaction did not succeed, or a
  transaction was signed by an account other than the recorded deployer (upload, deploy) or admin
  (initialize);
- the input has a field outside the allowlist. Known local-only fields (`alias`, `name`, `network`,
  `wasm`, `generatedAt`, `localAliases`) are dropped and listed; anything else is reported by path;
- secret-pattern validation matches. The scan runs before writing and reports the field path and
  rule, never the value. It rejects Stellar secret seeds, BIP-39-style mnemonic word runs, 64-hex
  values outside hash fields, absolute local paths (`/home/`, `/Users/`, drive letters), `.stellar`
  and `.soroban` paths, long base64 XDR envelopes, configured identity aliases (manifest aliases,
  `LUMENBAZAAR_*`/`STELLAR_ACCOUNT` variables, and `stellar keys ls`), and the repository's
  `lumenbazaar-*-deployer/admin/buyer/seller` alias naming convention;
- the result fails the same validation as `scripts/check-deployment-evidence.mjs`, which the
  generator also runs after writing.

When the CLI skips a WASM upload because another account already installed it, the deploy script
submits the install transaction explicitly so the published upload is signed by the deployer.

## Live rejection evidence

`scripts/testnet/smoke-testnet.mjs` records the live cases for wrong signers, over-cap settlement,
and the expiry boundary against a published deployment manifest. It needs five funded local
identities, created for example with `stellar keys generate <name> --network testnet --fund`, which
stay in the CLI's ignored configuration directory:

```bash
LUMENBAZAAR_ADMIN=<token admin> LUMENBAZAAR_BUYER=<buyer> LUMENBAZAAR_SELLER=<seller> \
LUMENBAZAAR_WRONG_SELLER=<other account> LUMENBAZAAR_WRONG_BUYER=<other account> \
  node scripts/testnet/smoke-testnet.mjs --deployment-manifest deployments/testnet-YYYY-MM-DD.json
node scripts/testnet/generate-public-manifest.mjs --kind evidence
```

Rejected attempts are submitted as real transactions so the network, not the CLI, rejects them.
Each is built without simulation, given the resource footprint of a valid sibling simulation, and
signed by the attempting account. Expiry cases poll RPC `getLatestLedger` with a bounded timeout
instead of sleeping. A case that could only be observed in simulation must be labelled
`simulation-only` with no transaction hash; the generator and checker reject anything else.

## Scheduled Horizon verification

`.github/workflows/horizon-evidence.yml` runs daily and on manual dispatch, never on pull requests,
with read-only permissions and no secrets. It runs `node scripts/verify-horizon-evidence.mjs`, which
resolves every transaction in `deployments/testnet-*.json`,
`artifacts/testnet/lifecycle-*.json`, and `artifacts/testnet/live-evidence-*.json` through Horizon
`/transactions/{hash}` and compares the ledger, close time, source account, and success flag with the
published expected outcome. Rejections published as on-ledger failures must verify as failed.
Simulation-only rejections have no transaction and are counted separately.

Reading a result:

- **Exit 0, VERIFIED:** every published transaction matches Horizon.
- **Exit 1, MISMATCH (job fails):** Horizon contradicts the evidence, for example a missing
  transaction, a different ledger or signer, or a success flag that differs from the expected
  outcome. Treat this as an evidence-integrity failure. The summary table lists each operation with
  an explorer link, and the manifest links point at the tested commit. A testnet reset also appears
  here as missing transactions.
- **Exit 2, OUTAGE (warning only):** Horizon timed out or returned 5xx/429 after one attempt plus
  three retries with backoff. Nothing was contradicted; rerun the workflow.

Pull-request CI never contacts testnet. It runs the offline generator and verifier tests, which
cover mismatches and outages against fixtures and a loopback server.

## Replacement deployments

Contracts here are never upgraded in place. A replacement is a new deployment with a new contract
ID, recorded as a new dated manifest; earlier manifests stay unchanged as evidence and are never
edited to point at the replacement. Contract IDs are never reused. Follow
[docs/deployment-migration.md](../docs/deployment-migration.md) for the rollout, registry
versioning, cutover order, open-session drain, rollback, and deprecation windows.

`replacement-rehearsal.example.json` documents the client routing state used by
the executable local replacement rehearsal. Live rehearsal evidence is written
under ignored `target/rehearsals/`; it is local evidence, not a public deployment
claim.

`scripts/testnet/smoke-lifecycle.mjs` is an opt-in runner that submits a bounded live lifecycle and
writes a sanitized report under ignored `target/testnet-smoke/`. See `docs/testnet-lifecycle-smoke.md`.
Reports are not committed unless the project chooses to publish one as evidence.

The exact build, deployment, and independent verification commands for the corrected deployment
are in `docs/testnet-deployment-2026-10-07.md`.

## Test-token release guard

LBT (`test-token`) is a custom test-only token. It is not USDC and is never part of a production
release. `node scripts/check-test-token-guard.mjs` enforces this:

- It scans every committed manifest under `deployments/` (except ignored receipts) and
  `artifacts/testnet/`, plus the repository README files and `docs/`.
- A manifest that references the test token (a `test-token`/`test_token` key, a
  `test_token.wasm` artifact, or the `LBT` asset code) passes only when every network or
  environment label in it is allowlisted: `local`, `standalone`, `testnet`, `Test SDF Network`,
  or the testnet or standalone passphrase. Labels are read from `environment`, `env`, `network`,
  `networkName`, `networkPassphrase`, `passphrase`, `label`, `releaseLabel`, `stage`, `tier`, and
  `name`/`passphrase` inside a `network` object.
- Matching ignores case and collapses whitespace. `pubnet`, `public`, `mainnet`, `production`,
  `prod` and the public network passphrase are rejected. Any other label, and a missing label,
  is treated as production and rejected (fail closed).
- A sentence that mentions LBT together with USDC or a production asset must negate the claim
  (for example "LBT is not USDC"); otherwise it is rejected.
- Errors name the file, the JSON path and the label, state that LBT is a test-only token and not
  USDC, and say to remove `test-token` from the production manifest or correct the label.

```bash
node scripts/check-test-token-guard.mjs                                   # repository scan (CI)
node scripts/check-test-token-guard.mjs --manifest deployments/<file>.json
node scripts/check-test-token-guard.mjs --release-label testnet --artifacts-dir <dir>
node --test scripts/lib/test-token-guard.test.mjs
```

The guard never removes local or testnet tooling: local manifests, testnet manifests, tests and
`scripts/local/deploy-test-token-local.mjs` keep using LBT. `artifacts/interface/release-plan.json`
is not scanned because it only maps crate versions and names no target network. Release
packaging is checked with `--release-label` by the workflow below.

## Testnet WASM releases

`.github/workflows/release-testnet-wasm.yml` publishes reproducible checksums for the WASM recorded
in one immutable deployment manifest. A release tag `testnet-wasm-YYYY-MM-DD` reproduces
`deployments/testnet-YYYY-MM-DD.json`. Manual dispatch runs the same build and verification as a dry
run and publishes nothing.

The workflow:

1. Verifies that the tag is annotated, points at the checked-out commit and has a signature that
   GitHub verified (`scripts/check-release-tag.mjs`). Otherwise it stops before building.
2. Reads the manifest's `sourceCommit`, `toolchain.rustc` and `toolchain.stellarCli`, and requires
   the source commit to be an ancestor of the tagged commit.
3. Installs exactly that Rust version and the Stellar CLI archive whose SHA-256 is pinned in
   `scripts/lib/wasm-release.mjs`. Unpinned CLI versions are refused.
4. Exports the source commit with `git archive` and runs `stellar contract build --locked` against
   that commit's own `Cargo.lock` and release profile. No other optimization step is added.
5. Compares the SHA-256 of every built WASM with `artifactSha256` in the manifest. The manifest is
   never edited. A mismatch, a missing artifact, or a built artifact that the manifest does not
   record fails the workflow, unless a reviewed explanation covers that exact artifact and hash.
6. Runs the test-token guard on the manifest and the artifact list with the `testnet` label.
7. Only for a verified tag, with `contents: write` limited to the publish job: creates a GitHub
   prerelease titled "TESTNET WASM checksums ... (not a production release)" with
   `checksums.json`, `SHA256SUMS` and the verified WASM files.

`checksums.json` records each artifact's name, contract, SHA-256, size, recorded hash and verification
status, plus the tag, tagged commit, manifest path and hash, source commit and its timestamp,
Cargo.lock SHA-256, Rust and Stellar CLI versions (including the CLI archive hash), the build command
and the verification result. It records no wall-clock time. `test_token.wasm` is labelled as the LBT
test-only token, not USDC.

All workflow actions are pinned to full commit SHAs. Permissions default to `contents: read`, and the
only token used is the built-in `GITHUB_TOKEN`.

### Toolchains for the recorded deployments

| Manifest | Source commit | Rust | Stellar CLI |
| --- | --- | --- | --- |
| `testnet-2026-09-06.json` | `95ea3270b8a5cb052fee5dd9402ed19dbd5a9001` | 1.93.1 | 27.1.0 |
| `testnet-2026-10-07.json` | `41940839d8f49c733b1815e108fc9740b1285f32` | 1.93.1 | 28.0.0 |
| `testnet-2026-10-20.json` | `a8ac359ae155a85b5d71f16d5476aa26d3310cd3` | 1.93.1 | 28.0.0 |

Commit `95ea327` pins `stable` in its `rust-toolchain.toml`, so the release overrides it with the
recorded rustc 1.93.1 through `RUSTUP_TOOLCHAIN`. `testnet-2026-10-20.json` records two of the three
contracts that its source commit builds, so releasing it needs a reviewed
`excluded-unrecorded-artifact` entry for `policy_wallet_example.wasm`.

### Verifying a testnet WASM release

On a clean x86_64 Linux machine with Node 22.12.0, rustup and the recorded toolchain:

```bash
git clone https://github.com/LumenRoute/lumenbazaar-contracts && cd lumenbazaar-contracts
git checkout testnet-wasm-2026-09-06 && git tag -v testnet-wasm-2026-09-06
rustup toolchain install 1.93.1 --profile minimal --target wasm32v1-none
curl -fsSLO https://github.com/stellar/stellar-cli/releases/download/v27.1.0/stellar-cli-27.1.0-x86_64-unknown-linux-gnu.tar.gz
echo "9915fe63753d7154c26bb9e6410cf1d1ff9727a9b378e663c2790ab7a266af94  stellar-cli-27.1.0-x86_64-unknown-linux-gnu.tar.gz" | sha256sum --check
tar -xzf stellar-cli-27.1.0-x86_64-unknown-linux-gnu.tar.gz && export PATH="$PWD:$PATH"

node scripts/reproduce-testnet-wasm.mjs --tag testnet-wasm-2026-09-06 --dry-run   # toolchain checks only
node scripts/reproduce-testnet-wasm.mjs --tag testnet-wasm-2026-09-06 --out-dir /tmp/release
(cd /tmp/release/wasm && sha256sum --check ../SHA256SUMS)
```

To check downloaded release files without rebuilding, run
`node scripts/reproduce-testnet-wasm.mjs --manifest deployments/testnet-2026-09-06.json --artifacts-dir <dir>`.
Compare the reproduced hashes with the release's `SHA256SUMS`, the manifest, and the on-chain hash
from `stellar contract info hash --id <contract> --network testnet`. The release logic tests run with
`node --test scripts/lib/wasm-release.test.mjs`. Only a real tag-triggered run on GitHub proves the
signed-tag check and the publish step end to end.

### Reviewed mismatches

When a clean build cannot reproduce a recorded hash, the release fails. Publishing anyway requires a
reviewed explanation committed at `deployments/reviewed-mismatches/<manifest file name>` (format in
`deployments/reviewed-mismatches/README.md`). Each entry names one artifact and the exact recorded and
reproduced hashes, gives a reason, the reviewer's GitHub handle, the review date and the reviewing pull
request. A mismatch is never accepted automatically, an entry with different hashes does not apply, and
an entry that does not apply to the build fails the release. Built artifacts that the manifest does not
record can only be excluded by an `excluded-unrecorded-artifact` entry, and excluded artifacts are never
published.

### Tag policy

Maintainers create release tags only after the manifest and any reviewed mismatch file are merged:

```bash
git switch develop && git pull --ff-only
git tag -s testnet-wasm-2026-09-06 -m "Testnet WASM checksums for deployments/testnet-2026-09-06.json"
git tag -v testnet-wasm-2026-09-06
git push origin testnet-wasm-2026-09-06
```

The tag must be annotated and signed (`-s`) with a GPG, SSH or S/MIME key registered on the tagger's
GitHub account, so that GitHub reports it as verified. Lightweight, unsigned or unverified tags fail the
release before anything is built. Never move or re-create a release tag. Protecting `testnet-wasm-*`
tags with a repository ruleset (no updates or deletions) is recommended as well. The workflow cannot
read rulesets with `GITHUB_TOKEN`, so the signature check is the enforced control.

## Deployment preflight

Run `node scripts/testnet/preflight-testnet.mjs` (optionally `--dry-run`, `--contracts <list>`, `--json`) before deploying. It is read-only: it never submits transactions or funds accounts. `deploy-testnet.mjs` runs the same checks first and exits 1 on failure. Output is redacted.

| Category | Checks |
| --- | --- |
| `[CONFIGURATION]` | `STELLAR_NETWORK=testnet`, source account (`LUMENBAZAAR_SOURCE_ACCOUNT` or `STELLAR_ACCOUNT`), `LUMENBAZAAR_EXPECTED_WASM_HASHES` (lowercase SHA-256 per contract), HTTPS `LUMENBAZAAR_TRUSTED_RPC_URL` distinct from the primary RPC, optional `LUMENBAZAAR_MIN_SOURCE_BALANCE_XLM` (default 10) |
| `[NETWORK]` | Primary RPC reachable, testnet passphrase, ledger lag against the trusted RPC within `LUMENBAZAAR_MAX_RPC_LEDGER_LAG` |
| `[FUNDING]` | Source account exists on testnet and holds at least the minimum XLM |
| `[ARTIFACTS]` | Each WASM in `target/wasm32v1-none/release/` exists and matches the reviewed SHA-256 |

`--dry-run` reports synthetic network, funding and artifact results and needs no network or built artifacts.
