// State-transition property tests for `upto-session`.
//
// A pure model of the permitted and forbidden transitions is replayed against the real contract
// and a real Stellar asset contract for bounded action sequences. Every step checks the exact
// result, the emitted events, all token balances, the escrow liability, and the stored session.
//
// Reproducibility: the proptest campaign uses a fixed seed. Override with
// `LUMENBAZAAR_TRANSITION_SEED` (decimal or 0x-prefixed hex) and
// `LUMENBAZAAR_TRANSITION_CASES`. A failure prints the shrunk action sequence, which can be
// replayed verbatim with `run_sequence`. See docs/property-testing.md.

extern crate std;

use super::*;
use proptest::{
    collection,
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use soroban_sdk::{
    testutils::{AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke},
    IntoVal, Symbol, Val, Vec as SorobanVec,
};
use std::vec::Vec as StdVec;

const TRANSITION_CASES: u32 = 128;
const TRANSITION_SEED: u64 = 0x4c55_4d45_5452_4e53;
const MAX_SEQUENCE_LEN: usize = 16;
const CAP: i128 = 1_000;
const EXPIRES_AT: u32 = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Amount {
    Zero,
    One,
    Half,
    Cap,
    OverCap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    Before,
    At,
    After,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Settle(Amount),
    Cancel,
    Reject,
    Recover,
    RecoverBatch,
    ExtendTtl,
    Advance(Target),
}

const ALL_ACTIONS: [Action; 13] = [
    Action::Settle(Amount::Zero),
    Action::Settle(Amount::One),
    Action::Settle(Amount::Half),
    Action::Settle(Amount::Cap),
    Action::Settle(Amount::OverCap),
    Action::Cancel,
    Action::Reject,
    Action::Recover,
    Action::RecoverBatch,
    Action::ExtendTtl,
    Action::Advance(Target::Before),
    Action::Advance(Target::At),
    Action::Advance(Target::After),
];

// Alphabet for the exhaustive depth-3 sweep. The ledger starts before expiry, so the sweep covers
// both sides of the expiry boundary; zero and exact-cap amounts are left to the random campaign.
const EXHAUSTIVE_ACTIONS: [Action; 7] = [
    Action::Settle(Amount::Half),
    Action::Settle(Amount::OverCap),
    Action::Cancel,
    Action::Reject,
    Action::Recover,
    Action::ExtendTtl,
    Action::Advance(Target::At),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelStatus {
    Open,
    Settled,
    Cancelled,
    Expired,
    Rejected,
}

#[derive(Clone, Copy, Debug)]
struct Model {
    status: ModelStatus,
    ledger: u32,
    settled: i128,
}

impl Model {
    fn amount(kind: Amount) -> i128 {
        match kind {
            Amount::Zero => 0,
            Amount::One => 1,
            Amount::Half => CAP / 2,
            Amount::Cap => CAP,
            Amount::OverCap => CAP + 1,
        }
    }

    fn terminal_error(&self) -> Option<ContractError> {
        match self.status {
            ModelStatus::Open => None,
            ModelStatus::Settled => Some(ContractError::SessionAlreadySettled),
            ModelStatus::Cancelled => Some(ContractError::SessionCancelled),
            ModelStatus::Expired => Some(ContractError::SessionExpired),
            ModelStatus::Rejected => Some(ContractError::SessionRejected),
        }
    }

    fn settle(&self, amount: i128) -> Result<(), ContractError> {
        if let Some(error) = self.terminal_error() {
            return Err(error);
        }
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        if amount > CAP {
            return Err(ContractError::AmountExceedsCap);
        }
        if self.ledger >= EXPIRES_AT {
            return Err(ContractError::ExpiredSession);
        }
        Ok(())
    }

    fn cancel(&self) -> Result<(), ContractError> {
        if let Some(error) = self.terminal_error() {
            return Err(error);
        }
        if self.ledger >= EXPIRES_AT {
            return Err(ContractError::ExpiredSession);
        }
        Ok(())
    }

    fn reject(&self) -> Result<(), ContractError> {
        if let Some(error) = self.terminal_error() {
            return Err(error);
        }
        if self.ledger >= EXPIRES_AT {
            return Err(ContractError::ExpiredSession);
        }
        Ok(())
    }

    fn recover(&self) -> Result<(), ContractError> {
        if let Some(error) = self.terminal_error() {
            return Err(error);
        }
        if self.ledger < EXPIRES_AT {
            return Err(ContractError::SessionNotExpired);
        }
        Ok(())
    }

    fn extend(&self) -> Result<(), ContractError> {
        match self.status {
            ModelStatus::Open => Ok(()),
            _ => Err(ContractError::TtlExtensionFailed),
        }
    }

    fn batch_status(&self) -> BatchRecoveryStatus {
        match self.status {
            ModelStatus::Settled => BatchRecoveryStatus::AlreadySettled,
            ModelStatus::Cancelled => BatchRecoveryStatus::AlreadyCancelled,
            ModelStatus::Expired => BatchRecoveryStatus::AlreadyRecovered,
            ModelStatus::Rejected => BatchRecoveryStatus::AlreadyRejected,
            ModelStatus::Open if self.ledger < EXPIRES_AT => BatchRecoveryStatus::NotExpired,
            ModelStatus::Open => BatchRecoveryStatus::Recovered,
        }
    }

    fn balances(&self) -> (i128, i128, i128) {
        match self.status {
            ModelStatus::Open => (0, 0, CAP),
            ModelStatus::Settled => (CAP - self.settled, self.settled, 0),
            ModelStatus::Cancelled | ModelStatus::Expired | ModelStatus::Rejected => (CAP, 0, 0),
        }
    }

    fn liability(&self) -> i128 {
        if self.status == ModelStatus::Open {
            CAP
        } else {
            0
        }
    }
}

/// What a replayed sequence proved, for targeted assertions in named tests.
#[derive(Debug, Default)]
struct Outcome {
    finalizations: u32,
    successful_events: u32,
}

fn check_result<T: core::fmt::Debug>(
    actual: Result<Result<T, soroban_sdk::ConversionError>, Result<ContractError, soroban_sdk::InvokeError>>,
    expected: Result<(), ContractError>,
    context: &str,
) -> bool {
    match expected {
        Ok(()) => {
            assert!(actual.is_ok(), "expected success, got {actual:?}: {context}");
            true
        }
        Err(error) => {
            assert_eq!(
                actual.map(|_| ()),
                Err(Ok(error)),
                "wrong failure: {context}"
            );
            false
        }
    }
}

fn run_sequence(actions: &[Action]) -> Outcome {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, CAP);
    let token = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &CAP,
        &EXPIRES_AT,
        &BytesN::from_array(&env, &[41; 32]),
        &unique_idempotency_key(&env),
    );

    let mut model = Model {
        status: ModelStatus::Open,
        ledger: 0,
        settled: 0,
    };
    let mut outcome = Outcome::default();

    for (step, action) in actions.iter().enumerate() {
        let context = std::format!("step {step} {action:?} in {actions:?}");
        let before = model;
        let session_before = client.get_session(&session_id);
        let usage_hash = BytesN::from_array(&env, &[u8::try_from(step % 200 + 50).unwrap(); 32]);

        let (succeeded, expect_event) = match *action {
            Action::Settle(kind) => {
                let amount = Model::amount(kind);
                let result = client.try_settle(&session_id, &amount, &usage_hash);
                let ok = check_result(result, before.settle(amount), &context);
                if ok {
                    model.status = ModelStatus::Settled;
                    model.settled = amount;
                }
                (ok, ok)
            }
            Action::Cancel => {
                let ok = check_result(client.try_cancel(&session_id), before.cancel(), &context);
                if ok {
                    model.status = ModelStatus::Cancelled;
                }
                (ok, ok)
            }
            Action::Reject => {
                let ok = check_result(client.try_reject(&session_id), before.reject(), &context);
                if ok {
                    model.status = ModelStatus::Rejected;
                }
                (ok, ok)
            }
            Action::Recover => {
                let ok = check_result(
                    client.try_recover_expired(&session_id),
                    before.recover(),
                    &context,
                );
                if ok {
                    model.status = ModelStatus::Expired;
                }
                (ok, ok)
            }
            Action::RecoverBatch => {
                let ids = soroban_sdk::vec![&env, session_id.clone()];
                let results = client.recover_expired_batch(&ids);
                let result = results.get(0).expect("one batch result");
                let expected = before.batch_status();
                assert_eq!(result.status, expected, "batch status: {context}");
                let recovered = expected == BatchRecoveryStatus::Recovered;
                assert_eq!(
                    result.refunded_amount,
                    if recovered { CAP } else { 0 },
                    "batch refund: {context}"
                );
                if recovered {
                    model.status = ModelStatus::Expired;
                }
                (recovered, recovered)
            }
            Action::ExtendTtl => {
                check_result(client.try_extend_ttl(&session_id), before.extend(), &context);
                (false, false)
            }
            Action::Advance(target) => {
                let ledger = match target {
                    Target::Before => EXPIRES_AT - 1,
                    Target::At => EXPIRES_AT,
                    Target::After => EXPIRES_AT + 7,
                };
                // Ledgers are monotonic.
                model.ledger = model.ledger.max(ledger);
                env.ledger().set_sequence_number(model.ledger);
                (false, false)
            }
        };

        // Events describe the most recent invocation, so read them before any query.
        let events = contract_event_count(&env, &contract_id);
        let session_after = client.get_session(&session_id);
        let finalized = succeeded && before.status == ModelStatus::Open;
        if finalized {
            outcome.finalizations += 1;
            assert_eq!(events, 1, "finalizing step must emit exactly one event: {context}");
            outcome.successful_events += 1;
        } else if matches!(action, Action::Advance(_)) {
            assert_eq!(session_after, session_before, "ledger advance mutated state: {context}");
        } else {
            // Invalid sequences and no-op successes emit no success event and move nothing.
            assert_eq!(events, 0, "unexpected event: {context}");
            assert_eq!(session_after, session_before, "state changed: {context}");
        }
        assert!(!expect_event || finalized, "event without finalization: {context}");
        assert!(outcome.finalizations <= 1, "finalized twice: {context}");

        // Side effects and conservation after every step.
        let (buyer_balance, seller_balance, contract_balance) = model.balances();
        assert_eq!(token.balance(&buyer), buyer_balance, "buyer: {context}");
        assert_eq!(token.balance(&seller), seller_balance, "seller: {context}");
        assert_eq!(token.balance(&contract_id), contract_balance, "escrow: {context}");
        assert_eq!(
            buyer_balance + seller_balance + contract_balance,
            CAP,
            "conservation: {context}"
        );
        assert_eq!(
            client.get_escrow_liability(&asset),
            model.liability(),
            "liability: {context}"
        );
        let expected_status = match model.status {
            ModelStatus::Open => SessionStatus::Open,
            ModelStatus::Settled => SessionStatus::Settled,
            ModelStatus::Cancelled => SessionStatus::Cancelled,
            ModelStatus::Expired => SessionStatus::Expired,
            ModelStatus::Rejected => SessionStatus::Rejected,
        };
        assert_eq!(session_after.status, expected_status, "status: {context}");
        assert_eq!(session_after.settled_amount, model.settled, "settled: {context}");
        assert!(session_after.settled_amount <= session_after.max_amount, "cap: {context}");
    }
    outcome
}

