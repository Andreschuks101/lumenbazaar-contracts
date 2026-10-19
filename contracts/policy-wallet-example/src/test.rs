extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    vec, Address, BytesN, Env,
};
use upto_session::{AssetPolicy, ContractError, UptoSessionContract, UptoSessionContractClient};

fn initialized_wallet(
    env: &Env,
) -> (
    PolicyWalletExampleContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
    BytesN<32>,
) {
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(env, &contract_id);
    let owner = Address::generate(env);
    let agent = Address::generate(env);
    let seller = Address::generate(env);
    let asset = Address::generate(env);
    let resource_hash = BytesN::from_array(env, &[12; 32]);

    client.initialize(&owner, &Address::generate(env));

    (client, agent, seller, asset, owner, resource_hash)
}

fn set_standard_policy(
    env: &Env,
    client: &PolicyWalletExampleContractClient<'_>,
    agent: &Address,
    seller: &Address,
    asset: &Address,
    resource_hash: &BytesN<32>,
) {
    client.set_policy(&standard_policy(
        env,
        agent,
        seller,
        asset,
        resource_hash,
        100,
    ));
}

fn standard_policy(
    env: &Env,
    agent: &Address,
    seller: &Address,
    asset: &Address,
    resource_hash: &BytesN<32>,
    valid_until_ledger: u32,
) -> SpendingPolicy {
    SpendingPolicy {
        agent: agent.clone(),
        max_amount_per_payment: 75,
        max_amount_per_day: 100,
        valid_until_ledger,
        allowed_sellers: vec![env, seller.clone()],
        allowed_assets: vec![env, asset.clone()],
        allowed_resource_hashes: vec![env, resource_hash.clone()],
    }
}

#[test]
fn policy_wallet_example_is_registered_independently() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);

    assert!(client.example_only());
}

#[test]
fn initializes_config_once_with_owner_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let upto_session_contract = Address::generate(&env);

    client.initialize(&owner, &upto_session_contract);

    let config = client.get_config();
    assert_eq!(config.owner, owner);
    assert_eq!(config.upto_session_contract, upto_session_contract);
    assert_eq!(config.initialized_at_ledger, env.ledger().sequence());
    assert!(config.example_only);
    assert_eq!(
        client.try_initialize(&owner, &upto_session_contract),
        Err(Ok(PolicyWalletError::AlreadyInitialized))
    );
}

#[test]
fn initialize_requires_owner_auth() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let upto_session_contract = Address::generate(&env);

    assert!(client
        .try_initialize(&owner, &upto_session_contract)
        .is_err());
    assert_eq!(
        client.try_get_config(),
        Err(Ok(PolicyWalletError::NotInitialized))
    );
}

#[test]
fn require_owner_uses_stored_owner_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let upto_session_contract = Address::generate(&env);

    client.initialize(&owner, &upto_session_contract);

    assert!(client.try_require_owner().is_ok());
}

#[test]
fn policy_wallet_storage_is_isolated_from_upto_session() {
    let env = Env::default();
    env.mock_all_auths();
    let wallet_id = env.register(PolicyWalletExampleContract, ());
    let wallet_client = PolicyWalletExampleContractClient::new(&env, &wallet_id);
    let upto_session_id = env.register(UptoSessionContract, ());
    let upto_client = UptoSessionContractClient::new(&env, &upto_session_id);
    let owner = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let missing_session_id = BytesN::from_array(&env, &[9; 32]);

    wallet_client.initialize(&owner, &upto_session_id);

    assert_eq!(
        upto_client.try_get_session(&missing_session_id),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert!(upto_client
        .try_initialize(
            &owner,
            &soroban_sdk::vec![
                &env,
                AssetPolicy {
                    asset,
                    max_session_amount: 1,
                }
            ]
        )
        .is_ok());
}

#[test]
fn owner_sets_spending_policy() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);

    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    let policy = client.get_policy(&agent);
    assert_eq!(policy.agent, agent);
    assert_eq!(policy.max_amount_per_payment, 75);
    assert_eq!(policy.max_amount_per_day, 100);
    assert_eq!(policy.valid_until_ledger, 100);
    assert_eq!(policy.allowed_sellers, vec![&env, seller]);
    assert_eq!(policy.allowed_assets, vec![&env, asset]);
    assert_eq!(policy.allowed_resource_hashes, vec![&env, resource_hash]);
}

#[test]
fn check_payment_accepts_policy_match_without_recording_spend() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert!(client
        .try_check_payment(&agent, &seller, &asset, &50, &resource_hash)
        .is_ok());
    assert_eq!(client.spent_today(&agent), 0);
}

