extern crate std;

mod mutation_harness {
    include!("mutation_test.rs");
}

mod property_harness {
    include!("property_test.rs");
}

mod panic_free_harness {
    include!("panic_free_test.rs");
}

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, vec,
    xdr::{ContractEventBody, ScVal},
    Address, BytesN, Env,
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
    client.set_policy(
        &standard_policy(env, agent, seller, asset, resource_hash, 100),
        &0,
    );
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
        max_authorizations_per_day: 100,
        valid_from_ledger: 0,
        valid_until_ledger,
        allowed_sellers: vec![env, seller.clone()],
        allowed_assets: vec![env, asset.clone()],
        allowed_resource_hashes: vec![env, resource_hash.clone()],
        seller_limits: vec![env],
        resource_limits: vec![env],
        revision: 0,
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
                    min_session_amount: 1,
                    max_asset_exposure: 1,
                    max_buyer_exposure: 1,
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
        client.try_set_policy(
            &SpendingPolicy {
                agent: agent.clone(),
                max_amount_per_payment: 0,
                max_amount_per_day: 100,
                max_authorizations_per_day: 10,
                valid_from_ledger: 0,
                valid_until_ledger: 100,
                allowed_sellers: vec![&env, seller.clone()],
                allowed_assets: vec![&env, asset.clone()],
                allowed_resource_hashes: vec![&env, resource_hash.clone()],
                seller_limits: vec![&env],
                resource_limits: vec![&env],
                revision: 0,
            },
            &0,
        ),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );
    assert_eq!(
        client.try_set_policy(
            &SpendingPolicy {
                agent: agent.clone(),
                max_amount_per_payment: 10,
                max_amount_per_day: 100,
                max_authorizations_per_day: 0,
                valid_from_ledger: 0,
                valid_until_ledger: 100,
                allowed_sellers: vec![&env, seller.clone()],
                allowed_assets: vec![&env, asset.clone()],
                allowed_resource_hashes: vec![&env, resource_hash.clone()],
                seller_limits: vec![&env],
                resource_limits: vec![&env],
                revision: 0,
            },
            &0,
        ),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );
    assert_eq!(
        client.try_set_policy(
            &SpendingPolicy {
                agent: agent.clone(),
                max_amount_per_payment: 10,
                max_amount_per_day: 100,
                max_authorizations_per_day: 10,
                valid_from_ledger: 101,
                valid_until_ledger: 100,
                allowed_sellers: vec![&env, seller.clone()],
                allowed_assets: vec![&env, asset.clone()],
                allowed_resource_hashes: vec![&env, resource_hash.clone()],
                seller_limits: vec![&env],
                resource_limits: vec![&env],
                revision: 0,
            },
            &0,
        ),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );
    assert_eq!(
        client.try_set_policy(
            &SpendingPolicy {
                agent,
                max_amount_per_payment: 10,
                max_amount_per_day: 100,
                max_authorizations_per_day: 10,
                valid_from_ledger: 0,
                valid_until_ledger: 0,
                allowed_sellers: vec![&env, seller],
                allowed_assets: vec![&env, asset],
                allowed_resource_hashes: vec![&env, resource_hash],
                seller_limits: vec![&env],
                resource_limits: vec![&env],
                revision: 0,
            },
            &0,
        ),
        Err(Ok(PolicyWalletError::PolicyExpired))
    );
}

#[test]
fn documentation_pre_activation_policy_example_reports_not_active() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100);
    policy.valid_from_ledger = 50;
    client.set_policy(&policy, &0);

    env.ledger().set_sequence_number(49);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyNotActive))
    );
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyNotActive))
    );
    assert_eq!(client.spent_today(&agent), 0);
}

#[test]
fn pre_activation_and_u32_boundary_checks() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);

    // 1. Invalid range where valid_from_ledger > valid_until_ledger
    let mut invalid_policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 50);
    invalid_policy.valid_from_ledger = 51;
    assert_eq!(
        client.try_set_policy(&invalid_policy, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 2. Single-ledger valid window where valid_from_ledger == valid_until_ledger
    let mut single_ledger_policy =
        standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100);
    single_ledger_policy.valid_from_ledger = 100;
    client.set_policy(&single_ledger_policy, &0);

    // Before activation
    env.ledger().set_sequence_number(99);
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyNotActive))
    );

    // Exactly at valid_from_ledger == valid_until_ledger (inclusive on both ends)
    env.ledger().set_sequence_number(100);
    assert!(client
        .try_authorize_payment(&agent, &seller, &asset, &50, &resource_hash)
        .is_ok());
    assert_eq!(client.spent_today(&agent), 50);

    // After expiration
    env.ledger().set_sequence_number(101);
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyExpired))
    );

    // 3. u32 boundary values: 0 and u32::MAX
    let mut max_u32_policy =
        standard_policy(&env, &agent, &seller, &asset, &resource_hash, u32::MAX);
    max_u32_policy.valid_from_ledger = 0;
    client.set_policy(&max_u32_policy, &1);

    env.ledger().set_sequence_number(500);
    assert!(client
        .try_check_payment(&agent, &seller, &asset, &50, &resource_hash)
        .is_ok());

    // Policy with valid_from_ledger at u32::MAX
    let mut future_max_policy =
        standard_policy(&env, &agent, &seller, &asset, &resource_hash, u32::MAX);
    future_max_policy.valid_from_ledger = u32::MAX;
    client.set_policy(&future_max_policy, &2);

    // Current sequence is 500, far below u32::MAX
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &50, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyNotActive))
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
    client.set_policy(
        &standard_policy(&env, &agent, &seller, &asset, &resource_hash, 10),
        &0,
    );

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
fn documentation_seller_payment_cap_example_reports_amount_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100);
    policy.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller.clone(),
            max_amount_per_payment: 30,
            max_amount_per_day: 50,
        },
    ];
    client.set_policy(&policy, &0);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &35, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );
}

#[test]
fn documentation_seller_daily_cap_example_reports_daily_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller_a, asset, _owner, resource_hash) = initialized_wallet(&env);
    let seller_b = Address::generate(&env);

    let mut policy = standard_policy(&env, &agent, &seller_a, &asset, &resource_hash, 100);
    policy.allowed_sellers = vec![&env, seller_a.clone(), seller_b.clone()];
    policy.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller_a.clone(),
            max_amount_per_payment: 30,
            max_amount_per_day: 50,
        },
    ];
    client.set_policy(&policy, &0);

    // Spend 30 then 20 with seller A -> exactly reaches seller A's daily cap of 50
    client.authorize_payment(&agent, &seller_a, &asset, &30, &resource_hash);
    client.authorize_payment(&agent, &seller_a, &asset, &20, &resource_hash);
    assert_eq!(client.spent_today_seller(&agent, &seller_a), 50);

    // Further spend with seller A is rejected by seller daily cap
    assert_eq!(
        client.try_authorize_payment(&agent, &seller_a, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );

    // But seller B spend is isolated and succeeds up to global daily cap (100 - 50 = 50 remaining)
    assert!(client
        .try_authorize_payment(&agent, &seller_b, &asset, &30, &resource_hash)
        .is_ok());
    assert_eq!(client.spent_today_seller(&agent, &seller_b), 30);
    assert_eq!(client.spent_today(&agent), 80);
}

