extern crate std;

mod authorization_harness {
    include!("auth_test.rs");
}

mod property_harness {
    include!("property_test.rs");
}

mod mutation_harness {
    include!("mutation_test.rs");
}

mod cap_harness {
    include!("cap_property_test.rs");
}

mod transition_harness {
    include!("transition_test.rs");
}

mod ttl_harness {
    include!("ttl_test.rs");
}

mod storage_lifecycle_harness {
    include!("storage_lifecycle_test.rs");
}

// Global auth mocks in the legacy tests below isolate validation, state,
// event, and resource-accounting behavior. They are not authorization
// evidence. Release-gating authorization paths belong in
// `authorization_harness` and must use explicit trees.

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke},
    token, Address, BytesN, Env, Event, IntoVal,
};
use test_token::{
    adversarial::{AdversarialBehavior, AdversarialTokenContract, AdversarialTokenContractClient},
    asset_probe::{
        IncompatibleContract, ProbeBehavior, ProbeTokenContract, ProbeTokenContractClient,
    },
    reentry::{ReentryAction, ReentryTokenContract, ReentryTokenContractClient},
    TestTokenContract, TestTokenContractClient,
};

pub(super) fn unique_idempotency_key(env: &Env) -> BytesN<32> {
    static NEXT_KEY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
    let seq = NEXT_KEY.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x42u8; 32];
    bytes[24..32].copy_from_slice(&seq.to_be_bytes());
    BytesN::from_array(env, &bytes)
}

fn create_test_asset(env: &Env, buyer: &Address, amount: i128) -> Address {
    let token_admin = Address::generate(env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    token::StellarAssetClient::new(env, &asset)
        .mock_auths(&[MockAuth {
            address: &token_admin,
            invoke: &MockAuthInvoke {
                contract: &asset,
                fn_name: "mint",
                args: (buyer.clone(), amount).into_val(env),
                sub_invokes: &[],
            },
        }])
        .mint(buyer, &amount);
    asset
}

fn create_local_test_token(env: &Env, buyer: &Address, amount: i128) -> Address {
    let token_admin = Address::generate(env);
    let token_id = env.register(TestTokenContract, ());
    let token_client = TestTokenContractClient::new(env, &token_id);

    token_client.initialize(&token_admin);
    token_client.mint(buyer, &amount);

    token_id
}

fn asset_policy(asset: &Address, max_session_amount: i128) -> AssetPolicy {
    AssetPolicy {
        asset: asset.clone(),
        max_session_amount,
        min_session_amount: 1,
        max_asset_exposure: i128::MAX,
        max_buyer_exposure: i128::MAX,
    }
}

fn create_adversarial_token(env: &Env, buyer: &Address, amount: i128) -> Address {
    let admin = Address::generate(env);
    let token_id = env.register(AdversarialTokenContract, ());
    let token = AdversarialTokenContractClient::new(env, &token_id);
    token.initialize(&admin);
    token.mint(buyer, &amount);
    token_id
}

fn assert_adversarial_creation_rolls_back(behavior: AdversarialBehavior) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_adversarial_token(&env, &buyer, 500);
    let token = AdversarialTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    token.set_behavior(&behavior);

    assert!(client
        .try_create_session(
            &buyer,
            &seller,
            &asset,
            &500,
            &50,
            &BytesN::from_array(&env, &[71; 32]),
            &unique_idempotency_key(&env),
        )
        .is_err());
    assert_eq!(token.raw_balance(&buyer), 500);
    assert_eq!(token.raw_balance(&contract_id), 0);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
        assert_eq!(storage::read_next_session_sequence(&env), 0);
    });
    assert!(env
        .events()
        .all()
        .filter_by_contract(&contract_id)
        .events()
        .is_empty());
}

fn create_reentry_token(env: &Env, buyer: &Address, amount: i128) -> (Address, Address) {
    let admin = Address::generate(env);
    let token_id = env.register(ReentryTokenContract, ());
    let token = ReentryTokenContractClient::new(env, &token_id);
    token.initialize(&admin);
    token.mint(buyer, &amount);
    (admin, token_id)
}

fn escrow_event_count(env: &Env, contract_id: &Address) -> usize {
    env.events()
        .all()
        .filter_by_contract(contract_id)
        .events()
        .len()
}

fn initialize_for_asset(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    admin: &Address,
    asset: &Address,
) {
    client.initialize(
        admin,
        &soroban_sdk::vec![env, asset_policy(asset, i128::MAX)],
    );
}

fn print_resource_usage(operation: &str, env: &Env) {
    let resources = env.cost_estimate().resources();
    let fee = env.cost_estimate().fee();

    std::println!(
        concat!(
            "RESOURCE_USAGE_JSON ",
            "{{",
            "\"operation\":\"{}\",",
            "\"instructions\":{},",
            "\"mem_bytes\":{},",
            "\"disk_read_entries\":{},",
            "\"memory_read_entries\":{},",
            "\"write_entries\":{},",
            "\"disk_read_bytes\":{},",
            "\"write_bytes\":{},",
            "\"contract_events_size_bytes\":{},",
            "\"persistent_rent_ledger_bytes\":{},",
            "\"persistent_entry_rent_bumps\":{},",
            "\"temporary_rent_ledger_bytes\":{},",
            "\"temporary_entry_rent_bumps\":{},",
            "\"estimated_fee_stroops\":{}",
            "}}"
        ),
        operation,
        resources.instructions,
        resources.mem_bytes,
        resources.disk_read_entries,
        resources.memory_read_entries,
        resources.write_entries,
        resources.disk_read_bytes,
        resources.write_bytes,
        resources.contract_events_size_bytes,
        resources.persistent_rent_ledger_bytes,
        resources.persistent_entry_rent_bumps,
        resources.temporary_rent_ledger_bytes,
        resources.temporary_entry_rent_bumps,
        fee.total,
    );
}

fn contract_event_count(env: &Env, contract_id: &Address) -> usize {
    env.events()
        .all()
        .filter_by_contract(contract_id)
        .events()
        .len()
}

fn print_native_rpc_fixture(
    name: &str,
    outcome: &str,
    result: &str,
    authorization_entries: usize,
    contract_events: usize,
    env: &Env,
) {
    let resources = env.cost_estimate().resources();
    let fee = env.cost_estimate().fee();
    std::println!(
        concat!(
            "NATIVE_RPC_DIFFERENTIAL_JSON ",
            "{{",
            "\"name\":\"{}\",",
            "\"outcome\":\"{}\",",
            "\"result\":\"{}\",",
            "\"authorizationEntries\":{},",
            "\"contractEventCount\":{},",
            "\"resources\":{{",
            "\"instructions\":{},",
            "\"memoryBytes\":{},",
            "\"diskReadBytes\":{},",
            "\"writeBytes\":{},",
            "\"estimatedFeeStroops\":{}",
            "}}",
            "}}"
        ),
        name,
        outcome,
        result,
        authorization_entries,
        contract_events,
        resources.instructions,
        resources.mem_bytes,
        resources.disk_read_bytes,
        resources.write_bytes,
        fee.total,
    );
}

#[test]
fn native_rpc_differential_create_session() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[76; 32]),
        &unique_idempotency_key(&env),
    );
    print_native_rpc_fixture(
        "create_session",
        "success",
        "Created",
        env.auths().len(),
        contract_event_count(&env, &contract_id),
        &env,
    );
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);
}

#[test]
fn native_rpc_differential_read_settled() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[77; 32]),
        &unique_idempotency_key(&env),
    );
    client.settle(&session_id, &60, &BytesN::from_array(&env, &[78; 32]));
    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Settled);
    print_native_rpc_fixture(
        "read_settled",
        "success",
        "Settled",
        env.auths().len(),
        contract_event_count(&env, &contract_id),
        &env,
    );
}

#[test]
fn native_rpc_differential_repeated_settle() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[79; 32]),
        &unique_idempotency_key(&env),
    );
    let usage_hash = BytesN::from_array(&env, &[80; 32]);
    client.settle(&session_id, &60, &usage_hash);
    assert_eq!(
        client.try_settle(&session_id, &60, &usage_hash),
        Err(Ok(ContractError::SessionAlreadySettled))
    );
    print_native_rpc_fixture(
        "repeated_settle",
        "error",
        "ContractError#6",
        env.auths().len(),
        contract_event_count(&env, &contract_id),
        &env,
    );
}

#[test]
fn native_rpc_differential_repeated_cancel() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[81; 32]),
        &unique_idempotency_key(&env),
    );
    client.cancel(&session_id);
    assert_eq!(
        client.try_cancel(&session_id),
        Err(Ok(ContractError::SessionCancelled))
    );
    print_native_rpc_fixture(
        "repeated_cancel",
        "error",
        "ContractError#7",
        env.auths().len(),
        contract_event_count(&env, &contract_id),
        &env,
    );
}

#[test]
fn native_rpc_differential_repeated_recovery() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[82; 32]),
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(50);
    client.recover_expired(&session_id);
    assert_eq!(
        client.try_recover_expired(&session_id),
        Err(Ok(ContractError::SessionExpired))
    );
    print_native_rpc_fixture(
        "repeated_recovery",
        "error",
        "ContractError#21",
        env.auths().len(),
        contract_event_count(&env, &contract_id),
        &env,
    );
}

#[test]
fn nested_create_during_funding_is_rejected_without_duplicate_state() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (token_admin, asset) = create_reentry_token(&env, &buyer, 500);
    let token = ReentryTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let resource_hash = BytesN::from_array(&env, &[70; 32]);
    token.configure_create(&contract_id, &buyer, &seller, &100, &50, &resource_hash);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let event_count = escrow_event_count(&env, &contract_id);
    let auth_count = env.auths().len();

    assert!(token.callback_rejected());
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);
    assert_eq!(token.balance(&buyer), 400);
    assert_eq!(token.balance(&contract_id), 100);
    assert_eq!(token.balance(&token_admin), 0);
    assert_eq!(event_count, 1);
    assert_eq!(auth_count, 1);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 1);
        assert_eq!(storage::read_liability(&env, &asset), 100);
    });
}

#[test]
fn nested_settle_during_payout_cannot_finalize_twice() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (_, asset) = create_reentry_token(&env, &buyer, 500);
    let token = ReentryTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[71; 32]),
        &unique_idempotency_key(&env),
    );
    let usage_hash = BytesN::from_array(&env, &[72; 32]);
    token.configure_terminal(
        &contract_id,
        &ReentryAction::Settle,
        &session_id,
        &60,
        &usage_hash,
    );

    client.settle(&session_id, &60, &usage_hash);
    let event_count = escrow_event_count(&env, &contract_id);
    let auth_count = env.auths().len();

    let session = client.get_session(&session_id);
    assert!(token.callback_rejected());
    assert_eq!(session.status, SessionStatus::Settled);
    assert_eq!(session.settled_amount, 60);
    assert_eq!(session.escrowed_amount, 0);
    assert_eq!(token.balance(&buyer), 440);
    assert_eq!(token.balance(&seller), 60);
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(event_count, 1);
    assert_eq!(auth_count, 1);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
}

#[test]
fn nested_cancel_during_refund_cannot_finalize_twice() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (_, asset) = create_reentry_token(&env, &buyer, 500);
    let token = ReentryTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[73; 32]),
        &unique_idempotency_key(&env),
    );
    token.configure_terminal(
        &contract_id,
        &ReentryAction::Cancel,
        &session_id,
        &1,
        &BytesN::from_array(&env, &[74; 32]),
    );

    client.cancel(&session_id);
    let event_count = escrow_event_count(&env, &contract_id);
    let auth_count = env.auths().len();

    assert!(token.callback_rejected());
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Cancelled
    );
    assert_eq!(token.balance(&buyer), 500);
    assert_eq!(token.balance(&seller), 0);
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(event_count, 1);
    assert_eq!(auth_count, 1);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
}

#[test]
fn nested_recovery_during_refund_cannot_finalize_twice() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let caller = Address::generate(&env);
    let (_, asset) = create_reentry_token(&env, &buyer, 500);
    let token = ReentryTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &10,
        &BytesN::from_array(&env, &[75; 32]),
        &unique_idempotency_key(&env),
    );
    token.configure_terminal(
        &contract_id,
        &ReentryAction::Recover,
        &session_id,
        &1,
        &BytesN::from_array(&env, &[76; 32]),
    );
    env.ledger().set_sequence_number(10);

    client.recover_expired(&session_id);
    let event_count = escrow_event_count(&env, &contract_id);
    let auth_count = env.auths().len();

    assert!(token.callback_rejected());
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Expired
    );
    assert_eq!(token.balance(&buyer), 500);
    assert_eq!(token.balance(&seller), 0);
    assert_eq!(token.balance(&caller), 0);
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(event_count, 1);
    assert_eq!(auth_count, 0);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
}

#[test]
fn public_interface_is_callable() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 300);
    let resource_hash = BytesN::from_array(&env, &[7; 32]);

    assert_eq!(client.interface_version(), 3);

    initialize_for_asset(&env, &client, &buyer, &asset);

    let idempotency_key = unique_idempotency_key(&env);
    let expected_id = env.as_contract(&contract_id, || {
        ids::derive_session_id(
            &env,
            &ids::SessionIdInput {
                buyer: &buyer,
                seller: &seller,
                asset: &asset,
                max_amount: 100,
                expires_at_ledger: 10,
                resource_hash: &resource_hash,
                sequence: 0,
                idempotency_key: &idempotency_key,
            },
        )
    });
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &10,
        &resource_hash,
        &idempotency_key,
    );

    assert_eq!(session_id, expected_id);
    let open_session = client.get_session(&session_id);
    assert_eq!(open_session.id, session_id);
    assert_eq!(open_session.creation_ledger, 0);
    assert_eq!(open_session.finalized_ledger, None);
    assert_eq!(open_session.resource_scheme_version, 1);
    assert_eq!(open_session.usage_scheme_version, None);

    // Test settle
    let usage_hash = BytesN::from_array(&env, &[25; 32]);
    client.settle(&session_id, &10, &usage_hash);
    let settled_session = client.get_session(&session_id);
    assert_eq!(settled_session.settled_amount, 10);
    assert_eq!(settled_session.status, SessionStatus::Settled);
    assert_eq!(settled_session.finalized_ledger, Some(0));
    assert_eq!(settled_session.usage_scheme_version, Some(1));

    // Test extend_ttl on settled session - should fail
    assert!(client.try_extend_ttl(&session_id).is_err());

    // Test cancel and extend_ttl on separate open sessions
    let session_id_2 = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &10,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.cancel(&session_id_2);
    let cancelled_session = client.get_session(&session_id_2);
    assert_eq!(cancelled_session.status, SessionStatus::Cancelled);
    assert_eq!(cancelled_session.finalized_ledger, Some(0));
    assert_eq!(
        client.try_extend_ttl(&session_id_2),
        Err(Ok(ContractError::TtlExtensionFailed))
    );

    let session_id_3 = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &10,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.extend_ttl(&session_id_3);
}

#[test]
fn resource_usage_create_session() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[31; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    print_resource_usage("create_session", &env);
}

#[test]
fn resource_usage_settle() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[32; 32]);
    let usage_hash = BytesN::from_array(&env, &[33; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    client.settle(&session_id, &125, &usage_hash);

    print_resource_usage("settle", &env);
}

#[test]
fn resource_usage_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[34; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    client.cancel(&session_id);

    print_resource_usage("cancel", &env);
}

#[test]
fn resource_usage_extend_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[35; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    client.extend_ttl(&session_id);

    print_resource_usage("extend_ttl", &env);
}

#[test]
fn resource_usage_recover_expired() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[57; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(50);

    client.recover_expired(&session_id);

    print_resource_usage("recover_expired", &env);
}

#[test]
fn initialize_stores_admin_once() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let asset_policies = soroban_sdk::vec![&env, asset_policy(&asset, 500)];
    client.initialize(&admin, &asset_policies);

    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_admin(&env), Some(admin.clone()));
        assert_eq!(
            storage::read_storage_layout_version(&env),
            Some(storage::STORAGE_LAYOUT_VERSION)
        );
    });

    assert_eq!(
        client.try_initialize(&admin, &asset_policies),
        Err(Ok(ContractError::AlreadyInitialized))
    );
}