#[test]
fn authorize_payment_records_daily_spend() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    client.authorize_payment(&agent, &seller, &asset, &60, &resource_hash);
    client.authorize_payment(&agent, &seller, &asset, &40, &resource_hash);

    assert_eq!(client.spent_today(&agent), 100);
}

#[test]
fn rejects_amounts_over_payment_or_daily_caps() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &76, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );

    client.authorize_payment(&agent, &seller, &asset, &75, &resource_hash);
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &26, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );
}

#[test]
fn rejects_disallowed_seller_asset_or_resource_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let wrong_seller = Address::generate(&env);
    let wrong_asset = Address::generate(&env);
    let wrong_resource_hash = BytesN::from_array(&env, &[13; 32]);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert_eq!(
        client.try_check_payment(&agent, &wrong_seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::SellerNotAllowed))
    );
    assert_eq!(
        client.try_check_payment(&agent, &seller, &wrong_asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::AssetNotAllowed))
    );
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &50, &wrong_resource_hash),
        Err(Ok(PolicyWalletError::ResourceHashNotAllowed))
    );
}

#[test]
fn rejects_invalid_or_expired_policy_windows() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);

    assert_eq!(
        client.try_set_policy(&SpendingPolicy {
            agent: agent.clone(),
            max_amount_per_payment: 0,
            max_amount_per_day: 100,
            valid_until_ledger: 100,
            allowed_sellers: vec![&env, seller.clone()],
            allowed_assets: vec![&env, asset.clone()],
            allowed_resource_hashes: vec![&env, resource_hash.clone()],
        }),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );
    assert_eq!(
        client.try_set_policy(&SpendingPolicy {
            agent,
            max_amount_per_payment: 10,
            max_amount_per_day: 100,
            valid_until_ledger: 0,
            allowed_sellers: vec![&env, seller],
            allowed_assets: vec![&env, asset],
            allowed_resource_hashes: vec![&env, resource_hash],
        }),
        Err(Ok(PolicyWalletError::PolicyExpired))
    );
}

#[test]
fn documentation_allowed_payment_example_authorizes() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert!(client
        .try_authorize_payment(&agent, &seller, &asset, &50, &resource_hash)
        .is_ok());
    assert_eq!(client.spent_today(&agent), 50);
}

#[test]
fn documentation_blocked_payment_example_reports_amount_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &76, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );
}

#[test]
fn documentation_expired_policy_example_reports_expired() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    client.set_policy(&standard_policy(
        &env,
        &agent,
        &seller,
        &asset,
        &resource_hash,
        10,
    ));

    env.ledger().set_sequence_number(11);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyExpired))
    );
}

#[test]
fn documentation_wrong_seller_example_reports_seller_error() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let wrong_seller = Address::generate(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert_eq!(
        client.try_check_payment(&agent, &wrong_seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::SellerNotAllowed))
    );
}

#[test]
fn documentation_wrong_asset_example_reports_asset_error() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let wrong_asset = Address::generate(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &wrong_asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::AssetNotAllowed))
    );
}

#[test]
fn owner_rotation_happy_path_and_privilege_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, owner, resource_hash) = initialized_wallet(&env);
    let new_owner = Address::generate(&env);

    assert_eq!(client.get_owner(), owner);
    assert_eq!(client.get_owner_rotation(), None);

    // Propose rotation
    let expiry = env.ledger().sequence() + 100;
    client.propose_owner(&new_owner, &expiry);

    let rotation = client.get_owner_rotation().expect("rotation should exist");
    assert_eq!(rotation.pending_owner, new_owner);
    assert_eq!(rotation.expires_at_ledger, expiry);

    // Accept rotation before expiry
    env.ledger().set_sequence_number(expiry - 1);
    client.accept_owner();

    assert_eq!(client.get_owner(), new_owner);
    assert_eq!(client.get_config().owner, new_owner);
    assert_eq!(client.get_owner_rotation(), None);

    // New owner can administer wallet
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);
    let policy = client.get_policy(&agent);
    assert_eq!(policy.agent, agent);
}

#[test]
fn owner_rotation_cannot_overwrite_live_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _agent, _seller, _asset, _owner, _resource_hash) = initialized_wallet(&env);
    let pending_1 = Address::generate(&env);
    let pending_2 = Address::generate(&env);

    client.propose_owner(&pending_1, &(env.ledger().sequence() + 50));

    assert_eq!(
        client.try_propose_owner(&pending_2, &(env.ledger().sequence() + 50)),
        Err(Ok(PolicyWalletError::OwnerRotationPending))
    );
}

