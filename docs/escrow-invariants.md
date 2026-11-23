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

Cancellation is available only before expiry; seller rejection (`reject`) is likewise
available to the stored seller before expiry and refunds the full escrow to the
stored buyer. At the exact expiry ledger and afterward, `recover_expired` is
permissionless but has no recipient or amount arguments: it always refunds the
complete escrow to the stored buyer and marks the session `Expired`. Settlement,
cancellation, seller rejection, and recovery are mutually exclusive terminal
paths: whichever valid terminal transaction lands first finalizes the session and
causes all subsequent terminal attempts to fail closed without duplicate payout or
refund.

`extend_session_expiry` requires both the recorded buyer and seller, only advances an open and unexpired session, and leaves escrow liability and deposited balances unchanged.

Before either terminal transfer, the contract checks that the stored open
session still carries its complete cap and that the contract token balance
covers the aggregate liability for that asset. A failed check returns
`EscrowUnderfunded` before liability or session state changes.

Asset policies are established at initialization. Each supported asset has a
positive explicit session cap; authenticated cap changes affect only later
creation and do not rewrite stored sessions. Assets follow an explicit lifecycle:
active assets can be deprecated by the administrator, which immediately rejects
new session creations while preserving every terminal exit (settle, cancel,
recover) for existing open sessions. A deprecated asset can only be removed once
its aggregate liability reaches zero.

Administrator authority follows a two-step expiring rotation. The current
administrator proposes a pending administrator with an expiration ledger. Only the
pending administrator can accept the rotation before the expiration ledger, which
atomically transfers administrator privileges. The current administrator may
cancel a pending rotation at any time before acceptance.

Each session ID commits to buyer, seller, asset, cap, expiry, resource hash, and
the contract sequence. No caller supplies a payout source or destination at
settlement, so the contract cannot act as a deputy for another account and one
session cannot use another session's authorization or parameters. Full-cap escrow
avoids shared allowance leakage and makes seller-only deferred settlement
enforceable.

Contract-to-token calls do not introduce a callback into `upto-session`; state
changes and token movements remain atomic under Soroban transaction semantics.
All arithmetic that changes aggregate liabilities uses checked operations.

Creation events use `event_version = 2` and expose only public settlement
metadata. Resource and usage hashes remain opaque fixed-length digests.

## Aggregate Exposure Ceilings

Per-session caps bound one payment. Two further ceilings bound the total open
escrow a deployment can accumulate. Both are part of every `AssetPolicy`, must
be positive at initialization, and can be changed by the administrator with
`set_asset_exposure_ceiling` and `set_buyer_exposure_ceiling`.

For every supported asset `a` and buyer `b`:

```text
buyer_exposure[b, a] = sum(session.escrowed_amount for every open session of b using a)
liability[a] + new_max_amount <= max_asset_exposure[a]          (checked at creation)
buyer_exposure[b, a] + new_max_amount <= max_buyer_exposure[a]  (checked at creation)
```

- `create_session` computes both sums with checked arithmetic and rejects with
  `AssetExposureCeilingExceeded` (36) or `BuyerExposureCeilingExceeded` (39)
  before buyer authorization, sequence consumption, or any token transfer. An
  overflowing sum fails with `LiabilityOverflow`. Reaching a ceiling exactly is
  allowed. Both sums are re-derived after the funding transfer, so a token
  callback cannot have raced a counter past its ceiling.
- Each buyer has an independent counter per asset, stored under
  `BuyerExposure(buyer, asset)`. No limit is shared between buyer addresses.
- Settlement, cancellation, single recovery, and batch recovery each release
  the session's full escrow from both counters once, in the same invocation
  that writes the terminal status. Repeated terminal calls fail on the session
  status and release nothing. A buyer counter that reaches zero is removed.
- The buyer decrement is computed before any state changes. A counter that
  would go negative fails the whole call with `BuyerExposureUnderflow` (40);
  batch recovery reports that session as `EscrowUnavailable` and continues.
- Lowering a ceiling below the current exposure is allowed. Existing sessions
  close normally; new sessions are rejected until capacity returns. Ceilings
  are per asset only: there is no fiat conversion, oracle pricing, or
  cross-asset aggregation.
- An active asset without a stored ceiling fails creation closed with
  `ExposureCeilingNotConfigured` (38).

### Same-ledger cap-reduction and session-creation ordering

When a cap or exposure change and a `create_session` call execute in the same ledger sequence, each transaction evaluates against the state committed by prior transactions in that ledger:

- **Cap/ceiling reduction before `create_session`**: `set_asset_max_session_amount` or `set_buyer_exposure_ceiling` takes effect immediately and emits `AssetSessionCapUpdated` or `BuyerExposureCeilingUpdated` before `SessionCreated`. A creation exceeding the reduced cap/ceiling fails closed with `AmountExceedsCap` or `BuyerExposureCeilingExceeded` before buyer auth or fund transfer.
- **`create_session` before cap/ceiling reduction**: creation is evaluated against the pre-reduction cap/ceiling, emits `SessionCreated` first, and establishes an open session at that authorized cap; the subsequent reduction updates stored policy for future creations, emits `AssetSessionCapUpdated` / `BuyerExposureCeilingUpdated` after `SessionCreated`, and leaves existing open sessions able to settle/cancel/recover cleanly.
- **Buyer `reduce_session_cap` vs `create_session`**: reducing an open session's cap atomically refunds the delta to the buyer, decrements asset liability, seller exposure, and `BuyerExposure(buyer, asset)`, and emits `SessionCapReduced`. When ordered before `create_session`, freed exposure capacity is immediately available to the new session; when ordered after, a creation exceeding prior headroom fails closed before moving funds, and accepted creations followed by `reduce_session_cap` emit `SessionCreated` then `SessionCapReduced` while keeping final exposure bounded and exact.