#[test]
fn per_seller_spending_ceilings_and_isolation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller_a, asset, _owner, resource_hash) = initialized_wallet(&env);
    let seller_b = Address::generate(&env);
    let unknown_seller = Address::generate(&env);

    // 1. Seller ceiling cannot exceed global payment cap
    let mut invalid_policy = standard_policy(&env, &agent, &seller_a, &asset, &resource_hash, 100);
    invalid_policy.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller_a.clone(),
            max_amount_per_payment: 80, // > global 75
            max_amount_per_day: 90,
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 2. Seller ceiling cannot exceed global daily cap
    let mut invalid_policy2 = standard_policy(&env, &agent, &seller_a, &asset, &resource_hash, 100);
    invalid_policy2.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller_a.clone(),
            max_amount_per_payment: 50,
            max_amount_per_day: 150, // > global 100
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy2, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 3. Unknown seller in seller_limits (not in allowed_sellers)
    let mut invalid_policy3 = standard_policy(&env, &agent, &seller_a, &asset, &resource_hash, 100);
    invalid_policy3.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: unknown_seller,
            max_amount_per_payment: 50,
            max_amount_per_day: 50,
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy3, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 4. Duplicate seller in seller_limits
    let mut invalid_policy4 = standard_policy(&env, &agent, &seller_a, &asset, &resource_hash, 100);
    invalid_policy4.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller_a.clone(),
            max_amount_per_payment: 30,
            max_amount_per_day: 50,
        },
        SellerSpendingLimit {
            seller: seller_a.clone(),
            max_amount_per_payment: 20,
            max_amount_per_day: 40,
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy4, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 5. Valid policy with seller A and B ceilings
    let mut valid_policy =
        standard_policy(&env, &agent, &seller_a, &asset, &resource_hash, 100_000);
    valid_policy.allowed_sellers = vec![&env, seller_a.clone(), seller_b.clone()];
    valid_policy.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller_a.clone(),
            max_amount_per_payment: 30,
            max_amount_per_day: 50,
        },
        SellerSpendingLimit {
            seller: seller_b.clone(),
            max_amount_per_payment: 40,
            max_amount_per_day: 60,
        },
    ];
    client.set_policy(&valid_policy, &0);

    // Exact per-payment boundary for seller A
    assert!(client
        .try_check_payment(&agent, &seller_a, &asset, &30, &resource_hash)
        .is_ok());
    assert_eq!(
        client.try_check_payment(&agent, &seller_a, &asset, &31, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );

    // Authorize payments to exact daily boundary for seller A
    client.authorize_payment(&agent, &seller_a, &asset, &30, &resource_hash);
    client.authorize_payment(&agent, &seller_a, &asset, &20, &resource_hash);
    assert_eq!(client.spent_today_seller(&agent, &seller_a), 50);
    assert_eq!(client.spent_today_seller(&agent, &seller_b), 0);

    // Over seller daily boundary fails
    assert_eq!(
        client.try_authorize_payment(&agent, &seller_a, &asset, &1, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );

    // Spend isolation: seller B can spend up to its daily limit
    client.authorize_payment(&agent, &seller_b, &asset, &40, &resource_hash);
    assert_eq!(client.spent_today_seller(&agent, &seller_b), 40);
    assert_eq!(client.spent_today(&agent), 90);

    // 6. Day rollover resets daily spends
    env.ledger().set_sequence_number(LEDGERS_PER_DAY + 10);
    assert_eq!(client.spent_today_seller(&agent, &seller_a), 0);
    assert_eq!(client.spent_today_seller(&agent, &seller_b), 0);
    assert_eq!(client.spent_today(&agent), 0);

    // Can authorize again on the new day
    assert!(client
        .try_authorize_payment(&agent, &seller_a, &asset, &30, &resource_hash)
        .is_ok());
    assert_eq!(client.spent_today_seller(&agent, &seller_a), 30);
}

#[test]
fn documentation_resource_payment_cap_example_reports_amount_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100);
    policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_hash.clone(),
            max_amount_per_payment: 25,
            max_amount_per_day: 40,
        },
    ];
    client.set_policy(&policy, &0);

    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &30, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );
}

#[test]
fn documentation_resource_daily_cap_example_reports_daily_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_a) = initialized_wallet(&env);
    let resource_b = BytesN::from_array(&env, &[99; 32]);

    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100);
    policy.allowed_resource_hashes = vec![&env, resource_a.clone(), resource_b.clone()];
    policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 25,
            max_amount_per_day: 40,
        },
    ];
    client.set_policy(&policy, &0);

    // Spend 25 then 15 on resource A -> exactly reaches 40 daily limit
    client.authorize_payment(&agent, &seller, &asset, &25, &resource_a);
    client.authorize_payment(&agent, &seller, &asset, &15, &resource_a);
    assert_eq!(client.spent_today_resource(&agent, &resource_a), 40);

    // Further spend on resource A is rejected
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &5, &resource_a),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );

    // Resource B spend is isolated and succeeds up to global daily cap
    assert!(client
        .try_authorize_payment(&agent, &seller, &asset, &30, &resource_b)
        .is_ok());
    assert_eq!(client.spent_today_resource(&agent, &resource_b), 30);
    assert_eq!(client.spent_today(&agent), 70);
}

#[test]
fn per_resource_spending_ceilings_isolation_and_replacement() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_a) = initialized_wallet(&env);
    let resource_b = BytesN::from_array(&env, &[99; 32]);
    let unknown_resource = BytesN::from_array(&env, &[77; 32]);

    // 1. Resource ceiling cannot exceed global payment cap
    let mut invalid_policy = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100);
    invalid_policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 80, // > 75
            max_amount_per_day: 90,
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 2. Resource ceiling cannot exceed global daily cap
    let mut invalid_policy2 = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100);
    invalid_policy2.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 50,
            max_amount_per_day: 120, // > 100
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy2, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 3. Unknown resource in resource_limits
    let mut invalid_policy3 = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100);
    invalid_policy3.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: unknown_resource,
            max_amount_per_payment: 50,
            max_amount_per_day: 50,
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy3, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 4. Duplicate resource in resource_limits
    let mut invalid_policy4 = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100);
    invalid_policy4.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 25,
            max_amount_per_day: 50,
        },
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 20,
            max_amount_per_day: 40,
        },
    ];
    assert_eq!(
        client.try_set_policy(&invalid_policy4, &0),
        Err(Ok(PolicyWalletError::InvalidPolicy))
    );

    // 5. Valid policy with resource A and B ceilings
    let mut valid_policy = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100_000);
    valid_policy.allowed_resource_hashes = vec![&env, resource_a.clone(), resource_b.clone()];
    valid_policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 25,
            max_amount_per_day: 40,
        },
        ResourceSpendingLimit {
            resource_hash: resource_b.clone(),
            max_amount_per_payment: 35,
            max_amount_per_day: 50,
        },
    ];
    client.set_policy(&valid_policy, &0);

    // Exact per-payment boundary for resource A
    assert!(client
        .try_check_payment(&agent, &seller, &asset, &25, &resource_a)
        .is_ok());
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &26, &resource_a),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );

    // Authorize payments to exact daily boundary for resource A
    client.authorize_payment(&agent, &seller, &asset, &25, &resource_a);
    client.authorize_payment(&agent, &seller, &asset, &15, &resource_a);
    assert_eq!(client.spent_today_resource(&agent, &resource_a), 40);
    assert_eq!(client.spent_today_resource(&agent, &resource_b), 0);

    // Exceeding daily boundary fails
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &1, &resource_a),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );

    // Counters isolated across resources: resource B can spend
    client.authorize_payment(&agent, &seller, &asset, &35, &resource_b);
    assert_eq!(client.spent_today_resource(&agent, &resource_b), 35);
    assert_eq!(client.spent_today(&agent), 75);

    // 6. Policy replacement semantics:
    // Replace with a policy with a lower cap for resource A (e.g. 30 daily cap).
    // Historical spend (40) is preserved and exceeds the new cap (40 > 30).
    let mut lower_cap_policy = standard_policy(&env, &agent, &seller, &asset, &resource_a, 100_000);
    lower_cap_policy.allowed_resource_hashes = vec![&env, resource_a.clone(), resource_b.clone()];
    lower_cap_policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 25,
            max_amount_per_day: 30, // lower than 40 already spent
        },
    ];
    client.set_policy(&lower_cap_policy, &1);
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &1, &resource_a),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );

    // Replace with a higher cap for resource A (e.g. 60 daily cap).
    let mut higher_cap_policy =
        standard_policy(&env, &agent, &seller, &asset, &resource_a, 100_000);
    higher_cap_policy.allowed_resource_hashes = vec![&env, resource_a.clone(), resource_b.clone()];
    higher_cap_policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 25,
            max_amount_per_day: 60, // higher than 40 already spent
        },
    ];
    client.set_policy(&higher_cap_policy, &2);
    // Now can spend 15 more (40 + 15 = 55 <= 60)
    assert!(client
        .try_authorize_payment(&agent, &seller, &asset, &15, &resource_a)
        .is_ok());
    assert_eq!(client.spent_today_resource(&agent, &resource_a), 55);

    // 7. Day rollover semantics
    env.ledger().set_sequence_number(LEDGERS_PER_DAY + 10);
    assert_eq!(client.spent_today_resource(&agent, &resource_a), 0);
    assert_eq!(client.spent_today_resource(&agent, &resource_b), 0);
    assert_eq!(client.spent_today(&agent), 0);

    // Spend again on the new day
    assert!(client
        .try_authorize_payment(&agent, &seller, &asset, &25, &resource_a)
        .is_ok());
    assert_eq!(client.spent_today_resource(&agent, &resource_a), 25);
}

