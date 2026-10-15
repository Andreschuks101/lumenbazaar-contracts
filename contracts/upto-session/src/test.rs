extern crate std;

mod authorization_harness {
    include!("auth_test.rs");
}

mod property_harness {
    include!("property_test.rs");
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
    reentry::{ReentryAction, ReentryTokenContract, ReentryTokenContractClient},
    TestTokenContract, TestTokenContractClient,
};

fn create_test_asset(env: &Env, buyer: &Address, amount: i128) -> Address {
    let token_admin = Address::generate(env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    token::StellarAssetClient::new(env, &asset).mint(buyer, &amount);
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
    client.initialize(admin, &soroban_sdk::vec![env, asset.clone()]);
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

    let session_id = client.create_session(&buyer, &seller, &asset, &100, &50, &resource_hash);
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

    let expected_id = ids::derive_session_id(
        &env,
        &ids::SessionIdInput {
            buyer: &buyer,
            seller: &seller,
            asset: &asset,
            max_amount: 100,
            expires_at_ledger: 10,
            resource_hash: &resource_hash,
            sequence: 0,
        },
    );
    let session_id = client.create_session(&buyer, &seller, &asset, &100, &10, &resource_hash);

    assert_eq!(session_id, expected_id);
    assert_eq!(client.get_session(&session_id).id, session_id);

    // Test settle
    let usage_hash = BytesN::from_array(&env, &[25; 32]);
    client.settle(&session_id, &10, &usage_hash);
    assert_eq!(client.get_session(&session_id).settled_amount, 10);
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Settled
    );

    // Test extend_ttl on settled session - should fail
    assert!(client.try_extend_ttl(&session_id).is_err());

    // Test cancel and extend_ttl on separate open sessions
    let session_id_2 = client.create_session(&buyer, &seller, &asset, &100, &10, &resource_hash);
    client.cancel(&session_id_2);
    assert_eq!(
        client.try_extend_ttl(&session_id_2),
        Err(Ok(ContractError::TtlExtensionFailed))
    );

    let session_id_3 = client.create_session(&buyer, &seller, &asset, &100, &10, &resource_hash);
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

    client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
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

    let asset = Address::generate(&env);
    let supported_assets = soroban_sdk::vec![&env, asset.clone()];
    client.initialize(&admin, &supported_assets);

    env.as_contract(&contract_id, || {
        assert_eq!(storage::read_admin(&env), Some(admin.clone()));
        assert_eq!(
            storage::read_storage_layout_version(&env),
            Some(storage::STORAGE_LAYOUT_VERSION)
        );
    });

    assert_eq!(
        client.try_initialize(&admin, &supported_assets),
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
    let first_asset = Address::generate(&env);
    let second_asset = Address::generate(&env);
    let assets = soroban_sdk::vec![&env, first_asset.clone(), second_asset.clone()];

    assert_eq!(
        client.try_get_configuration(),
        Err(Ok(ContractError::NotInitialized))
    );
    client.initialize(&admin, &assets);

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
    let asset = Address::generate(&env);
    client.initialize(&admin, &soroban_sdk::vec![&env, asset]);

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
fn initialize_rejects_empty_or_duplicate_supported_assets() {
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
    let asset = Address::generate(&env);
    assert_eq!(
        duplicate_client.try_initialize(&admin, &soroban_sdk::vec![&env, asset.clone(), asset]),
        Err(Ok(ContractError::InvalidSupportedAssets))
    );

    let oversized_id = env.register(UptoSessionContract, ());
    let oversized_client = UptoSessionContractClient::new(&env, &oversized_id);
    let mut oversized = soroban_sdk::Vec::new(&env);
    for _ in 0..=MAX_SUPPORTED_ASSETS {
        oversized.push_back(Address::generate(&env));
    }
    assert_eq!(
        oversized_client.try_initialize(&admin, &oversized),
        Err(Ok(ContractError::InvalidSupportedAssets))
    );
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
            &resource_hash
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
            &resource_hash
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
            &resource_hash
        ),
        Err(Ok(ContractError::SessionDurationTooLong))
    );
}

#[test]
fn liability_arithmetic_is_checked() {
    let env = Env::default();
    let contract_id = env.register(UptoSessionContract, ());
    let asset = Address::generate(&env);

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

    let session_id =
        client.create_session(&buyer, &seller, &asset, &i128::MAX, &50, &resource_hash);
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
        client.try_create_session(&buyer, &seller, &asset, &0, &10, &resource_hash),
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
        client.try_create_session(&buyer, &seller, &asset, &100, &20, &resource_hash),
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &25, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &25, &resource_hash);

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
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[5; 32]);
    let supported_assets = soroban_sdk::vec![&env, asset.clone()];
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (admin.clone(), supported_assets.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin, &supported_assets);

    let result = client.try_create_session(&buyer, &seller, &asset, &500, &25, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &900, &30, &resource_hash);
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
            },
        );
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
            },
        );
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
            },
        );
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
            },
        );
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
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

    let first = client.create_session(&buyer, &seller, &asset, &400, &50, &resource_one);
    let second = client.create_session(&buyer, &seller, &asset, &600, &50, &resource_two);
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
    client.settle(&session_id, &125, &usage_hash);

    assert_eq!(token_client.balance(&buyer), 375);
    assert_eq!(token_client.balance(&seller), 125);
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Settled
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

    let zero_amount_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
    let over_cap_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
    let expired_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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

    let settles_before = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
    let cancels_before = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
    let recovers_at = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
    );
    let cancelled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &20,
        &BytesN::from_array(&env, &[71; 32]),
    );
    let settled = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &20,
        &BytesN::from_array(&env, &[72; 32]),
    );
    let not_expired = client.create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &20,
        &BytesN::from_array(&env, &[73; 32]),
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
    );
    let second = client.create_session(
        &second_buyer,
        &seller,
        &asset,
        &100,
        &10,
        &BytesN::from_array(&env, &[77; 32]),
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
    let settled = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
    let cancelled = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
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
    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
        .try_create_session(&buyer, &seller, &asset, &500, &50, &resource_hash)
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);
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
            },
        );
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
            },
        );
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
            },
        );
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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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

    let session_id = client.create_session(&buyer, &seller, &asset, &500, &50, &resource_hash);

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
            },
        );
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
            },
        );
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