#[test]
fn configuration_query_returns_bounded_versioned_public_state() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let first_asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let second_asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let assets = soroban_sdk::vec![&env, first_asset.clone(), second_asset.clone()];
    let policies = soroban_sdk::vec![
        &env,
        asset_policy(&first_asset, i128::MAX),
        asset_policy(&second_asset, i128::MAX)
    ];

    assert_eq!(
        client.try_get_configuration(),
        Err(Ok(ContractError::NotInitialized))
    );
    client.initialize(&admin, &policies);

    assert_eq!(
        client.get_configuration(),
        ContractConfiguration {
            response_version: CONFIGURATION_RESPONSE_VERSION,
            interface_version: INTERFACE_VERSION,
            storage_layout_version: storage::STORAGE_LAYOUT_VERSION,
            admin,
            supported_assets: assets,
            max_supported_assets: MAX_SUPPORTED_ASSETS,
            operational_state: OperationalState::Active,
            instance_ttl_threshold: storage::DEFAULT_TTL_THRESHOLD,
            instance_ttl_extension: storage::TTL_EXTENSION_AMOUNT,
            max_session_duration_ledgers: validation::MAX_SESSION_DURATION_LEDGERS,
        }
    );
}

#[test]
fn configuration_query_decodes_paused_and_deprecated_states() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    client.initialize(
        &admin,
        &soroban_sdk::vec![&env, asset_policy(&asset, i128::MAX)],
    );

    env.as_contract(&contract_id, || {
        storage::write_operational_state(&env, &OperationalState::Paused);
    });
    assert_eq!(
        client.get_configuration().operational_state,
        OperationalState::Paused
    );

    env.as_contract(&contract_id, || {
        storage::write_operational_state(&env, &OperationalState::Deprecated);
    });
    assert_eq!(
        client.get_configuration().operational_state,
        OperationalState::Deprecated
    );
    assert_eq!(client.interface_version(), INTERFACE_VERSION);
}

#[test]
fn commitment_domain_metadata_query_returns_authoritative_constants() {
    let env = Env::default();
    env.ledger().set_network_id([7; 32]);
    let contract_id = Address::generate(&env);
    env.register_at(&contract_id, UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);

    // Call without initialization or authorization
    let metadata = client.get_commitment_domain_metadata();

    assert_eq!(
        metadata.response_version,
        COMMITMENT_DOMAIN_METADATA_RESPONSE_VERSION
    );
    assert_eq!(metadata.interface_version, INTERFACE_VERSION);
    assert_eq!(metadata.receipt_schema_version, RECEIPT_SCHEMA_VERSION);

    let (expected_session_domain, expected_usage_domain) = env.as_contract(&contract_id, || {
        (
            ids::session_id_domain_hash(&env),
            ids::usage_commitment_domain_hash(&env),
        )
    });

    assert_eq!(metadata.session_id_domain, expected_session_domain);
    assert_eq!(metadata.usage_commitment_domain, expected_usage_domain);
    assert_ne!(metadata.session_id_domain, metadata.usage_commitment_domain);
}

#[test]
fn activation_probes_valid_asset_without_moving_funds_and_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let asset = create_local_test_token(&env, &admin, 100);
    let token = TestTokenContractClient::new(&env, &asset);

    client.initialize(
        &admin,
        &soroban_sdk::vec![&env, asset_policy(&asset, i128::MAX)],
    );
    let activation_events = env.events().all().filter_by_contract(&contract_id);

    assert_eq!(token.balance(&admin), 100);
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(
        activation_events,
        std::vec![events::AssetActivated {
            asset: asset.clone(),
            decimals: 7,
            event_version: 3,
        }
        .to_xdr(&env, &contract_id)]
    );
    env.as_contract(&contract_id, || {
        assert!(storage::is_supported_asset(&env, &asset));
    });
}

#[test]
fn activation_rejects_missing_incompatible_reverting_and_malformed_assets() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let missing = Address::generate(&env);
    let incompatible = env.register(IncompatibleContract, ());
    let reverting = env.register(ProbeTokenContract, ());
    ProbeTokenContractClient::new(&env, &reverting).initialize(&ProbeBehavior::Reverting);
    let malformed = env.register(ProbeTokenContract, ());
    ProbeTokenContractClient::new(&env, &malformed).initialize(&ProbeBehavior::Malformed);

    for asset in [missing, incompatible, reverting] {
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        assert!(client
            .try_initialize(
                &admin,
                &soroban_sdk::vec![&env, asset_policy(&asset, i128::MAX)]
            )
            .is_err());
        let event_count = env
            .events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .len();
        assert_eq!(event_count, 0);
        env.as_contract(&contract_id, || {
            assert!(!storage::has_admin(&env));
            assert!(!storage::is_supported_asset(&env, &asset));
        });
    }

    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    assert_eq!(
        client.try_initialize(
            &admin,
            &soroban_sdk::vec![&env, asset_policy(&malformed, i128::MAX)]
        ),
        Err(Ok(ContractError::InvalidAssetInterface))
    );
    env.as_contract(&contract_id, || {
        assert!(!storage::has_admin(&env));
        assert!(!storage::is_supported_asset(&env, &malformed));
    });
}

#[test]
fn initialize_rejects_empty_duplicate_or_invalid_asset_policies() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let empty_id = env.register(UptoSessionContract, ());
    let empty_client = UptoSessionContractClient::new(&env, &empty_id);
    assert_eq!(
        empty_client.try_initialize(&admin, &soroban_sdk::vec![&env]),
        Err(Ok(ContractError::InvalidSupportedAssets))
    );

    let duplicate_id = env.register(UptoSessionContract, ());
    let duplicate_client = UptoSessionContractClient::new(&env, &duplicate_id);
    let asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    assert_eq!(
        duplicate_client.try_initialize(
            &admin,
            &soroban_sdk::vec![&env, asset_policy(&asset, 100), asset_policy(&asset, 200)]
        ),
        Err(Ok(ContractError::InvalidSupportedAssets))
    );

    let invalid_id = env.register(UptoSessionContract, ());
    let invalid_client = UptoSessionContractClient::new(&env, &invalid_id);
    assert_eq!(
        invalid_client.try_initialize(&admin, &soroban_sdk::vec![&env, asset_policy(&asset, 0)]),
        Err(Ok(ContractError::InvalidAssetCap))
    );

    let oversized_id = env.register(UptoSessionContract, ());
    let oversized_client = UptoSessionContractClient::new(&env, &oversized_id);
    let mut oversized = soroban_sdk::Vec::new(&env);
    for _ in 0..=MAX_SUPPORTED_ASSETS {
        oversized.push_back(asset_policy(&Address::generate(&env), i128::MAX));
    }
    assert_eq!(
        oversized_client.try_initialize(&admin, &oversized),
        Err(Ok(ContractError::InvalidSupportedAssets))
    );
}

#[test]
fn per_asset_caps_enforce_boundaries_before_buyer_authorization() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let first_asset = create_test_asset(&env, &buyer, 500);
    let second_asset = create_test_asset(&env, &buyer, 500);
    let policies = soroban_sdk::vec![
        &env,
        asset_policy(&first_asset, 100),
        asset_policy(&second_asset, 250)
    ];
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (admin.clone(), policies.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin, &policies);

    assert_eq!(client.get_asset_max_session_amount(&first_asset), 100);
    assert_eq!(client.get_asset_max_session_amount(&second_asset), 250);
    assert_eq!(
        client.try_get_asset_max_session_amount(&Address::generate(&env)),
        Err(Ok(ContractError::UnsupportedAsset))
    );

    let resource_hash = BytesN::from_array(&env, &[63; 32]);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &first_asset,
            &101,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::AmountExceedsCap))
    );
    assert_eq!(token::Client::new(&env, &first_asset).balance(&buyer), 500);

    let idempotency_key = unique_idempotency_key(&env);
    client
        .mock_auths(&[MockAuth {
            address: &buyer,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "create_session",
                args: (
                    buyer.clone(),
                    seller.clone(),
                    first_asset.clone(),
                    100_i128,
                    50_u32,
                    resource_hash.clone(),
                    idempotency_key.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[MockAuthInvoke {
                    contract: &first_asset,
                    fn_name: "transfer",
                    args: (buyer.clone(), contract_id.clone(), 100_i128).into_val(&env),
                    sub_invokes: &[],
                }],
            },
        }])
        .create_session(
            &buyer,
            &seller,
            &first_asset,
            &100,
            &50,
            &resource_hash,
            &idempotency_key,
        );
}

#[test]
fn admin_cap_updates_emit_events_and_do_not_rewrite_existing_sessions() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 250);
    client.initialize(&admin, &soroban_sdk::vec![&env, asset_policy(&asset, 100)]);
    let resource_hash = BytesN::from_array(&env, &[64; 32]);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    client.set_asset_max_session_amount(&asset, &50);

    let expected_event = events::AssetSessionCapUpdated {
        asset: asset.clone(),
        admin: admin.clone(),
        previous_max_session_amount: 100,
        new_max_session_amount: 50,
        event_version: 3,
    }
    .to_xdr(&env, &contract_id);
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .last(),
        Some(&expected_event)
    );
    assert_eq!(client.get_asset_max_session_amount(&asset), 50);
    assert_eq!(client.get_session(&session_id).max_amount, 100);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &51,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::AmountExceedsCap))
    );

    // Timelocked cap increase flow:
    // Immediate increase is rejected
    assert_eq!(
        client.try_set_asset_max_session_amount(&asset, &150),
        Err(Ok(ContractError::CapIncreasePending))
    );

    // Propose cap increase from 50 to 150
    // Bounded delay: active at sequence 100 ledgers from now
    let current_seq = env.ledger().sequence();
    let active_at = current_seq + 100;
    let expires_at = active_at + 1000;

    // Proposal before activation ledger cannot be accepted
    client.propose_asset_cap_increase(&asset, &150, &active_at, &expires_at);

    // Verify status is Waiting
    let status_waiting = client.get_asset_cap_increase_status(&asset);
    assert_eq!(status_waiting.status, CapIncreaseStatus::Waiting);
    assert_eq!(status_waiting.is_ready, false);
    assert_eq!(status_waiting.proposed_max_amount, Some(150));
    assert_eq!(status_waiting.active_at_ledger, Some(active_at));
    assert_eq!(status_waiting.expires_at_ledger, Some(expires_at));

    assert_eq!(
        client.try_accept_asset_cap_increase(&asset),
        Err(Ok(ContractError::CapIncreaseNotReady))
    );

    // Cannot propose another while pending
    assert_eq!(
        client.try_propose_asset_cap_increase(&asset, &200, &active_at, &expires_at),
        Err(Ok(ContractError::CapIncreasePending))
    );

    // Cancel proposal
    client.cancel_asset_cap_increase(&asset);
    let status_missing = client.get_asset_cap_increase_status(&asset);
    assert_eq!(status_missing.status, CapIncreaseStatus::Missing);
    assert_eq!(status_missing.is_ready, false);
    assert_eq!(status_missing.proposed_max_amount, None);

    assert_eq!(
        client.try_accept_asset_cap_increase(&asset),
        Err(Ok(ContractError::CapIncreaseNotFound))
    );

    // Re-propose and advance sequence past active_at
    client.propose_asset_cap_increase(&asset, &150, &active_at, &expires_at);
    env.ledger().set_sequence_number(active_at);

    // Verify status is Ready
    let status_ready = client.get_asset_cap_increase_status(&asset);
    assert_eq!(status_ready.status, CapIncreaseStatus::Ready);
    assert_eq!(status_ready.is_ready, true);

    client.accept_asset_cap_increase(&asset);
    assert_eq!(client.get_asset_max_session_amount(&asset), 150);

    // Expired proposals cannot be accepted
    client.propose_asset_cap_increase(&asset, &200, &(active_at + 200), &(active_at + 300));
    env.ledger().set_sequence_number(active_at + 300);

    // Verify status is Expired
    let status_expired = client.get_asset_cap_increase_status(&asset);
    assert_eq!(status_expired.status, CapIncreaseStatus::Expired);
    assert_eq!(status_expired.is_ready, false);

    assert_eq!(
        client.try_accept_asset_cap_increase(&asset),
        Err(Ok(ContractError::CapIncreaseExpired))
    );

    // Deprecated asset cannot report ready state
    client.cancel_asset_cap_increase(&asset);
    client.propose_asset_cap_increase(&asset, &300, &(active_at + 400), &(active_at + 500));
    client.deprecate_asset(&asset, &BytesN::from_array(&env, &[99; 32]));
    env.ledger().set_sequence_number(active_at + 400);
    let status_deprecated = client.get_asset_cap_increase_status(&asset);
    assert_eq!(status_deprecated.status, CapIncreaseStatus::Waiting);
    assert_eq!(status_deprecated.is_ready, false);
}

#[test]
fn cap_updates_require_the_current_admin() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let asset = create_test_asset(&env, &admin, 1);
    let policies = soroban_sdk::vec![&env, asset_policy(&asset, 100)];
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (admin.clone(), policies.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin, &policies);

    let result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_asset_max_session_amount",
                args: (asset.clone(), 50_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_asset_max_session_amount(&asset, &50);

    assert!(result.is_err());
    assert_eq!(client.get_asset_max_session_amount(&asset), 100);
}

#[test]
fn admin_rotation_requires_acceptance_and_changes_privileges_atomically() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let pending_admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let later_admin = Address::generate(&env);
    let asset = create_test_asset(&env, &admin, 1);
    let asset_policies = soroban_sdk::vec![&env, asset_policy(&asset, i128::MAX)];
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

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (pending_admin.clone(), 10_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&pending_admin, &10);
    let proposed_event = events::AdminRotationProposed {
        pending_admin: pending_admin.clone(),
        current_admin: admin.clone(),
        expires_at_ledger: 10,
        event_version: 3,
    }
    .to_xdr(&env, &contract_id);
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .last(),
        Some(&proposed_event)
    );
    assert_eq!(
        client.get_admin_rotation(),
        Some(AdminRotation {
            pending_admin: pending_admin.clone(),
            expires_at_ledger: 10,
        })
    );
    assert_eq!(client.get_admin(), admin);

    let wrong_canceller = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "cancel_admin_rotation",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_cancel_admin_rotation();
    assert!(wrong_canceller.is_err());
    assert!(client.get_admin_rotation().is_some());

    let wrong_signer = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_accept_admin();
    assert!(wrong_signer.is_err());
    assert_eq!(client.get_admin(), admin);

    client
        .mock_auths(&[MockAuth {
            address: &pending_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .accept_admin();
    let accepted_event = events::AdminRotationAccepted {
        new_admin: pending_admin.clone(),
        previous_admin: admin.clone(),
        event_version: 3,
    }
    .to_xdr(&env, &contract_id);
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .last(),
        Some(&accepted_event)
    );
    assert_eq!(client.get_admin(), pending_admin.clone());
    assert_eq!(client.get_admin_rotation(), None);

    let old_admin_result = client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (later_admin.clone(), 11_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_propose_admin(&later_admin, &11);
    assert!(old_admin_result.is_err());
}

#[test]
fn admin_rotation_rejects_overwrite_expiry_and_replay() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let first_pending = Address::generate(&env);
    let second_pending = Address::generate(&env);
    let asset = create_test_asset(&env, &admin, 1);
    client.initialize(
        &admin,
        &soroban_sdk::vec![&env, asset_policy(&asset, i128::MAX)],
    );

    client.propose_admin(&first_pending, &5);
    assert_eq!(
        client.try_propose_admin(&second_pending, &6),
        Err(Ok(ContractError::AdminRotationPending))
    );
    env.ledger().set_sequence_number(5);
    assert_eq!(
        client.try_accept_admin(),
        Err(Ok(ContractError::AdminRotationExpired))
    );
    assert_eq!(client.get_admin(), admin.clone());

    client.cancel_admin_rotation();
    let cancelled_event = events::AdminRotationCancelled {
        pending_admin: first_pending,
        current_admin: admin,
        event_version: 3,
    }
    .to_xdr(&env, &contract_id);
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .last(),
        Some(&cancelled_event)
    );
    assert_eq!(
        client.try_cancel_admin_rotation(),
        Err(Ok(ContractError::AdminRotationNotFound))
    );

    client.propose_admin(&second_pending, &6);
    client.accept_admin();
    assert_eq!(
        client.try_accept_admin(),
        Err(Ok(ContractError::AdminRotationNotFound))
    );
}