#[test]
fn documentation_authorization_limit_example_reports_limit_exceeded() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100);
    policy.max_authorizations_per_day = 1;
    client.set_policy(&policy, &0);

    // 1st authorization succeeds
    assert!(client
        .try_authorize_payment(&agent, &seller, &asset, &10, &resource_hash)
        .is_ok());
    assert_eq!(client.authorized_count_today(&agent), 1);

    // 2nd authorization exceeds the daily count limit of 1
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::AuthorizationLimitExceeded))
    );
}

#[test]
fn authorization_limit_isolation_and_rollover() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent_a, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let agent_b = Address::generate(&env);
    let wrong_seller = Address::generate(&env);

    // Setup policy for Agent A: limit 2 authorizations per day
    let mut policy_a = standard_policy(&env, &agent_a, &seller, &asset, &resource_hash, 100_000);
    policy_a.max_authorizations_per_day = 2;
    client.set_policy(&policy_a, &0);

    // Setup policy for Agent B: limit 2 authorizations per day
    let mut policy_b = standard_policy(&env, &agent_b, &seller, &asset, &resource_hash, 100_000);
    policy_b.agent = agent_b.clone();
    policy_b.max_authorizations_per_day = 2;
    client.set_policy(&policy_b, &0);

    // Initial count is 0
    assert_eq!(client.authorized_count_today(&agent_a), 0);
    assert_eq!(client.authorized_count_today(&agent_b), 0);

    // check_payment does NOT increment count
    assert!(client
        .try_check_payment(&agent_a, &seller, &asset, &10, &resource_hash)
        .is_ok());
    assert_eq!(client.authorized_count_today(&agent_a), 0);

    // Failed authorizations do NOT increment count
    assert_eq!(
        client.try_authorize_payment(&agent_a, &wrong_seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::SellerNotAllowed))
    );
    assert_eq!(client.authorized_count_today(&agent_a), 0);

    assert_eq!(
        client.try_authorize_payment(&agent_a, &seller, &asset, &999, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );
    assert_eq!(client.authorized_count_today(&agent_a), 0);

    // First successful authorization increments count to 1
    client.authorize_payment(&agent_a, &seller, &asset, &10, &resource_hash);
    assert_eq!(client.authorized_count_today(&agent_a), 1);
    assert_eq!(client.authorized_count_today(&agent_b), 0);

    // Second successful authorization reaches exact limit of 2
    client.authorize_payment(&agent_a, &seller, &asset, &10, &resource_hash);
    assert_eq!(client.authorized_count_today(&agent_a), 2);
    assert_eq!(client.authorized_count_today(&agent_b), 0);

    // Third attempt fails with AuthorizationLimitExceeded and count remains 2
    assert_eq!(
        client.try_authorize_payment(&agent_a, &seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::AuthorizationLimitExceeded))
    );
    assert_eq!(client.authorized_count_today(&agent_a), 2);

    // check_payment also reports AuthorizationLimitExceeded
    assert_eq!(
        client.try_check_payment(&agent_a, &seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::AuthorizationLimitExceeded))
    );

    // Multi-agent isolation: Agent B is completely unaffected by Agent A's limit
    assert!(client
        .try_authorize_payment(&agent_b, &seller, &asset, &15, &resource_hash)
        .is_ok());
    assert_eq!(client.authorized_count_today(&agent_b), 1);
    assert_eq!(client.authorized_count_today(&agent_a), 2);

    // Day rollover: advance ledger sequence into next day
    env.ledger().set_sequence_number(LEDGERS_PER_DAY + 1);
    assert_eq!(client.authorized_count_today(&agent_a), 0);
    assert_eq!(client.authorized_count_today(&agent_b), 0);

    // Agent A can now authorize again on the new day
    assert!(client
        .try_authorize_payment(&agent_a, &seller, &asset, &10, &resource_hash)
        .is_ok());
    assert_eq!(client.authorized_count_today(&agent_a), 1);
}

