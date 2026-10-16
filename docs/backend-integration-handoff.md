# Backend Integration Handoff

This document is the contract-side handoff for backend integration of capped `upto` payment sessions.

## Contract IDs

Contract IDs are recorded by deployment scripts:

- Local: `deployments/local.json`
- Testnet: `deployments/testnet.json`
- Published corrected v2 testnet: `deployments/testnet-2026-10-07.json`
- Testnet shape example: `deployments/testnet.example.json`

Backend environments should consume these values:

```txt
LUMENBAZAAR_UPTO_SESSION_CONTRACT_ID=C...
LUMENBAZAAR_TEST_TOKEN_CONTRACT_ID=C...
LUMENBAZAAR_POLICY_WALLET_CONTRACT_ID=C...
```

No production contract ID is committed in this repository yet.

Replacement deployments must store the originating `contract_id` with every
`session_id`. Switching or rolling back the active deployment changes only the
destination for new sessions; existing sessions always route to their recorded
contract. See `docs/replacement-deployment-rehearsal.md` and
`deployments/replacement-rehearsal.example.json`.

## Generated Interfaces

- TypeScript package: `bindings/upto-session`
- Canonical spec artifact: `artifacts/spec/upto-session.json`
- Current decoding fixture: `artifacts/interface/upto-session-v3.json`
- Retained v2 decoding fixture: `artifacts/interface/upto-session-v2.json`
- Binding regeneration: `node scripts/generate-bindings.mjs`
- Spec regeneration: `node scripts/generate-specs.mjs`

Backend services should prefer the generated TypeScript package for transaction construction and result decoding.

The escrow contract is version `0.3.0` and the generated package is version
`3.0.0`; `interface_version()` returns `3`. Existing lifecycle events retain
event version `2`; asset activation, asset-cap, and administrator-rotation
events use version `3`.

## Upto Session Functions

| Function | Backend use |
| --- | --- |
| `interface_version()` | Require result `3` before encoding v3 initialization, configuration, cap-policy, administrator-rotation, liability, or batch-recovery calls. |
| `initialize(admin, asset_policies)` | One-time deployment setup with one to 16 unique assets and positive explicit session caps. Every asset must pass the bounded read-only probe. Do not call from request handlers. |
| `get_asset_max_session_amount(asset)` | Read the active creation cap for a supported asset. |
| `set_asset_max_session_amount(asset, max_session_amount)` | Authenticated administrator update for future sessions only. |
| `get_admin()` | Read the active administrator. |
| `get_admin_rotation()` | Read the optional pending administrator and expiry ledger. |
| `propose_admin(pending_admin, expires_at_ledger)` | Current-admin-authorized proposal; does not transfer privileges. |
| `accept_admin()` | Pending-admin-authorized acceptance before expiry. |
| `cancel_admin_rotation()` | Current-admin-authorized removal of a pending or expired proposal. |
| `get_configuration()` | Read response/interface/storage versions, administrator, bounded supported assets, operational state, and instance TTL policy without authorization. |
| `create_session(buyer, seller, asset, max_amount, expires_at_ledger, resource_hash)` | Create a buyer-authorized spending cap and escrow the full cap for a resource. |
| `get_session(session_id)` | Read current session state for API responses, reconciliation, and settlement guards. |
| `get_escrow_liability(asset)` | Read aggregate open-session escrow accounting for a supported asset and compare it with contract token custody. |
| `settle(session_id, actual_amount, usage_hash)` | Seller-authorized settlement for actual usage. |
| `cancel(session_id)` | Buyer-authorized cancellation before settlement. |
| `recover_expired(session_id)` | Permissionless full refund to the stored buyer at or after expiry. |
| `recover_expired_batch(session_ids)` | Recover up to eight caller-supplied IDs with ordered per-item results. Missing, premature, terminal, and repeated IDs do not block eligible recoveries. |
| `extend_ttl(session_id)` | Extend storage TTL for still-open sessions. |

## V2 To V3 Compatibility

Version 3 is a new deployment, not an in-place upgrade. It replaces the v2
`initialize(admin, supported_assets)` payload with explicit `AssetPolicy`
values, adds cap query/update methods, adds two-step administrator rotation,
and exposes `get_escrow_liability(asset)`. It also adds `get_configuration`,
bounded batch recovery, a bounded initialization-order asset list plus
operational state, and supported-asset validation with activation events.
Existing v2 session methods, records, and event payloads retain their shapes.
The configuration response has its own `response_version` field, currently `1`,
so backend decoders can fail closed on future schema revisions.

`operational_state` can decode `Active`, `Paused`, or `Deprecated`. This release
initializes new contracts as `Active` and exposes no public state-transition
method; pause and deprecation authority remain outside this query-only change.
Backends must select the v3 fixture before invoking either new method. Version 3
also emits one `AssetActivated` event per accepted asset. Existing error codes
1 through 22 retain their names and values; codes 23 through 32 are additive.
Published v2 deployment records remain historical evidence and do not represent
a deployed v3 contract.

The probe reads balances for the administrator and escrow address, decimals,
name, and symbol. It rejects negative balances, decimals above 38, empty or
oversized metadata, missing methods, and reverting contracts. It never calls
`transfer` or `mint`. A successful probe is point-in-time compatibility
evidence, not a guarantee of future token behavior.

