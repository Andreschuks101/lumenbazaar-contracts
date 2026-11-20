//! Aggregate exposure ceilings (per asset and per buyer-asset pair) and the
//! per-asset solvency snapshot.
//!
//! Global auth mocks below isolate accounting behavior. Tests that assert a
//! check runs before buyer authorization use an environment without mocks.

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke},
    token, vec, Address, BytesN, Env, Event, IntoVal,
};
use test_token::{
    adversarial::{AdversarialBehavior, AdversarialTokenContract, AdversarialTokenContractClient},
    asset_probe::{ProbeBehavior, ProbeTokenContract, ProbeTokenContractClient},
    reentry::{ReentryAction, ReentryTokenContract, ReentryTokenContractClient},
};

struct Fixture {
    env: Env,
    contract_id: Address,
    client: UptoSessionContractClient<'static>,
    admin: Address,
    seller: Address,
}

fn hash(env: &Env, value: u8) -> BytesN<32> {
    BytesN::from_array(env, &[value; 32])
}

fn policy(
    asset: &Address,
    max_session_amount: i128,
    max_asset_exposure: i128,
    max_buyer_exposure: i128,
) -> AssetPolicy {
    AssetPolicy {
        asset: asset.clone(),
        max_session_amount,
        min_session_amount: 1,
        max_asset_exposure,
        max_buyer_exposure,
    }
}

fn mint_asset(env: &Env, holders: &[(&Address, i128)]) -> Address {
    let token_admin = Address::generate(env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin_client = token::StellarAssetClient::new(env, &asset);
    for (holder, amount) in holders {
        admin_client
            .mock_auths(&[MockAuth {
                address: &token_admin,
                invoke: &MockAuthInvoke {
                    contract: &asset,
                    fn_name: "mint",
                    args: ((*holder).clone(), *amount).into_val(env),
                    sub_invokes: &[],
                },
            }])
            .mint(holder, amount);
    }
    asset
}

fn fixture() -> Fixture {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let seller = Address::generate(&env);
    Fixture {
        env,
        contract_id,
        client,
        admin,
        seller,
    }
}

fn open(f: &Fixture, buyer: &Address, asset: &Address, amount: i128, salt: u8) -> BytesN<32> {
    let tag = hash(&f.env, salt);
    f.client
        .create_session(buyer, &f.seller, asset, &amount, &50, &tag, &tag)
}

fn try_open(
    f: &Fixture,
    buyer: &Address,
    asset: &Address,
    amount: i128,
    salt: u8,
) -> Result<BytesN<32>, ContractError> {
    let tag = hash(&f.env, salt);
    match f
        .client
        .try_create_session(buyer, &f.seller, asset, &amount, &50, &tag, &tag)
    {
        Ok(Ok(id)) => Ok(id),
        Err(Ok(error)) => Err(error),
        other => panic!("unexpected create result: {other:?}"),
    }
}

fn last_event(f: &Fixture) -> Option<soroban_sdk::xdr::ContractEvent> {
    f.env
        .events()
        .all()
        .filter_by_contract(&f.contract_id)
        .events()
        .last()
        .cloned()
}

fn sequence(f: &Fixture) -> u64 {
    f.env.as_contract(&f.contract_id, || {
        storage::read_next_session_sequence(&f.env)
    })
}

// ── initialization and configuration ────────────────────────────────────────

#[test]
fn initialize_requires_positive_exposure_ceilings_for_every_asset() {
    let f = fixture();
    let asset = mint_asset(&f.env, &[]);
    for (asset_ceiling, buyer_ceiling) in [(0, 10), (-1, 10), (10, 0), (10, -5)] {
        assert_eq!(
            f.client.try_initialize(
                &f.admin,
                &vec![&f.env, policy(&asset, 10, asset_ceiling, buyer_ceiling)],
            ),
            Err(Ok(ContractError::InvalidExposureCeiling))
        );
    }
    assert_eq!(
        f.client.try_get_admin(),
        Err(Ok(ContractError::NotInitialized))
    );

    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 10, 300, 200)]);
    assert_eq!(f.client.get_asset_exposure_ceiling(&asset), 300);
    assert_eq!(f.client.get_buyer_exposure_ceiling(&asset), 200);
    let unknown = Address::generate(&f.env);
    assert_eq!(
        f.client.try_get_asset_exposure_ceiling(&unknown),
        Err(Ok(ContractError::UnsupportedAsset))
    );
    assert_eq!(
        f.client.try_get_buyer_exposure_ceiling(&unknown),
        Err(Ok(ContractError::UnsupportedAsset))
    );
}