fn parse_u64(value: &str) -> Option<u64> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}

fn transition_config() -> ProptestConfig {
    let cases = std::env::var("LUMENBAZAAR_TRANSITION_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(TRANSITION_CASES);
    let seed = std::env::var("LUMENBAZAAR_TRANSITION_SEED")
        .ok()
        .and_then(|value| parse_u64(&value))
        .unwrap_or(TRANSITION_SEED);
    ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(seed),
        max_shrink_iters: 10_000,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn sequence_strategy() -> impl Strategy<Value = StdVec<Action>> {
    collection::vec(
        (0..ALL_ACTIONS.len()).prop_map(|index| ALL_ACTIONS[index]),
        1..=MAX_SEQUENCE_LEN,
    )
}

proptest! {
    #![proptest_config(transition_config())]

    #[test]
    fn generated_sequences_follow_the_transition_table(actions in sequence_strategy()) {
        let outcome = run_sequence(&actions);
        prop_assert!(outcome.finalizations <= 1);
    }
}

fn all_sequences(max_depth: usize) -> StdVec<StdVec<Action>> {
    let mut all = StdVec::new();
    let mut layer: StdVec<StdVec<Action>> = std::vec![StdVec::new()];
    for _ in 0..max_depth {
        layer = layer
            .iter()
            .flat_map(|prefix| {
                EXHAUSTIVE_ACTIONS.iter().map(move |action| {
                    let mut next = prefix.clone();
                    next.push(*action);
                    next
                })
            })
            .collect();
        all.extend(layer.iter().cloned());
    }
    all
}

#[test]
fn every_sequence_up_to_depth_three_follows_the_transition_table() {
    let sequences = all_sequences(3);
    // 7 + 49 + 343 sequences, each finalizing at most once.
    assert_eq!(sequences.len(), 399);
    let finalized: u32 = sequences
        .iter()
        .map(|actions| run_sequence(actions).finalizations)
        .sum();
    assert!(finalized > 0);
}

#[test]
fn settled_sessions_never_cancel_extend_recover_or_settle_again() {
    let outcome = run_sequence(&[
        Action::Settle(Amount::Half),
        Action::Cancel,
        Action::ExtendTtl,
        Action::Recover,
        Action::RecoverBatch,
        Action::Settle(Amount::Cap),
        Action::Advance(Target::After),
        Action::Cancel,
        Action::Recover,
        Action::RecoverBatch,
        Action::ExtendTtl,
    ]);
    assert_eq!(outcome.finalizations, 1);
    assert_eq!(outcome.successful_events, 1);
}

#[test]
fn cancelled_sessions_never_settle_or_recover() {
    let outcome = run_sequence(&[
        Action::Cancel,
        Action::Settle(Amount::One),
        Action::Settle(Amount::Cap),
        Action::Advance(Target::At),
        Action::Recover,
        Action::RecoverBatch,
        Action::ExtendTtl,
        Action::Cancel,
    ]);
    assert_eq!(outcome.finalizations, 1);
}

#[test]
fn expired_sessions_recover_once_and_then_reject_every_action() {
    let outcome = run_sequence(&[
        Action::Advance(Target::Before),
        Action::Recover,
        Action::Advance(Target::At),
        Action::Settle(Amount::One),
        Action::Cancel,
        Action::RecoverBatch,
        Action::Recover,
        Action::Settle(Amount::Half),
        Action::Cancel,
        Action::ExtendTtl,
        Action::RecoverBatch,
    ]);
    assert_eq!(outcome.finalizations, 1);
}

#[test]
fn invalid_amounts_leave_an_open_session_untouched_until_a_valid_settlement() {
    let outcome = run_sequence(&[
        Action::Settle(Amount::Zero),
        Action::Settle(Amount::OverCap),
        Action::ExtendTtl,
        Action::Settle(Amount::Cap),
        Action::Settle(Amount::Cap),
    ]);
    assert_eq!(outcome.finalizations, 1);
}

#[test]
fn transition_seed_override_parses_decimal_and_hex() {
    assert_eq!(parse_u64("42"), Some(42));
    assert_eq!(parse_u64("0x4c554d4554524e53"), Some(TRANSITION_SEED));
    assert_eq!(parse_u64("nope"), None);
}

#[test]
fn rejected_sessions_never_settle_cancel_extend_or_recover() {
    let outcome = run_sequence(&[
        Action::Reject,
        Action::Settle(Amount::One),
        Action::Settle(Amount::Cap),
        Action::Cancel,
        Action::ExtendTtl,
        Action::Advance(Target::At),
        Action::Recover,
        Action::RecoverBatch,
        Action::Reject,
    ]);
    assert_eq!(outcome.finalizations, 1);
    assert_eq!(outcome.successful_events, 1);
}

fn contract_auth_invocation(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: SorobanVec<Val>,
) -> AuthorizedInvocation {
    AuthorizedInvocation {
        function: AuthorizedFunction::Contract((contract.clone(), Symbol::new(env, fn_name), args)),
        sub_invocations: std::vec![],
    }
}

struct ExplicitSessionSetup {
    env: Env,
    contract_id: Address,
    buyer: Address,
    seller: Address,
    attacker: Address,
    asset: Address,
    session_id: BytesN<32>,
}

fn setup_explicit_auth_session() -> ExplicitSessionSetup {
    let env = Env::default();
    env.ledger().set_sequence_number(10);
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let attacker = Address::generate(&env);
    assert_ne!(admin, buyer);
    assert_ne!(buyer, seller);
    assert_ne!(buyer, attacker);
    assert_ne!(seller, attacker);

    let asset = create_test_asset(&env, &buyer, CAP);
    let asset_policies = soroban_sdk::vec![&env, asset_policy(&asset, CAP)];
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (admin.clone(), asset_policies.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin, &asset_policies);
    assert_eq!(
        env.auths(),
        std::vec![(
            admin,
            contract_auth_invocation(
                &env,
                &contract_id,
                "initialize",
                (
                    client.get_admin(),
                    asset_policies,
                )
                    .into_val(&env),
            ),
        )]
    );

    let resource_hash = BytesN::from_array(&env, &[0x44; 32]);
    let idempotency_key = unique_idempotency_key(&env);
    let session_id = client
        .mock_auths(&[MockAuth {
            address: &buyer,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "create_session",
                args: (
                    buyer.clone(),
                    seller.clone(),
                    asset.clone(),
                    CAP,
                    EXPIRES_AT,
                    resource_hash.clone(),
                    idempotency_key.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[MockAuthInvoke {
                    contract: &asset,
                    fn_name: "transfer",
                    args: (buyer.clone(), contract_id.clone(), CAP).into_val(&env),
                    sub_invokes: &[],
                }],
            },
        }])
        .create_session(
            &buyer,
            &seller,
            &asset,
            &CAP,
            &EXPIRES_AT,
            &resource_hash,
            &idempotency_key,
        );
    assert_eq!(
        env.auths(),
        std::vec![(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "create_session"),
                    (
                        buyer.clone(),
                        seller.clone(),
                        asset.clone(),
                        CAP,
                        EXPIRES_AT,
                        resource_hash,
                        idempotency_key,
                    )
                        .into_val(&env),
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        asset.clone(),
                        Symbol::new(&env, "transfer"),
                        (buyer.clone(), contract_id.clone(), CAP).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                }],
            },
        )]
    );

    ExplicitSessionSetup {
        env,
        contract_id,
        buyer,
        seller,
        attacker,
        asset,
        session_id,
    }
}