#[test]
fn admin_rotation_queries_are_deterministic_before_initialization() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_get_admin(),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(client.get_admin_rotation(), None);
}

#[test]
fn create_rejects_uninitialized_unsupported_and_excessive_duration() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let supported_asset = create_test_asset(&env, &buyer, 500);
    let unsupported_asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[44; 32]);

    let uninitialized_id = env.register(UptoSessionContract, ());
    let uninitialized = UptoSessionContractClient::new(&env, &uninitialized_id);
    assert_eq!(
        uninitialized.try_create_session(
            &buyer,
            &seller,
            &supported_asset,
            &500,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::NotInitialized))
    );

    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    initialize_for_asset(&env, &client, &buyer, &supported_asset);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &unsupported_asset,
            &500,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::UnsupportedAsset))
    );
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &supported_asset,
            &500,
            &(validation::MAX_SESSION_DURATION_LEDGERS + 1),
            &resource_hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::SessionDurationTooLong))
    );
}

#[test]
fn liability_arithmetic_is_checked() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();

    env.as_contract(&contract_id, || {
        assert_eq!(
            storage::increase_liability(&env, &asset, i128::MAX),
            Ok(i128::MAX)
        );
        assert_eq!(
            storage::increase_liability(&env, &asset, 1),
            Err(ContractError::LiabilityOverflow)
        );
        assert_eq!(storage::decrease_liability(&env, &asset, i128::MAX), Ok(0));
        assert_eq!(
            storage::decrease_liability(&env, &asset, 1),
            Err(ContractError::LiabilityUnderflow)
        );
    });
}

#[test]
fn public_liability_query_handles_initialization_support_and_multiple_assets() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let first_asset = create_test_asset(&env, &buyer, 300);
    let second_asset = create_test_asset(&env, &buyer, 400);
    let unsupported_asset = Address::generate(&env);

    assert_eq!(
        client.try_get_escrow_liability(&first_asset),
        Err(Ok(ContractError::NotInitialized))
    );
    client.initialize(
        &admin,
        &soroban_sdk::vec![
            &env,
            asset_policy(&first_asset, 300),
            asset_policy(&second_asset, 400)
        ],
    );
    assert_eq!(client.get_escrow_liability(&first_asset), 0);
    assert_eq!(client.get_escrow_liability(&second_asset), 0);
    assert_eq!(
        client.try_get_escrow_liability(&unsupported_asset),
        Err(Ok(ContractError::UnsupportedAsset))
    );

    let first_hash = BytesN::from_array(&env, &[65; 32]);
    let second_hash = BytesN::from_array(&env, &[66; 32]);
    client.create_session(
        &buyer,
        &seller,
        &first_asset,
        &100,
        &50,
        &first_hash,
        &unique_idempotency_key(&env),
    );
    client.create_session(
        &buyer,
        &seller,
        &second_asset,
        &250,
        &50,
        &second_hash,
        &unique_idempotency_key(&env),
    );

    assert_eq!(client.get_escrow_liability(&first_asset), 100);
    assert_eq!(client.get_escrow_liability(&second_asset), 250);
}

#[test]
fn public_liability_reconciles_concurrent_sessions_and_all_terminal_paths() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 600);
    initialize_for_asset(&env, &client, &admin, &asset);
    let token = token::Client::new(&env, &asset);

    let settled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[67; 32]),
        &unique_idempotency_key(&env),
    );
    let cancelled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &50,
        &BytesN::from_array(&env, &[68; 32]),
        &unique_idempotency_key(&env),
    );
    let expired_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &300,
        &50,
        &BytesN::from_array(&env, &[69; 32]),
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_escrow_liability(&asset), 600);
    assert_eq!(token.balance(&contract_id), 600);

    client.settle(&settled_id, &60, &BytesN::from_array(&env, &[70; 32]));
    assert_eq!(client.get_escrow_liability(&asset), 500);
    assert_eq!(token.balance(&contract_id), 500);

    client.cancel(&cancelled_id);
    assert_eq!(client.get_escrow_liability(&asset), 300);
    assert_eq!(token.balance(&contract_id), 300);

    env.ledger().set_sequence_number(50);
    client.recover_expired(&expired_id);
    assert_eq!(client.get_escrow_liability(&asset), 0);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn maximum_value_session_conserves_escrow() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, i128::MAX);
    let resource_hash = BytesN::from_array(&env, &[63; 32]);
    let usage_hash = BytesN::from_array(&env, &[64; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &i128::MAX,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&session_id, &1, &usage_hash);

    assert_eq!(token_client.balance(&buyer), i128::MAX - 1);
    assert_eq!(token_client.balance(&seller), 1);
    assert_eq!(token_client.balance(&contract_id), 0);
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Settled
    );
}

#[test]
fn invalid_create_inputs_do_not_consume_sequence() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[4; 32]);

    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &0,
            &10,
            &resource_hash,
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::InvalidAmount))
    );

    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 0);
    });
}

#[test]
fn expired_create_input_fails_before_storage_write() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(20);
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[4; 32]);

    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &20,
            &resource_hash,
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::ExpiredSession))
    );

    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 0);
    });
}

#[test]
fn create_session_stores_open_session() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[5; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &25,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    env.as_contract(&contract_id, || {
        let session = storage::read_session(&env, &session_id).unwrap();

        assert_eq!(session.id, session_id);
        assert_eq!(session.buyer, buyer);
        assert_eq!(session.seller, seller);
        assert_eq!(session.asset, asset);
        assert_eq!(session.max_amount, 500);
        assert_eq!(session.escrowed_amount, 500);
        assert_eq!(storage::read_liability(&env, &asset), 500);
        assert_eq!(session.settled_amount, 0);
        assert_eq!(session.expires_at_ledger, 25);
        assert_eq!(session.resource_hash, resource_hash);
        assert_eq!(session.usage_hash, None);
        assert_eq!(session.status, SessionStatus::Open);
    });
}

#[test]
fn create_session_emits_stable_event() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[5; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &25,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::SessionCreated {
            session_id,
            buyer,
            seller,
            asset,
            max_amount: 500,
            expires_at_ledger: 25,
            resource_hash,
            event_version: 2,
            escrowed_amount: 500,
        }
        .to_xdr(&env, &contract_id)]
    );
}

#[test]
fn create_session_requires_buyer_auth() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let resource_hash = BytesN::from_array(&env, &[5; 32]);
    let asset_policies = soroban_sdk::vec![&env, asset_policy(&asset, 500)];
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

    let result = client.try_create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &25,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    assert!(result.is_err());
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 0);
    });
}

#[test]
fn get_session_returns_full_state() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 900);
    let resource_hash = BytesN::from_array(&env, &[6; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &900,
        &30,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let session = client.get_session(&session_id);

    assert_eq!(session.id, session_id);
    assert_eq!(session.buyer, buyer);
    assert_eq!(session.seller, seller);
    assert_eq!(session.asset, asset);
    assert_eq!(session.max_amount, 900);
    assert_eq!(session.escrowed_amount, 900);
    assert_eq!(session.settled_amount, 0);
    assert_eq!(session.expires_at_ledger, 30);
    assert_eq!(session.resource_hash, resource_hash);
    assert_eq!(session.usage_hash, None);
    assert_eq!(session.status, SessionStatus::Open);
}

#[test]
fn get_session_returns_not_found_for_missing_session() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let missing_id = BytesN::from_array(&env, &[8; 32]);

    assert_eq!(
        client.try_get_session(&missing_id),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(
        client.try_get_session_status(&missing_id),
        Err(Ok(ContractError::SessionNotFound))
    );
}

#[test]
fn session_status_query_decodes_open_and_all_terminal_states_compatibly() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1000);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let open_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[101; 32]),
        &unique_idempotency_key(&env),
    );
    let settled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[102; 32]),
        &unique_idempotency_key(&env),
    );
    let cancelled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &BytesN::from_array(&env, &[103; 32]),
        &unique_idempotency_key(&env),
    );
    let expired_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &30,
        &BytesN::from_array(&env, &[104; 32]),
        &unique_idempotency_key(&env),
    );

    // Verify open state
    let open_full = client.get_session(&open_id);
    let open_status = client.get_session_status(&open_id);
    assert!(open_status.exists);
    assert_eq!(open_status.status, SessionStatus::Open);
    assert_eq!(open_status.status, open_full.status);
    assert_eq!(open_status.expires_at_ledger, 50);
    assert_eq!(open_status.expires_at_ledger, open_full.expires_at_ledger);

    // Settle settled_id
    client.settle(&settled_id, &50, &BytesN::from_array(&env, &[105; 32]));
    let settled_full = client.get_session(&settled_id);
    let settled_status = client.get_session_status(&settled_id);
    assert!(settled_status.exists);
    assert_eq!(settled_status.status, SessionStatus::Settled);
    assert_eq!(settled_status.status, settled_full.status);
    assert_eq!(settled_status.expires_at_ledger, 50);
    assert_eq!(
        settled_status.expires_at_ledger,
        settled_full.expires_at_ledger
    );

    // Cancel cancelled_id
    client.cancel(&cancelled_id);
    let cancelled_full = client.get_session(&cancelled_id);
    let cancelled_status = client.get_session_status(&cancelled_id);
    assert!(cancelled_status.exists);
    assert_eq!(cancelled_status.status, SessionStatus::Cancelled);
    assert_eq!(cancelled_status.status, cancelled_full.status);
    assert_eq!(cancelled_status.expires_at_ledger, 50);
    assert_eq!(
        cancelled_status.expires_at_ledger,
        cancelled_full.expires_at_ledger
    );

    // Expire expired_id
    env.ledger().set_sequence_number(35);
    client.recover_expired(&expired_id);
    let expired_full = client.get_session(&expired_id);
    let expired_status = client.get_session_status(&expired_id);
    assert!(expired_status.exists);
    assert_eq!(expired_status.status, SessionStatus::Expired);
    assert_eq!(expired_status.status, expired_full.status);
    assert_eq!(expired_status.expires_at_ledger, 30);
    assert_eq!(
        expired_status.expires_at_ledger,
        expired_full.expires_at_ledger
    );
}

#[test]
fn get_session_batch_returns_ordered_found_and_missing_results_with_duplicate_ids() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 400);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let open_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &30,
        &BytesN::from_array(&env, &[81; 32]),
        &unique_idempotency_key(&env),
    );
    let cancelled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &30,
        &BytesN::from_array(&env, &[82; 32]),
        &unique_idempotency_key(&env),
    );
    let settled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &30,
        &BytesN::from_array(&env, &[83; 32]),
        &unique_idempotency_key(&env),
    );
    let expired_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &10,
        &BytesN::from_array(&env, &[84; 32]),
        &unique_idempotency_key(&env),
    );

    client.cancel(&cancelled_id);
    client.settle(&settled_id, &60, &BytesN::from_array(&env, &[85; 32]));
    env.ledger().set_sequence_number(10);
    client.recover_expired(&expired_id);

    let missing_first = BytesN::from_array(&env, &[86; 32]);
    let missing_second = BytesN::from_array(&env, &[87; 32]);

    let open_session = client.get_session(&open_id);
    let cancelled_session = client.get_session(&cancelled_id);
    let settled_session = client.get_session(&settled_id);
    let expired_session = client.get_session(&expired_id);

    let results = client.get_session_batch(&soroban_sdk::vec![
        &env,
        open_id.clone(),
        missing_first.clone(),
        open_id.clone(),
        cancelled_id.clone(),
        settled_id.clone(),
        expired_id.clone(),
        missing_second.clone(),
        missing_first.clone(),
    ]);

    assert_eq!(
        results,
        soroban_sdk::vec![
            &env,
            BatchSessionResult {
                session_id: open_id.clone(),
                status: BatchSessionStatus::Found(open_session.clone()),
            },
            BatchSessionResult {
                session_id: missing_first.clone(),
                status: BatchSessionStatus::NotFound,
            },
            BatchSessionResult {
                session_id: open_id,
                status: BatchSessionStatus::Found(open_session),
            },
            BatchSessionResult {
                session_id: cancelled_id,
                status: BatchSessionStatus::Found(cancelled_session),
            },
            BatchSessionResult {
                session_id: settled_id,
                status: BatchSessionStatus::Found(settled_session),
            },
            BatchSessionResult {
                session_id: expired_id,
                status: BatchSessionStatus::Found(expired_session),
            },
            BatchSessionResult {
                session_id: missing_second,
                status: BatchSessionStatus::NotFound,
            },
            BatchSessionResult {
                session_id: missing_first,
                status: BatchSessionStatus::NotFound,
            },
        ]
    );
}

#[test]
fn get_session_batch_enforces_nonzero_and_max_batch_size_bounds() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 800);
    initialize_for_asset(&env, &client, &buyer, &asset);

    assert_eq!(
        client.try_get_session_batch(&soroban_sdk::vec![&env]),
        Err(Ok(ContractError::InvalidBatchSize))
    );

    let mut max_batch = soroban_sdk::Vec::new(&env);
    for idx in 0..MAX_SESSION_QUERY_BATCH_SIZE {
        let id = client.create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &30,
            &BytesN::from_array(&env, &[(90 + idx) as u8; 32]),
            &unique_idempotency_key(&env),
        );
        max_batch.push_back(id);
    }

    let max_results = client.get_session_batch(&max_batch);
    assert_eq!(max_results.len(), MAX_SESSION_QUERY_BATCH_SIZE);
    assert!(max_results
        .iter()
        .all(|item| matches!(item.status, BatchSessionStatus::Found(_))));

    let mut oversized = max_batch.clone();
    oversized.push_back(BytesN::from_array(&env, &[99; 32]));
    assert_eq!(
        client.try_get_session_batch(&oversized),
        Err(Ok(ContractError::InvalidBatchSize))
    );

    let mut oversized_duplicates = soroban_sdk::Vec::new(&env);
    let repeated_id = max_batch.get(0).unwrap();
    for _ in 0..=MAX_SESSION_QUERY_BATCH_SIZE {
        oversized_duplicates.push_back(repeated_id.clone());
    }
    assert_eq!(
        client.try_get_session_batch(&oversized_duplicates),
        Err(Ok(ContractError::InvalidBatchSize))
    );
}

#[test]
fn get_session_batch_resource_usage_is_read_only_and_bounded() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 800);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let mut batch_ids = soroban_sdk::Vec::new(&env);
    for idx in 0..MAX_SESSION_QUERY_BATCH_SIZE {
        let session_id = client.create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &50,
            &BytesN::from_array(&env, &[(110 + idx) as u8; 32]),
            &unique_idempotency_key(&env),
        );
        batch_ids.push_back(session_id);
    }

    let results = client.get_session_batch(&batch_ids);
    assert_eq!(results.len(), MAX_SESSION_QUERY_BATCH_SIZE);

    let resources = env.cost_estimate().resources();
    let fee = env.cost_estimate().fee();
    assert!(resources.instructions > 0);
    assert!(resources.mem_bytes > 0);
    assert_eq!(resources.write_entries, 0);
    assert_eq!(resources.write_bytes, 0);
    assert_eq!(resources.contract_events_size_bytes, 0);
    assert_eq!(resources.persistent_entry_rent_bumps, 0);
    assert!(fee.total > 0);
    assert!(env.auths().is_empty());
}

