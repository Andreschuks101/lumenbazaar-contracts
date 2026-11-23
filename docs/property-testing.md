# Contract Property Testing

`generated_sequences_preserve_value_caps_and_isolation` runs against the real
`upto-session` and Stellar asset contracts. Every case creates two sessions
with distinct buyers, sellers, and assets, then generates up to 23 actions from
this set:

| Code | Generated action |
| ---: | --- |
| 0 | Settle with zero, one, midpoint, exact-cap, or over-cap amount |
| 1 | Buyer cancellation |
| 2 | Advance to the exact expiry ledger |
| 3 | Permissionless expiry recovery |
| 4 | Valid settlement followed by an immediate replay |
| 5 | Settlement carrying only an attacker authorization |

After every action, the test compares contract state and all three token
balances with its state model. It asserts value conservation, cap enforcement,
terminal-state immutability, and isolation between both sessions and assets.

The CI/default campaign is 128 cases with fixed seed
`0x4c554d454e425a52`. Proptest shrinks a failure and prints the smallest
reproducing input. Every campaign prints its seed, bounded case count, and an
exact replay command. Override the seed with `LUMENBAZAAR_PROPERTY_SEED` or
`LUMENBAZAAR_POLICY_SEED`; both accept the displayed hexadecimal value.

Run the CI campaign:

```bash
cargo test -p upto-session generated_sequences_preserve_value_caps_and_isolation -- --nocapture --test-threads=1
```

Run an expanded campaign before deployment:

```bash
LUMENBAZAAR_PROPERTY_CASES=512 cargo test -p upto-session generated_sequences_preserve_value_caps_and_isolation -- --nocapture --test-threads=1
```

On PowerShell, set the override for the command process:

```powershell
$env:LUMENBAZAAR_PROPERTY_CASES = "512"
cargo test -p upto-session generated_sequences_preserve_value_caps_and_isolation -- --nocapture --test-threads=1
Remove-Item Env:LUMENBAZAAR_PROPERTY_CASES
```

The generated amount strategy always includes 1, 2, and 10,000,000 plus values
through 999,999. `maximum_value_session_conserves_escrow` separately executes a
full `i128::MAX` funding and settlement path, while `liability_arithmetic_is_checked`
covers aggregate overflow and zero underflow.

## State-Transition Campaign

`contracts/upto-session/src/transition_test.rs` replays bounded action sequences against one real
session and a real Stellar asset contract, with a pure model of every permitted and forbidden
transition. Actions: settle with zero, one, half, exact-cap, or over-cap amounts; cancel; reject; recover;
batch recover; extend TTL; and advance the ledger to one before, at, or after expiry. After every
step the test checks the exact error, emitted events, buyer/seller/escrow balances, the escrow
liability, and the full stored session.

Proved properties: a session finalizes at most once; cancelled and rejected sessions never settle; settled
sessions never cancel, reject, extend, recover, or settle again; failing steps and no-op steps (including a
successful TTL extension) emit no event, move no tokens, and leave the session byte-for-byte equal.

Two runs cover it:

- `generated_sequences_follow_the_transition_table`: 128 random sequences of 1 to 16 actions, fixed
  seed `0x4c554d4554524e53`.
- `every_sequence_up_to_depth_three_follows_the_transition_table`: every 1 to 3 step sequence over a
  seven-action alphabet (399 sequences), no randomness.

```bash
cargo test -p upto-session transition_harness -- --nocapture
LUMENBAZAAR_TRANSITION_CASES=512 LUMENBAZAAR_TRANSITION_SEED=0x1234 cargo test -p upto-session transition_harness::generated_sequences
```

A failure prints the shrunk sequence. Paste it into `run_sequence(&[...])` for a permanent
regression test. `LUMENBAZAAR_TRANSITION_SEED` accepts decimal or `0x` hex.

## Cap Invariant Campaign

`contracts/upto-session/src/cap_property_test.rs` proves a session never settles above its cap. Cap
values are drawn from 1, 2, 10,000,000, `i128::MAX`, `i128::MAX - 1`, half-range values, and uniform
random values. Amounts include zero, negative, `i128::MIN`, `i128::MIN + 1`, `i128::MAX`, the cap,
cap minus one, cap plus one (saturating), half the cap, and uniform random values.

- `settlement_validation_matches_the_specification`, `asset_cap_validation_accepts_only_positive_values`,
  and `create_validation_accepts_only_positive_amounts` exercise `validation.rs` directly.
- `random_settlements_never_exceed_the_cap_and_failures_change_nothing` settles random amounts
  against a funded session and asserts that every rejected attempt leaves the session, all balances,
  the liability, and the event log unchanged, and that the seller never holds more than the cap.
- `session_creation_enforces_the_asset_cap_without_side_effects` and
  `cap_updates_accept_only_positive_values_and_keep_the_previous_cap_on_failure` cover creation and
  per-asset cap updates, including that rejected creation does not consume a session sequence.
- `decimal_scaled_cap_boundaries_are_exact_for_every_supported_decimal_count` checks `10^d - 1`,
  `10^d`, and `10^d + 1` caps and amounts for every `d` from 0 to `MAX_TOKEN_DECIMALS` (38), and
  `decimal_scaled_caps_settle_exactly_through_the_contract` does the same through the contract.
- `overflow_adjacent_escrow_totals_fail_without_state_change` reaches exactly `i128::MAX` of
  aggregate liability and shows one more unit fails with no state change.

Fixed seed `0x4c554d4543415053`, 128 cases. Override with `LUMENBAZAAR_CAP_SEED` and
`LUMENBAZAAR_CAP_CASES`. CI pins all three campaigns (`LUMENBAZAAR_PROPERTY_CASES`,
`LUMENBAZAAR_TRANSITION_CASES`, `LUMENBAZAAR_CAP_CASES`) to 128 and uses the fixed seeds, so a CI
failure reproduces locally with the same command. These campaigns never touch a live network and do
not change contract behavior.
