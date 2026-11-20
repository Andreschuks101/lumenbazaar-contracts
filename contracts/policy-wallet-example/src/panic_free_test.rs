extern crate std;

use super::*;
use soroban_sdk::testutils::Events as _;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WalletStateSnapshot {
    storage_entries: std::string::String,
    spent_today: i128,
    spent_today_seller: i128,
    spent_today_resource: i128,
    authorized_count_today: u32,
    event_count: usize,
}

fn capture_wallet_state(
    env: &Env,
    client: &PolicyWalletExampleContractClient<'_>,
    agent: &Address,
    seller: &Address,
    resource_hash: &BytesN<32>,
) -> WalletStateSnapshot {
    let snap = env.to_ledger_snapshot();
    let filtered: std::vec::Vec<_> = snap
        .ledger_entries
        .into_iter()
        .filter(|(k, entry)| match &**k {
            soroban_sdk::xdr::LedgerKey::ContractData(cd) => {
                if matches!(cd.key, soroban_sdk::xdr::ScVal::LedgerKeyNonce(_)) {
                    return false;
                }
                if matches!(cd.key, soroban_sdk::xdr::ScVal::LedgerKeyContractInstance) {
                    if let soroban_sdk::xdr::LedgerEntryData::ContractData(cde) = &entry.0.data {
                        if let soroban_sdk::xdr::ScVal::ContractInstance(ci) = &cde.val {
                            return ci.storage.is_some();
                        }
                    }
                }
                true
            }
            _ => false,
        })
        .collect();
    let storage_entries = std::format!("{:?}", filtered);
    let spent_today = client.spent_today(agent);
    let spent_today_seller = client.spent_today_seller(agent, seller);
    let spent_today_resource = client.spent_today_resource(agent, resource_hash);
    let authorized_count_today = client.authorized_count_today(agent);
    let event_count = env.events().all().events().len();

    WalletStateSnapshot {
        storage_entries,
        spent_today,
        spent_today_seller,
        spent_today_resource,
        authorized_count_today,
        event_count,
    }
}

fn assert_wallet_unchanged(before: &WalletStateSnapshot, after: &WalletStateSnapshot) {
    assert_eq!(
        before, after,
        "Wallet storage, counters, or events changed during failed call!"
    );
}

#[test]
fn panic_free_initialize_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, _asset, owner, resource_hash) = initialized_wallet(&env);
    let upto_session = Address::generate(&env);

    // 1. Already initialized
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_initialize(&owner, &upto_session);
    assert_eq!(res, Err(Ok(PolicyWalletError::AlreadyInitialized)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 2. Zero limit (zero payment cap, zero daily cap, zero authorizations in policy setup)
    let mut zero_payment_policy =
        standard_policy(&env, &agent, &seller, &_asset, &resource_hash, 100);
    zero_payment_policy.max_amount_per_payment = 0;
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&zero_payment_policy, &0);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidPolicy)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    let mut zero_daily_policy =
        standard_policy(&env, &agent, &seller, &_asset, &resource_hash, 100);
    zero_daily_policy.max_amount_per_day = 0;
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&zero_daily_policy, &0);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidPolicy)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    let mut zero_auth_policy = standard_policy(&env, &agent, &seller, &_asset, &resource_hash, 100);
    zero_auth_policy.max_authorizations_per_day = 0;
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&zero_auth_policy, &0);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidPolicy)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);
}