#[test]
fn settle_returns_not_found_for_missing_session() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let missing_id = BytesN::from_array(&env, &[8; 32]);
    let usage_hash = BytesN::from_array(&env, &[9; 32]);

    assert_eq!(
        client.try_settle(&missing_id, &10, &usage_hash),
        Err(Ok(ContractError::SessionNotFound))
    );
}

#[test]
fn settle_requires_seller_auth() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[1; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[2; 32]);
    let usage_hash = BytesN::from_array(&env, &[9; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 10,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    assert!(client.try_settle(&session_id, &10, &usage_hash).is_err());
}

#[test]
fn settle_rejects_wrong_seller_auth() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[36; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let wrong_seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[37; 32]);
    let usage_hash = BytesN::from_array(&env, &[38; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 10,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    let result = client
        .mock_auths(&[MockAuth {
            address: &wrong_seller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle",
                args: (session_id.clone(), 10_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &10, &usage_hash);

    assert!(result.is_err());

    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Open);
    assert_eq!(session.settled_amount, 0);
    assert_eq!(session.usage_hash, None);
}

#[test]
fn settle_rejects_finalized_sessions() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[2; 32]);
    let usage_hash = BytesN::from_array(&env, &[9; 32]);
    let settled_id = BytesN::from_array(&env, &[10; 32]);
    let cancelled_id = BytesN::from_array(&env, &[11; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: settled_id.clone(),
                buyer: buyer.clone(),
                seller: seller.clone(),
                asset: asset.clone(),
                max_amount: 100,
                escrowed_amount: 0,
                settled_amount: 25,
                expires_at_ledger: 50,
                resource_hash: resource_hash.clone(),
                usage_hash: Some(usage_hash.clone()),
                status: SessionStatus::Settled,
                creation_ledger: 10,
                finalized_ledger: Some(25),
                resource_scheme_version: 1,
                usage_scheme_version: Some(1),
            },
        )
        .unwrap();
        storage::write_session(
            &env,
            &Session {
                id: cancelled_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 0,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Cancelled,
                creation_ledger: 10,
                finalized_ledger: Some(20),
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    assert_eq!(
        client.try_settle(&settled_id, &10, &usage_hash),
        Err(Ok(ContractError::SessionAlreadySettled))
    );
    assert_eq!(
        client.try_settle(&cancelled_id, &10, &usage_hash),
        Err(Ok(ContractError::SessionCancelled))
    );
}

#[test]
fn settle_stores_actual_amount_and_status() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[12; 32]);
    let usage_hash = BytesN::from_array(&env, &[13; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&session_id, &125, &usage_hash);
    let session = client.get_session(&session_id);

    assert_eq!(session.settled_amount, 125);
    assert_eq!(session.usage_hash, Some(usage_hash));
    assert_eq!(session.status, SessionStatus::Settled);
    assert_eq!(session.escrowed_amount, 0);
    assert_eq!(token_client.balance(&buyer), 375);
    assert_eq!(token_client.balance(&seller), 125);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn settle_accepts_exact_cap_without_refund() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[45; 32]);
    let usage_hash = BytesN::from_array(&env, &[46; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&session_id, &500, &usage_hash);

    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Settled);
    assert_eq!(session.settled_amount, 500);
    assert_eq!(session.escrowed_amount, 0);
    assert_eq!(token_client.balance(&buyer), 0);
    assert_eq!(token_client.balance(&seller), 500);
    assert_eq!(token_client.balance(&contract_id), 0);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
}

#[test]
fn settlement_isolates_concurrent_sessions_sharing_an_asset() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_000);
    let resource_one = BytesN::from_array(&env, &[47; 32]);
    let resource_two = BytesN::from_array(&env, &[48; 32]);
    let usage_one = BytesN::from_array(&env, &[49; 32]);
    let usage_two = BytesN::from_array(&env, &[50; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let first = client.create_session(
        &buyer,
        &seller,
        &asset,
        &400,
        &50,
        &resource_one,
        &unique_idempotency_key(&env),
    );
    let second = client.create_session(
        &buyer,
        &seller,
        &asset,
        &600,
        &50,
        &resource_two,
        &unique_idempotency_key(&env),
    );
    client.settle(&first, &150, &usage_one);

    assert_eq!(client.get_session(&first).settled_amount, 150);
    assert_eq!(client.get_session(&second).escrowed_amount, 600);
    assert_eq!(token_client.balance(&buyer), 250);
    assert_eq!(token_client.balance(&seller), 150);
    assert_eq!(token_client.balance(&contract_id), 600);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 600);
    });

    client.settle(&second, &600, &usage_two);
    assert_eq!(token_client.balance(&buyer), 250);
    assert_eq!(token_client.balance(&seller), 750);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn asset_deprecation_requires_admin_and_blocks_new_sessions() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let asset = create_test_asset(&env, &admin, 1);
    let asset_policies = soroban_sdk::vec![&env, asset_policy(&asset, i128::MAX)];

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

    let deprecate_reason = BytesN::from_array(&env, &[70; 32]);
    let empty_reason = BytesN::from_array(&env, &[0; 32]);

    assert_eq!(
        client.try_deprecate_asset(&asset, &empty_reason),
        Err(Ok(ContractError::InvalidResourceHash))
    );

    let unauthorized = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "deprecate_asset",
                args: (asset.clone(), deprecate_reason.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_deprecate_asset(&asset, &deprecate_reason);
    assert!(unauthorized.is_err());
    env.as_contract(&contract_id, || {
        assert_eq!(
            storage::read_asset_status(&env, &asset),
            Some(AssetStatus::Active)
        );
    });

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "deprecate_asset",
                args: (asset.clone(), deprecate_reason.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .deprecate_asset(&asset, &deprecate_reason);
    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::AssetDeprecated {
            asset: asset.clone(),
            admin: admin.clone(),
            reason_commitment: deprecate_reason.clone(),
            event_version: 3,
        }
        .to_xdr(&env, &contract_id)]
    );
    env.as_contract(&contract_id, || {
        assert_eq!(
            storage::read_asset_status(&env, &asset),
            Some(AssetStatus::Deprecated)
        );
    });
    assert_eq!(
        client
            .mock_auths(&[MockAuth {
                address: &admin,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "deprecate_asset",
                    args: (asset.clone(), deprecate_reason.clone()).into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .try_deprecate_asset(&asset, &deprecate_reason),
        Err(Ok(ContractError::AssetNotActive))
    );

    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[71; 32]);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::UnsupportedAsset))
    );
}

#[test]
fn deprecated_asset_terminal_paths_clear_liability_before_removal() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 900);
    initialize_for_asset(&env, &client, &admin, &asset);

    let settle_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &300,
        &50,
        &BytesN::from_array(&env, &[72; 32]),
        &unique_idempotency_key(&env),
    );
    let cancel_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &300,
        &50,
        &BytesN::from_array(&env, &[73; 32]),
        &unique_idempotency_key(&env),
    );
    let recover_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &300,
        &50,
        &BytesN::from_array(&env, &[74; 32]),
        &unique_idempotency_key(&env),
    );

    let deprecate_reason = BytesN::from_array(&env, &[77; 32]);
    let remove_reason = BytesN::from_array(&env, &[78; 32]);
    let empty_reason = BytesN::from_array(&env, &[0; 32]);

    client.deprecate_asset(&asset, &deprecate_reason);
    assert_eq!(
        client.try_remove_asset(&asset, &empty_reason),
        Err(Ok(ContractError::InvalidResourceHash))
    );
    assert_eq!(
        client.try_remove_asset(&asset, &remove_reason),
        Err(Ok(ContractError::AssetLiabilityOutstanding))
    );

    client.settle(&settle_id, &125, &BytesN::from_array(&env, &[75; 32]));
    client.cancel(&cancel_id);
    env.ledger().set_sequence_number(50);
    client.recover_expired(&recover_id);

    assert_eq!(
        client.get_session(&settle_id).status,
        SessionStatus::Settled
    );
    assert_eq!(
        client.get_session(&cancel_id).status,
        SessionStatus::Cancelled
    );
    assert_eq!(
        client.get_session(&recover_id).status,
        SessionStatus::Expired
    );
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });

    client.remove_asset(&asset, &remove_reason);
    let expected = events::AssetRemoved {
        asset: asset.clone(),
        admin: admin.clone(),
        reason_commitment: remove_reason.clone(),
        event_version: 3,
    }
    .to_xdr(&env, &contract_id);
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .last(),
        Some(&expected)
    );
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_asset_status(&env, &asset), None);
    });
    assert_eq!(
        client.try_remove_asset(&asset, &remove_reason),
        Err(Ok(ContractError::AssetNotDeprecated))
    );
}

#[test]
fn undercollateralized_settlement_fails_without_state_transition() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let sink = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[51; 32]);
    let usage_hash = BytesN::from_array(&env, &[52; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.as_contract(&contract_id, || {
        token_client.transfer(&contract_id, &sink, &1);
    });

    assert_eq!(
        client.try_settle(&session_id, &125, &usage_hash),
        Err(Ok(ContractError::EscrowUnderfunded))
    );
    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Open);
    assert_eq!(session.settled_amount, 0);
    assert_eq!(session.usage_hash, None);
    assert_eq!(session.escrowed_amount, 500);
    assert_eq!(token_client.balance(&seller), 0);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 500);
    });
}

#[test]
fn settle_accepts_local_test_token_contract() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_local_test_token(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[34; 32]);
    let usage_hash = BytesN::from_array(&env, &[35; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&session_id, &125, &usage_hash);

    assert_eq!(token_client.balance(&buyer), 375);
    assert_eq!(token_client.balance(&seller), 125);
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Settled
    );
}

#[test]
fn adversarial_create_failures_roll_back_funds_liability_and_sequence() {
    assert_adversarial_creation_rolls_back(AdversarialBehavior::FailTransfer);
    assert_adversarial_creation_rolls_back(AdversarialBehavior::FalseBalance);
    assert_adversarial_creation_rolls_back(AdversarialBehavior::PartialTransfer);
}

#[test]
fn adversarial_terminal_failures_leave_the_session_open_and_funded() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_adversarial_token(&env, &buyer, 500);
    let token = AdversarialTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &BytesN::from_array(&env, &[72; 32]),
        &unique_idempotency_key(&env),
    );
    let usage_hash = BytesN::from_array(&env, &[73; 32]);

    for behavior in [
        AdversarialBehavior::FailTransfer,
        AdversarialBehavior::FalseBalance,
        AdversarialBehavior::PartialTransfer,
    ] {
        token.set_behavior(&behavior);
        assert!(client.try_settle(&session_id, &100, &usage_hash).is_err());
        let session = client.get_session(&session_id);
        assert_eq!(session.status, SessionStatus::Open);
        assert_eq!(session.escrowed_amount, 500);
        assert_eq!(token.raw_balance(&contract_id), 500);
        assert_eq!(token.raw_balance(&seller), 0);
        assert_eq!(token.raw_balance(&buyer), 0);
        env.as_contract(&contract_id, || {
            assert_eq!(storage::read_liability(&env, &asset), 500);
        });
    }

    token.set_behavior(&AdversarialBehavior::Honest);
    client.settle(&session_id, &100, &usage_hash);
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Settled
    );
    assert_eq!(token.raw_balance(&seller), 100);
    assert_eq!(token.raw_balance(&buyer), 400);
    assert_eq!(token.raw_balance(&contract_id), 0);
}

#[test]
fn nested_token_callback_cannot_reenter_a_terminal_transition() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_adversarial_token(&env, &buyer, 500);
    let token = AdversarialTokenContractClient::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &BytesN::from_array(&env, &[74; 32]),
        &unique_idempotency_key(&env),
    );
    let usage_hash = BytesN::from_array(&env, &[75; 32]);
    token.configure_nested_attempt(&contract_id, &session_id, &100, &usage_hash);

    client.settle(&session_id, &100, &usage_hash);

    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Settled);
    assert_eq!(session.settled_amount, 100);
    assert!(token.callback_rejected());
    assert_eq!(token.raw_balance(&seller), 100);
    assert_eq!(token.raw_balance(&buyer), 400);
    assert_eq!(token.raw_balance(&contract_id), 0);
    assert_eq!(
        client.try_settle(&session_id, &100, &usage_hash),
        Err(Ok(ContractError::SessionAlreadySettled))
    );
}

#[test]
fn settle_rejects_invalid_amounts_and_expiry() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_500);
    let resource_hash = BytesN::from_array(&env, &[12; 32]);
    let usage_hash = BytesN::from_array(&env, &[13; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let zero_amount_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let over_cap_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let expired_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    assert_eq!(
        client.try_settle(&zero_amount_id, &0, &usage_hash),
        Err(Ok(ContractError::InvalidAmount))
    );
    assert_eq!(
        client.try_settle(&over_cap_id, &501, &usage_hash),
        Err(Ok(ContractError::AmountExceedsCap))
    );

    env.ledger().set_sequence_number(50);

    assert_eq!(
        client.try_settle(&expired_id, &100, &usage_hash),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(client.get_session(&expired_id).settled_amount, 0);
    assert_eq!(client.get_session(&expired_id).status, SessionStatus::Open);
}

#[test]
fn expired_open_session_remains_observable_and_permissionlessly_recoverable() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[32; 32]);
    let usage_hash = BytesN::from_array(&env, &[33; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    env.ledger().set_sequence_number(50);

    assert_eq!(
        client.try_settle(&session_id, &100, &usage_hash),
        Err(Ok(ContractError::ExpiredSession))
    );

    let expired_session = client.get_session(&session_id);
    assert_eq!(expired_session.status, SessionStatus::Open);
    assert_eq!(expired_session.settled_amount, 0);
    assert_eq!(expired_session.expires_at_ledger, 50);

    assert_eq!(
        client.try_cancel(&session_id),
        Err(Ok(ContractError::ExpiredSession))
    );
    client.recover_expired(&session_id);

    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Expired
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 500);
}

#[test]
fn expiry_boundary_selects_exactly_one_terminal_path() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_500);
    let resource_hash = BytesN::from_array(&env, &[53; 32]);
    let usage_hash = BytesN::from_array(&env, &[54; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let settles_before = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let cancels_before = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let recovers_at = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    assert_eq!(
        client.try_recover_expired(&recovers_at),
        Err(Ok(ContractError::SessionNotExpired))
    );
    env.ledger().set_sequence_number(49);
    client.settle(&settles_before, &250, &usage_hash);
    client.cancel(&cancels_before);

    env.ledger().set_sequence_number(50);
    assert_eq!(
        client.try_settle(&recovers_at, &250, &usage_hash),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_cancel(&recovers_at),
        Err(Ok(ContractError::ExpiredSession))
    );
    client.recover_expired(&recovers_at);

    assert_eq!(
        client.get_session(&settles_before).status,
        SessionStatus::Settled
    );
    assert_eq!(
        client.get_session(&cancels_before).status,
        SessionStatus::Cancelled
    );
    assert_eq!(
        client.get_session(&recovers_at).status,
        SessionStatus::Expired
    );
}

#[test]
fn expiry_recovery_is_permissionless_fixed_recipient_and_single_use() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let third_party = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[55; 32]);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    env.ledger().set_sequence_number(51);
    client.recover_expired(&session_id);

    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Expired);
    assert_eq!(session.escrowed_amount, 0);
    assert_eq!(token_client.balance(&buyer), 500);
    assert_eq!(token_client.balance(&seller), 0);
    assert_eq!(token_client.balance(&third_party), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
    assert!(env.auths().is_empty());
    assert_eq!(
        client.try_recover_expired(&session_id),
        Err(Ok(ContractError::SessionExpired))
    );
    assert_eq!(
        client.try_cancel(&session_id),
        Err(Ok(ContractError::SessionExpired))
    );
}