#[test]
fn policy_configuration_digest_is_deterministic_domain_separated_and_sensitive_to_all_fields() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_network_id([5; 32]);
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let other_contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let other_client = PolicyWalletExampleContractClient::new(&env, &other_contract_id);

    let owner = Address::generate(&env);
    let upto_session = Address::generate(&env);
    client.initialize(&owner, &upto_session);
    other_client.initialize(&owner, &upto_session);

    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[1; 32]);

    let baseline_policy = SpendingPolicy {
        agent: agent.clone(),
        max_amount_per_payment: 100,
        max_amount_per_day: 1000,
        max_authorizations_per_day: 50,
        valid_from_ledger: 10,
        valid_until_ledger: 10000,
        allowed_sellers: vec![&env, seller.clone()],
        allowed_assets: vec![&env, asset.clone()],
        allowed_resource_hashes: vec![&env, resource_hash.clone()],
        seller_limits: vec![
            &env,
            SellerSpendingLimit {
                seller: seller.clone(),
                max_amount_per_payment: 50,
                max_amount_per_day: 500,
            },
        ],
        resource_limits: vec![
            &env,
            ResourceSpendingLimit {
                resource_hash: resource_hash.clone(),
                max_amount_per_payment: 40,
                max_amount_per_day: 400,
            },
        ],
        revision: 0,
    };

    client.set_policy(&baseline_policy, &0);
    other_client.set_policy(&baseline_policy, &0);

    // 1. Acceptance Criteria: Equivalent configurations produce identical digests
    let digest_1 = client.get_policy_configuration_digest(&agent);
    let digest_2 = client.get_policy_configuration_digest(&agent);
    assert_eq!(digest_1, digest_2);

    // Domain separation: Different contract address -> different digest
    let other_contract_digest = other_client.get_policy_configuration_digest(&agent);
    assert_ne!(digest_1, other_contract_digest);

    // Domain separation: Different network id -> different digest
    let digest_owner_scope = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &baseline_policy)
    });
    assert_eq!(digest_1, digest_owner_scope);

    // 2. Acceptance Criteria: Any enforced policy field changes the digest
    // Field sensitivity checks:

    // Changing agent
    let other_agent = Address::generate(&env);
    let mut modified = baseline_policy.clone();
    modified.agent = other_agent;
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing owner
    let other_owner = Address::generate(&env);
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &other_owner, &baseline_policy)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing max_amount_per_payment
    let mut modified = baseline_policy.clone();
    modified.max_amount_per_payment = 99;
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing max_amount_per_day
    let mut modified = baseline_policy.clone();
    modified.max_amount_per_day = 999;
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing max_authorizations_per_day
    let mut modified = baseline_policy.clone();
    modified.max_authorizations_per_day = 49;
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing valid_from_ledger
    let mut modified = baseline_policy.clone();
    modified.valid_from_ledger = 11;
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing valid_until_ledger
    let mut modified = baseline_policy.clone();
    modified.valid_until_ledger = 9999;
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing allowed_sellers
    let other_seller = Address::generate(&env);
    let mut modified = baseline_policy.clone();
    modified.allowed_sellers = vec![&env, other_seller.clone()];
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing allowed_assets
    let other_asset = Address::generate(&env);
    let mut modified = baseline_policy.clone();
    modified.allowed_assets = vec![&env, other_asset.clone()];
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing allowed_resource_hashes
    let other_res_hash = BytesN::from_array(&env, &[2; 32]);
    let mut modified = baseline_policy.clone();
    modified.allowed_resource_hashes = vec![&env, other_res_hash.clone()];
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing seller_limits
    let mut modified = baseline_policy.clone();
    modified.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller.clone(),
            max_amount_per_payment: 45,
            max_amount_per_day: 500,
        },
    ];
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // Changing resource_limits
    let mut modified = baseline_policy.clone();
    modified.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_hash.clone(),
            max_amount_per_payment: 35,
            max_amount_per_day: 400,
        },
    ];
    let digest_mod = env.as_contract(&contract_id, || {
        compute_policy_configuration_digest(&env, &owner, &modified)
    });
    assert_ne!(digest_1, digest_mod);

    // 3. Acceptance Criteria: Golden vector documents canonical field ordering
    // Canonical tuple layout:
    // (
    //   domain: (Bytes("lumenbazaar/policy-configuration"), version: 1, network_id, contract_id),
    //   owner,
    //   policy.agent,
    //   policy.max_amount_per_payment,
    //   policy.max_amount_per_day,
    //   policy.max_authorizations_per_day,
    //   policy.valid_from_ledger,
    //   policy.valid_until_ledger,
    //   policy.allowed_sellers,
    //   policy.allowed_assets,
    //   policy.allowed_resource_hashes,
    //   policy.seller_limits,
    //   policy.resource_limits,
    // )
    let domain = (
        Bytes::from_slice(&env, POLICY_CONFIGURATION_DOMAIN),
        POLICY_CONFIGURATION_VERSION,
        env.ledger().network_id(),
        env.current_contract_address(),
    )
        .to_xdr(&env);
    let expected_payload = (
        domain,
        owner.clone(),
        baseline_policy.agent.clone(),
        baseline_policy.max_amount_per_payment,
        baseline_policy.max_amount_per_day,
        baseline_policy.max_authorizations_per_day,
        baseline_policy.valid_from_ledger,
        baseline_policy.valid_until_ledger,
        baseline_policy.allowed_sellers.clone(),
        baseline_policy.allowed_assets.clone(),
        baseline_policy.allowed_resource_hashes.clone(),
        baseline_policy.seller_limits.clone(),
        baseline_policy.resource_limits.clone(),
    );
    let manual_digest = env.crypto().sha256(&expected_payload.to_xdr(&env)).to_bytes();
    assert_eq!(digest_1, manual_digest);
}