#[test]
fn panic_free_set_policy_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    // 1. Wrong revision (current revision is 1, passing 0 or 5)
    let policy = standard_policy(&env, &agent, &seller, &asset, &resource_hash, 200);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&policy, &0);
    assert_eq!(res, Err(Ok(PolicyWalletError::StaleRevision)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&policy, &5);
    assert_eq!(res, Err(Ok(PolicyWalletError::StaleRevision)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 2. Negative payment cap
    let mut neg_payment_policy = policy.clone();
    neg_payment_policy.max_amount_per_payment = -1;
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&neg_payment_policy, &1);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidPolicy)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 3. Negative daily cap
    let mut neg_daily_policy = policy.clone();
    neg_daily_policy.max_amount_per_day = -1;
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&neg_daily_policy, &1);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidPolicy)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 4. Invalid validity window (valid_from > valid_until)
    let mut inverted_window_policy = policy.clone();
    inverted_window_policy.valid_from_ledger = 500;
    inverted_window_policy.valid_until_ledger = 100;
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&inverted_window_policy, &1);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidPolicy)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 5. Expired validity window (valid_until <= current_ledger)
    let mut expired_policy = policy;
    expired_policy.valid_until_ledger = env.ledger().sequence();
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_set_policy(&expired_policy, &1);
    assert_eq!(res, Err(Ok(PolicyWalletError::PolicyExpired)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);
}

#[test]
fn panic_free_revoke_policy_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);

    // 1. Non-existent policy
    let non_existent_agent = Address::generate(&env);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_revoke_policy(&non_existent_agent);
    assert_eq!(res, Err(Ok(PolicyWalletError::PolicyNotFound)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 2. Already revoked policy
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);
    client.revoke_policy(&agent);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_revoke_policy(&agent);
    assert_eq!(res, Err(Ok(PolicyWalletError::PolicyRevoked)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);
}

#[test]
fn panic_free_authorize_payment_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    // 1. Unauthorized agent (agent with no policy)
    let unauthorized_agent = Address::generate(&env);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res =
        client.try_authorize_payment(&unauthorized_agent, &seller, &asset, &10, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::PolicyNotFound)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 2. Unallowed seller
    let unallowed_seller = Address::generate(&env);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_authorize_payment(&agent, &unallowed_seller, &asset, &10, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::SellerNotAllowed)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 3. Unallowed asset
    let unallowed_asset = Address::generate(&env);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_authorize_payment(&agent, &seller, &unallowed_asset, &10, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::AssetNotAllowed)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 4. Unallowed resource hash
    let unallowed_hash = BytesN::from_array(&env, &[99; 32]);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_authorize_payment(&agent, &seller, &asset, &10, &unallowed_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::ResourceHashNotAllowed)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 5. Over per-payment cap (cap is 75)
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_authorize_payment(&agent, &seller, &asset, &76, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::AmountExceedsPaymentCap)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 6. Over daily cap (cap is 100, authorize 70 first, then 40 fails)
    client.authorize_payment(&agent, &seller, &asset, &70, &resource_hash);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_authorize_payment(&agent, &seller, &asset, &40, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::AmountExceedsDailyCap)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);
}

#[test]
fn panic_free_check_payment_regression() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, agent, seller, asset, _owner, resource_hash) = initialized_wallet(&env);
    set_standard_policy(&env, &client, &agent, &seller, &asset, &resource_hash);

    // 1. Amount <= 0
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_check_payment(&agent, &seller, &asset, &0, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidAmount)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_check_payment(&agent, &seller, &asset, &-5, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::InvalidAmount)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 2. Unallowed seller
    let unallowed_seller = Address::generate(&env);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_check_payment(&agent, &unallowed_seller, &asset, &10, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::SellerNotAllowed)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 3. Unallowed asset
    let unallowed_asset = Address::generate(&env);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_check_payment(&agent, &seller, &unallowed_asset, &10, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::AssetNotAllowed)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 4. Unallowed resource hash
    let unallowed_hash = BytesN::from_array(&env, &[99; 32]);
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_check_payment(&agent, &seller, &asset, &10, &unallowed_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::ResourceHashNotAllowed)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);

    // 5. Over per-payment cap
    let before = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    let res = client.try_check_payment(&agent, &seller, &asset, &80, &resource_hash);
    assert_eq!(res, Err(Ok(PolicyWalletError::AmountExceedsPaymentCap)));
    let after = capture_wallet_state(&env, &client, &agent, &seller, &resource_hash);
    assert_wallet_unchanged(&before, &after);
}
