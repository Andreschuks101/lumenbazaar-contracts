// Deterministic property tests for the `upto` cap invariants.
//
// Randomized values are drawn from strategies biased toward zero, negative, cap-adjacent,
// `i128::MAX`-adjacent, and decimal-scaled boundaries. The campaigns exercise the validation
// functions directly and the real contract against a real Stellar asset contract. They never
// touch a live network and do not change contract behaviour.
//
// Reproducibility: fixed seed `0x4c554d45_43415053`. Override with `LUMENBAZAAR_CAP_SEED`
// (decimal or 0x-prefixed hex) and `LUMENBAZAAR_CAP_CASES`. See docs/property-testing.md.

extern crate std;

use super::*;
use std::vec;
use crate::validation::{
    validate_asset_cap, validate_create_session, validate_settlement_amount, MAX_TOKEN_DECIMALS,
};
use proptest::{
    collection,
    prelude::*,
    prop_oneof,
    test_runner::{Config as ProptestConfig, RngSeed},
};

const CAP_CASES: u32 = 128;
const CAP_SEED: u64 = 0x4c55_4d45_4341_5053;
const EXPIRES_AT: u32 = 50;

fn parse_u64(value: &str) -> Option<u64> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}

fn cap_config() -> ProptestConfig {
    let cases = std::env::var("LUMENBAZAAR_CAP_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(CAP_CASES);
    let seed = std::env::var("LUMENBAZAAR_CAP_SEED")
        .ok()
        .and_then(|value| parse_u64(&value))
        .unwrap_or(CAP_SEED);
    ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(seed),
        max_shrink_iters: 10_000,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Valid caps: tiny, token-scale, and overflow-adjacent.
fn cap_strategy() -> impl Strategy<Value = i128> {
    prop_oneof![
        Just(1_i128),
        Just(2_i128),
        Just(10_000_000_i128),
        Just(i128::MAX),
        Just(i128::MAX - 1),
        Just(i128::MAX / 2),
        Just(i128::MAX / 2 + 1),
        1_i128..=1_000_000_000_i128,
        1_i128..=i128::MAX,
    ]
}

/// Candidate amounts: invalid, cap-adjacent, extreme, and uniformly random values.
fn amount_for(cap: i128) -> impl Strategy<Value = i128> {
    prop_oneof![
        Just(0_i128),
        Just(-1_i128),
        Just(1_i128),
        Just(i128::MIN),
        Just(i128::MIN + 1),
        Just(i128::MAX),
        Just(cap),
        Just(cap - 1),
        Just(cap.saturating_add(1)),
        Just(cap / 2),
        i128::MIN..=i128::MAX,
        1_i128..=cap,
    ]
}

fn cap_and_amount() -> impl Strategy<Value = (i128, i128)> {
    cap_strategy().prop_flat_map(|cap| (Just(cap), amount_for(cap)))
}

fn cap_and_amounts() -> impl Strategy<Value = (i128, std::vec::Vec<i128>)> {
    cap_strategy().prop_flat_map(|cap| (Just(cap), collection::vec(amount_for(cap), 1..=4)))
}

/// The specified outcome of a settlement against an open session.
fn expected_settlement(cap: i128, amount: i128, ledger: u32, expires: u32) -> Result<(), ContractError> {
    if amount <= 0 {
        Err(ContractError::InvalidAmount)
    } else if amount > cap {
        Err(ContractError::AmountExceedsCap)
    } else if ledger >= expires {
        Err(ContractError::ExpiredSession)
    } else {
        Ok(())
    }
}

fn open_session(env: &Env, cap: i128, expires: u32) -> Session {
    Session {
        id: BytesN::from_array(env, &[1; 32]),
        buyer: Address::generate(env),
        seller: Address::generate(env),
        asset: Address::generate(env),
        max_amount: cap,
        escrowed_amount: cap,
        settled_amount: 0,
        expires_at_ledger: expires,
        resource_hash: BytesN::from_array(env, &[2; 32]),
        usage_hash: None,
        status: SessionStatus::Open,
        creation_ledger: 0,
        finalized_ledger: None,
        resource_scheme_version: 1,
        usage_scheme_version: None,
    }
}

/// Replays settlement attempts against a funded session and checks every invariant.
fn run_settlements(cap: i128, amounts: &[i128]) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, cap);
    let token = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &cap,
        &EXPIRES_AT,
        &BytesN::from_array(&env, &[31; 32]),
        &BytesN::from_array(&env, &[30; 32]),
        &unique_idempotency_key(&env),
    );
    let usage_hash = BytesN::from_array(&env, &[32; 32]);
    let mut finalized = false;

    for (index, amount) in amounts.iter().copied().enumerate() {
        let context = std::format!("cap {cap} attempt {index} amount {amount} in {amounts:?}");
        let session_before = client.get_session(&session_id);
        let balances_before = (
            token.balance(&buyer),
            token.balance(&seller),
            token.balance(&contract_id),
        );
        let liability_before = client.get_escrow_liability(&asset);

        let result = client.try_settle(&session_id, &amount, &usage_hash);
        let events = contract_event_count(&env, &contract_id);
        let expected = if finalized {
            Err(ContractError::SessionAlreadySettled)
        } else {
            expected_settlement(cap, amount, 0, EXPIRES_AT)
        };

        match expected {
            Ok(()) => {
                assert!(result.is_ok(), "valid settlement rejected: {context}");
                assert_eq!(events, 1, "{context}");
                finalized = true;
                let session = client.get_session(&session_id);
                assert_eq!(session.status, SessionStatus::Settled, "{context}");
                assert_eq!(session.settled_amount, amount, "{context}");
                assert!(session.settled_amount <= session.max_amount, "{context}");
                assert_eq!(token.balance(&seller), amount, "{context}");
                assert_eq!(token.balance(&buyer), cap - amount, "{context}");
                assert_eq!(token.balance(&contract_id), 0, "{context}");
                assert_eq!(client.get_escrow_liability(&asset), 0, "{context}");
            }
            Err(error) => {
                assert_eq!(result.map(|_| ()), Err(Ok(error)), "{context}");
                assert_eq!(events, 0, "failed settlement emitted an event: {context}");
                assert_eq!(client.get_session(&session_id), session_before, "{context}");
                assert_eq!(
                    (
                        token.balance(&buyer),
                        token.balance(&seller),
                        token.balance(&contract_id)
                    ),
                    balances_before,
                    "{context}"
                );
                assert_eq!(
                    client.get_escrow_liability(&asset),
                    liability_before,
                    "{context}"
                );
            }
        }
        assert!(token.balance(&seller) <= cap, "seller exceeded the cap: {context}");
    }
}