#[test]
fn owner_rotation_rejects_self_targeting_and_invalid_expiry() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _agent, _seller, _asset, owner, _resource_hash) = initialized_wallet(&env);
    let other = Address::generate(&env);

    // Self-targeted proposal fails
    assert_eq!(
        client.try_propose_owner(&owner, &(env.ledger().sequence() + 10)),
        Err(Ok(PolicyWalletError::InvalidOwner))
    );

    // Expired or equal ledger proposal fails
    assert_eq!(
        client.try_propose_owner(&other, &env.ledger().sequence()),
        Err(Ok(PolicyWalletError::InvalidOwnerRotationExpiry))
    );
}

#[test]
fn owner_rotation_fails_on_expiry_and_replay() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _agent, _seller, _asset, owner, _resource_hash) = initialized_wallet(&env);
    let pending = Address::generate(&env);
    let expiry = env.ledger().sequence() + 20;

    client.propose_owner(&pending, &expiry);

    // At or after expiry, accept fails
    env.ledger().set_sequence_number(expiry);
    assert_eq!(
        client.try_accept_owner(),
        Err(Ok(PolicyWalletError::OwnerRotationExpired))
    );
    assert_eq!(client.get_owner(), owner);

    // Reset ledger to before expiry and accept
    env.ledger().set_sequence_number(expiry - 5);
    client.accept_owner();
    assert_eq!(client.get_owner(), pending);

    // Replaying accept fails with NotFound
    assert_eq!(
        client.try_accept_owner(),
        Err(Ok(PolicyWalletError::OwnerRotationNotFound))
    );
}

#[test]
fn owner_rotation_cancellation_flow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _agent, _seller, _asset, owner, _resource_hash) = initialized_wallet(&env);
    let pending = Address::generate(&env);

    // Cancel without proposal fails
    assert_eq!(
        client.try_cancel_owner_rotation(),
        Err(Ok(PolicyWalletError::OwnerRotationNotFound))
    );

    // Propose then cancel
    client.propose_owner(&pending, &(env.ledger().sequence() + 50));
    assert!(client.get_owner_rotation().is_some());

    client.cancel_owner_rotation();
    assert_eq!(client.get_owner_rotation(), None);
    assert_eq!(client.get_owner(), owner);

    // Pending owner cannot accept after cancellation
    assert_eq!(
        client.try_accept_owner(),
        Err(Ok(PolicyWalletError::OwnerRotationNotFound))
    );

    // Can propose again after cancellation
    let new_pending = Address::generate(&env);
    client.propose_owner(&new_pending, &(env.ledger().sequence() + 50));
    assert_eq!(
        client.get_owner_rotation().unwrap().pending_owner,
        new_pending
    );
}

#[test]
fn policies_and_spend_counters_survive_rotation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    // Spend 30
    client.authorize_payment(&agent, &seller, &asset, &30, &resource_hash);
    assert_eq!(client.spent_today(&agent), 30);

    // Rotate owner
    let new_owner = Address::generate(&env);
    client.propose_owner(&new_owner, &(env.ledger().sequence() + 100));
    client.accept_owner();
    assert_eq!(client.get_owner(), new_owner);
    assert_ne!(new_owner, owner);

    // Policy survives rotation
    let policy = client.get_policy(&agent);
    assert_eq!(policy.max_amount_per_payment, 75);
    assert_eq!(policy.max_amount_per_day, 100);

    // Spend counter survives rotation
    assert_eq!(client.spent_today(&agent), 30);

    // Agent can continue spending up to daily cap
    client.authorize_payment(&agent, &seller, &asset, &40, &resource_hash);
    assert_eq!(client.spent_today(&agent), 70);

    // Exceeding daily cap still rejected
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &31, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );
}

#[test]
fn owner_rotation_enforces_auth_boundaries() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let pending = Address::generate(&env);

    // Uninitialized calls fail
    assert_eq!(
        client.try_propose_owner(&pending, &100),
        Err(Ok(PolicyWalletError::NotInitialized))
    );
    assert_eq!(
        client.try_accept_owner(),
        Err(Ok(PolicyWalletError::NotInitialized))
    );
    assert_eq!(
        client.try_cancel_owner_rotation(),
        Err(Ok(PolicyWalletError::NotInitialized))
    );
    assert_eq!(
        client.try_get_owner(),
        Err(Ok(PolicyWalletError::NotInitialized))
    );

    // Initialize with auth
    env.mock_all_auths();
    client.initialize(&owner, &Address::generate(&env));
    assert_eq!(client.get_owner(), owner);
}
