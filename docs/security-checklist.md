# Security Checklist

This checklist records the contract controls that are ready for audit review.

| Control | Status | Test evidence | Notes |
| --- | --- | --- | --- |
| Authorization boundaries | Complete | `initialize_stores_admin_once`, `create_session_requires_buyer_auth`, `settle_requires_seller_auth`, `settle_rejects_wrong_seller_auth`, `cancel_requires_buyer_auth` | Buyer creates/cancels, seller settles, admin initializes once. |
| Cap enforcement | Complete | `settle_rejects_invalid_amounts_and_expiry`, `validation::tests::rejects_invalid_settlement_amounts` | Settlement must be positive and cannot exceed `max_amount`. |
| Double-settlement prevention | Complete | `settle_prevents_double_settlement`, `settle_rejects_finalized_sessions`, `cancel_rejects_already_settled_session` | Finalized sessions cannot be settled or cancelled again. |
| Expiry handling | Complete | `validation::tests::rejects_expired_sessions`, `expiry_boundary_selects_exactly_one_terminal_path`, `expiry_recovery_is_permissionless_fixed_recipient_and_single_use`, `expiry_recovery_emits_versioned_receipt` | Before expiry only settlement or buyer cancellation can finalize; at or after expiry only fixed-recipient recovery can finalize. |
| Asset and seller binding | Complete | `validation::tests::rejects_invalid_seller_or_asset_bindings`, `invalid_create_inputs_do_not_consume_sequence` | Seller cannot equal buyer and payment asset cannot equal buyer or seller. |
| Supported asset activation | Complete | `activation_probes_valid_asset_without_moving_funds_and_emits_event`, `activation_rejects_missing_incompatible_reverting_and_malformed_assets` | Admin-authorized initialization performs a fixed read-only interface/metadata probe before storage and emits a typed activation event. |
| Event correctness | Complete | `create_session_emits_stable_event`, `settle_emits_stable_event`, `cancel_emits_stable_event` | Event assertions compare typed event XDR for stable topics and payloads. |
| Escrow collateralization | Complete | `production_escrow_funds_and_settles_with_exact_authorization_trees`, `insufficient_funding_prevents_session_creation`, `liability_arithmetic_is_checked` | Open caps are held by the contract and tracked as checked per-asset liabilities. |
| Settlement atomicity and isolation | Complete | `settle_accepts_exact_cap_without_refund`, `settlement_isolates_concurrent_sessions_sharing_an_asset`, `undercollateralized_settlement_fails_without_state_transition` | A settlement consumes only its session escrow; collateral failure leaves state and liability open. |
| Generated state-machine invariants | Complete | `generated_sequences_preserve_value_caps_and_isolation`, `docs/property-testing.md` | Fixed-seed generated actions assert value conservation, caps, terminal immutability, wrong-signer rejection, and cross-session/cross-asset isolation. |

## Remaining Review Notes

- `policy-wallet-example` is not audited wallet infrastructure.
- Resource usage is recorded in `artifacts/resource-usage/upto-session.md`, but live RPC simulation is still required before production fee budgeting.
- `test-token` is utility code for local and testnet testing, not a production asset contract.
