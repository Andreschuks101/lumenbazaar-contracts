# Release Toolchain

## Release version mapping

`artifacts/interface/release-plan.json` records the previous release, intended
release class, and exact independently versioned artifacts. Run
`node scripts/check-release-versions.mjs` before release preparation; CI runs it
on PRs and tags. It checks contract manifests and Cargo.lock, interface and storage
constants, binding package/lock/generator versions, public deployment schema, ABI
classification, migration evidence and changelog presence, with expected/actual
diffs. Historical deployment manifests remain immutable release evidence.

Mapping policy:

- Patch: next crate patch and binding patch; interface/storage remain stable; no new ABI.
- Additive: next crate minor and binding minor; interface/storage remain stable; no breaking ABI.
- Migration: next crate major (next minor before 1.0), binding major, next interface
  version, and matching migration evidence. Storage stays stable or advances once.
- Auxiliary contract crates may stay stable or take one reviewed semver bump.
- Deployment schema stays stable or advances once independently; it does not follow
  crate/interface numbers. Current mapping is crate 0.3.0, interface/storage 3,
  bindings 3.0.0 and deployment schema 2.

Before creating a tag, finalize its exact `## v...` changelog heading and run
`node scripts/check-release-versions.mjs --tag <tag>`. Tag CI reruns this gate;
an Unreleased heading alone is insufficient for a tag. Changing the plan is a
reviewed release decision, not an automatic way to bless arbitrary version drift.

## Deployment logging

CLI subprocess output is piped through `scripts/lib/deployment-logging.mjs` before
logging. Secret-bearing environment variables (SECRET, PRIVATE_KEY, MNEMONIC,
SEED, PASSWORD, TOKEN) and explicit signer values are redacted, including JSON,
URL, and base64 encodings. Never pass signer material as an unconfigured argument.
Raw captured stdout is for internal parsing only; do not log it directly. Public
account IDs, transaction hashes and contract error codes remain diagnostic evidence.
Long word sequences are conservatively treated as possible mnemonic phrases.

## Transaction finality

`pollTransaction` in `scripts/lib/transaction-finality.mjs` is the shared finality
definition used by `fetchReceipt`, and therefore deployment, initialization and
receipt verification. Submission acceptance is not success. NOT_FOUND/PENDING
are nonterminal; SUCCESS/FAILED require an RPC ledger and close timestamp.
EXPIRED requires an explicit inclusive `maxLedger` and a later RPC-observed ledger;
without that evidence the outcome is TIMED_OUT, never guessed expiry. Timeout
and expiry errors retain a structured `.outcome` containing hash, last ledger,
elapsed monotonic time and poll count. RPC errors remain distinct protocol or
transport errors. A poll has both a deadline (120 seconds by default) and an
attempt cap (121); each RPC request receives the remaining time budget. Success
returns network ledger/time and result metadata. Receipt files keep schema 1.
Tests inject RPC fixtures, clock and sleep without making network transactions.

Contract release and audit evidence must use the following exact toolchain:

For credential-free local reproduction, build `Dockerfile.reproducible` and
run it with no secret environment variables or credential mounts. Its default
command performs two clean builds and compares both WASM hashes and generated
contract specs:

```bash
docker build -f Dockerfile.reproducible -t lumenbazaar-repro .
docker run --rm --network=none lumenbazaar-repro
```

| Component | Version | Enforcement |
| --- | --- | --- |
| Rust | 1.93.1 | `rust-toolchain.toml` and CI |
| WASM target | `wasm32v1-none` from Rust 1.93.1 | `rust-toolchain.toml` and CI |
| Soroban SDK | 28.0.0 | Workspace dependency and `Cargo.lock` |
| Stellar CLI | 28.0.0 | CI release archive |
| Node.js | 22.12.0 | CI setup |
| cargo-audit | 0.22.2 | CI install command |
| generated Stellar SDK | 17.2.1 | Binding generator output |