#[test]
fn ceiling_updates_are_admin_only_validated_and_evented() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 500, 500)]);
    open(&f, &buyer, &asset, 400, 1);

    assert_eq!(
        f.client.try_set_asset_exposure_ceiling(&asset, &0),
        Err(Ok(ContractError::InvalidExposureCeiling))
    );
    assert_eq!(
        f.client.try_set_buyer_exposure_ceiling(&asset, &-1),
        Err(Ok(ContractError::InvalidExposureCeiling))
    );
    assert_eq!(
        f.client
            .try_set_asset_exposure_ceiling(&Address::generate(&f.env), &10),
        Err(Ok(ContractError::UnsupportedAsset))
    );

    // Lowering below the current liability is allowed and reported.
    f.client.set_asset_exposure_ceiling(&asset, &300);
    let expected = events::AssetExposureCeilingUpdated {
        asset: asset.clone(),
        admin: f.admin.clone(),
        previous_ceiling: 500,
        new_ceiling: 300,
        current_liability: 400,
        event_version: 3,
    }
    .to_xdr(&f.env, &f.contract_id);
    assert_eq!(last_event(&f), Some(expected));

    f.client.set_buyer_exposure_ceiling(&asset, &450);
    let expected = events::BuyerExposureCeilingUpdated {
        asset: asset.clone(),
        admin: f.admin.clone(),
        previous_ceiling: 500,
        new_ceiling: 450,
        event_version: 3,
    }
    .to_xdr(&f.env, &f.contract_id);
    assert_eq!(last_event(&f), Some(expected));
    assert_eq!(f.client.get_asset_exposure_ceiling(&asset), 300);
    assert_eq!(f.client.get_buyer_exposure_ceiling(&asset), 450);

    // Without the admin's signature both updates fail and change nothing.
    f.env.set_auths(&[]);
    assert!(f
        .client
        .try_set_asset_exposure_ceiling(&asset, &900)
        .is_err());
    assert!(f
        .client
        .try_set_buyer_exposure_ceiling(&asset, &900)
        .is_err());
    assert_eq!(f.client.get_asset_exposure_ceiling(&asset), 300);
    assert_eq!(f.client.get_buyer_exposure_ceiling(&asset), 450);
}

// ── per-asset aggregate ceiling (#64) ───────────────────────────────────────

#[test]
fn asset_ceiling_is_exact_and_checked_before_authorization_or_transfer() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = mint_asset(&env, &[(&buyer, 1_000)]);
    env.mock_all_auths_allowing_non_root_auth();
    client.initialize(&admin, &vec![&env, policy(&asset, 300, 300, 1_000)]);
    client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &hash(&env, 1),
        &hash(&env, 1),
    );
    client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &50,
        &hash(&env, 2),
        &hash(&env, 2),
    );
    assert_eq!(client.get_escrow_liability(&asset), 300);

    // No authorization is supplied: the ceiling must reject first.
    env.set_auths(&[]);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &1,
            &50,
            &hash(&env, 3),
            &hash(&env, 3),
        ),
        Err(Ok(ContractError::AssetExposureCeilingExceeded))
    );
    let token = token::Client::new(&env, &asset);
    assert_eq!(token.balance(&buyer), 700);
    assert_eq!(token.balance(&contract_id), 300);
    assert_eq!(client.get_escrow_liability(&asset), 300);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 2);
    });
}

