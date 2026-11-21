# Changelog

## Unreleased

### Added

- Add policy active-window status query (`get_policy_status`) in `policy-wallet-example`
  reporting whether a policy is pending, active, expired, or revoked at the current ledger
  sequence, with the active ledger window and next transition boundary.
  Compatibility: additive method and return types (`PolicyStatus`, `PolicyActiveWindowStatus`);
  existing authorization semantics are unchanged.
  Migration: none.

- Add typed transaction evidence helpers to the TypeScript binding
  (`createSessionWithEvidence`, `settleWithEvidence`, `cancelWithEvidence`,
  `extendTtlWithEvidence`) that sign through an injected signer, confirm through RPC and
  return confirmed, failed or pending evidence with RPC-reported hashes and ledgers.
  Compatibility: additive binding exports; generated client methods are unchanged.
  Migration: none.

- Add Rust-generated, versioned backend encoding compatibility vectors
  (`artifacts/backend-handoff/encoding-vectors.v1.json`) with a TypeScript verifier and a
  compatibility lock that requires a new version for any encoding change.
  Compatibility: test and handoff artifacts only; contract encoding is unchanged.
  Migration: none.

- Add a reproducible testnet WASM release workflow that rebuilds a deployment manifest's
  source commit with its recorded toolchain, verifies every recorded SHA-256 and publishes
  machine-readable checksums for signed `testnet-wasm-*` tags.
  Compatibility: release tooling only; deployment manifests are unchanged.
  Migration: none.

- Add bounded, read-only session queries with ordered found/missing results and preserved
  duplicate IDs. Compatibility: additive method and result types; existing session and
  error definitions are unchanged. Migration: none beyond the planned v3 release.
- Document the immutable replacement-deployment migration procedure, storage lifecycle and
  missing-state policy (rules SR-1 to SR-10 with boundary tests), policy-wallet invariant
  properties, and bounded fuzz targets with a seeded CI corpus. No contract source, interface,
  storage layout, or deployment manifest changes; `policy-wallet-example` gains a `proptest`
  dev-dependency only.
  Compatibility: none; tests and documentation only.
  Migration: none; replacements follow [deployment migration](docs/deployment-migration.md).

### Security

- Reject the LBT `test-token` in deployment manifests and release artifact lists under
  pubnet, production, unknown or missing network labels, and reject text that presents
  LBT as USDC. Compatibility: local and testnet manifests and tooling are unchanged.
  Migration: none.

### Breaking

- Reconcile the unreleased v3 generated interface with merged contract source, including
  administrative reason commitments and the completed feature definitions.
  Compatibility: breaking for consumers of stale unreleased v3 bindings; released v2
  deployments retain their original contract IDs and error meanings.
  Migration: [v2-to-v3 replacement routing](artifacts/interface/migrations/v2-to-v3.json).

### Fixed

- Bind `idempotencyKey` in the TypeScript `deriveSessionId` helper so it matches the
  contract's v3 session ID preimage. Compatibility: `SessionIdInputV3` now requires
  `idempotencyKey`; IDs derived without it never matched the contract. Migration: pass the
  32-byte idempotency key used for `create_session`.

- Optimize release WASM for size with link-time optimization, one codegen unit and
  stripped compiler symbols, retaining checked arithmetic and all contract APIs.
  Compatibility: contract behavior, storage and wire interfaces are unchanged.
  Migration: none; the existing WASM size limits remain unchanged.

- Repair SDK conformance test registration, authorization probes, and decimal fixtures,
  and preserve handwritten binding helpers and tests during generation.
  Compatibility: test and generation tooling only; contract behavior is unchanged.
  Migration: none.

- Refresh stale resource and WASM budgets from the restored merged-feature build,
  retaining existing percentage headroom and documenting every changed ceiling.
  Compatibility: no additional wire change; proposed budgets require maintainer review.
  Migration: none; see [budget proposal](docs/wasm-size-reviews/2026-10-22-merged-features.md).

- Repair interrupted merged contract definitions, restore interleaved regression tests,
  and assign unique codes to unreleased error additions while preserving published codes.
  Compatibility: existing checked-in error meanings retained; new feature errors are additive.
  Migration: none for existing deployments; see [validation repairs](docs/validation-repairs.md).

### Security

- Redact signer material and configured secret values from CLI diagnostics and dry-run logs.
  Compatibility: no contract interface change. Migration: none.

### Fixed

- Bound transaction finality polling by deadline and attempt budget, preserving distinct
  success, failure, expiry and timeout evidence. Compatibility: receipt schema unchanged.
  Migration: none.

### Added

- Check explicit release-version mappings across contract crates, bindings, interface,
  storage and deployment schema before tagging. Compatibility: tooling only. Migration: none.

- Require categorized release notes for contract-facing diffs and migration references for
  breaking ABI changes. Compatibility: tooling only. Migration: none.

- Require a positive maximum session amount for every supported asset.
- Enforce asset caps before buyer authorization and escrow transfer.
- Add authenticated cap updates, a public cap query, and versioned update events.
- Add two-step administrator proposal, acceptance, cancellation, expiry, and query behavior.
- Preserve the current administrator's exclusive privileges until explicit acceptance.
- Add a supported-asset escrow liability query for custody reconciliation.
- Require positive per-asset and per-buyer aggregate exposure ceilings, enforce them before buyer authorization, and release them on every terminal path.
- Add a read-only, authorization-free per-asset solvency snapshot query.
- Add owner-authorized policy revocation to the example policy wallet.
- Add branch, review, ownership, release, and emergency-change governance.
- Generate sanitized, deterministic public deployment manifests from network receipts with an
  allowlist and a pre-write secret scan.
- Publish live on-ledger wrong-seller, wrong-buyer, over-cap, and expiry-boundary rejection evidence
  on testnet.
- Verify every published testnet transaction against Horizon in a scheduled, PR-independent workflow.
- Add admin-authorized `activate_asset` for onboarding a reviewed asset after initialization, with errors 36 to 38 for duplicate, deprecated, and over-limit assets.
- Make `remove_asset` delete the asset's cap, zero liability record, and entry in the configured asset order; hide stale entries from earlier removals.
- Add the read-only, versioned `get_asset_policy` lifecycle query.
- Add typed TypeScript contract error decoding (`decodeContractError`, `CONTRACT_ERROR_CODES`) with a generation-time drift check against the interface fixture.
- Add deterministic ledger-boundary tests for config, sequence, session, and liability TTL and document the boundary semantics.
- Add state-transition and cap-invariant property tests with documented reproducible seeds.
- Add an opt-in live testnet lifecycle smoke runner that emits a sanitized JSON report, plus an offline test suite and report checker.

## v0.2.0-testnet.20261007

- Publish escrow-funded `upto` session version 2 with cancellation and expiry recovery.
- Publish reproducible WASM, generated bindings/interface, deployment evidence, and lifecycle proof.