macro_rules! contract_mock_auth {
    ($env:expr, $contract_id:expr, $signer:expr, $fn_name:expr, $args:expr) => {
        MockAuth {
            address: $signer,
            invoke: &MockAuthInvoke {
                contract: $contract_id,
                fn_name: $fn_name,
                args: ($args).into_val($env),
                sub_invokes: &[],
            },
        }
    };
}

#[test]
fn seller_rejection_before_settlement_prevents_payout_and_asserts_auth_evidence() {
    let s = setup_explicit_auth_session();
    let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.asset);
    let usage_hash = BytesN::from_array(&s.env, &[0x55; 32]);

    // 1. Seller rejects first in ledger 10 with explicit seller authorization tree.
    client
        .mock_auths(&[contract_mock_auth!(
            &s.env,
            &s.contract_id,
            &s.seller,
            "reject",
            (s.session_id.clone(),)
        )])
        .reject(&s.session_id);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.seller.clone(),
            contract_auth_invocation(
                &s.env,
                &s.contract_id,
                "reject",
                (s.session_id.clone(),).into_val(&s.env),
            ),
        )]
    );
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.contract_id),
        std::vec![events::SessionRejected {
            session_id: s.session_id.clone(),
            seller: s.seller.clone(),
            buyer: s.buyer.clone(),
            asset: s.asset.clone(),
            event_version: 2,
            refunded_amount: CAP,
        }
        .to_xdr(&s.env, &s.contract_id)]
    );

    assert_eq!(token.balance(&s.buyer), CAP);
    assert_eq!(token.balance(&s.seller), 0);
    assert_eq!(token.balance(&s.contract_id), 0);
    assert_eq!(client.get_escrow_liability(&s.asset), 0);
    assert_eq!(client.get_buyer_exposure(&s.buyer, &s.asset), 0);
    assert_eq!(client.get_seller_exposure(&s.seller, &s.asset), 0);
    assert_eq!(client.get_buyer_open_sessions(&s.buyer), 0);
    assert_eq!(client.get_seller_open_sessions(&s.seller), 0);
    let session_after_reject = client.get_session(&s.session_id);
    assert_eq!(session_after_reject.status, SessionStatus::Rejected);
    assert_eq!(session_after_reject.finalized_ledger, Some(10));
    match client.get_terminal_evidence(&s.session_id) {
        TerminalEvidence::Terminal(evidence) => {
            assert_eq!(evidence.status, SessionStatus::Rejected);
            assert_eq!(evidence.finalized_ledger, 10);
            assert_eq!(evidence.usage_hash, None);
        }
        other => panic!("expected terminal evidence, got {other:?}"),
    }

    // 2. Seller settlement in the same ledger must fail closed with SessionRejected.
    let settle_res = client
        .mock_auths(&[contract_mock_auth!(
            &s.env,
            &s.contract_id,
            &s.seller,
            "settle",
            (s.session_id.clone(), 400_i128, usage_hash.clone())
        )])
        .try_settle(&s.session_id, &400, &usage_hash);
    assert_eq!(settle_res, Err(Ok(ContractError::SessionRejected)));
    assert_eq!(s.env.auths(), std::vec![]);
    assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);

    // Balances, exposures, and session state remain completely unchanged (refund occurred once).
    assert_eq!(token.balance(&s.buyer), CAP);
    assert_eq!(token.balance(&s.seller), 0);
    assert_eq!(token.balance(&s.contract_id), 0);
    assert_eq!(client.get_escrow_liability(&s.asset), 0);
    assert_eq!(client.get_session(&s.session_id), session_after_reject);
}