#[test]
fn liability_overflow_is_rejected_before_funds_move() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 10)]);
    f.client.initialize(
        &f.admin,
        &vec![&f.env, policy(&asset, i128::MAX, i128::MAX, i128::MAX)],
    );
    f.env.as_contract(&f.contract_id, || {
        storage::increase_liability(&f.env, &asset, i128::MAX).unwrap();
    });
    assert_eq!(
        try_open(&f, &buyer, &asset, 1, 1),
        Err(ContractError::LiabilityOverflow)
    );
    assert_eq!(token::Client::new(&f.env, &asset).balance(&buyer), 10);
    assert_eq!(sequence(&f), 0);
}

#[test]
fn every_terminal_path_releases_asset_capacity_exactly_once() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 10_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 100, 300, 10_000)]);

    let settled = open(&f, &buyer, &asset, 100, 1);
    let cancelled = open(&f, &buyer, &asset, 100, 2);
    let expired = open(&f, &buyer, &asset, 100, 3);
    assert_eq!(
        try_open(&f, &buyer, &asset, 1, 4),
        Err(ContractError::AssetExposureCeilingExceeded)
    );

    // Partial settlement still releases the whole escrowed amount.
    f.client.settle(&settled, &40, &hash(&f.env, 90));
    assert_eq!(f.client.get_escrow_liability(&asset), 200);
    assert_eq!(
        f.client.try_settle(&settled, &40, &hash(&f.env, 90)),
        Err(Ok(ContractError::SessionAlreadySettled))
    );
    assert_eq!(f.client.get_escrow_liability(&asset), 200);
    let replacement = open(&f, &buyer, &asset, 100, 5);
    assert_eq!(
        try_open(&f, &buyer, &asset, 1, 6),
        Err(ContractError::AssetExposureCeilingExceeded)
    );

    f.client.cancel(&cancelled);
    assert_eq!(
        f.client.try_cancel(&cancelled),
        Err(Ok(ContractError::SessionCancelled))
    );
    assert_eq!(f.client.get_escrow_liability(&asset), 200);

    f.env.ledger().set_sequence_number(50);
    f.client.recover_expired(&expired);
    assert_eq!(
        f.client.try_recover_expired(&expired),
        Err(Ok(ContractError::SessionExpired))
    );
    let batch = f
        .client
        .recover_expired_batch(&vec![&f.env, expired.clone(), replacement.clone()]);
    assert_eq!(
        batch.get(0).unwrap().status,
        BatchRecoveryStatus::AlreadyRecovered
    );
    assert_eq!(batch.get(1).unwrap().status, BatchRecoveryStatus::Recovered);
    assert_eq!(f.client.get_escrow_liability(&asset), 0);
    assert_eq!(
        token::Client::new(&f.env, &asset).balance(&f.contract_id),
        0
    );
}

#[test]
fn ceiling_updates_gate_only_new_sessions() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 500, 1_000)]);
    let first = open(&f, &buyer, &asset, 300, 1);
    let second = open(&f, &buyer, &asset, 200, 2);

    f.client.set_asset_exposure_ceiling(&asset, &250);
    assert_eq!(
        try_open(&f, &buyer, &asset, 1, 3),
        Err(ContractError::AssetExposureCeilingExceeded)
    );
    // Existing sessions above the new ceiling still close normally.
    f.client.settle(&first, &300, &hash(&f.env, 91));
    assert_eq!(f.client.get_escrow_liability(&asset), 200);
    assert_eq!(
        try_open(&f, &buyer, &asset, 51, 4),
        Err(ContractError::AssetExposureCeilingExceeded)
    );
    open(&f, &buyer, &asset, 50, 5);
    assert_eq!(f.client.get_escrow_liability(&asset), 250);

    f.client.set_asset_exposure_ceiling(&asset, &1_000);
    open(&f, &buyer, &asset, 400, 6);
    f.client.cancel(&second);
    assert_eq!(f.client.get_escrow_liability(&asset), 450);
}