#[test]
fn batch_recovery_is_bounded_and_reports_partial_results_in_order() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 700);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let recoverable = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &10,
        &BytesN::from_array(&env, &[70; 32]),
        &unique_idempotency_key(&env),
    );
    let cancelled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &20,
        &BytesN::from_array(&env, &[71; 32]),
        &unique_idempotency_key(&env),
    );
    let settled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &20,
        &BytesN::from_array(&env, &[72; 32]),
        &unique_idempotency_key(&env),
    );
    let not_expired = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &20,
        &BytesN::from_array(&env, &[73; 32]),
        &unique_idempotency_key(&env),
    );
    client.cancel(&cancelled);
    client.settle(&settled, &100, &BytesN::from_array(&env, &[74; 32]));
    env.ledger().set_sequence_number(10);
    let missing = BytesN::from_array(&env, &[75; 32]);

    let results = client.recover_expired_batch(&soroban_sdk::vec![
        &env,
        recoverable.clone(),
        missing.clone(),
        recoverable.clone(),
        cancelled.clone(),
        settled.clone(),
        not_expired.clone(),
    ]);
    assert_eq!(
        results,
        soroban_sdk::vec![
            &env,
            BatchRecoveryResult {
                session_id: recoverable.clone(),
                status: BatchRecoveryStatus::Recovered,
                refunded_amount: 100,
            },
            BatchRecoveryResult {
                session_id: missing,
                status: BatchRecoveryStatus::NotFound,
                refunded_amount: 0,
            },
            BatchRecoveryResult {
                session_id: recoverable.clone(),
                status: BatchRecoveryStatus::Duplicate,
                refunded_amount: 0,
            },
            BatchRecoveryResult {
                session_id: cancelled,
                status: BatchRecoveryStatus::AlreadyCancelled,
                refunded_amount: 0,
            },
            BatchRecoveryResult {
                session_id: settled,
                status: BatchRecoveryStatus::AlreadySettled,
                refunded_amount: 0,
            },
            BatchRecoveryResult {
                session_id: not_expired,
                status: BatchRecoveryStatus::NotExpired,
                refunded_amount: 0,
            },
        ]
    );
    assert_eq!(token_client.balance(&buyer), 500);
    assert_eq!(token_client.balance(&seller), 100);
    assert_eq!(token_client.balance(&contract_id), 100);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 100);
    });

    let repeated = client.recover_expired_batch(&soroban_sdk::vec![&env, recoverable.clone()]);
    assert_eq!(
        repeated.get(0).unwrap().status,
        BatchRecoveryStatus::AlreadyRecovered
    );
    assert_eq!(client.get_session(&recoverable).escrowed_amount, 0);

    assert_eq!(
        client.try_recover_expired_batch(&soroban_sdk::vec![&env]),
        Err(Ok(ContractError::InvalidBatchSize))
    );
    let mut oversized = soroban_sdk::Vec::new(&env);
    for byte in 0..=MAX_RECOVERY_BATCH_SIZE {
        oversized.push_back(BytesN::from_array(&env, &[byte as u8; 32]));
    }
    assert_eq!(
        client.try_recover_expired_batch(&oversized),
        Err(Ok(ContractError::InvalidBatchSize))
    );
}

#[test]
fn batch_recovery_refunds_each_stored_buyer_only() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let first_buyer = Address::generate(&env);
    let second_buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let caller = Address::generate(&env);
    let asset = create_test_asset(&env, &first_buyer, 100);
    token::StellarAssetClient::new(&env, &asset).mint(&second_buyer, &100);
    let token_client = token::Client::new(&env, &asset);
    initialize_for_asset(&env, &client, &first_buyer, &asset);
    let first = client.create_session(
        &first_buyer,
        &seller,
        &asset,
        &100,
        &10,
        &BytesN::from_array(&env, &[76; 32]),
        &unique_idempotency_key(&env),
    );
    let second = client.create_session(
        &second_buyer,
        &seller,
        &asset,
        &100,
        &10,
        &BytesN::from_array(&env, &[77; 32]),
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(10);

    let results = client.recover_expired_batch(&soroban_sdk::vec![&env, first, second]);
    assert!(results
        .iter()
        .all(|result| result.status == BatchRecoveryStatus::Recovered));
    assert_eq!(token_client.balance(&first_buyer), 100);
    assert_eq!(token_client.balance(&second_buyer), 100);
    assert_eq!(token_client.balance(&caller), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
}

#[test]
fn expiry_recovery_rejects_missing_settled_and_cancelled_sessions() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_000);
    let resource_hash = BytesN::from_array(&env, &[58; 32]);
    let usage_hash = BytesN::from_array(&env, &[59; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let settled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let cancelled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&settled, &250, &usage_hash);
    client.cancel(&cancelled);
    env.ledger().set_sequence_number(50);

    assert_eq!(
        client.try_recover_expired(&BytesN::from_array(&env, &[60; 32])),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(
        client.try_recover_expired(&settled),
        Err(Ok(ContractError::SessionAlreadySettled))
    );
    assert_eq!(
        client.try_recover_expired(&cancelled),
        Err(Ok(ContractError::SessionCancelled))
    );
}

#[test]
fn expiry_recovery_emits_versioned_receipt() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[56; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    env.ledger().set_sequence_number(50);
    client.recover_expired(&session_id);

    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::SessionRecovered {
            session_id,
            buyer,
            asset,
            event_version: 2,
            refunded_amount: 500,
            expired_at_ledger: 50,
            recovered_at_ledger: 50,
        }
        .to_xdr(&env, &contract_id),]
    );
}

#[test]
fn insufficient_funding_prevents_session_creation() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 10);
    let resource_hash = BytesN::from_array(&env, &[12; 32]);
    let usage_hash = BytesN::from_array(&env, &[13; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    assert!(client
        .try_create_session(
            &buyer,
            &seller,
            &asset,
            &500,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env)
        )
        .is_err());
    let _ = usage_hash;
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 0);
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
}

#[test]
fn settle_rejects_empty_usage_hash_before_transfer() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let token_client = token::Client::new(&env, &asset);
    let resource_hash = BytesN::from_array(&env, &[12; 32]);
    let usage_hash = BytesN::from_array(&env, &[0; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    assert_eq!(
        client.try_settle(&session_id, &125, &usage_hash),
        Err(Ok(ContractError::InvalidUsageHash))
    );
    let session = client.get_session(&session_id);

    assert_eq!(session.usage_hash, None);
    assert_eq!(session.settled_amount, 0);
    assert_eq!(session.status, SessionStatus::Open);
    assert_eq!(token_client.balance(&buyer), 0);
    assert_eq!(token_client.balance(&seller), 0);
    assert_eq!(token_client.balance(&contract_id), 500);
}

#[test]
fn settle_prevents_double_settlement() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let token_client = token::Client::new(&env, &asset);
    let resource_hash = BytesN::from_array(&env, &[14; 32]);
    let usage_hash = BytesN::from_array(&env, &[15; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    // First settlement succeeds
    client.settle(&session_id, &100, &usage_hash);
    let session_after_first = client.get_session(&session_id);
    assert_eq!(session_after_first.settled_amount, 100);
    assert_eq!(session_after_first.status, SessionStatus::Settled);
    assert_eq!(token_client.balance(&buyer), 400);
    assert_eq!(token_client.balance(&seller), 100);

    // Second settlement with same amount fails
    let result_same = client.try_settle(&session_id, &100, &usage_hash);
    assert_eq!(result_same, Err(Ok(ContractError::SessionAlreadySettled)));

    // Second settlement with different amount also fails
    let result_different = client.try_settle(&session_id, &50, &usage_hash);
    assert_eq!(
        result_different,
        Err(Ok(ContractError::SessionAlreadySettled))
    );

    // Session state unchanged after failed settlement attempts
    let session_after_attempts = client.get_session(&session_id);
    assert_eq!(session_after_attempts.settled_amount, 100);
    assert_eq!(session_after_attempts.status, SessionStatus::Settled);
    assert_eq!(token_client.balance(&buyer), 400);
    assert_eq!(token_client.balance(&seller), 100);
}

#[test]
fn settle_emits_stable_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[16; 32]);
    let usage_hash = BytesN::from_array(&env, &[17; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    client.settle(&session_id, &125, &usage_hash);

    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::SessionSettled {
            session_id,
            seller,
            asset,
            actual_amount: 125,
            usage_hash,
            event_version: 2,
            refunded_amount: 375,
        }
        .to_xdr(&env, &contract_id)]
    );
}

#[test]
fn cancel_allows_buyer_cancellation_before_settlement() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[18; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let session_before = client.get_session(&session_id);
    assert_eq!(session_before.status, SessionStatus::Open);

    client.cancel(&session_id);
    let session_after = client.get_session(&session_id);

    assert_eq!(session_after.status, SessionStatus::Cancelled);
}

#[test]
fn cancel_requires_buyer_auth() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[1; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[2; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 10,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    assert!(client.try_cancel(&session_id).is_err());
}

#[test]
fn cancel_returns_not_found_for_missing_session() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let missing_id = BytesN::from_array(&env, &[8; 32]);

    assert_eq!(
        client.try_cancel(&missing_id),
        Err(Ok(ContractError::SessionNotFound))
    );
}

#[test]
fn cancel_rejects_already_settled_session() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[19; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[20; 32]);
    let usage_hash = BytesN::from_array(&env, &[21; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 0,
                settled_amount: 50,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: Some(usage_hash),
                status: SessionStatus::Settled,
                creation_ledger: 10,
                finalized_ledger: Some(30),
                resource_scheme_version: 1,
                usage_scheme_version: Some(1),
            },
        )
        .unwrap();
    });

    assert_eq!(
        client.try_cancel(&session_id),
        Err(Ok(ContractError::SessionAlreadySettled))
    );
}

#[test]
fn cancel_rejects_already_cancelled_session() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[22; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[23; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 0,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Cancelled,
                creation_ledger: 10,
                finalized_ledger: Some(25),
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    assert_eq!(
        client.try_cancel(&session_id),
        Err(Ok(ContractError::SessionCancelled))
    );
}

#[test]
fn cancel_emits_stable_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[24; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    // Verify session is open before cancel
    let session_before = client.get_session(&session_id);
    assert_eq!(session_before.status, SessionStatus::Open);

    let cancel_result = client.try_cancel(&session_id);
    assert!(cancel_result.is_ok(), "Cancel should succeed");

    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::SessionCancelled {
            session_id: session_id.clone(),
            buyer: buyer.clone(),
            asset: asset.clone(),
            event_version: 2,
            refunded_amount: 500,
        }
        .to_xdr(&env, &contract_id)]
    );

    // Verify session is cancelled after cancel
    let session_after = client.get_session(&session_id);
    assert_eq!(session_after.status, SessionStatus::Cancelled);
}

#[test]
fn reject_allows_seller_rejection_and_refunds_buyer_only() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[81; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    let session_before = client.get_session(&session_id);
    assert_eq!(session_before.status, SessionStatus::Open);

    let reject_result = client.try_reject(&session_id);
    assert!(reject_result.is_ok(), "Reject should succeed");

    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::SessionRejected {
            session_id: session_id.clone(),
            seller: seller.clone(),
            buyer: buyer.clone(),
            asset: asset.clone(),
            event_version: 2,
            refunded_amount: 500,
        }
        .to_xdr(&env, &contract_id)]
    );

    let session_after = client.get_session(&session_id);
    assert_eq!(session_after.status, SessionStatus::Rejected);
    assert_eq!(session_after.escrowed_amount, 0);

    let token_client = token::Client::new(&env, &asset);
    assert_eq!(token_client.balance(&buyer), 500);
    assert_eq!(token_client.balance(&seller), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn reject_requires_seller_auth() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[82; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[83; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 0,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    assert!(client.try_reject(&session_id).is_err());
}

#[test]
fn reject_rejects_terminal_and_expired_states() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 2_000);
    let resource_hash = BytesN::from_array(&env, &[84; 32]);
    let usage_hash = BytesN::from_array(&env, &[85; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let missing_id = BytesN::from_array(&env, &[86; 32]);
    assert_eq!(
        client.try_reject(&missing_id),
        Err(Ok(ContractError::SessionNotFound))
    );

    let rejected_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.reject(&rejected_id);
    assert_eq!(
        client.try_reject(&rejected_id),
        Err(Ok(ContractError::SessionRejected))
    );
    assert_eq!(
        client.try_settle(&rejected_id, &100, &usage_hash),
        Err(Ok(ContractError::SessionRejected))
    );
    assert_eq!(
        client.try_cancel(&rejected_id),
        Err(Ok(ContractError::SessionRejected))
    );
    env.ledger().set_sequence_number(50);
    assert_eq!(
        client.try_recover_expired(&rejected_id),
        Err(Ok(ContractError::SessionRejected))
    );

    let settled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&settled_id, &100, &usage_hash);
    assert_eq!(
        client.try_reject(&settled_id),
        Err(Ok(ContractError::SessionAlreadySettled))
    );

    let cancelled_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.cancel(&cancelled_id);
    assert_eq!(
        client.try_reject(&cancelled_id),
        Err(Ok(ContractError::SessionCancelled))
    );

    let expired_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &70,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(70);
    assert_eq!(
        client.try_reject(&expired_id),
        Err(Ok(ContractError::ExpiredSession))
    );
}

#[test]
fn preview_settle_calculates_exact_payout_and_refund_without_state_changes_or_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_000);
    let resource_hash = BytesN::from_array(&env, &[87; 32]);
    let usage_hash = BytesN::from_array(&env, &[88; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    // 1. Valid partial settlement preview
    let preview = client.preview_settle(&session_id, &200);
    assert_eq!(
        preview,
        SettlementPreview {
            eligible: true,
            actual_amount: 200,
            seller_payout: 200,
            buyer_refund: 300,
            is_expired: false,
            rejection_reason: SettlementRejectionReason::None,
        }
    );

    // 2. Valid full settlement preview (payout == cap, refund == 0)
    let full_preview = client.preview_settle(&session_id, &500);
    assert_eq!(
        full_preview,
        SettlementPreview {
            eligible: true,
            actual_amount: 500,
            seller_payout: 500,
            buyer_refund: 0,
            is_expired: false,
            rejection_reason: SettlementRejectionReason::None,
        }
    );

    // Verify session remains Open, balances untouched, no events emitted
    let session_before = client.get_session(&session_id);
    assert_eq!(session_before.status, SessionStatus::Open);
    assert_eq!(session_before.escrowed_amount, 500);
    let token_client = token::Client::new(&env, &asset);
    assert_eq!(token_client.balance(&contract_id), 500);
    assert_eq!(token_client.balance(&seller), 0);
    assert_eq!(token_client.balance(&buyer), 500);

    // 3. Settle and verify actual settlement amounts match preview exactly
    client.settle(&session_id, &200, &usage_hash);
    let session_after = client.get_session(&session_id);
    assert_eq!(session_after.settled_amount, preview.seller_payout);
    assert_eq!(token_client.balance(&seller), preview.seller_payout);
    assert_eq!(token_client.balance(&buyer), 500 + preview.buyer_refund);
}

#[test]
fn preview_settle_reports_deterministic_rejection_reasons() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 2_000);
    let resource_hash = BytesN::from_array(&env, &[89; 32]);
    let usage_hash = BytesN::from_array(&env, &[90; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    // 1. Missing session
    let missing_id = BytesN::from_array(&env, &[91; 32]);
    let missing_preview = client.preview_settle(&missing_id, &100);
    assert_eq!(
        missing_preview,
        SettlementPreview {
            eligible: false,
            actual_amount: 100,
            seller_payout: 0,
            buyer_refund: 0,
            is_expired: false,
            rejection_reason: SettlementRejectionReason::NotFound,
        }
    );

    // 2. Invalid zero and negative amounts
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let zero_preview = client.preview_settle(&session_id, &0);
    assert_eq!(
        zero_preview.rejection_reason,
        SettlementRejectionReason::InvalidAmount
    );
    assert!(!zero_preview.eligible);

    let neg_preview = client.preview_settle(&session_id, &-10);
    assert_eq!(
        neg_preview.rejection_reason,
        SettlementRejectionReason::InvalidAmount
    );
    assert!(!neg_preview.eligible);

    // 3. Amount exceeds cap
    let exceed_preview = client.preview_settle(&session_id, &501);
    assert_eq!(
        exceed_preview.rejection_reason,
        SettlementRejectionReason::AmountExceedsCap
    );
    assert!(!exceed_preview.eligible);

    // 4. Expired state
    let expired_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &30,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(30);
    let expired_preview = client.preview_settle(&expired_id, &100);
    assert_eq!(
        expired_preview,
        SettlementPreview {
            eligible: false,
            actual_amount: 100,
            seller_payout: 0,
            buyer_refund: 0,
            is_expired: true,
            rejection_reason: SettlementRejectionReason::Expired,
        }
    );

    // 5. Terminal states: Settled, Cancelled, Expired, Rejected
    env.ledger().set_sequence_number(1);
    let s_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&s_id, &100, &usage_hash);
    assert_eq!(
        client.preview_settle(&s_id, &100).rejection_reason,
        SettlementRejectionReason::AlreadySettled
    );

    let c_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.cancel(&c_id);
    assert_eq!(
        client.preview_settle(&c_id, &100).rejection_reason,
        SettlementRejectionReason::AlreadyCancelled
    );

    let r_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.reject(&r_id);
    assert_eq!(
        client.preview_settle(&r_id, &100).rejection_reason,
        SettlementRejectionReason::AlreadyRejected
    );

    let exp_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &40,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(40);
    client.recover_expired(&exp_id);
    assert_eq!(
        client.preview_settle(&exp_id, &100).rejection_reason,
        SettlementRejectionReason::AlreadyExpired
    );
}

