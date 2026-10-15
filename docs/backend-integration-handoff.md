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
- Decoding fixture: `artifacts/interface/upto-session-v2.json`
- Binding regeneration: `node scripts/generate-bindings.mjs`
- Spec regeneration: `node scripts/generate-specs.mjs`

Backend services should prefer the generated TypeScript package for transaction construction and result decoding.

The corrected escrow contract and generated package are version `0.2.0`; the
public `interface_version()` result and every public event use interface/event
version `2`.

## Upto Session Functions

| Function | Backend use |
| --- | --- |
| `interface_version()` | Require result `2` before decoding v2 session state or events. |
| `initialize(admin, supported_assets)` | One-time deployment setup with an immutable non-empty token allowlist. Do not call from request handlers. |
| `create_session(buyer, seller, asset, max_amount, expires_at_ledger, resource_hash)` | Create a buyer-authorized spending cap and escrow the full cap for a resource. |
| `get_session(session_id)` | Read current session state for API responses, reconciliation, and settlement guards. |
| `settle(session_id, actual_amount, usage_hash)` | Seller-authorized settlement for actual usage. |
| `cancel(session_id)` | Buyer-authorized cancellation before settlement. |
| `recover_expired(session_id)` | Permissionless full refund to the stored buyer at or after expiry. |
| `extend_ttl(session_id)` | Extend storage TTL for still-open sessions. |

## V1 To V2 Compatibility

Version 2 is a new deployment, not an in-place upgrade. It changes
`initialize` by adding `supported_assets`, adds `escrowed_amount` and the
`Expired` status to session decoding, adds `recover_expired` and
`interface_version`, and replaces the old event payloads with versioned escrow
payloads. A backend must reject a contract whose `interface_version()` is not
`2`; decoding it with v2 types is not supported. Error codes 1 through 13 retain
their names and values, while codes 14 through 22 are additive.

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

## Hash Inputs

`resource_hash` and `usage_hash` are both `BytesN<32>`.

Usage commitments use the versioned schema in
`artifacts/backend-handoff/usage-receipt-v1.schema.json`. Encode exactly these
fields in this order with no whitespace or trailing newline:

1. `schemaVersion`, fixed to `1`.
2. `protocol`, fixed to `lumenbazaar`.
3. `interfaceVersion`, the target contract interface.
4. `networkPassphrase` and `contractId`, which bind the environment and deployment.
5. `sessionId`, lowercase 32-byte hex.
6. `amount`, a positive base-unit integer encoded as a canonical decimal string.
7. `resourceHash`, lowercase 32-byte hex.
8. `measurement`, containing `unit`, decimal-string `quantity`, `startedAtLedger`, and `endedAtLedger` in that order.
9. `receiptId`, the backend's non-secret idempotency identifier.

Strings must already be NFC-normalized and must not contain leading or trailing
whitespace. Objects must contain exactly the schema fields. Producers must
reject duplicate or unknown fields, numeric encodings of `amount` or
`quantity`, reordered fields, non-lowercase hashes, and any byte sequence that
does not equal the canonical re-encoding. Hash the resulting UTF-8 bytes with
SHA-256 and pass that digest as `usage_hash`.

The generated binding exports `canonicalizeUsageReceipt`, `hashUsageReceipt`,
and `parseCanonicalUsageReceipt`. Shared Rust and TypeScript vectors are in
`artifacts/backend-handoff/usage-receipt-v1.fixtures.json`.

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
  "schemaVersion": 1,
  "protocol": "lumenbazaar",
  "interfaceVersion": 2,
  "networkPassphrase": "Test SDF Network ; September 2015",
  "contractId": "CCENNI5ZMMD3DCJXG5MURDXWUU3NG6JCFHDCDSEI4OMNWDJRY2IR36L3",
  "sessionId": "4a0c93c17417d5d2d603db55a20bc2fd06fca1a3129ca68a38d0420960c57742",
  "amount": "420000000",
  "resourceHash": "e71d5d8aa20ab6db9ae07e7095e6781b20a5996337493f7397c033f18e1be989",
  "measurement": {
    "unit": "request",
    "quantity": "42",
    "startedAtLedger": 5067500,
    "endedAtLedger": 5067542
  },
  "receiptId": "usage_01HZYK7G7MN5P85V3QP4Q8Y9X2"
}
```

Example hashes are committed in `artifacts/backend-handoff/usage-hashes.example.json`.

Only the canonical commitment belongs on-chain. Raw request/response bodies,
customer identifiers, access tokens, IP addresses, URLs containing private
parameters, and fine-grained usage logs must remain in access-controlled
off-chain storage. Use opaque `receiptId` values and coarse measurement units
that do not reveal private activity.

## Settlement Guardrails

- Verify the session is `Open` before constructing settlement.
- Verify `actual_amount` is greater than zero and less than or equal to `max_amount`.
- Verify the current ledger is before `expires_at_ledger`.
- Verify backend seller identity matches `session.seller`.
- Record settlement attempt ID, `usage_hash`, ledger, transaction hash, and decoded result.
- Treat duplicate settlement attempts as idempotent by fetching `get_session` after `SessionAlreadySettled`.
- At or after expiry, call `recover_expired`; it requires no caller signature and can refund only the stored buyer.