#[test]
fn asset_ceilings_are_independent_per_asset() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let first = mint_asset(&f.env, &[(&buyer, 1_000)]);
    let second = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client.initialize(
        &f.admin,
        &vec![
            &f.env,
            policy(&first, 100, 100, 1_000),
            policy(&second, 400, 400, 1_000),
        ],
    );
    open(&f, &buyer, &first, 100, 1);
    assert_eq!(
        try_open(&f, &buyer, &first, 1, 2),
        Err(ContractError::AssetExposureCeilingExceeded)
    );
    open(&f, &buyer, &second, 400, 3);
    assert_eq!(f.client.get_escrow_liability(&first), 100);
    assert_eq!(f.client.get_escrow_liability(&second), 400);
}

#[test]
fn deprecated_assets_reject_creation_but_still_release_capacity() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 500)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 500, 500)]);
    let session = open(&f, &buyer, &asset, 200, 1);
    f.client.deprecate_asset(&asset, &hash(&f.env, 9));
    assert_eq!(
        try_open(&f, &buyer, &asset, 1, 2),
        Err(ContractError::UnsupportedAsset)
    );
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 200);
    f.client.cancel(&session);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 0);
    f.env.as_contract(&f.contract_id, || {
        assert_eq!(storage::read_liability(&f.env, &asset), 0);
    });
    f.client.remove_asset(&asset, &hash(&f.env, 9));
    assert_eq!(
        f.client.try_get_buyer_exposure(&buyer, &asset),
        Err(Ok(ContractError::UnsupportedAsset))
    );
}

// ── per-buyer ceiling (#65) ─────────────────────────────────────────────────

#[test]
fn buyer_ceiling_is_exact_and_isolated_across_buyers_and_assets() {
    let f = fixture();
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);
    let usd = mint_asset(&f.env, &[(&alice, 1_000), (&bob, 1_000)]);
    let eur = mint_asset(&f.env, &[(&alice, 1_000)]);
    f.client.initialize(
        &f.admin,
        &vec![
            &f.env,
            policy(&usd, 250, 10_000, 250),
            policy(&eur, 100, 10_000, 100),
        ],
    );

    open(&f, &alice, &usd, 100, 1);
    open(&f, &alice, &usd, 150, 2);
    assert_eq!(f.client.get_buyer_exposure(&alice, &usd), 250);
    let before = sequence(&f);
    assert_eq!(
        try_open(&f, &alice, &usd, 1, 3),
        Err(ContractError::BuyerExposureCeilingExceeded)
    );
    assert_eq!(sequence(&f), before);
    assert_eq!(token::Client::new(&f.env, &usd).balance(&alice), 750);

    // Another buyer and another asset have their own counters.
    open(&f, &bob, &usd, 250, 4);
    open(&f, &alice, &eur, 100, 5);
    assert_eq!(f.client.get_buyer_exposure(&bob, &usd), 250);
    assert_eq!(f.client.get_buyer_exposure(&alice, &eur), 100);
    assert_eq!(f.client.get_buyer_exposure(&bob, &eur), 0);
    assert_eq!(f.client.get_escrow_liability(&usd), 500);
}

#[test]
fn buyer_ceiling_is_checked_before_buyer_authorization() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = mint_asset(&env, &[(&buyer, 1_000)]);
    env.mock_all_auths_allowing_non_root_auth();
    client.initialize(&admin, &vec![&env, policy(&asset, 500, 1_000, 200)]);
    client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &50,
        &hash(&env, 1),
        &hash(&env, 1),
    );

    env.set_auths(&[]);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &1,
            &50,
            &hash(&env, 2),
            &hash(&env, 2),
        ),
        Err(Ok(ContractError::BuyerExposureCeilingExceeded))
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 800);
}