#[test]
fn preview_settle_detects_undercollateralized_escrow() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[92; 32]);
    let session_id = BytesN::from_array(&env, &[93; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 500,
                escrowed_amount: 100, // Partial escrow amount != max_amount
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 0,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    let preview = client.preview_settle(&session_id, &50);
    assert_eq!(
        preview.rejection_reason,
        SettlementRejectionReason::EscrowUnderfunded
    );
    assert!(!preview.eligible);
}

#[test]
fn extend_ttl_succeeds_for_open_sessions() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[25; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    assert!(client.try_extend_ttl(&session_id).is_ok());
}

#[test]
fn extend_ttl_rejects_cancelled_sessions() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[26; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[27; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 0,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Cancelled,
                creation_ledger: 10,
                finalized_ledger: Some(25),
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        )
        .unwrap();
    });

    assert_eq!(
        client.try_extend_ttl(&session_id),
        Err(Ok(ContractError::TtlExtensionFailed))
    );
}

#[test]
fn extend_ttl_rejects_settled_sessions() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let session_id = BytesN::from_array(&env, &[28; 32]);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[29; 32]);
    let usage_hash = BytesN::from_array(&env, &[30; 32]);

    env.as_contract(&contract_id, || {
        storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer,
                seller,
                asset,
                max_amount: 100,
                escrowed_amount: 0,
                settled_amount: 50,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: Some(usage_hash),
                status: SessionStatus::Settled,
                creation_ledger: 10,
                finalized_ledger: Some(30),
                resource_scheme_version: 1,
                usage_scheme_version: Some(1),
            },
        )
        .unwrap();
    });

    assert_eq!(
        client.try_extend_ttl(&session_id),
        Err(Ok(ContractError::TtlExtensionFailed))
    );
}

#[test]
fn extend_ttl_rejects_missing_session() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let missing_id = BytesN::from_array(&env, &[31; 32]);

    assert_eq!(
        client.try_extend_ttl(&missing_id),
        Err(Ok(ContractError::SessionNotFound))
    );
}

#[test]
fn recoverability_query_matches_recover_expired_boundary_and_terminal_states() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 2_000);
    let resource_hash = BytesN::from_array(&env, &[94; 32]);
    let usage_hash = BytesN::from_array(&env, &[95; 32]);
    initialize_for_asset(&env, &client, &buyer, &asset);

    let missing = client.get_recoverability(&BytesN::from_array(&env, &[96; 32]));
    assert_eq!(missing.status, RecoverabilityStatus::NotFound);
    assert_eq!(missing.response_version, 1);
    assert_eq!(missing.recipient, None);
    assert_eq!(missing.refundable_amount, 0);

    let open_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(49);
    let before = client.get_recoverability(&open_id);
    assert_eq!(before.status, RecoverabilityStatus::NotExpired);
    assert_eq!(before.expires_at_ledger, 50);
    assert_eq!(before.current_ledger, 49);
    assert_eq!(before.recipient, Some(buyer.clone()));
    assert_eq!(before.asset, Some(asset.clone()));
    assert_eq!(before.refundable_amount, 0);
    assert_eq!(
        client.try_recover_expired(&open_id),
        Err(Ok(ContractError::SessionNotExpired))
    );

    env.ledger().set_sequence_number(50);
    let at = client.get_recoverability(&open_id);
    assert_eq!(at.status, RecoverabilityStatus::Recoverable);
    assert_eq!(at.refundable_amount, 200);
    assert_eq!(at.recipient, Some(buyer.clone()));
    // Query is storage-neutral: session still open.
    assert_eq!(client.get_session(&open_id).status, SessionStatus::Open);
    client.recover_expired(&open_id);
    assert_eq!(
        client.get_recoverability(&open_id).status,
        RecoverabilityStatus::AlreadyRecovered
    );

    env.ledger().set_sequence_number(1);
    let settled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.settle(&settled, &100, &usage_hash);
    assert_eq!(
        client.get_recoverability(&settled).status,
        RecoverabilityStatus::AlreadySettled
    );
    let cancelled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.cancel(&cancelled);
    assert_eq!(
        client.get_recoverability(&cancelled).status,
        RecoverabilityStatus::AlreadyCancelled
    );
    let rejected = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &60,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    client.reject(&rejected);
    let rej = client.get_recoverability(&rejected);
    assert_eq!(rej.status, RecoverabilityStatus::AlreadyRejected);
    assert_eq!(rej.refundable_amount, 0);
}

#[test]
fn solvency_query_reports_ordered_mixed_health_with_defined_edge_cases() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let healthy = create_test_asset(&env, &buyer, 300);
    let surplus = create_test_asset(&env, &buyer, 300);
    let underfunded = create_test_asset(&env, &buyer, 300);
    let deprecated = create_test_asset(&env, &buyer, 300);
    let unknown = Address::generate(&env);

    assert_eq!(
        client.try_get_solvency(&soroban_sdk::vec![&env, healthy.clone()]),
        Err(Ok(ContractError::NotInitialized))
    );
    client.initialize(
        &admin,
        &soroban_sdk::vec![
            &env,
            asset_policy(&healthy, 300),
            asset_policy(&surplus, 300),
            asset_policy(&underfunded, 300),
            asset_policy(&deprecated, 300)
        ],
    );
    let hash = BytesN::from_array(&env, &[97; 32]);
    client.create_session(
        &buyer,
        &seller,
        &healthy,
        &100,
        &50,
        &hash,
        &unique_idempotency_key(&env),
    );
    client.create_session(
        &buyer,
        &seller,
        &surplus,
        &100,
        &50,
        &hash,
        &unique_idempotency_key(&env),
    );
    client.create_session(
        &buyer,
        &seller,
        &underfunded,
        &100,
        &50,
        &hash,
        &unique_idempotency_key(&env),
    );
    token::StellarAssetClient::new(&env, &surplus).mint(&contract_id, &40);
    env.as_contract(&contract_id, || {
        storage::increase_liability(&env, &underfunded, 25).unwrap();
    });
    client.deprecate_asset(&deprecated, &BytesN::from_array(&env, &[99; 32]));

    let query = soroban_sdk::vec![
        &env,
        underfunded.clone(),
        unknown.clone(),
        healthy.clone(),
        deprecated.clone(),
        surplus.clone(),
        healthy.clone()
    ];
    let results = client.get_solvency(&query);
    assert_eq!(results.len(), 6);
    let expected = [
        (&underfunded, SolvencyAssetState::Active, 100, 125, -25),
        (&unknown, SolvencyAssetState::Unknown, 0, 0, 0),
        (&healthy, SolvencyAssetState::Active, 100, 100, 0),
        (&deprecated, SolvencyAssetState::Deprecated, 0, 0, 0),
        (&surplus, SolvencyAssetState::Active, 140, 100, 40),
        (&healthy, SolvencyAssetState::Active, 100, 100, 0),
    ];
    for (index, (asset, state, balance, liability, delta)) in expected.iter().enumerate() {
        let item = results.get(index as u32).unwrap();
        assert_eq!(&item.asset, *asset);
        assert_eq!(&item.state, state);
        assert_eq!(item.balance, *balance);
        assert_eq!(item.liability, *liability);
        assert_eq!(item.surplus, *delta);
    }

    assert_eq!(
        client.try_get_solvency(&soroban_sdk::Vec::new(&env)),
        Err(Ok(ContractError::InvalidBatchSize))
    );
    let mut oversized = soroban_sdk::Vec::new(&env);
    for _ in 0..=MAX_SOLVENCY_QUERY_ASSETS {
        oversized.push_back(healthy.clone());
    }
    assert_eq!(
        client.try_get_solvency(&oversized),
        Err(Ok(ContractError::InvalidBatchSize))
    );
    let mut at_limit = soroban_sdk::Vec::new(&env);
    for _ in 0..MAX_SOLVENCY_QUERY_ASSETS {
        at_limit.push_back(healthy.clone());
    }
    assert_eq!(
        client.get_solvency(&at_limit).len(),
        MAX_SOLVENCY_QUERY_ASSETS
    );
}

#[test]
fn per_asset_minimum_enforces_boundaries_and_policy_invariants() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_000);
    let other = create_test_asset(&env, &buyer, 1_000);

    let bad = |min: i128, max: i128| AssetPolicy {
        asset: asset.clone(),
        max_session_amount: max,
        min_session_amount: min,
        max_asset_exposure: i128::MAX,
        max_buyer_exposure: i128::MAX,
    };
    for policy in [bad(0, 100), bad(-1, 100), bad(101, 100)] {
        assert_eq!(
            client.try_initialize(&admin, &soroban_sdk::vec![&env, policy]),
            Err(Ok(ContractError::InvalidAssetCap))
        );
    }

    client.initialize(
        &admin,
        &soroban_sdk::vec![
            &env,
            AssetPolicy {
                asset: asset.clone(),
                max_session_amount: 100,
                min_session_amount: 10,
                max_asset_exposure: i128::MAX,
                max_buyer_exposure: i128::MAX,
            },
            AssetPolicy {
                asset: other.clone(),
                max_session_amount: 500,
                min_session_amount: 50,
                max_asset_exposure: i128::MAX,
                max_buyer_exposure: i128::MAX,
            }
        ],
    );
    assert_eq!(client.get_asset_min_session_amount(&asset), 10);
    assert_eq!(client.get_asset_min_session_amount(&other), 50);

    let hash = |n: u8| BytesN::from_array(&env, &[n; 32]);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &9,
            &50,
            &hash(90),
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::InvalidAmount))
    );
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &101,
            &50,
            &hash(91),
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::AmountExceedsCap))
    );
    // Same amount is valid for one asset and below the other asset's minimum.
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &other,
            &10,
            &50,
            &hash(92),
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::InvalidAmount))
    );
    let at_min = client.create_session(
        &buyer,
        &seller,
        &asset,
        &10,
        &50,
        &hash(93),
        &unique_idempotency_key(&env),
    );
    let at_max = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &hash(94),
        &unique_idempotency_key(&env),
    );

    // The cap cannot be moved below the minimum; a valid update leaves sessions intact.
    assert_eq!(
        client.try_set_asset_max_session_amount(&asset, &9),
        Err(Ok(ContractError::InvalidAssetCap))
    );
    client.set_asset_max_session_amount(&asset, &10);
    assert_eq!(client.get_session(&at_min).max_amount, 10);
    assert_eq!(client.get_session(&at_max).max_amount, 100);
    assert_eq!(client.get_asset_min_session_amount(&asset), 10);
}

#[test]
fn seller_exposure_ceiling_enforces_limits_and_releases_exposure() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 10_000);

    let policies = soroban_sdk::vec![&env, asset_policy(&asset, 1_000)];
    client.initialize(&admin, &policies);

    // Set seller exposure ceiling for this asset to 1,500
    client.set_seller_exposure_ceiling(&asset, &1_500);
    assert_eq!(client.get_seller_exposure_ceiling(&asset), 1_500);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 0);

    let resource_hash = BytesN::from_array(&env, &[1; 32]);
    let usage_hash = BytesN::from_array(&env, &[2; 32]);

    // Create 1st session with max_amount 1,000 -> seller exposure = 1,000
    let session_1 = client.create_session(
        &buyer,
        &seller,
        &asset,
        &1_000,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_exposure(&seller, &asset), 1_000);

    // Attempting 2nd session with 600 would exceed ceiling 1,500 (1000 + 600 > 1500) -> fails before transfer
    let err = client.try_create_session(
        &buyer,
        &seller,
        &asset,
        &600,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::SellerExposureCeilingExceeded)));
    assert_eq!(client.get_seller_exposure(&seller, &asset), 1_000);

    // Session with 500 reaches exact ceiling (1000 + 500 = 1500)
    let session_2 = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_exposure(&seller, &asset), 1_500);

    // Settle session 1: releases 1,000 exposure exactly once
    client.settle(&session_1, &800, &usage_hash);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 500);

    // Cancel session 2: releases 500 exposure exactly once
    client.cancel(&session_2);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 0);

    // Recovery path: test expired session releases exposure
    let session_3 = client.create_session(
        &buyer,
        &seller,
        &asset,
        &700,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_exposure(&seller, &asset), 700);
    env.ledger().set_sequence_number(55);
    client.recover_expired(&session_3);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 0);
}

#[test]
fn buyer_open_session_limits_enforces_boundaries_and_maintains_isolation() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer_1 = Address::generate(&env);
    let buyer_2 = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer_1, 10_000);
    token::StellarAssetClient::new(&env, &asset).mint(&buyer_2, &10_000);

    let policies = soroban_sdk::vec![&env, asset_policy(&asset, 1_000)];
    client.initialize(&admin, &policies);

    // Limit buyer to at most 2 concurrent open sessions
    client.set_max_buyer_open_sessions(&2);
    assert_eq!(client.get_max_buyer_open_sessions(), Some(2));
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 0);
    assert_eq!(client.get_buyer_open_sessions(&buyer_2), 0);

    let resource_hash = BytesN::from_array(&env, &[1; 32]);
    let usage_hash = BytesN::from_array(&env, &[2; 32]);

    // Buyer 1 creates 1st session -> count = 1
    let b1_s1 = client.create_session(
        &buyer_1,
        &seller,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 1);
    assert_eq!(client.get_buyer_open_sessions(&buyer_2), 0);

    // Buyer 1 creates 2nd session -> count = 2 (exact limit succeeds)
    let b1_s2 = client.create_session(
        &buyer_1,
        &seller,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 2);

    // Limit plus one fails for Buyer 1
    let err = client.try_create_session(
        &buyer_1,
        &seller,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::BuyerOpenSessionsExceeded)));
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 2);

    // Buyer 2 remains independent and can create sessions up to their limit
    let b2_s1 = client.create_session(
        &buyer_2,
        &seller,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_buyer_open_sessions(&buyer_2), 1);
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 2);

    // Settlement releases slot for Buyer 1
    client.settle(&b1_s1, &50, &usage_hash);
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 1);

    // Now Buyer 1 can create another session
    let b1_s3 = client.create_session(
        &buyer_1,
        &seller,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 2);

    // Cancellation releases slot for Buyer 1
    client.cancel(&b1_s2);
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 1);

    // Expiry recovery releases slot for Buyer 1
    env.ledger().set_sequence_number(105);
    client.recover_expired(&b1_s3);
    assert_eq!(client.get_buyer_open_sessions(&buyer_1), 0);

    // Batch recovery releases slot for Buyer 2
    let results = client.recover_expired_batch(&soroban_sdk::vec![&env, b2_s1]);
    assert_eq!(results.len(), 1);
    assert_eq!(client.get_buyer_open_sessions(&buyer_2), 0);
}

