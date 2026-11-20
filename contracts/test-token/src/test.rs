extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

mod conformance_harness {
    include!("conformance_test.rs");
}

#[test]
fn test_token_utility_is_registered_independently() {
    let env = Env::default();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);

    assert!(client.utility_only());
    assert_eq!(client.decimals(), 7);
    assert_eq!(client.symbol(), String::from_str(&env, "LBT"));
}

#[test]
fn initializes_admin_once() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(TokenError::AlreadyInitialized))
    );
}

#[test]
fn mint_requires_admin_auth_and_updates_balance() {
    let env = Env::default();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);

    env.as_contract(&contract_id, || {
        env.storage().instance().set(&DataKey::Admin, &admin);
    });

    assert!(client.try_mint(&buyer, &100).is_err());

    env.mock_all_auths();
    client.mint(&buyer, &100);

    assert_eq!(client.balance(&buyer), 100);
}

#[test]
fn transfer_requires_holder_auth_and_moves_balances() {
    let env = Env::default();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);

    env.as_contract(&contract_id, || {
        write_balance(&env, &buyer, 100);
    });

    assert!(client.try_transfer(&buyer, &seller, &25).is_err());

    env.mock_all_auths();
    client.transfer(&buyer, &seller, &25);

    assert_eq!(client.balance(&buyer), 75);
    assert_eq!(client.balance(&seller), 25);
}

#[test]
fn rejects_invalid_amounts_and_insufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);

    client.initialize(&admin);

    assert_eq!(
        client.try_mint(&buyer, &0),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_transfer(&buyer, &seller, &1),
        Err(Ok(TokenError::InsufficientBalance))
    );
}

#[test]
fn default_initialization_applies_unbounded_test_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);

    client.initialize(&admin);

    assert_eq!(client.mint_ceiling(), DEFAULT_MINT_CEILING);
    assert_eq!(client.total_supply(), 0);

    // Legacy callers that only supply `admin` keep working unchanged.
    client.mint(&buyer, &100);
    assert_eq!(client.balance(&buyer), 100);
    assert_eq!(client.total_supply(), 100);
}

#[test]
fn mint_at_exact_ceiling_succeeds_and_any_excess_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);

    client.initialize_with_ceiling(&admin, &1000);

    // Minting up to the exact ceiling succeeds.
    client.mint(&buyer, &600);
    client.mint(&buyer, &400);
    assert_eq!(client.total_supply(), 1000);
    assert_eq!(client.balance(&buyer), 1000);

    // Any excess fails with MintCeilingExceeded.
    assert_eq!(
        client.try_mint(&buyer, &1),
        Err(Ok(TokenError::MintCeilingExceeded))
    );

    // A single mint larger than the remaining ceiling also fails.
    let other = Address::generate(&env);
    assert_eq!(
        client.try_mint(&other, &1001),
        Err(Ok(TokenError::MintCeilingExceeded))
    );
}

#[test]
fn failed_mint_leaves_supply_and_balances_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let other = Address::generate(&env);

    client.initialize_with_ceiling(&admin, &100);
    client.mint(&buyer, &90);

    assert_eq!(
        client.try_mint(&buyer, &20),
        Err(Ok(TokenError::MintCeilingExceeded))
    );
    assert_eq!(
        client.try_mint(&other, &11),
        Err(Ok(TokenError::MintCeilingExceeded))
    );

    // Neither total supply nor any balance changed after the failed mints.
    assert_eq!(client.total_supply(), 90);
    assert_eq!(client.balance(&buyer), 90);
    assert_eq!(client.balance(&other), 0);

    // The remaining headroom is still available.
    client.mint(&other, &10);
    assert_eq!(client.total_supply(), 100);
    assert_eq!(client.balance(&other), 10);
}

#[test]
fn initialize_with_ceiling_rejects_non_positive_ceiling_and_is_immutable() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &0),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &-1),
        Err(Ok(TokenError::InvalidAmount))
    );

    client.initialize_with_ceiling(&admin, &500);
    assert_eq!(client.mint_ceiling(), 500);

    // The ceiling cannot be re-configured once initialization has happened.
    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &1_000),
        Err(Ok(TokenError::AlreadyInitialized))
    );
    assert_eq!(client.mint_ceiling(), 500);
}

#[test]
fn ceiling_checks_use_checked_supply_accounting() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let buyer = Address::generate(&env);

    client.initialize_with_ceiling(&admin, &i128::MAX);
    client.mint(&buyer, &i128::MAX);

    // The next mint overflows supply accounting and fails closed.
    assert_eq!(
        client.try_mint(&buyer, &1),
        Err(Ok(TokenError::MintCeilingExceeded))
    );
    assert_eq!(client.total_supply(), i128::MAX);
    assert_eq!(client.balance(&buyer), i128::MAX);
}

#[test]
fn zero_and_boundary_token_amounts_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TestTokenContract, ());
    let client = TestTokenContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);

    // 1. Uninitialized mint
    assert_eq!(
        client.try_mint(&user1, &100),
        Err(Ok(TokenError::NotInitialized))
    );

    // 2. Boundary ceiling values in initialize_with_ceiling
    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &0),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &-1),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &i128::MIN),
        Err(Ok(TokenError::InvalidAmount))
    );

    // Initialize with valid ceiling
    client.initialize_with_ceiling(&admin, &1_000);
    assert_eq!(client.mint_ceiling(), 1_000);
    assert_eq!(client.total_supply(), 0);

    // 3. Double initialize fails
    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(TokenError::AlreadyInitialized))
    );
    assert_eq!(
        client.try_initialize_with_ceiling(&admin, &500),
        Err(Ok(TokenError::AlreadyInitialized))
    );

    // 4. Zero and negative mint amounts
    assert_eq!(
        client.try_mint(&user1, &0),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_mint(&user1, &-1),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_mint(&user1, &i128::MIN),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(client.total_supply(), 0);
    assert_eq!(client.balance(&user1), 0);

    // Mint valid amount
    client.mint(&user1, &500);
    assert_eq!(client.total_supply(), 500);
    assert_eq!(client.balance(&user1), 500);

    // 5. Zero and negative transfer amounts
    assert_eq!(
        client.try_transfer(&user1, &user2, &0),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_transfer(&user1, &user2, &-1),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(
        client.try_transfer(&user1, &user2, &i128::MIN),
        Err(Ok(TokenError::InvalidAmount))
    );
    assert_eq!(client.balance(&user1), 500);
    assert_eq!(client.balance(&user2), 0);

    // 6. Insufficient balance transfer
    assert_eq!(
        client.try_transfer(&user1, &user2, &501),
        Err(Ok(TokenError::InsufficientBalance))
    );
    assert_eq!(client.balance(&user1), 500);
    assert_eq!(client.balance(&user2), 0);

    // 7. Querying balance for fresh address returns 0 without panic
    let unused = Address::generate(&env);
    assert_eq!(client.balance(&unused), 0);

    // 8. Public read-only methods run without panic
    assert_eq!(client.decimals(), 7);
    assert_eq!(client.symbol(), String::from_str(&env, "LBT"));
    assert_eq!(
        client.name(),
        String::from_str(&env, "LumenBazaar Test Token")
    );
    assert!(client.utility_only());
}
