# Policy Wallet Examples

These examples are mirrored by executable tests in `src/test.rs`.

Spending policy validity boundaries are inclusive: a policy is active for ledger sequences satisfying `valid_from_ledger <= ledger_sequence <= valid_until_ledger`.

| Scenario | Test name | Expected result |
| --- | --- | --- |
| Allowed payment | `documentation_allowed_payment_example_authorizes` | The agent-authorized payment succeeds and increments daily spend. |
| Blocked payment | `documentation_blocked_payment_example_reports_amount_cap` | A payment over the per-payment cap returns `AmountExceedsPaymentCap`. |
| Seller payment cap | `documentation_seller_payment_cap_example_reports_amount_cap` | A payment over the seller-specific per-payment cap returns `AmountExceedsPaymentCap`. |
| Seller daily cap | `documentation_seller_daily_cap_example_reports_daily_cap` | A payment over the seller-specific daily cap returns `AmountExceedsDailyCap` while other sellers remain spendable. |
| Resource payment cap | `documentation_resource_payment_cap_example_reports_amount_cap` | A payment over the resource-specific per-payment cap returns `AmountExceedsPaymentCap`. |
| Resource daily cap | `documentation_resource_daily_cap_example_reports_daily_cap` | A payment over the resource-specific daily cap returns `AmountExceedsDailyCap` while other resources remain spendable. |
| Authorization count limit | `documentation_authorization_limit_example_reports_limit_exceeded` | An authorization exceeding `max_authorizations_per_day` returns `AuthorizationLimitExceeded`. |
| Pre-activation policy | `documentation_pre_activation_policy_example_reports_not_active` | A payment checked before `valid_from_ledger` returns `PolicyNotActive` and leaves spend unchanged. |
| Expired policy | `documentation_expired_policy_example_reports_expired` | A payment checked after `valid_until_ledger` returns `PolicyExpired`. |
| Wrong seller | `documentation_wrong_seller_example_reports_seller_error` | A seller outside the allowlist returns `SellerNotAllowed`. |
| Wrong asset | `documentation_wrong_asset_example_reports_asset_error` | An asset outside the allowlist returns `AssetNotAllowed`. |
| Revoked agent | `owner_revocation_denies_later_checks_and_authorizations` | After `revoke_policy`, checks and authorizations return `PolicyRevoked`. |
| Stale revision write | `stale_expected_revision_fails_without_modifying_policy_or_spend` | An outdated expected revision returns `StaleRevision` without changing policy or spend. |
| Oversized allowlist | `allowlists_enforce_exact_limits_and_fail_at_limit_plus_one` | More than 16 sellers, assets, or resource hashes returns `InvalidPolicy`. |
| Duplicate allowlist entries | `allowlists_reject_duplicate_sellers_deterministically` | Duplicate seller, asset, or resource hash entries return `InvalidPolicy`. |
| Same-ledger cap reduction & creation | `same_ledger_policy_cap_reduction_and_session_creation_both_orderings` | Both transaction orderings follow visible policy state and never exceed authorized exposure. |
| Concurrent cap reduction & seller/resource ordering | `same_ledger_concurrent_cap_reductions_enforce_revision_and_seller_resource_ordering` | Stale revisions fail with `StaleRevision` and tight caps hold across resources. |

## Policy Replacement and Day Rollover Semantics

- **Policy Replacement**: When a spending policy is replaced via `set_policy`, stored same-day and historical spend counters (`DailySpend`, `DailySellerSpend`, and `DailyResourceSpend`) are preserved. New caps take effect immediately against already-accumulated daily spend.
- **Day Rollover**: All daily spending counters are partitioned by ledger day (`sequence / LEDGERS_PER_DAY`). When the ledger sequence advances past the day boundary, all spend queries and authorization checks reset to zero automatically.

## Same-Ledger Cap-Reduction and Session-Creation Ordering Semantics

When a wallet owner reduces spending caps (`set_policy`) and an agent authorizes a payment and opens an escrow session (`authorize_payment` -> `upto-session::create_session`) within the same ledger sequence:

- **Transaction-order visibility**: `authorize_payment` and `upto-session::create_session` validate against the policy state visible at their exact execution point in the ledger.
  - If `set_policy` executes first, the authorization sees the reduced per-payment and daily caps immediately and rejects any excess amount before incrementing spend or creating a session.
  - If `authorize_payment` + `create_session` executes first, it is evaluated against the prior revision and establishes a valid session; the subsequent `set_policy` preserves same-day spend counters, immediately preventing any additional authorization that would exceed the lowered cap.
- **No post-reduction excess**: After a cap reduction lands, no subsequent transaction path in that ledger (including per-seller or per-resource authorizations) can exceed the lowered policy ceiling. Concurrent same-ledger policy updates require the current monotonic `revision` via `expected_revision` and fail closed with `StaleRevision`.
- **Auditability**: Policy revisions (`get_policy_revision`) paired with `upto-session` events (`SessionCreated`, `AssetSessionCapUpdated`, `BuyerExposureCeilingUpdated`, and `SessionCapReduced`) provide a deterministic, ordered audit trail of when limits changed relative to created or reduced escrow sessions.