#[test]
fn every_terminal_path_releases_buyer_exposure_exactly_once() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 100, 10_000, 300)]);
    let settled = open(&f, &buyer, &asset, 100, 1);
    let cancelled = open(&f, &buyer, &asset, 100, 2);
    let expired = open(&f, &buyer, &asset, 100, 3);
    let batched = {
        f.client.settle(&settled, &1, &hash(&f.env, 92));
        open(&f, &buyer, &asset, 100, 4)
    };
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 300);

    f.client.cancel(&cancelled);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 200);
    assert!(f.client.try_cancel(&cancelled).is_err());
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 200);

    f.env.ledger().set_sequence_number(50);
    f.client.recover_expired(&expired);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 100);
    let results =
        f.client
            .recover_expired_batch(&vec![&f.env, batched.clone(), batched.clone(), expired]);
    assert_eq!(
        results.get(0).unwrap().status,
        BatchRecoveryStatus::Recovered
    );
    assert_eq!(
        results.get(1).unwrap().status,
        BatchRecoveryStatus::Duplicate
    );
    assert_eq!(
        results.get(2).unwrap().status,
        BatchRecoveryStatus::AlreadyRecovered
    );
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 0);
    // The zero counter is removed rather than stored.
    f.env.as_contract(&f.contract_id, || {
        assert!(!f
            .env
            .storage()
            .persistent()
            .has(&storage::DataKey::BuyerExposure(
                buyer.clone(),
                asset.clone()
            )));
    });
}

#[test]
fn corrupted_buyer_exposure_fails_atomically_and_cannot_underflow() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 1_000, 1_000)]);
    let cancelled = open(&f, &buyer, &asset, 200, 1);
    let recovered = open(&f, &buyer, &asset, 300, 2);
    let healthy_buyer = Address::generate(&f.env);
    token::StellarAssetClient::new(&f.env, &asset).mint(&healthy_buyer, &100);
    let healthy = open(&f, &healthy_buyer, &asset, 100, 3);

    // Simulate a counter that no longer covers the open sessions.
    f.env.as_contract(&f.contract_id, || {
        storage::write_buyer_exposure(&f.env, &buyer, &asset, 50);
    });
    assert_eq!(
        f.client.try_cancel(&cancelled),
        Err(Ok(ContractError::BuyerExposureUnderflow))
    );
    assert_eq!(
        f.client.try_settle(&cancelled, &10, &hash(&f.env, 93)),
        Err(Ok(ContractError::BuyerExposureUnderflow))
    );
    assert_eq!(f.client.get_session(&cancelled).status, SessionStatus::Open);
    assert_eq!(f.client.get_escrow_liability(&asset), 600);
    assert_eq!(token::Client::new(&f.env, &asset).balance(&buyer), 500);

    f.env.ledger().set_sequence_number(50);
    let results = f
        .client
        .recover_expired_batch(&vec![&f.env, recovered.clone(), healthy.clone()]);
    assert_eq!(
        results.get(0).unwrap().status,
        BatchRecoveryStatus::EscrowUnavailable
    );
    assert_eq!(
        results.get(1).unwrap().status,
        BatchRecoveryStatus::Recovered
    );
    assert_eq!(f.client.get_session(&recovered).status, SessionStatus::Open);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 50);
    assert_eq!(f.client.get_buyer_exposure(&healthy_buyer, &asset), 0);
    assert_eq!(f.client.get_escrow_liability(&asset), 500);
}

#[test]
fn failed_funding_leaves_buyer_exposure_untouched() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let token_admin = Address::generate(&f.env);
    let asset = f.env.register(AdversarialTokenContract, ());
    let token = AdversarialTokenContractClient::new(&f.env, &asset);
    token.initialize(&token_admin);
    token.mint(&buyer, &500);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 500, 500)]);

    for behavior in [
        AdversarialBehavior::FailTransfer,
        AdversarialBehavior::PartialTransfer,
        AdversarialBehavior::FalseBalance,
    ] {
        token.set_behavior(&behavior);
        assert!(try_open(&f, &buyer, &asset, 200, 1).is_err());
        assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 0);
        assert_eq!(f.client.get_escrow_liability(&asset), 0);
    }
    token.set_behavior(&AdversarialBehavior::Honest);
    open(&f, &buyer, &asset, 500, 2);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 500);
}

