# Contributing

LumenBazaar contracts are intentionally narrow. Put logic on-chain only when the
project needs contract-level enforcement.

## Local Checks

Install `stellar-cli` 25.2.0 or newer before building contract WASM files.

Run these commands before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
stellar contract build --locked
```

Enable the repository hook and run the dependency-free Python malware checks before pushing:

```bash
git config core.hooksPath .githooks
cargo security-malware-scan
cargo security-malware-test
```

## Contract-facing changelog gate

For changes to contract production source (including methods, errors and events),
generated specs/interface, bindings, deployment manifests or deployment scripts,
add a new entry under `## Unreleased`, categorized `### Added`, `### Changed`,
`### Fixed`, `### Security` or `### Breaking`. Each new entry states
`Compatibility: <impact>. Migration: <impact>.` Existing entries do not satisfy a
new PR. A Breaking entry must reference an existing migration document, e.g.
`Migration: [replacement routing](docs/replacement-routing.md)`. Use an actual
repository path in `docs/` or `artifacts/interface/migrations/`.

Run `node scripts/check-changelog.mjs --base origin/develop` after committing your
changes; CI uses the PR base SHA, including for forks and renamed/deleted files.
Test files, test directories and inline `#[cfg(test)]` items are exempt when
production code is unchanged. The gate conservatively treats other production
source edits as contract-facing; it does not replace review of semantic behavior.
If the generated ABI removes or changes public methods, errors, events or types,
CI requires a Breaking entry and migration reference.

## Scope

- Keep `upto-session` focused on capped metered payment sessions.
- Keep exact x402 payments contract-free where Stellar Asset Contract behavior
  and Soroban authorization entries are sufficient.
- Keep `policy-wallet-example` clearly marked as example code.
- Add regression tests for every authorization, cap, expiry, and settlement rule.
