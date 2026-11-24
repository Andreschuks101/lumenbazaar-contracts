# Storage Lifecycle, Expiry, and Recovery

This document specifies how long each `upto-session` storage entry lives, who must extend it, what
the contract does when state is missing or expired, and what a backend must do in each case. Every
rule has an ID (`SR-n`) and at least one boundary test in
`contracts/upto-session/src/storage_lifecycle_test.rs`. Test names start with their rule ID.

It is referenced from the [threat model](threat-model.md#amount-expiry-and-storage-risks), the
[backend handoff](backend-integration-handoff.md#missing-or-expired-on-chain-state), and the
[migration procedure](deployment-migration.md#4-open-sessions-on-the-old-deployment).

Run the boundary tests:

```bash
cargo test -p upto-session storage_lifecycle_harness
```

## Soroban background

- Every ledger entry has a `live_until_ledger`. Its remaining TTL is
  `live_until_ledger - current_ledger`.
- `extend_ttl(threshold, extend_to)` sets `live_until_ledger = current_ledger + extend_to` only when
  the remaining TTL is **at or below** `threshold`, and otherwise does nothing (soroban-env-host 28).
- Instance storage is one ledger entry shared by all instance keys and the contract instance
  itself. Extending it also extends the contract code entry. Persistent keys are separate entries
  with separate TTLs.
- When a persistent or instance entry's TTL runs out it is **archived**, not deleted. Archived
  entries cannot be read or written by a transaction until they are restored. Restoration is a
  network-level operation (for example `RestoreFootprint`, or automatic restoration where the
  network protocol and the submitting client support it). It costs fees, it needs the caller to
  build the right footprint, and its availability depends on the network and tooling.
  **This contract neither implements nor relies on restoration, and no rule here promises that
  archived state can be restored.**
- The unit-test host does not model archival, so tests model a lapsed entry by removing it. On a
  real network an archived entry makes the transaction fail rather than read as absent. Both fail
  closed, and the tests pin the contract's behavior for the "absent" case, which is the stricter
  one.

## Constants

| Constant | Value | Source |
| --- | --- | --- |
| `DEFAULT_TTL_THRESHOLD` | 100,000 ledgers (about 5.8 days at a nominal 5 s close) | `contracts/upto-session/src/storage.rs:11` |
| `TTL_EXTENSION_AMOUNT` | 1,000,000 ledgers (about 58 days at a nominal 5 s close) | `contracts/upto-session/src/storage.rs:12` |
| `MAX_SESSION_DURATION_LEDGERS` | 518,400 ledgers (about 30 days) | `contracts/upto-session/src/validation.rs:4` |

Every extension in this contract uses the same pair: `extend_instance_ttl`
(`storage.rs:86`) for the instance, and `extend_session_and_liability_ttl` (`storage.rs:381`) /
`extend_session_ttl` (`storage.rs:429`) for persistent keys. Ledger close time varies, so durations
in days are approximate. Contract logic uses ledger sequences only.

### Rollover-adjacent ledger behavior

- Session creation (`MAX_SESSION_DURATION_LEDGERS`), expiry extension, admin rotation
  (`MIN_ADMIN_ROTATION_DURATION_LEDGERS..=MAX_ADMIN_ROTATION_DURATION_LEDGERS`), and cap-increase
  cooldown/timelock windows (`MIN_CAP_INCREASE_DELAY_LEDGERS..=MAX_CAP_INCREASE_DELAY_LEDGERS`)
  compute bounds with checked `u32` addition/subtraction before any storage write or TTL extension.
- Wrapped ledgers (`0`, `1`, ...) never satisfy `expires_at_ledger > current_ledger` or
  `current_ledger.checked_add(...)` bounds, so rollover-adjacent calls fail closed with typed
  `ContractError` values without trapping or extending TTL.
- In `soroban-env-host`, `Storage::extend_ttl` requires
  `sequence_number.checked_add(TTL_EXTENSION_AMOUNT)` to fit in `u32`, so succeeding mutating
  entry points that call `extend_ttl` (and Stellar Asset Contract `balance()` when `preview_settle`
  checks escrow funding for non-expired sessions) are bounded to
  `sequence_number <= u32::MAX - TTL_EXTENSION_AMOUNT`. Furthermore,
  `LedgerInfo::min_live_until_ledger_checked` and `max_live_until_ledger_checked` (invoked by auth
  nonce tracking) compute `sequence_number.checked_add(min_persistent_entry_ttl - 1)` and
  `sequence_number.checked_add(max_entry_ttl - 1)`; with `min_persistent_entry_ttl = 1`,
  `min_temp_entry_ttl = 1`, and `max_entry_ttl = 1`, `ttl_test.rs`, `test.rs`, and the
  `token_amount` fuzz target (`fuzz/src/amount.rs`) verify read-only expiry views and all rejected
  entry points up to `u32::MAX` inclusive without host traps.

## Key inventory

"Admin entry points" below means exactly these functions, each of which calls `extend_instance_ttl`
on success: `initialize`, `set_asset_max_session_amount`, `propose_asset_cap_increase`,
`accept_asset_cap_increase`, `cancel_asset_cap_increase`, `set_seller_exposure_ceiling`,
`set_max_buyer_open_sessions`, `set_max_seller_open_sessions`, `propose_admin`, `accept_admin`,
`cancel_admin_rotation`, `deprecate_asset`, `remove_asset`, `pause_creation`, `resume_creation`.

### Instance keys (one shared TTL)

| Key | Holds | Created | Changed / removed | Extended by | Lifecycle |
| --- | --- | --- | --- | --- | --- |
| `Admin` | Administrator address | `initialize` | `accept_admin` | Admin entry points (SR-1) | Lives with the instance. |
| `AdminRotation` | Pending admin and expiry | `propose_admin` | Removed by `accept_admin`, `cancel_admin_rotation` | Admin entry points | Optional; present only while a rotation is pending. |
| `NextSessionSequence` | Next session sequence (`u64`) | First `create_session` | Incremented by every `create_session` | Admin entry points only; `create_session` does not extend (SR-2) | Lives with the instance. |
| `StorageLayoutVersion` | Layout version `3` | `initialize` | Never | Admin entry points | Lives with the instance. |
| `SupportedAsset(asset)` | `Active` / `Deprecated` | `initialize` | `deprecate_asset`; removed by `remove_asset` | Admin entry points | Per configured asset. |
| `AssetMaxSessionAmount(asset)` | Per-session cap | `initialize` | Cap updates and accepted increases | Admin entry points | **Finding:** not removed by `remove_asset`; remains as an inert instance value. |
| `AssetMinSessionAmount(asset)` | Per-session minimum | `initialize` | Never | Admin entry points | **Finding:** not removed by `remove_asset`; inert. |
| `CapIncreaseProposal(asset)` | Pending cap increase | `propose_asset_cap_increase` | Removed by accept / cancel | Admin entry points | Optional. |
| `SupportedAssets` | Ordered asset list | `initialize` | Never at this layout | Admin entry points | **Finding:** still lists an asset after `remove_asset`; readers must check `SupportedAsset(asset)`. |
| `OperationalState` | `Active` / `Paused` | `initialize` | `pause_creation`, `resume_creation` | Admin entry points | Lives with the instance; absent reads as `Active`. |
| `SellerExposureCeiling(asset)` | Optional ceiling | `set_seller_exposure_ceiling` | Same | Admin entry points | Optional. |
| `MaxBuyerOpenSessions` | Optional limit | `set_max_buyer_open_sessions` | Same | Admin entry points | Optional. |
| `MaxSellerOpenSessions` | Optional limit | `set_max_seller_open_sessions` | Same | Admin entry points | Optional. |

All instance keys live and die together: if the instance entry is archived, every invocation of
the contract fails until it is restored, including settlement, cancellation, and recovery of open
escrow. **Finding:** nothing on the session path extends the instance, so its lifetime depends
entirely on admin activity (SR-2). Operators own this (see Responsibilities).

`NextSessionSequence` shares the instance entry with the rest of the config, so it cannot lapse or
reset on its own. `create_session` has no "session ID already exists" guard and relies on that
coupling. This is an observation, not a reachable bug: a sequence reset would need the instance to
be partially lost, which the network does not do.

### Persistent keys (one TTL each)

| Key | Holds | Created | Changed | Extended by | Lifecycle |
| --- | --- | --- | --- | --- | --- |
| `Session(id)` | Full session record | `create_session` | `settle`, `cancel`, `recover_expired`(`_batch`), `reduce_session_cap` | Every `write_session` (create, finalize, cap reduction) and `extend_ttl` while open (SR-3, SR-4, SR-6) | created → extended while open → finalized (last extension) → lapses. Never deleted by the contract. |
| `Liability(asset)` | Aggregate open escrow for the asset | First `create_session` for the asset | Every create and finalize of a session of that asset | Alongside any session of that asset (`write_session`, `extend_ttl`) | Lives as long as its most recently written or extended session. Never deleted; lapses when no session of the asset is touched for a full window. |
| `SellerExposure(seller, asset)` | Aggregate open escrow for seller and asset | First matching `create_session` | Create and finalize of matching sessions | Alongside matching sessions | Same as `Liability`. |
| `BuyerOpenSessions(buyer)` | Count of the buyer's open sessions | Buyer's first `create_session` | Create (+1) and finalize (−1) | Alongside the buyer's sessions | Same as `Liability`. |
| `SellerOpenSessions(seller)` | Count of the seller's open sessions | Seller's first `create_session` | Create (+1) and finalize (−1) | Alongside the seller's sessions | Same as `Liability`. |

No temporary storage is used.

## Session policies

### Open sessions

- Creation extends the session and its four aggregate keys to the full extension (SR-3).
- A session can be open for at most 518,400 ledgers, below the 900,000-ledger point at which a
  freshly extended entry reaches the threshold. An open session therefore cannot reach the TTL
  threshold before its logical expiry, and needs no extension to settle or cancel (SR-5).
- After logical expiry, an open session can only be recovered. `extend_ttl` still works on it and is
  the way to keep it readable until recovery (SR-4). Prefer recovering promptly over extending.
- Aggregate keys are extended only as a side effect of a session write or `extend_ttl`. A busy
  asset, seller, or buyer keeps them alive. An idle one lets them lapse (SR-3, SR-4).

### Finalized sessions (settled, cancelled, expired)

- The finalizing write extends the session once, and only if its TTL is at or below the threshold
  at that moment (SR-6). After that, no contract path extends it: `extend_ttl` returns
  `TtlExtensionFailed` (error 11) and changes nothing.
- A finalized record is retained on chain for at most `TTL_EXTENSION_AMOUNT` ledgers after its last
  extension, and then lapses. The contract never deletes it. The **backend's off-chain record is
  the long-term system of record** for finalized sessions.
- A lapsed finalized session reads as `SessionNotFound`. That is expected and is not an error
  condition for a session the backend already recorded as final.

## Responsibilities

| Who | Must | When |
| --- | --- | --- |
| Operators (contract admin) | Call an admin entry point, for example re-setting an existing cap with `set_asset_max_session_amount(asset, current_cap)`, so the instance TTL is extended. | Before the instance TTL falls to the threshold. Monitor it (`getLedgerEntries` on the instance) and act at or below 100,000 ledgers remaining. **Policy decision:** recommended alert at 150,000. |
| Backend | Call `extend_ttl(session_id)` for any open session it intends to keep after expiry, and call `recover_expired` / `recover_expired_batch` for expired open sessions. | Expired open sessions: recover within one reconciliation cycle. Extension: only when remaining TTL ≤ threshold, since extension above it is a no-op. |
| Backend | Persist every session's route and final state off-chain at finalization. | Immediately on a finalized result. On-chain finalized records lapse. |
| Anyone | `recover_expired` is permissionless and pays only the stored buyer. | Any time at or after expiry. |

## Rules

| ID | Rule | Tests |
| --- | --- | --- |
| SR-1 | All instance keys share one TTL. `initialize` and every admin entry point extend it to `TTL_EXTENSION_AMOUNT`, only when the remaining TTL is at or below `DEFAULT_TTL_THRESHOLD`. | `sr1_instance_keys_share_one_ttl_set_by_initialize`, `sr1_admin_call_extends_instance_only_at_threshold` |
| SR-2 | Session-path entry points (`create_session`, `settle`, `cancel`, `recover_expired`, `extend_ttl`) and reads never extend the instance TTL. Instance liveness is an operator duty. | `sr2_session_paths_never_extend_instance_ttl` |
| SR-3 | `create_session` extends the new session and its `Liability`, `SellerExposure`, `BuyerOpenSessions`, and `SellerOpenSessions` keys. Shared aggregate keys are re-extended by later sessions that touch them. | `sr3_create_extends_session_and_counters_to_full_ttl` |
| SR-4 | `extend_ttl` on an open session extends the session and its four aggregate keys exactly at or below the threshold (a no-op one ledger above), emits no event, and is not gated by logical expiry, so expired escrow stays recoverable. | `sr4_open_session_extend_ttl_boundary_is_exact`, `sr4_extend_ttl_after_logical_expiry_keeps_escrow_recoverable` |
| SR-5 | `MAX_SESSION_DURATION_LEDGERS` is below `TTL_EXTENSION_AMOUNT - DEFAULT_TTL_THRESHOLD`, so a session reaches logical expiry while its TTL is still above the threshold. | `sr5_max_duration_session_expires_before_ttl_threshold` (plus a compile-time assertion) |
| SR-6 | A finalizing write is a finalized session's last extension. Afterwards `extend_ttl` fails with `TtlExtensionFailed` for settled, cancelled, and expired sessions and leaves TTL, state, balances, and events unchanged. | `sr6_finalizing_write_is_the_last_extension`, `sr6_finalized_sessions_reject_extension_for_every_terminal_status` |
| SR-7 | A missing session (never created, or lapsed) returns `SessionNotFound` from `get_session`, `get_session_status`, `settle`, `cancel`, `extend_ttl`, and `recover_expired`, and `NotFound` from `recover_expired_batch`, with no transfer and no event. Its escrow and liability stay in the contract. | `sr7_missing_session_fails_closed_everywhere` |
| SR-8 | If any aggregate key of an open session is missing, `settle`, `cancel`, and `recover_expired` fail with `LiabilityUnderflow` (error 18): no payout, no event, the session stays open, and no other key changes. | `sr8_missing_aggregate_counter_fails_closed_without_payout` |
| SR-9 | If instance config is missing (modelled by removing `Admin`), `create_session` and admin entry points return `NotInitialized`, while existing sessions can still be cancelled and settled through their own records. On a real network an archived instance blocks every call instead. | `sr9_missing_instance_config_blocks_creation_and_admin_but_not_exits` |
| SR-10 | Read-only queries never extend any TTL, so polling does not keep state alive. | `sr10_reads_never_extend_ttl` |

## Missing or expired state decision table

"Contract" is what `upto-session` does. "Backend" is what the integration **must** do. In every row
the backend must never report a payment as settled, refunded, or absent because of missing
on-chain state alone.

| Scenario | Contract | Backend |
| --- | --- | --- |
| Session ID unknown to the backend and absent on chain | `SessionNotFound` (SR-7). | Reject the request. No retry. |
| Backend has an **open** session; chain returns `SessionNotFound` | Fails closed, no transfer (SR-7). | Do not mark it settled, cancelled, or refunded. Check the route first (wrong contract or network is a routing fault). Then check whether the session lapsed: look for its `session_*` finalization events. If none, treat the escrow as still held and **alert**. Attempt restoration only through network tooling, as best effort. If it cannot be restored, mark the session `stuck` and escalate. |
| Backend has a **finalized** session; chain returns `SessionNotFound` | Same. | Expected after the retention window (SR-6). Serve the off-chain final record, marked as historical. No alert. |
| Chain has a session the backend has no record of | Normal session. | Split-brain. Record the route from the creation event, reconcile its state, and alert. Never treat a missing off-chain record as proof the on-chain session is absent. |
| Settle / cancel / recover returns `LiabilityUnderflow` | Fails closed, no payout (SR-8). | Do not retry blindly. An aggregate key lapsed or accounting is inconsistent. Alert, stop automated retries for that asset, and reconcile `get_escrow_liability` against the open sessions you know. |
| `extend_ttl` returns `TtlExtensionFailed` | Session is finalized (SR-6). | Fetch state and record the final status. Do not retry. |
| Transaction fails because an entry is archived | Nothing executes. | Treat the outcome as "not executed". Restore through network tooling if available, then resubmit. Never assume success. |
| Instance config missing / instance archived | Creation and admin calls fail; on a real network every call fails (SR-9). | Stop creating sessions on this contract and page operators. Keep existing sessions in their current state; do not report them final. |
| `recover_expired_batch` item `EscrowUnavailable` | That item was not recovered. | Treat it as still held. See the finding below before relying on the rest of that batch. |
| Settlement result unknown (timeout, RPC error) | n/a | Query `get_session` on the stored route before retrying. Settlement is single-use, so a retry of a completed settlement returns `SessionAlreadySettled`. |

**Finding (reported, not fixed here):** in `recover_expired_batch`, `recover_open_session` writes
`Liability` and `SellerExposure` before decrementing the open-session counters. If a later step
fails with `LiabilityUnderflow`, the batch records `EscrowUnavailable` and continues, so the
transaction commits with the earlier writes kept while the session stays open with its escrow.
This is only reachable when an aggregate key is inconsistent, and the single-session entry points
are not affected because their failure reverts the whole call. Until it is fixed, backends should
recover sessions with an inconsistent aggregate key individually (`recover_expired`), not in a
batch.

## Reconciliation

Off-chain records and on-chain state diverge in predictable ways: finalized sessions lapse on
chain while the backend keeps them; aggregate keys lapse on idle assets, sellers, or buyers;
transactions can land without the backend seeing the result; and a client can create a session
the backend did not initiate.

Expectations (**policy decision** on the intervals; recommended defaults shown):

- **Every 1,000 ledgers** (about 1.4 hours), for every open session the backend knows of: read
  `get_session` on its stored route, compare status and amounts, recover expired sessions, and
  call `extend_ttl` for any open session with remaining TTL at or below the threshold.
- **Every 17,280 ledgers** (about 1 day), per contract and asset: compare `get_escrow_liability`
  with the sum of the backend's open sessions for that asset, compare the contract's token balance
  with that liability, and check the instance TTL against the operator alert level.
- **Continuously:** consume `session_created`, `session_settled`, `session_cancelled`, and
  `session_recovered` events (see the handoff's indexed event topics) and match each to a stored
  route.
- **On mismatch:** the chain is authoritative for open escrow and the backend is authoritative for
  lapsed finalized history. Never "fix" a mismatch by assuming success, never retry a settlement
  without first reading state, freeze automated actions for the affected session or asset, and
  alert. A liability that is higher than the backend's open sessions means unknown open escrow.
  A lower one means either an unseen finalization or the batch finding above.

## Limits of this specification

- The tests use the unit-test host. They do not exercise archival, restoration, rent fees, or real
  network TTL limits.
- No rule promises that archived entries can be restored, or restored in time to meet a session
  deadline.
- Durations in days are approximate. Contract behavior is defined only in ledgers.