## V1 To V2 Compatibility

Version 2 is a new deployment, not an in-place upgrade. It changes
`initialize` by adding `supported_assets`, adds `escrowed_amount` and the
`Expired` status to session decoding, adds `recover_expired` and
`interface_version`, and replaces the old event payloads with versioned escrow
payloads. A backend must reject a contract whose `interface_version()` is not
`2`; decoding it with v2 types is not supported. Error codes 1 through 13 retain
their names and values, while codes 14 through 22 are additive. Version 3 adds
codes 23 through 32 without changing the earlier mappings.

## Stable Errors

| Code | Error | Backend handling |
| ---: | --- | --- |
| 1 | `AlreadyInitialized` | Treat setup as already complete; do not retry blindly. |
| 2 | `Unauthorized` | Return authorization failure to caller. |
| 3 | `InvalidAmount` | Reject request validation before submitting. |
| 4 | `ExpiredSession` | Stop settlement and ask caller to create a new session. |
| 5 | `SessionNotFound` | Return not found and trigger reconciliation if expected. |
| 6 | `SessionAlreadySettled` | Treat as finalized and fetch state. |
| 7 | `SessionCancelled` | Treat as finalized and do not settle. |
| 8 | `AmountExceedsCap` | Reject settlement above buyer cap. |
| 9 | `InvalidAsset` | Reject requests where asset equals buyer or seller. |
| 10 | `InvalidSeller` | Reject buyer-as-seller requests. |
| 11 | `TtlExtensionFailed` | Do not retry for finalized sessions. |
| 12 | `InvalidResourceHash` | Reject empty or malformed resource hash input. |
| 13 | `InvalidUsageHash` | Reject empty or malformed usage hash input. |
| 14 | `UnsupportedAsset` | Reject assets outside the immutable deployment allowlist. |
| 15 | `InvalidSupportedAssets` | Reject an empty or duplicate initialization allowlist. |
| 16 | `SessionDurationTooLong` | Reject an expiry more than 518,400 ledgers ahead. |
| 17 | `LiabilityOverflow` | Stop and investigate liability arithmetic or corrupted state. |
| 18 | `LiabilityUnderflow` | Stop and investigate liability arithmetic or corrupted state. |
| 19 | `EscrowUnderfunded` | Reject creation if token custody does not cover aggregate liability. |
| 20 | `NotInitialized` | Initialize a new deployment before creating sessions. |
| 21 | `SessionExpired` | Treat as finalized by expiry recovery. |
| 22 | `SessionNotExpired` | Do not recover before the exact expiry ledger. |
| 23 | `InvalidAssetCap` | Reject zero or negative configured session caps. |
| 24 | `AssetCapNotConfigured` | Stop creation and investigate incomplete asset policy state. |
| 25 | `InvalidAdmin` | Reject proposals that target the active administrator. |
| 26 | `InvalidAdminRotationExpiry` | Reject expired or excessively long proposal windows. |
| 27 | `AdminRotationPending` | Cancel or complete the existing proposal before creating another. |
| 28 | `AdminRotationNotFound` | Treat acceptance or cancellation as stale and refresh state. |
| 29 | `AdminRotationExpired` | Do not accept at or after the recorded expiry ledger. |
| 30 | `TokenInvariantViolation` | Stop processing and quarantine the configured asset; an observed transfer balance delta did not match the requested amount. |
| 31 | `InvalidBatchSize` | Supply between one and eight session IDs. |
| 32 | `InvalidAssetInterface` | Reject an asset whose read-only SEP-41 behavior or metadata is malformed. |

## Hash Inputs

`resource_hash` and `usage_hash` are both `BytesN<32>`.

Use SHA-256 over canonical UTF-8 JSON with sorted keys. Store the original JSON in backend records so API, worker, and audit flows can recompute the hash.

Example resource descriptor:

```json
{
  "asset": "C...",
  "method": "GET",
  "resourceId": "seller-api-weather-hourly",
  "seller": "G...",
  "unit": "request",
  "url": "https://api.example.test/weather/hourly"
}
```

Example usage receipt:

```json
{
  "meteredAtLedger": 123456,
  "quantity": 42,
  "requestId": "req_01HZYK7G7MN5P85V3QP4Q8Y9X2",
  "sessionId": "32-byte-session-id-hex",
  "unit": "request"
}
```

Example hashes are committed in `artifacts/backend-handoff/usage-hashes.example.json`.

## Settlement Guardrails

- Verify the session is `Open` before constructing settlement.
- Verify `actual_amount` is greater than zero and less than or equal to `max_amount`.
- Verify the current ledger is before `expires_at_ledger`.
- Verify backend seller identity matches `session.seller`.
- Record settlement attempt ID, `usage_hash`, ledger, transaction hash, and decoded result.
- Treat duplicate settlement attempts as idempotent by fetching `get_session` after `SessionAlreadySettled`.
- At or after expiry, call `recover_expired`; it requires no caller signature and can refund only the stored buyer.
- Batch recovery has partial semantics for invalid session IDs and returns one ordered result per input. Repeated IDs are marked `Duplicate` and cannot double-refund. A trapped token invocation aborts the transaction atomically.