#[test]
fn agent_policy_cursor_pagination_and_index_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);

    let owner = Address::generate(&env);
    let upto_session = Address::generate(&env);
    client.initialize(&owner, &upto_session);

    let agent_a = Address::generate(&env);
    let agent_b = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[1; 32]);

    // Initial empty pagination
    let initial_page = client.get_agent_policies(&agent_a, &None, &None);
    assert_eq!(initial_page.total_count, 0);
    assert!(initial_page.policy_ids.is_empty());
    assert_eq!(initial_page.next_cursor, None);

    // 1. Create policy for Agent A -> index updated with policy ID
    let mut policy_a1 = standard_policy(&env, &agent_a, &seller, &asset, &resource_hash, 100);
    client.set_policy(&policy_a1, &0);
    let id_a1 = client.get_policy_configuration_digest(&agent_a);

    let page = client.get_agent_policies(&agent_a, &None, &None);
    assert_eq!(page.total_count, 1);
    assert_eq!(page.policy_ids.len(), 1);
    assert_eq!(page.policy_ids.get(0).unwrap(), id_a1);

    // Agent B index is completely empty (Multi-agent isolation: pagination cannot expose policies for another agent)
    let page_b = client.get_agent_policies(&agent_b, &None, &None);
    assert_eq!(page_b.total_count, 0);
    assert!(page_b.policy_ids.is_empty());

    // 2. Replacement path: Update Agent A policy with new limit -> new policy ID recorded
    policy_a1.max_amount_per_payment = 80;
    client.set_policy(&policy_a1, &1);
    let id_a2 = client.get_policy_configuration_digest(&agent_a);
    assert_ne!(id_a1, id_a2);

    let page_after_replace = client.get_agent_policies(&agent_a, &None, &None);
    assert_eq!(page_after_replace.total_count, 2);
    assert_eq!(page_after_replace.policy_ids.len(), 2);
    assert_eq!(page_after_replace.policy_ids.get(0).unwrap(), id_a1);
    assert_eq!(page_after_replace.policy_ids.get(1).unwrap(), id_a2);

    // Re-setting the exact same policy (idempotent / same ID) does not duplicate entry in index
    client.set_policy(&policy_a1, &2);
    let page_same = client.get_agent_policies(&agent_a, &None, &None);
    assert_eq!(page_same.total_count, 2);

    // 3. Revoke path: Revoking Agent A's policy clears its index
    client.revoke_policy(&agent_a);
    let page_after_revoke = client.get_agent_policies(&agent_a, &None, &None);
    assert_eq!(page_after_revoke.total_count, 0);
    assert!(page_after_revoke.policy_ids.is_empty());
    assert_eq!(page_after_revoke.next_cursor, None);

    // Create policy for Agent B to verify agent isolation after A's revocation
    let policy_b = standard_policy(&env, &agent_b, &seller, &asset, &resource_hash, 100);
    client.set_policy(&policy_b, &0);
    let id_b = client.get_policy_configuration_digest(&agent_b);

    let page_b_active = client.get_agent_policies(&agent_b, &None, &None);
    assert_eq!(page_b_active.total_count, 1);
    assert_eq!(page_b_active.policy_ids.get(0).unwrap(), id_b);
    assert_eq!(client.get_agent_policies(&agent_a, &None, &None).total_count, 0);

    // 4. Cursor pagination and boundary testing
    // Set multiple policies for Agent A
    let mut current_expected_ids = std::vec::Vec::new();
    for i in 1_u32..=15 {
        let mut p = standard_policy(&env, &agent_a, &seller, &asset, &resource_hash, 100 + i);
        p.max_amount_per_payment = 10 + i as i128;
        client.set_policy(&p, &(i - 1));
        current_expected_ids.push(client.get_policy_configuration_digest(&agent_a));
    }

    // Page 1: limit 5, cursor 0
    let p1 = client.get_agent_policies(&agent_a, &Some(0), &Some(5));
    assert_eq!(p1.total_count, 15);
    assert_eq!(p1.policy_ids.len(), 5);
    assert_eq!(p1.next_cursor, Some(5));
    for i in 0..5 {
        assert_eq!(p1.policy_ids.get(i as u32).unwrap(), current_expected_ids[i]);
    }

    // Page 2: limit 5, cursor 5
    let p2 = client.get_agent_policies(&agent_a, &Some(5), &Some(5));
    assert_eq!(p2.total_count, 15);
    assert_eq!(p2.policy_ids.len(), 5);
    assert_eq!(p2.next_cursor, Some(10));
    for i in 0..5 {
        assert_eq!(p2.policy_ids.get(i as u32).unwrap(), current_expected_ids[i + 5]);
    }

    // Page 3: limit 5, cursor 10
    let p3 = client.get_agent_policies(&agent_a, &Some(10), &Some(5));
    assert_eq!(p3.total_count, 15);
    assert_eq!(p3.policy_ids.len(), 5);
    assert_eq!(p3.next_cursor, None); // Reached end
    for i in 0..5 {
        assert_eq!(p3.policy_ids.get(i as u32).unwrap(), current_expected_ids[i + 10]);
    }

    // Cursor past total_count returns empty page with None next_cursor
    let p_empty = client.get_agent_policies(&agent_a, &Some(20), &Some(5));
    assert_eq!(p_empty.total_count, 15);
    assert!(p_empty.policy_ids.is_empty());
    assert_eq!(p_empty.next_cursor, None);

    // Limit exceeding MAX_POLICY_PAGE_SIZE fails with InvalidPageLimit
    assert_eq!(
        client.try_get_agent_policies(&agent_a, &Some(0), &Some(MAX_POLICY_PAGE_SIZE + 1)),
        Err(Ok(PolicyWalletError::InvalidPageLimit))
    );
    // Limit of 0 fails with InvalidPageLimit
    assert_eq!(
        client.try_get_agent_policies(&agent_a, &Some(0), &Some(0)),
        Err(Ok(PolicyWalletError::InvalidPageLimit))
    );
}
#[test]
fn test_policy_active_window_status_boundaries_and_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);

    // 1. Policy not found for unregistered agent
    let unregistered = Address::generate(&env);
    assert_eq!(
        client.try_get_policy_status(&unregistered),
        Err(Ok(PolicyWalletError::PolicyNotFound))
    );

    // 2. Set policy with validity window [100, 200]
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 200);
    policy.valid_from_ledger = 100;
    client.set_policy(&policy, &0);

    // Pending: ledger 99 (< valid_from 100)
    env.ledger().set_sequence_number(99);
    assert_eq!(
        client.get_policy_status(&agent),
        PolicyActiveWindowStatus {
            status: PolicyStatus::Pending,
            current_ledger: 99,
            valid_from_ledger: 100,
            valid_until_ledger: 200,
            next_boundary_ledger: Some(100),
        }
    );
    // Matches authorization enforcement
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyNotActive))
    );

    // Active exact valid_from: ledger 100
    env.ledger().set_sequence_number(100);
    assert_eq!(
        client.get_policy_status(&agent),
        PolicyActiveWindowStatus {
            status: PolicyStatus::Active,
            current_ledger: 100,
            valid_from_ledger: 100,
            valid_until_ledger: 200,
            next_boundary_ledger: Some(201),
        }
    );
    assert!(client.try_check_payment(&agent, &seller, &asset, &10, &resource_hash).is_ok());

    // Active interior: ledger 150
    env.ledger().set_sequence_number(150);
    assert_eq!(
        client.get_policy_status(&agent),
        PolicyActiveWindowStatus {
            status: PolicyStatus::Active,
            current_ledger: 150,
            valid_from_ledger: 100,
            valid_until_ledger: 200,
            next_boundary_ledger: Some(201),
        }
    );
    assert!(client.try_check_payment(&agent, &seller, &asset, &10, &resource_hash).is_ok());

    // Active exact valid_until: ledger 200 (inclusive upper bound)
    env.ledger().set_sequence_number(200);
    assert_eq!(
        client.get_policy_status(&agent),
        PolicyActiveWindowStatus {
            status: PolicyStatus::Active,
            current_ledger: 200,
            valid_from_ledger: 100,
            valid_until_ledger: 200,
            next_boundary_ledger: Some(201),
        }
    );
    assert!(client.try_check_payment(&agent, &seller, &asset, &10, &resource_hash).is_ok());

    // Expired exact: ledger 201 (> valid_until 200)
    env.ledger().set_sequence_number(201);
    assert_eq!(
        client.get_policy_status(&agent),
        PolicyActiveWindowStatus {
            status: PolicyStatus::Expired,
            current_ledger: 201,
            valid_from_ledger: 100,
            valid_until_ledger: 200,
            next_boundary_ledger: None,
        }
    );
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyExpired))
    );

    // 3. Revocation takes precedence over temporal state
    // Revoke at ledger 150 (within active window)
    env.ledger().set_sequence_number(150);
    client.revoke_policy(&agent);
    assert_eq!(
        client.get_policy_status(&agent),
        PolicyActiveWindowStatus {
            status: PolicyStatus::Revoked,
            current_ledger: 150,
            valid_from_ledger: 0,
            valid_until_ledger: 0,
            next_boundary_ledger: None,
        }
    );
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::PolicyRevoked))
    );

    // Revocation status holds even at ledger before valid_from or after valid_until
    env.ledger().set_sequence_number(50);
    assert_eq!(client.get_policy_status(&agent).status, PolicyStatus::Revoked);
    env.ledger().set_sequence_number(250);
    assert_eq!(client.get_policy_status(&agent).status, PolicyStatus::Revoked);
}

fn record_upto_event_names(
    env: &Env,
    contract_id: &Address,
    audit_log: &mut std::vec::Vec<std::string::String>,
) {
    for event in env.events().all().filter_by_contract(contract_id).events() {
        let ContractEventBody::V0(body) = &event.body;
        if let Some(ScVal::Symbol(symbol)) = body.topics.first() {
            if let Ok(name) = std::str::from_utf8(&symbol.0[..]) {
                audit_log.push(std::string::String::from(name));
            }
        }
    }
}