#[test]
fn seller_open_session_limits_enforces_boundaries_and_maintains_isolation() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller_1 = Address::generate(&env);
    let seller_2 = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 10_000);

    let policies = soroban_sdk::vec![&env, asset_policy(&asset, 1_000)];
    client.initialize(&admin, &policies);

    // Limit seller to at most 2 concurrent open sessions
    client.set_max_seller_open_sessions(&2);
    assert_eq!(client.get_max_seller_open_sessions(), Some(2));
    assert_eq!(client.get_seller_open_sessions(&seller_1), 0);
    assert_eq!(client.get_seller_open_sessions(&seller_2), 0);

    let resource_hash = BytesN::from_array(&env, &[1; 32]);
    let usage_hash = BytesN::from_array(&env, &[2; 32]);

    // Seller 1 receives 1st session -> count = 1
    let s1_sess1 = client.create_session(
        &buyer,
        &seller_1,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_open_sessions(&seller_1), 1);
    assert_eq!(client.get_seller_open_sessions(&seller_2), 0);

    // Seller 1 receives 2nd session -> count = 2 (exact limit succeeds)
    let s1_sess2 = client.create_session(
        &buyer,
        &seller_1,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_open_sessions(&seller_1), 2);

    // Limit plus one fails for Seller 1
    let err = client.try_create_session(
        &buyer,
        &seller_1,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(err, Err(Ok(ContractError::SellerOpenSessionsExceeded)));
    assert_eq!(client.get_seller_open_sessions(&seller_1), 2);

    // Seller 2 remains independent and can receive sessions up to their limit
    let s2_sess1 = client.create_session(
        &buyer,
        &seller_2,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_open_sessions(&seller_2), 1);
    assert_eq!(client.get_seller_open_sessions(&seller_1), 2);

    // Settlement releases slot for Seller 1
    client.settle(&s1_sess1, &50, &usage_hash);
    assert_eq!(client.get_seller_open_sessions(&seller_1), 1);

    // Now Seller 1 can receive another session
    let s1_sess3 = client.create_session(
        &buyer,
        &seller_1,
        &asset,
        &100,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_seller_open_sessions(&seller_1), 2);

    // Cancellation releases slot for Seller 1
    client.cancel(&s1_sess2);
    assert_eq!(client.get_seller_open_sessions(&seller_1), 1);

    // Expiry recovery releases slot for Seller 1
    env.ledger().set_sequence_number(105);
    client.recover_expired(&s1_sess3);
    assert_eq!(client.get_seller_open_sessions(&seller_1), 0);

    // Batch recovery releases slot for Seller 2
    let results = client.recover_expired_batch(&soroban_sdk::vec![&env, s2_sess1]);
    assert_eq!(results.len(), 1);
    assert_eq!(client.get_seller_open_sessions(&seller_2), 0);
}

#[test]
fn reduce_session_cap_lowers_cap_refunds_buyer_and_reduces_liability() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 10_000);
    let token = token::Client::new(&env, &asset);

    let policies = soroban_sdk::vec![&env, asset_policy(&asset, 1_000)];
    client.initialize(&admin, &policies);
    client.set_seller_exposure_ceiling(&asset, &2_000);

    let resource_hash = BytesN::from_array(&env, &[1; 32]);
    let usage_hash = BytesN::from_array(&env, &[2; 32]);

    // Create session for 1,000
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &1_000,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(token.balance(&buyer), 9_000);
    assert_eq!(client.get_escrow_liability(&asset), 1_000);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 1_000);

    // Reducing cap to >= current cap or <= 0 fails
    let err_increase = client.try_reduce_session_cap(&session_id, &1_200);
    assert_eq!(err_increase, Err(Ok(ContractError::InvalidCapReduction)));

    let err_same = client.try_reduce_session_cap(&session_id, &1_000);
    assert_eq!(err_same, Err(Ok(ContractError::InvalidCapReduction)));

    let err_zero = client.try_reduce_session_cap(&session_id, &0);
    assert_eq!(err_zero, Err(Ok(ContractError::InvalidCapReduction)));

    // Valid reduction from 1,000 to 600
    // Refunds 400 to buyer, reduces liability, seller exposure, and buyer exposure to 600
    client.reduce_session_cap(&session_id, &600);
    assert_eq!(token.balance(&buyer), 9_400);
    assert_eq!(client.get_escrow_liability(&asset), 600);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 600);
    assert_eq!(client.get_buyer_exposure(&buyer, &asset), 600);

    let session = client.get_session(&session_id);
    assert_eq!(session.max_amount, 600);
    assert_eq!(session.escrowed_amount, 600);
    assert_eq!(session.seller, seller);
    assert_eq!(session.resource_hash, resource_hash);
    assert_eq!(session.expires_at_ledger, 100);

    // Repeated reduction: reduce from 600 to 400
    client.reduce_session_cap(&session_id, &400);
    assert_eq!(token.balance(&buyer), 9_600);
    assert_eq!(client.get_escrow_liability(&asset), 400);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 400);
    assert_eq!(client.get_buyer_exposure(&buyer, &asset), 400);

    // Settle with actual amount 350 <= 400
    client.settle(&session_id, &350, &usage_hash);
    assert_eq!(token.balance(&seller), 350);
    assert_eq!(token.balance(&buyer), 9_650); // 9,600 + 50 refund
    assert_eq!(client.get_escrow_liability(&asset), 0);
    assert_eq!(client.get_seller_exposure(&seller, &asset), 0);
    assert_eq!(client.get_buyer_exposure(&buyer, &asset), 0);

    // Terminal session (Settled) reduction must fail
    let err_settled = client.try_reduce_session_cap(&session_id, &200);
    assert_eq!(err_settled, Err(Ok(ContractError::SessionAlreadySettled)));

    // Expired session reduction must fail
    let s2_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    env.ledger().set_sequence_number(55);
    let err_expired = client.try_reduce_session_cap(&s2_id, &300);
    assert_eq!(err_expired, Err(Ok(ContractError::ExpiredSession)));
}

#[test]
fn creation_pause_requires_admin_and_fails_closed_before_transfer() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let asset = create_test_asset(&env, &admin, 1);
    let asset_policies = soroban_sdk::vec![&env, asset_policy(&asset, 500)];

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

    // An unauthorized administrator cannot pause creation.
    assert!(client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "pause_creation",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_pause_creation()
        .is_err());
    env.as_contract(&contract_id, || {
        assert_eq!(
            storage::read_operational_state(&env),
            OperationalState::Active
        );
    });

    // The stored administrator pauses creation and emits one stable event.
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "pause_creation",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .pause_creation();
    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::CreationPaused {
            admin: admin.clone(),
            event_version: 3,
        }
        .to_xdr(&env, &contract_id)]
    );
    assert_eq!(
        client.get_configuration().operational_state,
        OperationalState::Paused
    );

    // Creation fails before buyer authorization or any token transfer.
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[71; 32]);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::CreationPaused))
    );
    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&env), 0);
        assert_eq!(storage::read_liability(&env, &asset), 0);
    });
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 0);
}

#[test]
fn creation_pause_rejects_repeated_calls_and_resume_restores_creation() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_000);
    let resource_hash = BytesN::from_array(&env, &[72; 32]);
    initialize_for_asset(&env, &client, &admin, &asset);

    // Resuming while already active is rejected rather than silently accepted.
    assert_eq!(
        client.try_resume_creation(),
        Err(Ok(ContractError::OperationalStateUnchanged))
    );

    client.pause_creation();
    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::CreationPaused {
            admin: admin.clone(),
            event_version: 3,
        }
        .to_xdr(&env, &contract_id)]
    );
    assert_eq!(
        client.get_configuration().operational_state,
        OperationalState::Paused
    );

    // A repeated pause is rejected and does not clear the pause.
    assert_eq!(
        client.try_pause_creation(),
        Err(Ok(ContractError::OperationalStateUnchanged))
    );
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &50,
            &resource_hash,
            &unique_idempotency_key(&env)
        ),
        Err(Ok(ContractError::CreationPaused))
    );

    client.resume_creation();
    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![events::CreationResumed {
            admin: admin.clone(),
            event_version: 3,
        }
        .to_xdr(&env, &contract_id)]
    );
    assert_eq!(
        client.get_configuration().operational_state,
        OperationalState::Active
    );
    assert_eq!(
        client.try_resume_creation(),
        Err(Ok(ContractError::OperationalStateUnchanged))
    );

    // Creation is available again after an authorized resume.
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);
}

#[test]
fn paused_creation_preserves_all_exit_paths() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 3_000);
    let resource_hash = BytesN::from_array(&env, &[73; 32]);
    let usage_hash = BytesN::from_array(&env, &[74; 32]);
    initialize_for_asset(&env, &client, &admin, &asset);

    let to_settle = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let to_cancel = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let to_extend = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );
    let to_recover = client.create_session(
        &buyer,
        &seller,
        &asset,
        &500,
        &100,
        &resource_hash,
        &unique_idempotency_key(&env),
    );

    client.pause_creation();
    assert_eq!(
        client.get_configuration().operational_state,
        OperationalState::Paused
    );

    // Settlement, cancellation, TTL extension, and expiry recovery stay available.
    client.settle(&to_settle, &300, &usage_hash);
    assert_eq!(
        client.get_session(&to_settle).status,
        SessionStatus::Settled
    );

    client.cancel(&to_cancel);
    assert_eq!(
        client.get_session(&to_cancel).status,
        SessionStatus::Cancelled
    );

    client.extend_ttl(&to_extend);

    env.ledger().set_sequence_number(100);
    client.recover_expired(&to_recover);
    client.recover_expired(&to_extend);
    assert_eq!(
        client.get_session(&to_recover).status,
        SessionStatus::Expired
    );

    // Read-only queries remain available even while paused.
    assert_eq!(client.get_escrow_liability(&asset), 0);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn creation_pause_and_resume_require_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_pause_creation(),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_resume_creation(),
        Err(Ok(ContractError::NotInitialized))
    );
}

fn last_contract_event(env: &Env, contract_id: &Address) -> soroban_sdk::xdr::ContractEvent {
    env.events()
        .all()
        .filter_by_contract(contract_id)
        .events()
        .last()
        .expect("an event should be emitted")
        .clone()
}

// Asserts the raw XDR layout an indexer consumes: a stable prefix topic, one
// bounded indexed topic, and an `event_version` entry in the data map. Returns the
// indexed topic so callers can prove the session id is reused across events.
fn assert_indexable_event(
    env: &Env,
    contract_id: &Address,
    expected_prefix: &str,
    expect_session_topic: bool,
    expected_version: u32,
) -> soroban_sdk::xdr::ScVal {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};

    let event = last_contract_event(env, contract_id);
    let ContractEventBody::V0(body) = &event.body;

    assert_eq!(
        body.topics.len(),
        2,
        "each event exposes one prefix topic and one indexed topic"
    );
    assert!(
        body.topics.len() <= 4,
        "event topics must stay within the Soroban limit of four"
    );

    match body.topics.first() {
        Some(ScVal::Symbol(symbol)) => assert_eq!(
            std::str::from_utf8(&symbol.0[..]),
            Ok(expected_prefix),
            "event prefix topic changed"
        ),
        other => panic!("event prefix topic must be a symbol, found: {:?}", other),
    }

    let topic = body
        .topics
        .get(1)
        .expect("event must carry an indexed topic")
        .clone();
    match &topic {
        ScVal::Bytes(bytes) => {
            assert!(
                expect_session_topic,
                "only session events carry a session id topic"
            );
            assert_eq!(
                bytes.len(),
                32,
                "session id topics must be exactly 32 bytes"
            );
        }
        ScVal::Address(_) => {
            assert!(
                !expect_session_topic,
                "admin and asset events carry an address topic"
            );
        }
        other => panic!("unexpected topic value: {:?}", other),
    }

    let data = match &body.data {
        ScVal::Map(Some(map)) => map,
        other => panic!("event data must be a map, found: {:?}", other),
    };
    let version = data.iter().find_map(|entry| {
        let symbol = match &entry.key {
            ScVal::Symbol(symbol) => symbol,
            _ => return None,
        };
        let value = match &entry.val {
            ScVal::U32(value) => value,
            _ => return None,
        };
        if std::str::from_utf8(&symbol.0[..]) == Ok("event_version") {
            Some(*value)
        } else {
            None
        }
    });
    assert_eq!(
        version,
        Some(expected_version),
        "event data must expose event_version for indexers"
    );

    topic
}