#[test]
fn seller_settlement_before_rejection_prevents_refund_and_asserts_auth_evidence() {
    for actual_amount in [400_i128, CAP] {
        let s = setup_explicit_auth_session();
        let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
        let token = token::Client::new(&s.env, &s.asset);
        let usage_hash = BytesN::from_array(&s.env, &[0x66; 32]);
        let expected_refund = CAP - actual_amount;

        // 1. Seller settles first in ledger 10 with explicit seller authorization tree.
        client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.seller,
                "settle",
                (s.session_id.clone(), actual_amount, usage_hash.clone())
            )])
            .settle(&s.session_id, &actual_amount, &usage_hash);
        assert_eq!(
            s.env.auths(),
            std::vec![(
                s.seller.clone(),
                contract_auth_invocation(
                    &s.env,
                    &s.contract_id,
                    "settle",
                    (s.session_id.clone(), actual_amount, usage_hash.clone()).into_val(&s.env),
                ),
            )]
        );
        assert_eq!(
            s.env.events().all().filter_by_contract(&s.contract_id),
            std::vec![events::SessionSettled {
                session_id: s.session_id.clone(),
                seller: s.seller.clone(),
                asset: s.asset.clone(),
                actual_amount,
                usage_hash: usage_hash.clone(),
                event_version: 2,
                refunded_amount: expected_refund,
            }
            .to_xdr(&s.env, &s.contract_id)]
        );

        assert_eq!(token.balance(&s.seller), actual_amount);
        assert_eq!(token.balance(&s.buyer), expected_refund);
        assert_eq!(token.balance(&s.contract_id), 0);
        assert_eq!(client.get_escrow_liability(&s.asset), 0);
        assert_eq!(client.get_buyer_exposure(&s.buyer, &s.asset), 0);
        assert_eq!(client.get_seller_exposure(&s.seller, &s.asset), 0);
        assert_eq!(client.get_buyer_open_sessions(&s.buyer), 0);
        assert_eq!(client.get_seller_open_sessions(&s.seller), 0);
        let session_after_settle = client.get_session(&s.session_id);
        assert_eq!(session_after_settle.status, SessionStatus::Settled);
        assert_eq!(session_after_settle.settled_amount, actual_amount);
        assert_eq!(session_after_settle.finalized_ledger, Some(10));
        match client.get_terminal_evidence(&s.session_id) {
            TerminalEvidence::Terminal(evidence) => {
                assert_eq!(evidence.status, SessionStatus::Settled);
                assert_eq!(evidence.finalized_ledger, 10);
                assert_eq!(evidence.usage_hash, Some(usage_hash.clone()));
            }
            other => panic!("expected terminal evidence, got {other:?}"),
        }

        // 2. Seller rejection in the same ledger must fail closed with SessionAlreadySettled.
        let reject_res = client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.seller,
                "reject",
                (s.session_id.clone(),)
            )])
            .try_reject(&s.session_id);
        assert_eq!(reject_res, Err(Ok(ContractError::SessionAlreadySettled)));
        assert_eq!(s.env.auths(), std::vec![]);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);

        // Payout + refund occurred once; rejected follow-up moved zero tokens.
        assert_eq!(token.balance(&s.seller), actual_amount);
        assert_eq!(token.balance(&s.buyer), expected_refund);
        assert_eq!(token.balance(&s.contract_id), 0);
        assert_eq!(client.get_escrow_liability(&s.asset), 0);
        assert_eq!(client.get_session(&s.session_id), session_after_settle);
    }
}

