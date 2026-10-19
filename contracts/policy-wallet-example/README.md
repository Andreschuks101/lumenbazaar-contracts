# Policy Wallet Example

This package is example code only. It is not production wallet infrastructure, is not audited, and should not custody funds without a full wallet security design.

The contract demonstrates the shape of a policy-controlled smart account for LumenBazaar agents:

- Stores an owner address that authorizes wallet administration.
- Stores the target `upto-session` contract address used by the surrounding application.
- Exposes `require_owner` as the minimal owner gate used by later spending-policy examples.
- Keeps all state in this contract instance so it remains isolated from `upto-session`.

Spending policies are keyed by agent address and can restrict:

- Maximum amount per payment.
- Maximum total amount per ledger-day bucket.
- Allowed sellers.
- Allowed assets.
- Allowed resource hashes.
- Validity window by ledger sequence.

`check_payment` validates a proposed payment without mutating state. `authorize_payment` requires the agent signature and records the amount against the current ledger-day bucket.

## Owner Rotation

The contract provides an expiring two-step owner rotation modeled independently from `upto-session` administration:

- `propose_owner(pending_owner, expires_at_ledger)`: The current owner proposes a replacement owner before a specified ledger expiry. Self-targeted proposals or proposals overwriting an active proposal fail.
- `accept_owner()`: The pending owner accepts the proposal before `expires_at_ledger`. Expired proposals, missing proposals, and unauthorized calls fail. Upon acceptance, the owner updates atomically, the proposal is cleared, and existing spending policies and daily counters survive intact.
- `cancel_owner_rotation()`: The current owner cancels an active proposal.
- `get_owner_rotation()`: Queries the active proposal, if one exists.
- `get_owner()`: Queries the current owner address.

See [docs/policy-examples.md](docs/policy-examples.md) for the executable examples mirrored by the test suite.