## Bounded State Model

`formal/escrow-model` exhaustively explores the reachable states of one
session through bounded sequences of initialization, creation, settlement,
cancellation, seller rejection, expiry, recovery, asset lifecycle (deprecation and removal), and
two-step admin rotation (proposal, acceptance, cancellation, expiry) actions.
The finite model checks:

- at most one terminal transition succeeds;
- seller payout never exceeds the recorded cap;
- deposited value equals contract balance plus seller payout plus buyer refund;
- open liability equals the full escrowed cap and terminal liability is zero;
- cancellation and expiry recovery refund only the recorded buyer;
- deprecated assets reject new session creation while preserving all terminal exits;
- asset removal is impossible until modeled liability reaches zero;
- only accepted, non-expired rotation changes the administrator; and
- initialization, creation, settlement, cancellation, deprecation, removal,
  and admin rotation authorization gates reject unauthorized actions without
  changing state.

The model checker visits actions in a fixed order and reports the shortest
known action trace when an invariant fails. CI runs it with `--nocapture`, so a
counterexample can be reproduced locally with:

```console
cargo test -p escrow-model -- --nocapture
```

The model is a protocol-level abstraction. It does not prove that the Rust
contract implements the model. The following traceability table links each
model action to implementation and test evidence that must remain green.

| Model action | Rust method or gate | Implementation tests |
| --- | --- | --- |
| `Initialize` | `initialize` and `admin.require_auth()` | `initialize_stores_admin_once`, `initialize_rejects_empty_duplicate_or_invalid_asset_policies` |
| `Create` | `create_session`, supported-asset gate, buyer authorization, token transfer, liability increase | `create_session_stores_open_session`, `create_session_requires_buyer_auth`, `insufficient_funding_prevents_session_creation` |
| `Settle` | `settle`, seller authorization, amount and expiry validation, liability decrease | `settle_rejects_wrong_seller_auth`, `settle_accepts_exact_cap_without_refund`, `settlement_isolates_concurrent_sessions_sharing_an_asset`, `seller_settlement_before_rejection_prevents_refund_and_asserts_auth_evidence`, `counterexample_seller_rejection_and_settlement_ordering_is_mutually_exclusive` |
| `Cancel` | `cancel`, buyer authorization, pre-expiry gate, full refund | `cancel_requires_buyer_auth`, `cancel_allows_buyer_cancellation_before_settlement`, `cancel_rejects_already_cancelled_session`, `distinct_valid_signers_buyer_cancel_and_seller_terminal_actions_both_orderings`, `counterexample_buyer_cancel_and_seller_terminal_actions_are_mutually_exclusive` |
| `Reject` | `reject`, seller authorization, pre-expiry gate, full buyer refund | `reject_allows_seller_rejection_and_refunds_buyer_only`, `reject_requires_seller_auth`, `reject_rejects_terminal_and_expired_states`, `rejected_sessions_never_settle_cancel_extend_or_recover`, `seller_rejection_before_settlement_prevents_payout_and_asserts_auth_evidence`, `seller_settlement_before_rejection_prevents_refund_and_asserts_auth_evidence`, `wrong_signer_cannot_trigger_reject_or_settle_before_valid_terminal_action`, `counterexample_seller_rejection_and_settlement_ordering_is_mutually_exclusive` |
| `AdvanceToExpiry` | ledger sequence compared with `expires_at_ledger` | `expiry_boundary_selects_exactly_one_terminal_path` |
| `ExtendExpiry` | `extend_session_expiry`, buyer and seller authorization, pre-expiry and maximum-duration gates | `extend_session_expiry_succeeds_and_emits_event`, `production_expiry_extension_requires_both_buyer_and_seller_authorization_trees` |
| `Recover` | `recover_expired`, fixed stored buyer recipient, full refund | `expiry_recovery_is_permissionless_fixed_recipient_and_single_use`, `expiry_recovery_rejects_missing_settled_and_cancelled_sessions` |
| `DeprecateAsset` | `deprecate_asset`, admin authorization, active-asset status gate | `asset_deprecation_requires_admin_and_blocks_new_sessions` |
| `RemoveAsset` | `remove_asset`, admin authorization, deprecated status and zero liability gates | `deprecated_asset_terminal_paths_clear_liability_before_removal` |
| `ProposeAdmin` | `propose_admin`, admin authorization, single pending rotation gate, expiry validation | `admin_rotation_requires_acceptance_and_changes_privileges_atomically`, `admin_rotation_rejects_overwrite_expiry_and_replay` |
| `AcceptAdmin` | `accept_admin`, pending admin authorization, non-expired ledger gate | `admin_rotation_requires_acceptance_and_changes_privileges_atomically`, `admin_rotation_rejects_overwrite_expiry_and_replay` |
| `CancelAdminRotation` | `cancel_admin_rotation`, current admin authorization, pending rotation existence gate | `admin_rotation_requires_acceptance_and_changes_privileges_atomically`, `admin_rotation_rejects_overwrite_expiry_and_replay` |