proptest! {
    #![proptest_config(cap_config())]

    #[test]
    fn settlement_validation_matches_the_specification((cap, amount) in cap_and_amount(),
                                                       ledger in 0_u32..100,
                                                       expires in 1_u32..100) {
        let env = Env::default();
        env.ledger().set_sequence_number(ledger);
        let session = open_session(&env, cap, expires);

        let result = validate_settlement_amount(&env, &session, amount);
        prop_assert_eq!(result, expected_settlement(cap, amount, ledger, expires));
        if amount > cap || amount <= 0 {
            prop_assert!(result.is_err());
        }
    }

    #[test]
    fn asset_cap_validation_accepts_only_positive_values(value in prop_oneof![
        Just(0_i128), Just(-1_i128), Just(1_i128), Just(i128::MIN), Just(i128::MAX),
        i128::MIN..=i128::MAX,
    ]) {
        prop_assert_eq!(validate_asset_cap(value).is_ok(), value > 0);
    }

    #[test]
    fn create_validation_accepts_only_positive_amounts(amount in prop_oneof![
        Just(0_i128), Just(-1_i128), Just(1_i128), Just(i128::MIN), Just(i128::MAX),
        i128::MIN..=i128::MAX,
    ]) {
        let env = Env::default();
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let hash = BytesN::from_array(&env, &[3; 32]);

        let result = validate_create_session(&env, &buyer, &seller, &asset, amount, 10, &hash);
        if amount > 0 {
            prop_assert_eq!(result, Ok(()));
        } else {
            prop_assert_eq!(result, Err(ContractError::InvalidAmount));
        }
    }

    #[test]
    fn random_settlements_never_exceed_the_cap_and_failures_change_nothing(
        (cap, amounts) in cap_and_amounts(),
    ) {
        run_settlements(cap, &amounts);
    }

    #[test]
    fn session_creation_enforces_the_asset_cap_without_side_effects(
        (policy_cap, amount) in cap_strategy().prop_flat_map(|cap| (Just(cap), amount_for(cap))),
    ) {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let funded = if amount > 0 { amount } else { 1 };
        let asset = create_test_asset(&env, &buyer, funded);
        let token = token::Client::new(&env, &asset);
        client.initialize(&admin, &soroban_sdk::vec![&env, asset_policy(&asset, policy_cap)]);

        let result = client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &amount,
            &EXPIRES_AT,
            &BytesN::from_array(&env, &[33; 32]),
            &BytesN::from_array(&env, &[34; 32]),
            &unique_idempotency_key(&env),
        );
        let events = contract_event_count(&env, &contract_id);

        let expected = if amount <= 0 {
            Err(ContractError::InvalidAmount)
        } else if amount > policy_cap {
            Err(ContractError::AmountExceedsCap)
        } else {
            Ok(())
        };
        match expected {
            Ok(()) => prop_assert!(result.is_ok()),
            Err(error) => prop_assert_eq!(result.map(|_| ()), Err(Ok(error))),
        }
        if expected.is_ok() {
            prop_assert_eq!(token.balance(&contract_id), amount);
            prop_assert_eq!(client.get_escrow_liability(&asset), amount);
        } else {
            prop_assert_eq!(events, 0);
            prop_assert_eq!(token.balance(&buyer), funded);
            prop_assert_eq!(token.balance(&contract_id), 0);
            prop_assert_eq!(client.get_escrow_liability(&asset), 0);
            env.as_contract(&contract_id, || {
                assert_eq!(storage::read_next_session_sequence(&env), 0);
            });
        }
    }

    #[test]
    fn cap_updates_accept_only_positive_values_and_keep_the_previous_cap_on_failure(
        (initial, update) in cap_strategy().prop_flat_map(|cap| (Just(cap), amount_for(cap))),
    ) {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let asset = create_test_asset(&env, &admin, 1);
        client.initialize(&admin, &soroban_sdk::vec![&env, asset_policy(&asset, initial)]);

        let result = client.try_set_asset_max_session_amount(&asset, &update);
        if update <= 0 {
            prop_assert_eq!(result.map(|_| ()), Err(Ok(ContractError::InvalidAssetCap)));
            prop_assert_eq!(client.get_asset_max_session_amount(&asset), initial);
        } else if update > initial {
            prop_assert_eq!(result.map(|_| ()), Err(Ok(ContractError::CapIncreasePending)));
            prop_assert_eq!(
                result.map(|_| ()),
                Err(Ok(ContractError::CapIncreasePending))
            );
            prop_assert_eq!(client.get_asset_max_session_amount(&asset), initial);
        } else {
            prop_assert!(result.is_ok());
            prop_assert_eq!(client.get_asset_max_session_amount(&asset), update);
        }
    }
}

