# Upto Session Contract

`upto-session` enforces capped metered payment sessions for LumenBazaar x402
`upto` flows.

This is contract version `0.3.0` and interface version `3`.
`interface_version()` is the decoder compatibility check for clients. Version
3 changes initialization to require an explicit positive cap for every
supported asset and adds two-step administrator rotation. It requires a new
deployment and is not wire-compatible with the v2 initialization payload.

## Per-Asset Session Caps

`initialize(admin, asset_policies)` accepts a non-empty list of `AssetPolicy`
values. Each value pairs a unique supported asset with a positive
`max_session_amount`. `create_session` rejects a requested cap above the
current asset policy before buyer authorization or token transfer.

The authenticated administrator may call
`set_asset_max_session_amount(asset, max_session_amount)` for an existing
supported asset. A change applies only to later session creation; it does not
rewrite the cap or escrow accounting of an existing session. Integrators can
read the active value with `get_asset_max_session_amount(asset)`.

## Administrator Rotation

The current administrator calls `propose_admin(pending_admin,
expires_at_ledger)` to create one pending rotation. The proposal cannot target
the current administrator, cannot overwrite another pending rotation, and must
expire within 17,280 ledgers.

Only the stored pending administrator can call `accept_admin()` before the
expiry ledger. Acceptance changes the active administrator and removes the
proposal atomically. The old administrator loses privileges in that same
invocation. The current administrator can call `cancel_admin_rotation()` before
or after expiry; cancellation never transfers privileges. `get_admin()` and
`get_admin_rotation()` expose the current state without granting authority.

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

### `AssetSessionCapUpdated`

Topics:

- `asset`

Data:

- `admin`
- `previous_max_session_amount`
- `new_max_session_amount`
- `event_version` (`3`)

### `AdminRotationProposed`

Topics: `pending_admin`.

Data: `current_admin`, `expires_at_ledger`, and `event_version` (`3`).

### `AdminRotationAccepted`

Topics: `new_admin`.

Data: `previous_admin` and `event_version` (`3`).

### `AdminRotationCancelled`

Topics: `pending_admin`.

Data: `current_admin` and `event_version` (`3`).

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