Historical deployments keep the toolchain they recorded. The testnet WASM release workflow
(`.github/workflows/release-testnet-wasm.yml`) rebuilds a manifest's source commit with the Rust
and Stellar CLI versions in that manifest, from archives pinned in `scripts/lib/wasm-release.mjs`.
See "Testnet WASM releases" in `deployments/README.md`.

## Dependency Audit Policy

CI runs `cargo audit --deny warnings` and `npm audit --omit=dev`. A Rust advisory may be ignored only when it has a matching record in `audits/dependency-exceptions.json` with an owner, dependency path, risk statement, planned resolution, and future expiry date. The exception checker fails on expired, duplicate, malformed, or incomplete records.

The current `RUSTSEC-2024-0436` exception covers the unmaintained `paste` crate inherited through released Soroban host-side dependencies. It is not a vulnerability report and the package is absent from deployed contract WASM. Contracts maintainers must remove the ignore as soon as a compatible Soroban release removes the dependency, and no later than the recorded recheck date without a fresh review.

The generated TypeScript binding pins Stellar SDK 17.2.1 because the CLI-generated 16.x range resolves Axios versions with published high-severity advisories. Regeneration reapplies the reviewed version so the package cannot silently regress.

## Release SBOM and Build Provenance

`node scripts/generate-provenance.mjs` runs after `stellar contract build` and writes
two machine-readable artifacts under `artifacts/provenance/`:

- `sbom.cdx.json` — a CycloneDX 1.5 document listing every crate in `Cargo.lock`
  (first-party workspace crates plus transitive registry crates) with version,
  registry `purl`, and the recorded registry checksum.
- `build-provenance.json` — binds each released WASM to the source commit, the
  `Cargo.lock` digest, the pinned toolchain identity, and the artifact SHA-256 and
  size.

`node scripts/check-provenance.mjs` recomputes every digest from the checked-out
source, the pinned `rust-toolchain.toml`, and the built WASM, and exits non-zero when
an artifact is missing, a checksum or size drifts, the lockfile or toolchain no longer
matches, a transitive crate is dropped from the SBOM, or a built WASM has no
provenance entry. Verification is fully offline and requires no private service.

CI runs the provenance unit tests and both scripts on every pull request and release
build, and uploads `artifacts/provenance` for reviewers. Provenance is supply-chain
evidence that a released artifact matches a reviewed source and toolchain; it does not
replace code review or an external security audit.

## Offline release signatures

Release checksum and provenance metadata are accepted only with Ed25519
signatures from active identities in `release-signers.json`. The registry pins
public keys, validity windows, and revocation state; unknown, expired, or
revoked keys fail closed. Tag workflows verify committed detached signature
metadata under `release-signatures/<tag>/` without contacting a key server.
Private keys are never stored or passed to CI. Maintainers sign the exact file
bytes offline, recording `keyId`, `algorithm`, `artifactSha256`, and a base64
signature in the detached JSON file.

## Resource and Rent Regression Guardrails

`node scripts/report-resource-usage.mjs` runs one native invocation of each lifecycle method
(`create_session`, `settle`, `cancel`, `recover_expired`, `extend_ttl`) and records the Soroban
cost estimate for each. The report separates work that does not touch ledger entries (instruction
and memory counters) from ledger footprint and TTL cost (`write_bytes`, `disk_read_bytes`,
`contract_events_size_bytes`, persistent and temporary rent ledger bytes and bump counts, and the
estimated fee), so a compute regression and a storage/rent regression are distinguishable.

`artifacts/resource-usage/thresholds.json` records the reviewed ceiling for every measured field.
`node scripts/check-resource-budgets.mjs` fails when a lifecycle method is missing from the report,
a measured field is absent, or a field exceeds its reviewed ceiling. Ceilings are the reviewed
native baseline plus headroom; update them only when a change to the measured behaviour is
intentional and reviewed, and record the reviewer, date, and rationale in the file.

Native estimates are deterministic for the pinned SDK but they are **not** a deployment quote:
they measure local host execution, not RPC simulation or live ledger rent on the target network.
Use them for regression tracking, and run RPC simulation before production fee budgeting. The
scheduled native/RPC differential in `docs/native-rpc-differential.md` is the complementary check.
