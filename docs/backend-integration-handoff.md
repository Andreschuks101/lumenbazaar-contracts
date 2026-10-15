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

## Generated Interfaces

- TypeScript package: `bindings/upto-session`
- Canonical spec artifact: `artifacts/spec/upto-session.json`
- Decoding fixture: `artifacts/interface/upto-session-v3.json`
- Binding regeneration: `node scripts/generate-bindings.mjs`
- Spec regeneration: `node scripts/generate-specs.mjs`

Backend services should prefer the generated TypeScript package for transaction construction and result decoding.

The current escrow contract and generated package are version `0.3.0`; the
public `interface_version()` result is `3`. Existing lifecycle events retain
event version `2`; asset-cap and administrator-rotation events use version `3`.

## Upto Session Functions

| Function | Backend use |
| --- | --- |
| `interface_version()` | Require result `3` before encoding v3 initialization, cap-policy calls, administrator rotation, or liability queries. |
| `initialize(admin, asset_policies)` | One-time deployment setup with a non-empty list of unique assets and positive explicit session caps. Do not call from request handlers. |
| `get_asset_max_session_amount(asset)` | Read the active creation cap for a supported asset. |
| `set_asset_max_session_amount(asset, max_session_amount)` | Authenticated administrator update for future sessions only. |
| `get_admin()` | Read the active administrator. |
| `get_admin_rotation()` | Read the optional pending administrator and expiry ledger. |
| `propose_admin(pending_admin, expires_at_ledger)` | Current-admin-authorized proposal; does not transfer privileges. |
| `accept_admin()` | Pending-admin-authorized acceptance before expiry. |
| `cancel_admin_rotation()` | Current-admin-authorized removal of a pending or expired proposal. |
| `create_session(buyer, seller, asset, max_amount, expires_at_ledger, resource_hash)` | Create a buyer-authorized spending cap and escrow the full cap for a resource. |
| `get_session(session_id)` | Read current session state for API responses, reconciliation, and settlement guards. |
| `get_escrow_liability(asset)` | Read aggregate open-session escrow accounting for a supported asset and compare it with contract token custody. |
| `settle(session_id, actual_amount, usage_hash)` | Seller-authorized settlement for actual usage. |
| `cancel(session_id)` | Buyer-authorized cancellation before settlement. |
| `recover_expired(session_id)` | Permissionless full refund to the stored buyer at or after expiry. |
| `extend_ttl(session_id)` | Extend storage TTL for still-open sessions. |

## V2 To V3 Compatibility

Version 3 is a new deployment, not an in-place upgrade. It replaces the v2
`initialize(admin, supported_assets)` payload with explicit `AssetPolicy`
values, adds cap query/update methods, adds two-step administrator rotation,
and exposes `get_escrow_liability(asset)`. A backend must reject a contract
whose `interface_version()` is not `3` before using v3 calls. Existing error
codes 1 through 22 retain their names and values; codes 23 through 29 are
additive. Published v2 deployment records remain historical evidence and do
not represent a deployed v3 contract.

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