#[test]
fn reentrant_tokens_cannot_double_count_or_double_release_exposure() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let token_admin = Address::generate(&f.env);
    let asset = f.env.register(ReentryTokenContract, ());
    let token = ReentryTokenContractClient::new(&f.env, &asset);
    token.initialize(&token_admin);
    token.mint(&buyer, &1_000);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 300, 300, 300)]);

    // A nested create during funding is rejected; only one session counts.
    token.configure_create(
        &f.contract_id,
        &buyer,
        &f.seller,
        &300,
        &50,
        &hash(&f.env, 20),
    );
    let session = open(&f, &buyer, &asset, 300, 21);
    assert!(token.callback_rejected());
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 300);
    assert_eq!(f.client.get_escrow_liability(&asset), 300);

    // A nested cancel during the refund cannot release a second time.
    token.configure_terminal(
        &f.contract_id,
        &ReentryAction::Cancel,
        &session,
        &1,
        &hash(&f.env, 22),
    );
    f.client.cancel(&session);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 0);
    assert_eq!(f.client.get_escrow_liability(&asset), 0);
    assert_eq!(token.balance(&buyer), 1_000);
}

#[test]
fn many_sessions_from_one_buyer_track_the_running_total() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 10, 1_000, 95)]);
    let mut sessions = std::vec::Vec::new();
    for salt in 1..10u8 {
        sessions.push(open(&f, &buyer, &asset, 10, salt));
    }
    assert_eq!(
        try_open(&f, &buyer, &asset, 10, 50),
        Err(ContractError::BuyerExposureCeilingExceeded)
    );
    open(&f, &buyer, &asset, 5, 51);
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 95);
    for session in sessions.iter() {
        f.client.cancel(session);
    }
    assert_eq!(f.client.get_buyer_exposure(&buyer, &asset), 5);
    assert_eq!(f.client.get_escrow_liability(&asset), 5);
}

#[test]
fn buyer_exposure_query_requires_initialization() {
    let f = fixture();
    let asset = Address::generate(&f.env);
    assert_eq!(
        f.client
            .try_get_buyer_exposure(&Address::generate(&f.env), &asset),
        Err(Ok(ContractError::NotInitialized))
    );
}

// ── solvency snapshot (#69) ─────────────────────────────────────────────────

fn snapshot(f: &Fixture, asset: &Address) -> AssetSolvencySnapshot {
    f.client.get_asset_solvency(asset)
}

#[test]
fn solvency_reports_exact_collateralization_and_surplus() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 1_000)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 1_000, 1_000)]);

    let empty = snapshot(&f, &asset);
    assert_eq!(empty.response_version, SOLVENCY_RESPONSE_VERSION);
    assert_eq!(empty.asset_status, AssetStatus::Active);
    assert_eq!(empty.token_balance, Some(0));
    assert_eq!(empty.status, SolvencyStatus::Collateralized);

    open(&f, &buyer, &asset, 300, 1);
    let exact = snapshot(&f, &asset);
    assert_eq!(exact.token_balance, Some(300));
    assert_eq!(exact.liability, 300);
    assert_eq!((exact.surplus, exact.deficit), (0, 0));
    assert_eq!(exact.status, SolvencyStatus::Collateralized);

    // An unsolicited deposit is surplus, not liability.
    token::StellarAssetClient::new(&f.env, &asset).mint(&f.contract_id, &25);
    let surplus = snapshot(&f, &asset);
    assert_eq!(surplus.token_balance, Some(325));
    assert_eq!(surplus.liability, 300);
    assert_eq!((surplus.surplus, surplus.deficit), (25, 0));
    assert_eq!(surplus.status, SolvencyStatus::Surplus);
}

#[test]
fn solvency_reports_underfunding_without_negative_surplus() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let token_admin = Address::generate(&f.env);
    let asset = f.env.register(AdversarialTokenContract, ());
    let token = AdversarialTokenContractClient::new(&f.env, &asset);
    token.initialize(&token_admin);
    token.mint(&buyer, &500);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 500, 500, 500)]);
    open(&f, &buyer, &asset, 400, 1);

    token.set_behavior(&AdversarialBehavior::FalseBalance);
    let underfunded = snapshot(&f, &asset);
    assert_eq!(underfunded.token_balance, Some(0));
    assert_eq!(underfunded.liability, 400);
    assert_eq!(underfunded.surplus, 0);
    assert_eq!(underfunded.deficit, 400);
    assert_eq!(underfunded.status, SolvencyStatus::Underfunded);
}