#[test]
fn zero_negative_maximum_and_overflow_adjacent_amounts_are_rejected_or_exact() {
    // Every invalid value is rejected without changing state; the final maximum value settles
    // exactly with no refund.
    run_settlements(
        i128::MAX,
        &[
            0,
            -1,
            i128::MIN,
            i128::MIN + 1,
            i128::MAX,
            i128::MAX,
        ],
    );
    // One below the maximum cap: the maximum amount is over the cap by exactly one.
    run_settlements(i128::MAX - 1, &[i128::MAX, i128::MAX - 1]);
    // Smallest cap: only the amount 1 is valid.
    run_settlements(1, &[0, 2, -1, i128::MAX, 1]);
}

#[test]
fn decimal_scaled_cap_boundaries_are_exact_for_every_supported_decimal_count() {
    let env = Env::default();
    for decimals in 0..=MAX_TOKEN_DECIMALS {
        let unit = 10_i128.pow(decimals);
        for cap in [unit - 1, unit, unit + 1] {
            if cap < 1 {
                assert_eq!(validate_asset_cap(cap), Err(ContractError::InvalidAssetCap));
                continue;
            }
            assert_eq!(validate_asset_cap(cap), Ok(()));
            let session = open_session(&env, cap, 100);
            for amount in [0, 1, unit - 1, unit, unit + 1, cap - 1, cap, cap + 1] {
                assert_eq!(
                    validate_settlement_amount(&env, &session, amount),
                    expected_settlement(cap, amount, 0, 100),
                    "decimals {decimals} cap {cap} amount {amount}"
                );
            }
        }
    }
    // 10^38 is the largest decimal unit that fits an i128 and sits just under i128::MAX.
    assert!(10_i128.pow(MAX_TOKEN_DECIMALS) < i128::MAX);
}