#[test]
fn distinct_valid_signers_buyer_cancel_and_seller_terminal_actions_both_orderings() {
    let usage_byte = 0x77_u8;

    // 1. Buyer cancel first, then seller settle in the same ledger.
    {
        let s = setup_explicit_auth_session();
        let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
        let token = token::Client::new(&s.env, &s.asset);
        let usage_hash = BytesN::from_array(&s.env, &[usage_byte; 32]);

        client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.buyer,
                "cancel",
                (s.session_id.clone(),)
            )])
            .cancel(&s.session_id);
        assert_eq!(
            s.env.auths(),
            std::vec![(
                s.buyer.clone(),
                contract_auth_invocation(
                    &s.env,
                    &s.contract_id,
                    "cancel",
                    (s.session_id.clone(),).into_val(&s.env),
                ),
            )]
        );
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 1);

        let settle_err = client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.seller,
                "settle",
                (s.session_id.clone(), 250_i128, usage_hash.clone())
            )])
            .try_settle(&s.session_id, &250, &usage_hash);
        assert_eq!(settle_err, Err(Ok(ContractError::SessionCancelled)));
        assert_eq!(s.env.auths(), std::vec![]);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);
        assert_eq!(token.balance(&s.buyer), CAP);
        assert_eq!(token.balance(&s.seller), 0);
        assert_eq!(client.get_session(&s.session_id).status, SessionStatus::Cancelled);
    }

    // 2. Seller settle first, then buyer cancel in the same ledger.
    {
        let s = setup_explicit_auth_session();
        let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
        let token = token::Client::new(&s.env, &s.asset);
        let usage_hash = BytesN::from_array(&s.env, &[usage_byte; 32]);

        client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.seller,
                "settle",
                (s.session_id.clone(), 250_i128, usage_hash.clone())
            )])
            .settle(&s.session_id, &250, &usage_hash);
        assert_eq!(
            s.env.auths(),
            std::vec![(
                s.seller.clone(),
                contract_auth_invocation(
                    &s.env,
                    &s.contract_id,
                    "settle",
                    (s.session_id.clone(), 250_i128, usage_hash).into_val(&s.env),
                ),
            )]
        );
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 1);

        let cancel_err = client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.buyer,
                "cancel",
                (s.session_id.clone(),)
            )])
            .try_cancel(&s.session_id);
        assert_eq!(cancel_err, Err(Ok(ContractError::SessionAlreadySettled)));
        assert_eq!(s.env.auths(), std::vec![]);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);
        assert_eq!(token.balance(&s.seller), 250);
        assert_eq!(token.balance(&s.buyer), CAP - 250);
        assert_eq!(client.get_session(&s.session_id).status, SessionStatus::Settled);
    }

    // 3. Buyer cancel first, then seller reject in the same ledger.
    {
        let s = setup_explicit_auth_session();
        let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
        let token = token::Client::new(&s.env, &s.asset);

        client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.buyer,
                "cancel",
                (s.session_id.clone(),)
            )])
            .cancel(&s.session_id);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 1);

        let reject_err = client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.seller,
                "reject",
                (s.session_id.clone(),)
            )])
            .try_reject(&s.session_id);
        assert_eq!(reject_err, Err(Ok(ContractError::SessionCancelled)));
        assert_eq!(s.env.auths(), std::vec![]);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);
        assert_eq!(token.balance(&s.buyer), CAP);
        assert_eq!(token.balance(&s.seller), 0);
        assert_eq!(client.get_session(&s.session_id).status, SessionStatus::Cancelled);
    }

    // 4. Seller reject first, then buyer cancel in the same ledger.
    {
        let s = setup_explicit_auth_session();
        let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
        let token = token::Client::new(&s.env, &s.asset);

        client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.seller,
                "reject",
                (s.session_id.clone(),)
            )])
            .reject(&s.session_id);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 1);

        let cancel_err = client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                &s.buyer,
                "cancel",
                (s.session_id.clone(),)
            )])
            .try_cancel(&s.session_id);
        assert_eq!(cancel_err, Err(Ok(ContractError::SessionRejected)));
        assert_eq!(s.env.auths(), std::vec![]);
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);
        assert_eq!(token.balance(&s.buyer), CAP);
        assert_eq!(token.balance(&s.seller), 0);
        assert_eq!(client.get_session(&s.session_id).status, SessionStatus::Rejected);
    }
}