#[test]
fn lifecycle_events_expose_indexable_topics_and_versions() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let session_id = BytesN::from_array(&env, &[7; 32]);
    let resource_hash = BytesN::from_array(&env, &[8; 32]);
    let usage_hash = BytesN::from_array(&env, &[9; 32]);
    let session = Session {
        id: session_id.clone(),
        buyer: buyer.clone(),
        seller: seller.clone(),
        asset: asset.clone(),
        max_amount: 100,
        escrowed_amount: 100,
        settled_amount: 0,
        expires_at_ledger: 42,
        resource_hash,
        usage_hash: None,
        status: SessionStatus::Open,
        creation_ledger: 1,
        finalized_ledger: None,
        resource_scheme_version: 1,
        usage_scheme_version: None,
    };

    // Session lifecycle events share one event version and expose the session id as
    // their indexed topic, so an indexer can group every state change by session.
    env.as_contract(&contract_id, || {
        events::publish_session_created(&env, &session);
    });
    let created_topic = assert_indexable_event(&env, &contract_id, "session_created", true, 2);

    env.as_contract(&contract_id, || {
        events::publish_session_settled(&env, &session_id, &seller, &asset, 90, 10, &usage_hash);
    });
    let settled_topic = assert_indexable_event(&env, &contract_id, "session_settled", true, 2);
    assert_eq!(created_topic, settled_topic);

    env.as_contract(&contract_id, || {
        events::publish_session_cancelled(&env, &session_id, &buyer, &asset, 100);
    });
    let cancelled_topic = assert_indexable_event(&env, &contract_id, "session_cancelled", true, 2);
    assert_eq!(created_topic, cancelled_topic);

    env.as_contract(&contract_id, || {
        events::publish_session_recovered(&env, &session, 100);
    });
    let recovered_topic = assert_indexable_event(&env, &contract_id, "session_recovered", true, 2);
    assert_eq!(created_topic, recovered_topic);

    // Administrative and asset events keep one address topic and use version 3.
    env.as_contract(&contract_id, || {
        events::publish_asset_activated(&env, &asset, 7);
    });
    assert_indexable_event(&env, &contract_id, "asset_activated", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_asset_session_cap_updated(&env, &asset, &admin, 100, 50);
    });
    assert_indexable_event(&env, &contract_id, "asset_session_cap_updated", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_asset_deprecated(
            &env,
            &asset,
            &admin,
            &BytesN::from_array(&env, &[41; 32]),
        );
    });
    assert_indexable_event(&env, &contract_id, "asset_deprecated", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_asset_removed(&env, &asset, &admin, &BytesN::from_array(&env, &[42; 32]));
    });
    assert_indexable_event(&env, &contract_id, "asset_removed", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_admin_rotation_proposed(&env, &admin, &buyer, 100);
    });
    assert_indexable_event(&env, &contract_id, "admin_rotation_proposed", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_admin_rotation_accepted(&env, &admin, &buyer);
    });
    assert_indexable_event(&env, &contract_id, "admin_rotation_accepted", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_admin_rotation_cancelled(&env, &admin, &buyer);
    });
    assert_indexable_event(&env, &contract_id, "admin_rotation_cancelled", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_cap_increase_proposed(&env, &asset, &admin, 200, 10, 20);
    });
    assert_indexable_event(&env, &contract_id, "cap_increase_proposed", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_cap_increase_cancelled(&env, &asset, &admin, 200);
    });
    assert_indexable_event(&env, &contract_id, "cap_increase_cancelled", false, 3);

    env.as_contract(&contract_id, || {
        events::publish_seller_exposure_ceiling_updated(&env, &asset, &admin, 100, 200);
    });
    assert_indexable_event(
        &env,
        &contract_id,
        "seller_exposure_ceiling_updated",
        false,
        1,
    );

    env.as_contract(&contract_id, || {
        events::publish_max_buyer_open_sessions_updated(&env, &admin, 10, 20);
    });
    assert_indexable_event(
        &env,
        &contract_id,
        "max_buyer_open_sessions_updated",
        false,
        1,
    );

    env.as_contract(&contract_id, || {
        events::publish_max_seller_open_sessions_updated(&env, &admin, 10, 20);
    });
    assert_indexable_event(
        &env,
        &contract_id,
        "max_seller_open_sessions_updated",
        false,
        1,
    );

    env.as_contract(&contract_id, || {
        events::publish_session_cap_reduced(&env, &session_id, &buyer, &asset, 100, 80, 20);
    });
    assert_indexable_event(&env, &contract_id, "session_cap_reduced", true, 1);

    env.as_contract(&contract_id, || {
        events::publish_creation_paused(&env, &admin)
    });
    assert_indexable_event(&env, &contract_id, "creation_paused", false, 3);
    env.as_contract(&contract_id, || {
        events::publish_creation_resumed(&env, &admin)
    });
    assert_indexable_event(&env, &contract_id, "creation_resumed", false, 3);
}

fn record_contract_event_names(
    env: &Env,
    contract_id: &Address,
    audit_log: &mut std::vec::Vec<std::string::String>,
) {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};

    for event in env.events().all().filter_by_contract(contract_id).events() {
        let ContractEventBody::V0(body) = &event.body;
        if let Some(ScVal::Symbol(symbol)) = body.topics.first() {
            if let Ok(name) = std::str::from_utf8(&symbol.0[..]) {
                audit_log.push(std::string::String::from(name));
            }
        }
    }
}

#[test]
fn same_ledger_asset_cap_reduction_and_session_creation_both_orderings() {
    let resource_hash = |env: &Env, tag: u8| BytesN::from_array(env, &[tag; 32]);

    // Order A: admin lowers asset session cap before buyer create_session in the same ledger.
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 10_000,
                    max_buyer_exposure: 10_000,
                }
            ],
        );

        let mut audit_log = std::vec::Vec::new();
        client.set_asset_max_session_amount(&asset, &400);
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        // Creation in the same ledger sees the reduced cap (400).
        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &600,
                &150,
                &resource_hash(&env, 11),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::AmountExceedsCap))
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_escrow_liability(&asset), 0);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 0);

        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &400,
            &150,
            &resource_hash(&env, 12),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        let final_cap = client.get_asset_max_session_amount(&asset);
        assert_eq!(final_cap, 400);
        assert_eq!(client.get_session(&s1).max_amount, 400);
        assert_eq!(client.get_escrow_liability(&asset), 400);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 400);
        assert!(client.get_escrow_liability(&asset) <= final_cap);
        assert_eq!(
            audit_log,
            std::vec!["asset_session_cap_updated", "session_created"]
        );
    }

    // Order B: buyer create_session lands before admin lowers asset session cap in the same ledger.
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 10_000,
                    max_buyer_exposure: 10_000,
                }
            ],
        );

        let mut audit_log = std::vec::Vec::new();
        // Pre-reduction creation sees cap = 1,000.
        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &600,
            &150,
            &resource_hash(&env, 13),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        // Same-ledger admin cap reduction lowers cap to 400 without rewriting s1.
        client.set_asset_max_session_amount(&asset, &400);
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        assert_eq!(client.get_session(&s1).max_amount, 600);
        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &600,
                &150,
                &resource_hash(&env, 14),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::AmountExceedsCap))
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        // Subsequent creation in the same ledger uses the updated 400 cap.
        let s2 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &400,
            &150,
            &resource_hash(&env, 15),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        assert_eq!(client.get_session(&s2).max_amount, 400);
        // Total exposure equals sum of authorizations granted in transaction order (600 + 400).
        assert_eq!(client.get_escrow_liability(&asset), 1_000);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 1_000);
        assert_eq!(
            audit_log,
            std::vec![
                "session_created",
                "asset_session_cap_updated",
                "session_created"
            ]
        );
    }
}

#[test]
fn same_ledger_buyer_exposure_ceiling_reduction_and_session_creation_both_orderings() {
    let resource_hash = |env: &Env, tag: u8| BytesN::from_array(env, &[tag; 32]);

    // Order A: admin reduces buyer exposure ceiling before creation in the same ledger.
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 10_000,
                    max_buyer_exposure: 1_000,
                }
            ],
        );

        let mut audit_log = std::vec::Vec::new();
        client.set_buyer_exposure_ceiling(&asset, &400);
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &500,
                &150,
                &resource_hash(&env, 21),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::BuyerExposureCeilingExceeded))
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &400,
            &150,
            &resource_hash(&env, 22),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &1,
                &150,
                &resource_hash(&env, 23),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::BuyerExposureCeilingExceeded))
        );
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 400);
        assert!(
            client.get_buyer_exposure(&buyer, &asset) <= client.get_buyer_exposure_ceiling(&asset)
        );
        assert_eq!(client.get_session(&s1).status, SessionStatus::Open);
        assert_eq!(
            audit_log,
            std::vec!["buyer_exposure_ceiling_updated", "session_created"]
        );
    }

    // Order B: create_session(500) before ceiling reduction to 400 in the same ledger.
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 10_000,
                    max_buyer_exposure: 1_000,
                }
            ],
        );

        let mut audit_log = std::vec::Vec::new();
        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &500,
            &150,
            &resource_hash(&env, 24),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        client.set_buyer_exposure_ceiling(&asset, &400);
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        // No further session can exceed the lowered 400 ceiling while exposure is 500.
        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &1,
                &150,
                &resource_hash(&env, 25),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::BuyerExposureCeilingExceeded))
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 500);

        // Existing session exits normally and releases exposure so 400 fits under the new ceiling.
        client.cancel(&s1);
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 0);

        client.create_session(
            &buyer,
            &seller,
            &asset,
            &400,
            &150,
            &resource_hash(&env, 26),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 400);
        assert_eq!(
            audit_log,
            std::vec![
                "session_created",
                "buyer_exposure_ceiling_updated",
                "session_cancelled",
                "session_created"
            ]
        );
    }
}

#[test]
fn same_ledger_buyer_session_cap_reduction_and_new_session_creation_both_orderings() {
    let resource_hash = |env: &Env, tag: u8| BytesN::from_array(env, &[tag; 32]);
    let usage_hash = |env: &Env, tag: u8| BytesN::from_array(env, &[tag; 32]);

    // Order A: reduce_session_cap(s1: 600 -> 300) frees headroom before create_session(500) in same ledger.
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        let token = token::Client::new(&env, &asset);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 800,
                    max_buyer_exposure: 800,
                }
            ],
        );
        client.set_seller_exposure_ceiling(&asset, &800);

        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &600,
            &150,
            &resource_hash(&env, 31),
            &unique_idempotency_key(&env),
        );
        let mut audit_log = std::vec::Vec::new();

        client.reduce_session_cap(&s1, &300);
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(token.balance(&buyer), 1_700);
        assert_eq!(client.get_escrow_liability(&asset), 300);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 300);
        assert_eq!(client.get_seller_exposure(&seller, &asset), 300);

        let s2 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &500,
            &150,
            &resource_hash(&env, 32),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        // Exact 800 ceiling is reached; 1 more unit must fail.
        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &1,
                &150,
                &resource_hash(&env, 33),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::AssetExposureCeilingExceeded))
        );
        assert_eq!(token.balance(&buyer), 1_200);
        assert_eq!(client.get_escrow_liability(&asset), 800);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 800);
        assert_eq!(client.get_seller_exposure(&seller, &asset), 800);
        assert_eq!(
            audit_log,
            std::vec!["session_cap_reduced", "session_created"]
        );

        // Finalizing both sessions cleanly zeroes all exposure counters without underflow.
        client.settle(&s1, &250, &usage_hash(&env, 91));
        client.cancel(&s2);
        assert_eq!(client.get_escrow_liability(&asset), 0);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 0);
        assert_eq!(client.get_seller_exposure(&seller, &asset), 0);
        assert_eq!(token.balance(&buyer), 1_750);
        assert_eq!(token.balance(&seller), 250);
    }

    // Order B1: create_session(500) before reduce_session_cap(600 -> 300) hits buyer ceiling first.
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 2_000,
                    max_buyer_exposure: 800,
                }
            ],
        );

        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &600,
            &150,
            &resource_hash(&env, 34),
            &unique_idempotency_key(&env),
        );
        let mut audit_log = std::vec::Vec::new();

        assert_eq!(
            client.try_create_session(
                &buyer,
                &seller,
                &asset,
                &500,
                &150,
                &resource_hash(&env, 35),
                &unique_idempotency_key(&env),
            ),
            Err(Ok(ContractError::BuyerExposureCeilingExceeded))
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);

        client.reduce_session_cap(&s1, &300);
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 300);
        assert_eq!(client.get_escrow_liability(&asset), 300);
        assert_eq!(audit_log, std::vec!["session_cap_reduced"]);
    }

    // Order B2: create_session(200) within prior headroom before reduce_session_cap(600 -> 300).
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(100);
        let contract_id = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, 2_000);
        client.initialize(
            &admin,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset: asset.clone(),
                    max_session_amount: 1_000,
                    min_session_amount: 1,
                    max_asset_exposure: 800,
                    max_buyer_exposure: 800,
                }
            ],
        );

        let s1 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &600,
            &150,
            &resource_hash(&env, 36),
            &unique_idempotency_key(&env),
        );
        let mut audit_log = std::vec::Vec::new();

        let s2 = client.create_session(
            &buyer,
            &seller,
            &asset,
            &200,
            &150,
            &resource_hash(&env, 37),
            &unique_idempotency_key(&env),
        );
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 800);

        client.reduce_session_cap(&s1, &300);
        record_contract_event_names(&env, &contract_id, &mut audit_log);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 500);
        assert_eq!(client.get_escrow_liability(&asset), 500);
        assert_eq!(
            audit_log,
            std::vec!["session_created", "session_cap_reduced"]
        );

        client.cancel(&s1);
        client.cancel(&s2);
        assert_eq!(client.get_buyer_exposure(&buyer, &asset), 0);
        assert_eq!(client.get_escrow_liability(&asset), 0);
    }
}

#[test]
fn ledger_rollover_boundary_returns_typed_errors_without_trapping() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(1_000);
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let pending_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = create_test_asset(&env, &buyer, 1_000);
    client.initialize(&admin, &soroban_sdk::vec![&env, asset_policy(&asset, 500)]);

    let hash = BytesN::from_array(&env, &[0x2A; 32]);
    let session_id = client.create_session(
        &buyer,
        &seller,
        &asset,
        &200,
        &2_000,
        &hash,
        &unique_idempotency_key(&env),
    );
    let mut stored = client.get_session(&session_id);
    stored.expires_at_ledger = u32::MAX;
    env.as_contract(&contract_id, || {
        storage::write_session(&env, &stored).unwrap();
    });

    let set_rollover_ledger = |seq: u32| {
        env.ledger().with_mut(|li| {
            li.min_persistent_entry_ttl = 1;
            li.min_temp_entry_ttl = 1;
            li.max_entry_ttl = 1;
            li.sequence_number = seq;
        });
    };

    // At u32::MAX - MAX_SESSION_DURATION_LEDGERS - 1, expires_at = u32::MAX exceeds max duration.
    set_rollover_ledger(u32::MAX - validation::MAX_SESSION_DURATION_LEDGERS - 1);
    let events_before = env
        .events()
        .all()
        .filter_by_contract(&contract_id)
        .events()
        .len();
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &u32::MAX,
            &hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::SessionDurationTooLong))
    );
    assert_eq!(
        client.try_extend_session_expiry(&session_id, &u32::MAX),
        Err(Ok(ContractError::ExpiredSession))
    );

    // At u32::MAX - MAX_CAP_INCREASE_DELAY_LEDGERS + 1, max window additions overflow u32::MAX.
    set_rollover_ledger(u32::MAX - validation::MAX_CAP_INCREASE_DELAY_LEDGERS + 1);
    assert_eq!(
        client.try_propose_admin(&pending_admin, &u32::MAX),
        Err(Ok(ContractError::InvalidAdminRotationExpiry))
    );
    assert_eq!(
        client.try_propose_asset_cap_increase(&asset, &1_000, &(u32::MAX - 1), &u32::MAX),
        Err(Ok(ContractError::InvalidCapIncreaseDelay))
    );

    // At u32::MAX - MIN_CAP_INCREASE_DELAY_LEDGERS + 1, minimum cooldown addition overflows u32::MAX.
    set_rollover_ledger(u32::MAX - validation::MIN_CAP_INCREASE_DELAY_LEDGERS + 1);
    assert_eq!(
        client.try_propose_asset_cap_increase(&asset, &1_000, &1, &100),
        Err(Ok(ContractError::InvalidCapIncreaseDelay))
    );

    // At u32::MAX - 1, recovery is not yet allowed because session expires at u32::MAX.
    set_rollover_ledger(u32::MAX - 1);
    assert_eq!(
        client.try_recover_expired(&session_id),
        Err(Ok(ContractError::SessionNotExpired))
    );
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .len(),
        events_before
    );

    // At u32::MAX, wrapped ledgers (0, 1) and overflowed timelocks fail with typed errors.
    set_rollover_ledger(u32::MAX);
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &0,
            &hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &u32::MAX,
            &hash,
            &unique_idempotency_key(&env),
        ),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_extend_session_expiry(&session_id, &1),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_propose_admin(&pending_admin, &0),
        Err(Ok(ContractError::InvalidAdminRotationExpiry))
    );
    assert_eq!(
        client.try_propose_admin(&pending_admin, &u32::MAX),
        Err(Ok(ContractError::InvalidAdminRotationExpiry))
    );
    assert_eq!(
        client.try_propose_asset_cap_increase(&asset, &1_000, &u32::MAX, &u32::MAX),
        Err(Ok(ContractError::InvalidCapIncreaseDelay))
    );
    assert_eq!(
        client.try_propose_asset_cap_increase(&asset, &1_000, &1, &100),
        Err(Ok(ContractError::InvalidCapIncreaseDelay))
    );
    assert_eq!(
        client.try_settle(&session_id, &100, &hash),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_cancel(&session_id),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_reject(&session_id),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_reduce_session_cap(&session_id, &100),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        env.events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .len(),
        events_before
    );
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);
    assert_eq!(client.get_escrow_liability(&asset), 200);
}