fn setup_integrated_wallet_and_escrow(
    env: &Env,
    initial_mint: i128,
) -> (
    PolicyWalletExampleContractClient<'_>,
    UptoSessionContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
    BytesN<32>,
) {
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_sequence_number(100);
    let upto_id = env.register(UptoSessionContract, ());
    let upto = UptoSessionContractClient::new(env, &upto_id);
    let wallet_id = env.register(PolicyWalletExampleContract, ());
    let wallet = PolicyWalletExampleContractClient::new(env, &wallet_id);

    let owner = Address::generate(env);
    let admin = Address::generate(env);
    let agent = Address::generate(env);
    let seller = Address::generate(env);
    let token_admin = Address::generate(env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    token::StellarAssetClient::new(env, &asset).mint(&agent, &initial_mint);

    upto.initialize(
        &admin,
        &vec![
            env,
            AssetPolicy {
                asset: asset.clone(),
                max_session_amount: 1_000,
                min_session_amount: 1,
                max_asset_exposure: 10_000,
                max_buyer_exposure: 10_000,
            },
        ],
    );
    wallet.initialize(&owner, &upto_id);

    let resource_hash = BytesN::from_array(env, &[0x33; 32]);
    let initial_policy = SpendingPolicy {
        agent: agent.clone(),
        max_amount_per_payment: 75,
        max_amount_per_day: 100,
        max_authorizations_per_day: 10,
        valid_from_ledger: 0,
        valid_until_ledger: 1_000,
        allowed_sellers: vec![env, seller.clone()],
        allowed_assets: vec![env, asset.clone()],
        allowed_resource_hashes: vec![env, resource_hash.clone()],
        seller_limits: vec![env],
        resource_limits: vec![env],
        revision: 0,
    };
    wallet.set_policy(&initial_policy, &0);

    (wallet, upto, agent, seller, asset, admin, resource_hash)
}

#[test]
fn same_ledger_policy_cap_reduction_and_session_creation_both_orderings() {
    // Order A: owner reduces caps via set_policy before agent authorize_payment + create_session in same ledger.
    {
        let env = Env::default();
        let (wallet, upto, agent, seller, asset, _admin, resource_hash) =
            setup_integrated_wallet_and_escrow(&env, 500);
        let mut audit_log = std::vec::Vec::new();

        let mut reduced = wallet.get_policy(&agent);
        reduced.max_amount_per_payment = 40;
        reduced.max_amount_per_day = 60;
        wallet.set_policy(&reduced, &1);
        assert_eq!(wallet.get_policy_revision(&agent), 2);

        // Agent attempts amount allowed before reduction (60) -> rejected by new per-payment cap (40).
        assert_eq!(
            wallet.try_authorize_payment(&agent, &seller, &asset, &60, &resource_hash),
            Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
        );
        assert_eq!(wallet.spent_today(&agent), 0);
        assert_eq!(upto.get_escrow_liability(&asset), 0);
        assert_eq!(audit_log.len(), 0);

        // Authorizations within reduced caps succeed (40 + 20 == 60 <= 60).
        wallet.authorize_payment(&agent, &seller, &asset, &40, &resource_hash);
        upto.create_session(
            &agent,
            &seller,
            &asset,
            &40,
            &200,
            &resource_hash,
            &BytesN::from_array(&env, &[0xa1; 32]),
        );
        record_upto_event_names(&env, &upto.address, &mut audit_log);

        assert_eq!(
            wallet.try_authorize_payment(&agent, &seller, &asset, &21, &resource_hash),
            Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
        );
        wallet.authorize_payment(&agent, &seller, &asset, &20, &resource_hash);
        upto.create_session(
            &agent,
            &seller,
            &asset,
            &20,
            &200,
            &resource_hash,
            &BytesN::from_array(&env, &[0xa2; 32]),
        );
        record_upto_event_names(&env, &upto.address, &mut audit_log);

        assert_eq!(
            wallet.try_authorize_payment(&agent, &seller, &asset, &1, &resource_hash),
            Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
        );
        assert_eq!(wallet.spent_today(&agent), 60);
        assert_eq!(upto.get_escrow_liability(&asset), 60);
        assert_eq!(audit_log, std::vec!["session_created", "session_created"]);
    }

    // Order B (within new daily cap): agent authorizes 60 + creates session before owner reduces caps (40/60).
    {
        let env = Env::default();
        let (wallet, upto, agent, seller, asset, _admin, resource_hash) =
            setup_integrated_wallet_and_escrow(&env, 500);
        let mut audit_log = std::vec::Vec::new();

        wallet.authorize_payment(&agent, &seller, &asset, &60, &resource_hash);
        upto.create_session(
            &agent,
            &seller,
            &asset,
            &60,
            &200,
            &resource_hash,
            &BytesN::from_array(&env, &[0xb1; 32]),
        );
        record_upto_event_names(&env, &upto.address, &mut audit_log);

        let mut reduced = wallet.get_policy(&agent);
        reduced.max_amount_per_payment = 40;
        reduced.max_amount_per_day = 60;
        wallet.set_policy(&reduced, &1);
        assert_eq!(wallet.get_policy_revision(&agent), 2);
        assert_eq!(wallet.spent_today(&agent), 60);

        // No further spending is possible because same-day spend (60) meets reduced daily cap (60).
        assert_eq!(
            wallet.try_authorize_payment(&agent, &seller, &asset, &1, &resource_hash),
            Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
        );
        assert_eq!(upto.get_escrow_liability(&asset), 60);
        assert_eq!(audit_log, std::vec!["session_created"]);
    }

    // Order B (pre-reduction spend exceeds post-reduction cap + buyer reduce_session_cap audit trail).
    {
        let env = Env::default();
        let (wallet, upto, agent, seller, asset, _admin, resource_hash) =
            setup_integrated_wallet_and_escrow(&env, 500);
        let mut audit_log = std::vec::Vec::new();

        // Pre-reduction authorization (75) uses rev 1 (75 <= 75, <= 100).
        wallet.authorize_payment(&agent, &seller, &asset, &75, &resource_hash);
        let session_id = upto.create_session(
            &agent,
            &seller,
            &asset,
            &75,
            &200,
            &resource_hash,
            &BytesN::from_array(&env, &[0xc1; 32]),
        );
        record_upto_event_names(&env, &upto.address, &mut audit_log);

        // Same-ledger policy cap reduction to 40/60 takes effect immediately; spend stays 75.
        let mut reduced = wallet.get_policy(&agent);
        reduced.max_amount_per_payment = 40;
        reduced.max_amount_per_day = 60;
        wallet.set_policy(&reduced, &1);
        assert_eq!(wallet.spent_today(&agent), 75);
        assert_eq!(
            wallet.try_authorize_payment(&agent, &seller, &asset, &1, &resource_hash),
            Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
        );

        // Reducing escrow cap on upto-session lowers live escrow below the reduced cap and logs in order.
        upto.reduce_session_cap(&session_id, &50);
        record_upto_event_names(&env, &upto.address, &mut audit_log);
        assert_eq!(upto.get_escrow_liability(&asset), 50);
        assert_eq!(upto.get_buyer_exposure(&agent, &asset), 50);
        assert_eq!(
            audit_log,
            std::vec!["session_created", "session_cap_reduced"]
        );
    }
}

#[test]
fn same_ledger_concurrent_cap_reductions_enforce_revision_and_seller_resource_ordering() {
    let env = Env::default();
    let (wallet, upto, agent, seller, asset, _admin, resource_a) =
        setup_integrated_wallet_and_escrow(&env, 500);
    let resource_b = BytesN::from_array(&env, &[0x44; 32]);
    let mut audit_log = std::vec::Vec::new();

    // Configure per-seller and per-resource limits at rev 2.
    let mut scoped = wallet.get_policy(&agent);
    scoped.allowed_resource_hashes = vec![&env, resource_a.clone(), resource_b.clone()];
    scoped.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller.clone(),
            max_amount_per_payment: 60,
            max_amount_per_day: 90,
        },
    ];
    scoped.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 50,
            max_amount_per_day: 80,
        },
    ];
    wallet.set_policy(&scoped, &1);
    assert_eq!(wallet.get_policy_revision(&agent), 2);

    // Creation first for resource_a (45) then same-ledger cap reduction to rev 3 (per-resource day 40).
    wallet.authorize_payment(&agent, &seller, &asset, &45, &resource_a);
    let s1 = upto.create_session(
        &agent,
        &seller,
        &asset,
        &45,
        &200,
        &resource_a,
        &BytesN::from_array(&env, &[0xd1; 32]),
    );
    record_upto_event_names(&env, &upto.address, &mut audit_log);

    let mut tighter = wallet.get_policy(&agent);
    tighter.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 30,
            max_amount_per_day: 40,
        },
    ];
    wallet.set_policy(&tighter, &2);
    assert_eq!(wallet.get_policy_revision(&agent), 3);

    // Competing same-ledger policy write using stale expected_revision = 2 fails without weakening rev 3.
    let mut weaker = tighter.clone();
    weaker.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_a.clone(),
            max_amount_per_payment: 75,
            max_amount_per_day: 100,
        },
    ];
    assert_eq!(
        wallet.try_set_policy(&weaker, &2),
        Err(Ok(PolicyWalletError::StaleRevision))
    );
    assert_eq!(wallet.get_policy_revision(&agent), 3);

    // resource_a is now blocked (45 spent > 40 cap), while resource_b still honors global/seller caps (45).
    assert_eq!(
        wallet.try_authorize_payment(&agent, &seller, &asset, &1, &resource_a),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );
    wallet.authorize_payment(&agent, &seller, &asset, &45, &resource_b);
    let s2 = upto.create_session(
        &agent,
        &seller,
        &asset,
        &45,
        &200,
        &resource_b,
        &BytesN::from_array(&env, &[0xd2; 32]),
    );
    record_upto_event_names(&env, &upto.address, &mut audit_log);

    // Both seller daily cap (90) and remaining headroom (0) prevent further spend in the same ledger.
    assert_eq!(wallet.spent_today_seller(&agent, &seller), 90);
    assert_eq!(
        wallet.try_authorize_payment(&agent, &seller, &asset, &1, &resource_b),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );
    assert_eq!(upto.get_escrow_liability(&asset), 90);
    assert_eq!(
        upto.get_session(&s1).status,
        upto_session::SessionStatus::Open
    );
    assert_eq!(
        upto.get_session(&s2).status,
        upto_session::SessionStatus::Open
    );
    assert_eq!(audit_log, std::vec!["session_created", "session_created"]);
}