#[test]
fn decimal_scaled_caps_settle_exactly_through_the_contract() {
    for decimals in [0_u32, 7, 18, MAX_TOKEN_DECIMALS] {
        let unit = 10_i128.pow(decimals);
        run_settlements(unit, &[0, unit + 1, unit - 1]);
        run_settlements(unit, &[unit + 1, unit]);
    }
}

#[test]
fn overflow_adjacent_escrow_totals_fail_without_state_change() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let seller = Address::generate(&env);
    let first_buyer = Address::generate(&env);
    let second_buyer = Address::generate(&env);
    let third_buyer = Address::generate(&env);
    let asset = create_test_asset(&env, &first_buyer, i128::MAX - 5);
    token::StellarAssetClient::new(&env, &asset).mint(&second_buyer, &5);
    token::StellarAssetClient::new(&env, &asset).mint(&third_buyer, &1);
    let token = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &first_buyer, &asset);
    let hash = BytesN::from_array(&env, &[34; 32]);

    client.create_session(
        &first_buyer,
        &seller,
        &asset,
        &(i128::MAX - 5),
        &EXPIRES_AT,
        &hash,
        &BytesN::from_array(&env, &[35; 32]),
        &unique_idempotency_key(&env),
    );
    // Exactly reaching i128::MAX of aggregate liability is still valid.
    client.create_session(
        &second_buyer,
        &seller,
        &asset,
        &5,
        &EXPIRES_AT,
        &hash,
        &BytesN::from_array(&env, &[36; 32]),
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_escrow_liability(&asset), i128::MAX);
    assert_eq!(token.balance(&contract_id), i128::MAX);

    // One more unit would overflow the aggregate; it must fail with no side effect.
    let result = client.try_create_session(
        &third_buyer,
        &seller,
        &asset,
        &1,
        &EXPIRES_AT,
        &hash,
        &BytesN::from_array(&env, &[37; 32]),
        &unique_idempotency_key(&env),
    );
    assert!(result.is_err());
    assert_eq!(client.get_escrow_liability(&asset), i128::MAX);
    assert_eq!(token.balance(&third_buyer), 1);
    assert_eq!(token.balance(&contract_id), i128::MAX);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 2);
    });
}

#[test]
fn cap_seed_override_parses_decimal_and_hex() {
    assert_eq!(parse_u64("7"), Some(7));
    assert_eq!(parse_u64("0x4c554d4543415053"), Some(CAP_SEED));
    assert_eq!(parse_u64("-1"), None);
}
