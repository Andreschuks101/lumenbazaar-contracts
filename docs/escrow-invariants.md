# Escrow Invariants

The `upto-session` storage layout version 2 uses full-cap escrow. A successful
`create_session` transfers `max_amount` from the authenticated buyer to the
contract before the session is stored as open.

For every supported asset `a`:

```text
liability[a] = sum(session.escrowed_amount for every open session using a)
token.balance(contract, a) >= liability[a]
```

`get_escrow_liability(a)` exposes the first value for supported assets without
enumerating sessions. It returns the stored aggregate maintained by checked
arithmetic. The token balance remains a separate token-contract query so
integrators can reconcile custody against accounting.

Creation, liability accounting, session storage, and event publication execute
in one Soroban transaction. A failed authorization, insufficient token balance,
arithmetic error, or under-collateralization check rolls the complete invocation
back. The sequence number therefore cannot be consumed by a failed creation.

Settlement removes the session's full escrow liability, pays only the recorded
seller, refunds the unused cap to the recorded buyer, clears
`escrowed_amount`, and writes the terminal state in the same invocation.
Cancellation removes the same liability and returns the entire escrow to the
recorded buyer.

Cancellation is available only before expiry. At the exact expiry ledger and
afterward, `recover_expired` is permissionless but has no recipient or amount
arguments: it always refunds the complete escrow to the stored buyer and marks
the session `Expired`. Settlement, cancellation, and recovery are mutually
exclusive terminal paths.

Before either terminal transfer, the contract checks that the stored open
session still carries its complete cap and that the contract token balance
covers the aggregate liability for that asset. A failed check returns
`EscrowUnderfunded` before liability or session state changes.

The asset allowlist is immutable after initialization. Each session ID commits
to buyer, seller, asset, cap, expiry, resource hash, and the contract sequence.
No caller supplies a payout source or destination at settlement, so the
contract cannot act as a deputy for another account and one session cannot use
another session's authorization or parameters. Full-cap escrow avoids shared
allowance leakage and makes seller-only deferred settlement enforceable.

Contract-to-token calls do not introduce a callback into `upto-session`; state
changes and token movements remain atomic under Soroban transaction semantics.
All arithmetic that changes aggregate liabilities uses checked operations.

Creation events use `event_version = 2` and expose only public settlement
metadata. Resource and usage hashes remain opaque fixed-length digests.