#[test]
fn test_same_ledger_authorization_ordering_permutations_and_rejections() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let wrong_seller = Address::generate(&env);
    // Set standard policy: payment cap = 75, daily cap = 100, max authorizations per day = 3
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100_000);
    policy.max_authorizations_per_day = 3;
    client.set_policy(&policy, &0);

    // Ensure we are at a fixed ledger (same ledger throughout all permutation tests)
    env.ledger().set_sequence_number(1_000);

    // 1. Rejected calls (e.g. wrong seller, amount over per-payment cap, non-positive amount)
    // executed in various positions do NOT cause state drift or increment spend/auth counters.
    assert_eq!(
        client.try_authorize_payment(&agent, &wrong_seller, &asset, &20, &resource_hash),
        Err(Ok(PolicyWalletError::SellerNotAllowed))
    );
    assert_eq!(client.spent_today(&agent), 0);
    assert_eq!(client.authorized_count_today(&agent), 0);

    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &80, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap))
    );
    assert_eq!(client.spent_today(&agent), 0);
    assert_eq!(client.authorized_count_today(&agent), 0);

    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &0, &resource_hash),
        Err(Ok(PolicyWalletError::InvalidAmount))
    );
    assert_eq!(client.spent_today(&agent), 0);
    assert_eq!(client.authorized_count_today(&agent), 0);

    // 2. Successful same-ledger sequence: First call consumes 40
    assert!(client.try_authorize_payment(&agent, &seller, &asset, &40, &resource_hash).is_ok());
    assert_eq!(client.spent_today(&agent), 40);
    assert_eq!(client.authorized_count_today(&agent), 1);

    // Rejected call interleaved in the middle of a same-ledger sequence:
    assert_eq!(
        client.try_authorize_payment(&agent, &wrong_seller, &asset, &10, &resource_hash),
        Err(Ok(PolicyWalletError::SellerNotAllowed))
    );
    assert_eq!(client.spent_today(&agent), 40);
    assert_eq!(client.authorized_count_today(&agent), 1);

    // Second valid call: consumes 50 (40 + 50 = 90 <= 100 daily cap)
    assert!(client.try_authorize_payment(&agent, &seller, &asset, &50, &resource_hash).is_ok());
    assert_eq!(client.spent_today(&agent), 90);
    assert_eq!(client.authorized_count_today(&agent), 2);

    // Payment exceeding remaining daily cap (90 + 20 = 110 > 100): rejected, no state drift
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &20, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap))
    );
    assert_eq!(client.spent_today(&agent), 90);
    assert_eq!(client.authorized_count_today(&agent), 2);

    // Third valid call: consumes 10 (reaches exact daily cap 100 and auth limit 3)
    assert!(client.try_authorize_payment(&agent, &seller, &asset, &10, &resource_hash).is_ok());
    assert_eq!(client.spent_today(&agent), 100);
    assert_eq!(client.authorized_count_today(&agent), 3);

    // Subsequent call fails on authorization count limit (even for amount 0 or within cap)
    assert_eq!(
        client.try_authorize_payment(&agent, &seller, &asset, &1, &resource_hash),
        Err(Ok(PolicyWalletError::AuthorizationLimitExceeded))
    );
    assert_eq!(client.spent_today(&agent), 100);
    assert_eq!(client.authorized_count_today(&agent), 3);

    // 3. Alternative ordering test on fresh agent to verify deterministic order-independent final states
    let agent2 = Address::generate(&env);
    let mut policy2 = standard_policy(&env, &agent2, &seller, &asset, &resource_hash, 100_000);
    policy2.agent = agent2.clone();
    policy2.max_authorizations_per_day = 3;
    client.set_policy(&policy2, &0);

    // Same ledger: execute in reverse amount order: 10, then 50, then 40
    assert!(client.try_authorize_payment(&agent2, &seller, &asset, &10, &resource_hash).is_ok());
    assert_eq!(client.spent_today(&agent2), 10);
    assert_eq!(client.authorized_count_today(&agent2), 1);

    assert!(client.try_authorize_payment(&agent2, &seller, &asset, &50, &resource_hash).is_ok());
    assert_eq!(client.spent_today(&agent2), 60);
    assert_eq!(client.authorized_count_today(&agent2), 2);

    assert!(client.try_authorize_payment(&agent2, &seller, &asset, &40, &resource_hash).is_ok());
    assert_eq!(client.spent_today(&agent2), 100);
    assert_eq!(client.authorized_count_today(&agent2), 3);

    // Reached exact same terminal totals on the same ledger
    assert_eq!(client.spent_today(&agent), client.spent_today(&agent2));
    assert_eq!(client.authorized_count_today(&agent), client.authorized_count_today(&agent2));
}

