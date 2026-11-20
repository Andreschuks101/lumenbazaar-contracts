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
