# Upto Session Contract

`upto-session` enforces capped metered payment sessions for LumenBazaar x402
`upto` flows.

This is contract version `0.3.0` and interface version `3`.
`interface_version()` is the decoder compatibility check for clients. Version
3 adds the public aggregate escrow-liability query and requires clients that
use that method to target a v3 deployment.

## Escrow Liability Query

`get_escrow_liability(asset)` returns the checked aggregate amount recorded as
escrowed across all open sessions for a supported asset. It returns zero when a
supported asset has no open sessions, `NotInitialized` before setup, and
`UnsupportedAsset` for an address outside the immutable allowlist.

The liability increases atomically with successful session creation and
decreases by the full session escrow on settlement, cancellation, or expiry
recovery. It is an accounting value, not session enumeration. Integrators
should compare it with the contract's token balance; a balance lower than the
reported liability indicates under-collateralization, while a higher balance
may include unsolicited token transfers.

## Session ID Derivation

Session IDs are deterministic SHA-256 digests of the XDR encoding of:

- buyer
- seller
- asset
- max amount
- expiry ledger
- resource hash
- contract-local monotonic sequence

The sequence is stored in instance storage and increments before each new
session ID is returned. This prevents identical resource requests from
overwriting an existing session while keeping the derivation reproducible from
stored inputs and sequence evidence.

## Events

### `SessionCreated`

Topics:

- `session_id`

Data:

- `buyer`
- `seller`
- `asset`
- `max_amount`
- `expires_at_ledger`
- `resource_hash`
- `event_version` (`2`)
- `escrowed_amount`

### `SessionSettled`

Topics:

- `session_id`

Data:

- `seller`
- `asset`
- `actual_amount`
- `usage_hash`
- `event_version` (`2`)
- `refunded_amount`

Settlement is single-use. Once a session reaches `Settled`, any repeated
`settle` call returns `SessionAlreadySettled` and cannot move additional funds.

### `SessionCancelled`

Topics:

- `session_id`

Data:

- `buyer`
- `asset`
- `event_version` (`2`)
- `refunded_amount`

Only the buyer can cancel an open session. Cancellation is rejected for missing
sessions, settled sessions, and already cancelled sessions.

### `SessionRecovered`

Topics:

- `session_id`

Data:

- `buyer`
- `asset`
- `event_version` (`2`)
- `refunded_amount`
- `expired_at_ledger`
- `recovered_at_ledger`

Anyone may call `recover_expired(session_id)` at or after expiry. The caller
cannot choose the recipient or amount; the complete escrow returns to the
stored buyer.

## Usage Hash

`settle` requires a non-zero `usage_hash`. The backend should derive this hash
from a canonical usage receipt that includes:

- session ID
- resource hash
- buyer
- seller
- asset
- measured usage unit
- measured usage quantity
- settled amount
- request hash
- response hash, when available
- metering start and end ledger or timestamp

The contract stores only the digest. The backend remains responsible for
retaining and serving the canonical receipt body.

## TTL Strategy

Session entries use persistent storage. The public `extend_ttl(session_id)`
entrypoint renews only open sessions with:

- threshold: `100000` ledgers
- extension amount: `1000000` ledgers

Missing sessions return `SessionNotFound`. Settled and cancelled sessions return
`TtlExtensionFailed`, so finalized lifecycle states cannot be kept alive through
the active-session TTL renewal path.

Instance storage keeps administrator, layout version, and session sequence
state. Those entries are intentionally separate from per-session persistent
storage so sequence and contract layout evidence survive independent session
lifecycle cleanup.

## Expiry Behavior

`settle` rejects sessions at or after `expires_at_ledger` with
`ExpiredSession`, before seller authorization or token transfer. Expiry does not
rewrite stored session state by itself: the session remains observable through
`get_session` so backend indexers can retain the original cap, parties, asset,
and resource hash.

The buyer may cancel only before expiry. At the exact expiry ledger and later,
settlement and cancellation return `ExpiredSession`, while permissionless
`recover_expired` returns the complete escrow to the stored buyer and records
the `Expired` terminal state. Repeated recovery returns `SessionExpired`.