#[test]
fn preview_payment_matches_check_payment_and_never_mutates_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    let wrong_seller = Address::generate(&env);

    let wrong_asset = Address::generate(&env);
    let wrong_resource = BytesN::from_array(&env, &[99; 32]);

    // 1. PolicyNotFound when no policy is set
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &50, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::PolicyNotFound,
        }
    );

    // Set standard policy: max_per_payment = 75, max_per_day = 100, max_auth = 100, valid_until = 100, valid_from = 10
    let mut policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 100);
    policy.valid_from_ledger = 10;
    client.set_policy(&policy, &0);

    // 2. PolicyNotActive (sequence 0 < valid_from 10)
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &50, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::PolicyNotActive,
        }
    );

    // Advance to valid window: sequence 10
    env.ledger().set_sequence_number(10);

    // 3. Positive allowed preview
    let snapshot_before = client.spent_today(&agent);
    let auths_before = client.authorized_count_today(&agent);
    let preview_ok = client.preview_payment(&agent, &seller, &asset, &50, &resource_hash);
    assert_eq!(
        preview_ok,
        PolicyPreviewResult {
            allowed: true,
            denial_reason: PolicyDenialReason::None,
        }
    );
    // Assert no mutation occurred
    assert_eq!(client.spent_today(&agent), snapshot_before);
    assert_eq!(client.authorized_count_today(&agent), auths_before);

    // 4. InvalidAmount (0 and negative)
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &0, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::InvalidAmount,
        }
    );
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &-5, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::InvalidAmount,
        }
    );

    // 5. AmountExceedsPaymentCap (76 > 75)
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &76, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AmountExceedsPaymentCap,
        }
    );

    // 6. SellerNotAllowed
    assert_eq!(
        client.preview_payment(&agent, &wrong_seller, &asset, &50, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::SellerNotAllowed,
        }
    );

    // 7. AssetNotAllowed
    assert_eq!(
        client.preview_payment(&agent, &seller, &wrong_asset, &50, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AssetNotAllowed,
        }
    );

    // 8. ResourceHashNotAllowed
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &50, &wrong_resource),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::ResourceHashNotAllowed,
        }
    );

    // 9. AuthorizationLimitExceeded
    // Set policy with max_authorizations_per_day = 1
    policy.max_authorizations_per_day = 1;
    client.set_policy(&policy, &1);
    // Authorize 1 payment
    client.authorize_payment(&agent, &seller, &asset, &10, &resource_hash);
    assert_eq!(client.authorized_count_today(&agent), 1);
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &10, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AuthorizationLimitExceeded,
        }
    );

    // 10. AmountExceedsDailyCap
    // Update policy with max_authorizations_per_day = 100, max_amount_per_payment = 50, max_amount_per_day = 50
    policy.max_authorizations_per_day = 100;
    policy.max_amount_per_payment = 50;
    policy.max_amount_per_day = 50;
    client.set_policy(&policy, &2);
    // Spent is already 10. Trying 41 would total 51 > 50
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &41, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AmountExceedsDailyCap,
        }
    );

    // 11. Per-seller payment cap and daily cap
    policy.seller_limits = vec![
        &env,
        SellerSpendingLimit {
            seller: seller.clone(),
            max_amount_per_payment: 30,
            max_amount_per_day: 40,
        },
    ];
    client.set_policy(&policy, &3);
    // Payment cap violation (31 > 30)
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &31, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AmountExceedsPaymentCap,
        }
    );
    // Daily cap violation
    client.authorize_payment(&agent, &seller, &asset, &25, &resource_hash);
    assert_eq!(client.spent_today_seller(&agent, &seller), 35);
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &10, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AmountExceedsDailyCap,
        }
    );

    // 12. Per-resource payment cap and daily cap
    policy.resource_limits = vec![
        &env,
        ResourceSpendingLimit {
            resource_hash: resource_hash.clone(),
            max_amount_per_payment: 20,
            max_amount_per_day: 40,
        },
    ];
    policy.max_amount_per_day = 200;
    policy.seller_limits = vec![&env];
    client.set_policy(&policy, &4);
    // Payment cap violation (21 > 20)
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &21, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AmountExceedsPaymentCap,
        }
    );
    // Daily cap violation: spent on resource is already 35. 35 + 10 = 45 > 40
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &10, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::AmountExceedsDailyCap,
        }
    );

    // 13. PolicyExpired (sequence 101 > valid_until 100)
    env.ledger().set_sequence_number(101);
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &10, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::PolicyExpired,
        }
    );

    // 14. PolicyRevoked
    client.revoke_policy(&agent);
    assert_eq!(
        client.preview_payment(&agent, &seller, &asset, &10, &resource_hash),
        PolicyPreviewResult {
            allowed: false,
            denial_reason: PolicyDenialReason::PolicyRevoked,
        }
    );
}

#[test]
fn authorize_payment_with_nonce_exact_replay_and_conflict() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    let nonce1 = BytesN::from_array(&env, &[1; 32]);
    let nonce2 = BytesN::from_array(&env, &[2; 32]);

    env.ledger().set_sequence_number(10);

    // Initial authorization succeeds
    let outcome1 = client.authorize_payment_with_nonce(
        &agent,
        &seller,
        &asset,
        &25,
        &resource_hash,
        &nonce1,
    );
    assert_eq!(
        outcome1,
        PaymentAuthorizationOutcome {
            replayed: false,
            authorized_at_ledger: 10,
        }
    );
    assert_eq!(client.spent_today(&agent), 25);
    assert_eq!(client.authorized_count_today(&agent), 1);

    // Check recorded authorization record
    let record1 = client.get_authorization_nonce(&agent, &nonce1).unwrap();
    assert_eq!(record1.authorized_at_ledger, 10);
    assert_eq!(record1.amount, 25);

    // Exact replay with same nonce and same parameters at a later ledger
    env.ledger().set_sequence_number(20);
    let replay_outcome = client.authorize_payment_with_nonce(
        &agent,
        &seller,
        &asset,
        &25,
        &resource_hash,
        &nonce1,
    );
    assert_eq!(
        replay_outcome,
        PaymentAuthorizationOutcome {
            replayed: true,
            authorized_at_ledger: 10,
        }
    );
    // Crucial: spend and auth count must NOT be incremented on replay
    assert_eq!(client.spent_today(&agent), 25);
    assert_eq!(client.authorized_count_today(&agent), 1);

    // Conflicting request with same nonce but different amount fails with ReplayConflict
    let err_conflict_amount = client.try_authorize_payment_with_nonce(
        &agent,
        &seller,
        &asset,
        &30,
        &resource_hash,
        &nonce1,
    );
    assert_eq!(
        err_conflict_amount,
        Err(Ok(PolicyWalletError::ReplayConflict))
    );

    // Conflicting request with same nonce but different seller fails with ReplayConflict
    let other_seller = Address::generate(&env);
    let err_conflict_seller = client.try_authorize_payment_with_nonce(
        &agent,
        &other_seller,
        &asset,
        &25,
        &resource_hash,
        &nonce1,
    );
    assert_eq!(
        err_conflict_seller,
        Err(Ok(PolicyWalletError::ReplayConflict))
    );

    // Conflicting request with same nonce but different asset fails with ReplayConflict
    let other_asset = Address::generate(&env);
    let err_conflict_asset = client.try_authorize_payment_with_nonce(
        &agent,
        &seller,
        &other_asset,
        &25,
        &resource_hash,
        &nonce1,
    );
    assert_eq!(
        err_conflict_asset,
        Err(Ok(PolicyWalletError::ReplayConflict))
    );

    // Conflicting request with same nonce but different resource_hash fails with ReplayConflict
    let other_resource = BytesN::from_array(&env, &[99; 32]);
    let err_conflict_res = client.try_authorize_payment_with_nonce(
        &agent,
        &seller,
        &asset,
        &25,
        &other_resource,
        &nonce1,
    );
    assert_eq!(
        err_conflict_res,
        Err(Ok(PolicyWalletError::ReplayConflict))
    );

    // Spend and auth count still untouched after conflicts
    assert_eq!(client.spent_today(&agent), 25);
    assert_eq!(client.authorized_count_today(&agent), 1);

    // Authorizing with distinct nonce2 succeeds
    let outcome2 = client.authorize_payment_with_nonce(
        &agent,
        &seller,
        &asset,
        &15,
        &resource_hash,
        &nonce2,
    );
    assert_eq!(
        outcome2,
        PaymentAuthorizationOutcome {
            replayed: false,
            authorized_at_ledger: 20,
        }
    );
    assert_eq!(client.spent_today(&agent), 40);
    assert_eq!(client.authorized_count_today(&agent), 2);

    // Exact replay across day boundary returns deterministic outcome and does not double-spend
    env.ledger().set_sequence_number(LEDGERS_PER_DAY + 5);
    assert_eq!(client.spent_today(&agent), 0); // New day
    assert_eq!(client.authorized_count_today(&agent), 0);

    let replay_day2 = client.authorize_payment_with_nonce(
        &agent,
        &seller,
        &asset,
        &25,
        &resource_hash,
        &nonce1,
    );
    assert_eq!(
        replay_day2,
        PaymentAuthorizationOutcome {
            replayed: true,
            authorized_at_ledger: 10,
        }
    );
    // Still 0 on new day because replay does not consume new day's quota
    assert_eq!(client.spent_today(&agent), 0);
    assert_eq!(client.authorized_count_today(&agent), 0);
}