#[test]
fn solvency_marks_reverting_and_malformed_balances_unavailable() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let honest = mint_asset(&f.env, &[(&buyer, 10)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&honest, 10, 10, 10)]);

    for (behavior, expected_balance) in [
        (ProbeBehavior::Reverting, None),
        (ProbeBehavior::Malformed, Some(-1)),
    ] {
        let probe = f.env.register(ProbeTokenContract, ());
        ProbeTokenContractClient::new(&f.env, &probe).initialize(&behavior);
        // Activation would reject these tokens; mark one supported directly
        // to model a token whose behavior changed after activation.
        f.env.as_contract(&f.contract_id, || {
            storage::write_supported_asset(&f.env, &probe);
            storage::increase_liability(&f.env, &probe, 5).unwrap();
        });
        let view = snapshot(&f, &probe);
        assert_eq!(view.token_balance, expected_balance);
        assert_eq!(view.liability, 5);
        assert_eq!((view.surplus, view.deficit), (0, 0));
        assert_eq!(view.status, SolvencyStatus::BalanceUnavailable);
    }
}

#[test]
fn solvency_is_read_only_authorization_free_and_covers_deprecated_assets() {
    let f = fixture();
    let buyer = Address::generate(&f.env);
    let asset = mint_asset(&f.env, &[(&buyer, 100)]);
    f.client
        .initialize(&f.admin, &vec![&f.env, policy(&asset, 100, 100, 100)]);
    let session = open(&f, &buyer, &asset, 100, 1);
    f.client.deprecate_asset(&asset, &hash(&f.env, 9));

    f.env.set_auths(&[]);
    let first = snapshot(&f, &asset);
    assert_eq!(first.asset_status, AssetStatus::Deprecated);
    assert_eq!(first.status, SolvencyStatus::Collateralized);
    assert!(f.env.auths().is_empty());
    assert!(f
        .env
        .events()
        .all()
        .filter_by_contract(&f.contract_id)
        .events()
        .is_empty());
    assert_eq!(snapshot(&f, &asset), first);

    f.env.mock_all_auths_allowing_non_root_auth();
    f.client.cancel(&session);
    f.client.remove_asset(&asset, &hash(&f.env, 9));
    assert_eq!(
        f.client.try_get_asset_solvency(&asset),
        Err(Ok(ContractError::UnsupportedAsset))
    );
}

#[test]
fn solvency_requires_initialization() {
    let f = fixture();
    assert_eq!(
        f.client.try_get_asset_solvency(&Address::generate(&f.env)),
        Err(Ok(ContractError::NotInitialized))
    );
}

#[test]
fn solvency_classification_uses_checked_arithmetic() {
    assert_eq!(
        solvency_of(Some(i128::MAX), 0),
        (i128::MAX, 0, SolvencyStatus::Surplus)
    );
    assert_eq!(
        solvency_of(Some(0), i128::MAX),
        (0, i128::MAX, SolvencyStatus::Underfunded)
    );
    assert_eq!(
        solvency_of(Some(i128::MAX), i128::MAX),
        (0, 0, SolvencyStatus::Collateralized)
    );
    assert_eq!(
        solvency_of(Some(-1), 0),
        (0, 0, SolvencyStatus::BalanceUnavailable)
    );
    assert_eq!(
        solvency_of(Some(i128::MIN), 1),
        (0, 0, SolvencyStatus::BalanceUnavailable)
    );
    assert_eq!(
        solvency_of(Some(10), -1),
        (0, 0, SolvencyStatus::BalanceUnavailable)
    );
    assert_eq!(
        solvency_of(None, 0),
        (0, 0, SolvencyStatus::BalanceUnavailable)
    );
}
