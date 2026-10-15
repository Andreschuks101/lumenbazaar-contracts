# Audit Readiness

This package describes the current review boundary for LumenBazaar contracts.

## Public Interface Freeze

The review target is the `upto-session` contract public interface:

- `interface_version()` -> `3`
- `initialize(admin, supported_assets)`
- `create_session(buyer, seller, asset, max_amount, expires_at_ledger, resource_hash)`
- `get_session(session_id)`
- `settle(session_id, actual_amount, usage_hash)`
- `cancel(session_id)`
- `recover_expired(session_id)`
- `extend_ttl(session_id)`

The frozen interface evidence is stored in `artifacts/audit-readiness/interface-freeze.json`. It hashes the public contract files, generated XDR spec, versioned TypeScript binding, and the complete public event/error fixture at `artifacts/interface/upto-session-v3.json`.

Regenerate only after an intentional interface change:

```bash
node scripts/generate-audit-freeze.mjs
node scripts/generate-interface-fixtures.mjs --check
node scripts/check-audit-readiness.mjs
```

## Known Limitations

- No mainnet deployment is approved by this repository.
- Corrected v2 testnet IDs and lifecycle evidence are published, but testnet evidence is not a
  production or mainnet approval.
- Local SDK resource usage is a regression signal, not a replacement for target-network RPC simulation.
- `policy-wallet-example` is example-only smart account policy code and is not production wallet infrastructure.
- `test-token` is utility code for local and testnet testing and is not a production asset.
- Canonical receipt payload storage remains a backend responsibility; only domain-separated hashes belong on-chain.
- V2 identifiers are not reinterpreted by v3; clients must route them to their original deployment.

## Threat Model

Protected assets:

- Buyer token balances covered by capped sessions.
- Seller settlement rights for actual usage.
- Session state, expiry, and finalization status.
- Backend hash records used to prove resource and usage details.

Trusted actors:

- Buyer signs session creation and cancellation.
- Seller signs settlement.
- Deployment admin initializes contracts.
- Backend records canonical resource and usage payloads.

Threats covered by current tests:

- Unauthorized creation, settlement, cancellation, or initialization.
- Settlement above cap.
- Double settlement or settlement after cancellation.
- Settlement after expiry.
- Invalid seller or asset binding.
- Event topic or payload drift.

Out of scope until later review:

- Production smart wallet custody.
- Mainnet operational signing.
- Backend replay protection outside contract state.
- Marketplace policy enforcement outside the `upto-session` contract.

## Test Coverage

The current workspace test suite covers:

- `upto-session`: 69 tests, including a 128-case deterministic property campaign.
- `policy-wallet-example`: 16 tests.
- `test-token`: 5 tests.

Primary evidence:

- `docs/security-checklist.md`
- `artifacts/resource-usage/upto-session.md`
- `docs/backend-integration-handoff.md`
- `docs/release-toolchain.md`
- `deployments/testnet-2026-10-07.json`
- `artifacts/testnet/lifecycle-2026-10-07.json`

## Deployment Procedure

Before deployment:

1. Confirm every version in `docs/release-toolchain.md`.
2. Run `node scripts/check-dependency-exceptions.mjs`.
3. Run `cargo audit --deny warnings --ignore RUSTSEC-2024-0436`.
4. Run `cargo fmt --all -- --check`.
5. Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
6. Run `cargo test --workspace --all-features`.
7. Run `stellar contract build --locked`.
8. Run `node scripts/generate-specs.mjs --skip-build` and check no spec drift.
9. Run `node scripts/generate-bindings.mjs --skip-build` and check no binding drift.
10. Run `npm --prefix bindings/upto-session audit --omit=dev`.
11. Run `node scripts/check-backend-handoff.mjs`.
12. Run `node scripts/check-security-checklist.mjs`.
13. Run `node scripts/check-audit-readiness.mjs`.

Local deployment:

```bash
node scripts/local/deploy-local.mjs
node scripts/local/initialize-local.mjs
```

Testnet deployment:

```bash
node scripts/testnet/deploy-testnet.mjs
node scripts/testnet/initialize-testnet.mjs
node scripts/testnet/verify-testnet.mjs
```

Mainnet deployment is intentionally not scripted as an automatic path in this review package.