#[test]
fn wrong_signer_cannot_trigger_reject_or_settle_before_valid_terminal_action() {
    let s = setup_explicit_auth_session();
    let client = UptoSessionContractClient::new(&s.env, &s.contract_id);
    let token = token::Client::new(&s.env, &s.asset);
    let usage_hash = BytesN::from_array(&s.env, &[0x88; 32]);

    // Buyer and attacker cannot authorize reject or settle.
    for wrong_signer in [&s.buyer, &s.attacker] {
        assert!(client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                wrong_signer,
                "reject",
                (s.session_id.clone(),)
            )])
            .try_reject(&s.session_id)
            .is_err());
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);
        assert!(client
            .mock_auths(&[contract_mock_auth!(
                &s.env,
                &s.contract_id,
                wrong_signer,
                "settle",
                (s.session_id.clone(), 100_i128, usage_hash.clone())
            )])
            .try_settle(&s.session_id, &100, &usage_hash)
            .is_err());
        assert_eq!(contract_event_count(&s.env, &s.contract_id), 0);
    }

    assert_eq!(client.get_session(&s.session_id).status, SessionStatus::Open);
    assert_eq!(token.balance(&s.buyer), 0);
    assert_eq!(token.balance(&s.contract_id), CAP);

    // Valid seller reject still succeeds cleanly afterwards.
    client
        .mock_auths(&[contract_mock_auth!(
            &s.env,
            &s.contract_id,
            &s.seller,
            "reject",
            (s.session_id.clone(),)
        )])
        .reject(&s.session_id);
    assert_eq!(
        s.env.auths(),
        std::vec![(
            s.seller.clone(),
            contract_auth_invocation(
                &s.env,
                &s.contract_id,
                "reject",
                (s.session_id.clone(),).into_val(&s.env),
            ),
        )]
    );
    assert_eq!(token.balance(&s.buyer), CAP);
    assert_eq!(token.balance(&s.contract_id), 0);
    assert_eq!(client.get_session(&s.session_id).status, SessionStatus::Rejected);
}
