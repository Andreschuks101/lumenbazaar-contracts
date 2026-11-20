extern crate std;

use super::*;
use soroban_sdk::vec;

#[derive(Clone, Debug, PartialEq, Eq)]
struct StateSnapshot {
    storage_entries: std::string::String,
    buyer_balance: i128,
    seller_balance: i128,
    contract_balance: i128,
    liability: i128,
    event_count: usize,
}

struct PanicFreeFixture {
    env: Env,
    contract_id: Address,
    _admin: Address,
    buyer: Address,
    seller: Address,
    asset: Address,
    token_client: TestTokenContractClient<'static>,
}

impl PanicFreeFixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();

        let contract_id = env.register(UptoSessionContract, ());
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let token_id = env.register(TestTokenContract, ());
        let token_client = TestTokenContractClient::new(&env, &token_id);
        token_client.initialize(&token_admin);
        token_client.mint(&buyer, &100_000);

        let client = UptoSessionContractClient::new(&env, &contract_id);
        let policies = vec![
            &env,
            AssetPolicy {
                asset: token_id.clone(),
                max_session_amount: 1_000,
                min_session_amount: 1,
                max_asset_exposure: 100_000,
                max_buyer_exposure: 100_000,
            },
        ];
        client.initialize(&admin, &policies);

        // Safe transmute of lifetime to 'static for holding in fixture
        let static_client = unsafe {
            core::mem::transmute::<TestTokenContractClient<'_>, TestTokenContractClient<'static>>(
                token_client,
            )
        };

        Self {
            env,
            contract_id,
            _admin: admin,
            buyer,
            seller,
            asset: token_id,
            token_client: static_client,
        }
    }

    fn client(&self) -> UptoSessionContractClient<'_> {
        UptoSessionContractClient::new(&self.env, &self.contract_id)
    }

    fn capture_state(&self) -> StateSnapshot {
        let snap = self.env.to_ledger_snapshot();
        let filtered: std::vec::Vec<_> = snap
            .ledger_entries
            .into_iter()
            .filter(|(k, entry)| match &**k {
                soroban_sdk::xdr::LedgerKey::ContractData(cd) => {
                    if matches!(cd.key, soroban_sdk::xdr::ScVal::LedgerKeyNonce(_)) {
                        return false;
                    }
                    if matches!(cd.key, soroban_sdk::xdr::ScVal::LedgerKeyContractInstance) {
                        if let soroban_sdk::xdr::LedgerEntryData::ContractData(cde) = &entry.0.data
                        {
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
        let buyer_balance = self.token_client.balance(&self.buyer);
        let seller_balance = self.token_client.balance(&self.seller);
        let contract_balance = self.token_client.balance(&self.contract_id);
        let liability = self.env.as_contract(&self.contract_id, || {
            storage::read_liability(&self.env, &self.asset)
        });
        let event_count = self.env.events().all().events().len();

        StateSnapshot {
            storage_entries,
            buyer_balance,
            seller_balance,
            contract_balance,
            liability,
            event_count,
        }
    }

    fn assert_unchanged(&self, before: &StateSnapshot) {
        let after = self.capture_state();
        assert_eq!(
            before, &after,
            "State, balances, liability, or event log mutated during failed call!"
        );
    }

    fn create_open_session(&self, max_amount: i128, duration: u32) -> (BytesN<32>, u32) {
        let current_ledger = self.env.ledger().sequence();
        let expires_at_ledger = current_ledger + duration;
        let resource_hash = BytesN::from_array(&self.env, &[42; 32]);
        let session_id = self.client().create_session(
            &self.buyer,
            &self.seller,
            &self.asset,
            &max_amount,
            &expires_at_ledger,
            &resource_hash,
            &next_idempotency_key(&self.env),
        );
        (session_id, expires_at_ledger)
    }
}

#[test]
fn panic_free_create_session() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let current_ledger = f.env.ledger().sequence();
    let resource_hash = BytesN::from_array(&f.env, &[1; 32]);
    let empty_hash = BytesN::from_array(&f.env, &[0; 32]);

    // 1. Amount 0
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &f.asset,
        &0,
        &(current_ledger + 50),
        &resource_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    f.assert_unchanged(&before);

    // 2. Negative amount
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &f.asset,
        &-10,
        &(current_ledger + 50),
        &resource_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    f.assert_unchanged(&before);

    // 3. i128::MAX (exceeds asset cap 1_000)
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &f.asset,
        &i128::MAX,
        &(current_ledger + 50),
        &resource_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::AmountExceedsCap)));
    f.assert_unchanged(&before);

    // 4. Duration 0 (expires_at <= current_ledger)
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &f.asset,
        &100,
        &current_ledger,
        &resource_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::ExpiredSession)));
    f.assert_unchanged(&before);

    // 5. Duration > MAX_SESSION_DURATION
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &f.asset,
        &100,
        &(current_ledger + validation::MAX_SESSION_DURATION_LEDGERS + 1),
        &resource_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::SessionDurationTooLong)));
    f.assert_unchanged(&before);

    // 6. Empty resource hash
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &f.asset,
        &100,
        &(current_ledger + 50),
        &empty_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::InvalidResourceHash)));
    f.assert_unchanged(&before);

    // 7. Unsupported asset
    let unsupported = Address::generate(&f.env);
    let before = f.capture_state();
    let res = client.try_create_session(
        &f.buyer,
        &f.seller,
        &unsupported,
        &100,
        &(current_ledger + 50),
        &resource_hash,
        &next_idempotency_key(&f.env),
    );
    assert_eq!(res, Err(Ok(ContractError::UnsupportedAsset)));
    f.assert_unchanged(&before);

    // 8. Uninitialized contract
    let fresh_env = Env::default();
    fresh_env.mock_all_auths_allowing_non_root_auth();
    let uninit_id = fresh_env.register(UptoSessionContract, ());
    let uninit_client = UptoSessionContractClient::new(&fresh_env, &uninit_id);
    let snap_before = fresh_env.to_ledger_snapshot();
    let uninit_resource_hash = BytesN::from_array(&fresh_env, &[1; 32]);
    let res = uninit_client.try_create_session(
        &Address::generate(&fresh_env),
        &Address::generate(&fresh_env),
        &Address::generate(&fresh_env),
        &100,
        &50,
        &uninit_resource_hash,
        &next_idempotency_key(&fresh_env),
    );
    assert_eq!(res, Err(Ok(ContractError::NotInitialized)));
    let snap_after = fresh_env.to_ledger_snapshot();
    assert_eq!(
        snap_before.ledger_entries.len(),
        snap_after.ledger_entries.len()
    );

    // 9. Unapproved caller: fails closed with host auth error without panicking
    let no_auth_env = Env::default();
    let no_auth_contract = no_auth_env.register(UptoSessionContract, ());
    let no_auth_client = UptoSessionContractClient::new(&no_auth_env, &no_auth_contract);
    let admin = Address::generate(&no_auth_env);
    let token_admin = Address::generate(&no_auth_env);
    let token_id = no_auth_env.register(TestTokenContract, ());
    let token_client = TestTokenContractClient::new(&no_auth_env, &token_id);
    token_client
        .mock_auths(&[MockAuth {
            address: &token_admin,
            invoke: &MockAuthInvoke {
                contract: &token_id,
                fn_name: "initialize",
                args: (token_admin.clone(),).into_val(&no_auth_env),
                sub_invokes: &[],
            },
        }])
        .initialize(&token_admin);

    let buyer = Address::generate(&no_auth_env);
    let seller = Address::generate(&no_auth_env);
    let policies = vec![
        &no_auth_env,
        AssetPolicy {
            asset: token_id.clone(),
            max_session_amount: 1_000,
            min_session_amount: 1,
            max_asset_exposure: 100_000,
            max_buyer_exposure: 100_000,
        },
    ];
    no_auth_client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &no_auth_contract,
                fn_name: "initialize",
                args: (admin.clone(), policies.clone()).into_val(&no_auth_env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin, &policies);

    let snap_before = no_auth_env.to_ledger_snapshot();
    let no_auth_resource_hash = BytesN::from_array(&no_auth_env, &[1; 32]);
    let res = no_auth_client.try_create_session(
        &buyer,
        &seller,
        &token_id,
        &100,
        &50,
        &no_auth_resource_hash,
        &next_idempotency_key(&no_auth_env),
    );
    assert!(res.is_err());
    let snap_after = no_auth_env.to_ledger_snapshot();
    assert_eq!(
        snap_before.ledger_entries.len(),
        snap_after.ledger_entries.len()
    );
}

#[test]
fn panic_free_settle() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let (session_id, _) = f.create_open_session(500, 100);
    let usage_hash = BytesN::from_array(&f.env, &[7; 32]);
    let empty_hash = BytesN::from_array(&f.env, &[0; 32]);

    // 1. Amount 0
    let before = f.capture_state();
    let res = client.try_settle(&session_id, &0, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    f.assert_unchanged(&before);

    // 2. Negative amount
    let before = f.capture_state();
    let res = client.try_settle(&session_id, &-1, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    f.assert_unchanged(&before);

    // 3. Amount > max_amount (501 > 500)
    let before = f.capture_state();
    let res = client.try_settle(&session_id, &501, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::AmountExceedsCap)));
    f.assert_unchanged(&before);

    // 4. Empty usage hash
    let before = f.capture_state();
    let res = client.try_settle(&session_id, &250, &empty_hash);
    assert_eq!(res, Err(Ok(ContractError::InvalidUsageHash)));
    f.assert_unchanged(&before);

    // 5. Non-existent session
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    let res = client.try_settle(&random_id, &250, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::SessionNotFound)));
    f.assert_unchanged(&before);

    // 6. Expired session
    let (exp_session_id, exp_expiry) = f.create_open_session(300, 20);
    f.env.ledger().set_sequence_number(exp_expiry + 1);
    let before = f.capture_state();
    let res = client.try_settle(&exp_session_id, &200, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::ExpiredSession)));
    f.assert_unchanged(&before);

    // 7. Already settled session
    let (settled_session_id, _) = f.create_open_session(400, 200);
    client.settle(&settled_session_id, &200, &usage_hash);
    let before = f.capture_state();
    let res = client.try_settle(&settled_session_id, &100, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::SessionAlreadySettled)));
    f.assert_unchanged(&before);

    // 8. Already cancelled session
    let (cancelled_session_id, _) = f.create_open_session(400, 200);
    client.cancel(&cancelled_session_id);
    let before = f.capture_state();
    let res = client.try_settle(&cancelled_session_id, &100, &usage_hash);
    assert_eq!(res, Err(Ok(ContractError::SessionCancelled)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_cancel() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let usage_hash = BytesN::from_array(&f.env, &[7; 32]);

    // 1. Non-existent session
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    let res = client.try_cancel(&random_id);
    assert_eq!(res, Err(Ok(ContractError::SessionNotFound)));
    f.assert_unchanged(&before);

    // 2. Already cancelled
    let (session_id, _) = f.create_open_session(500, 100);
    client.cancel(&session_id);
    let before = f.capture_state();
    let res = client.try_cancel(&session_id);
    assert_eq!(res, Err(Ok(ContractError::SessionCancelled)));
    f.assert_unchanged(&before);

    // 3. Already settled
    let (session_id_2, _) = f.create_open_session(500, 100);
    client.settle(&session_id_2, &300, &usage_hash);
    let before = f.capture_state();
    let res = client.try_cancel(&session_id_2);
    assert_eq!(res, Err(Ok(ContractError::SessionAlreadySettled)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_extend_session_expiry() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let current_ledger = f.env.ledger().sequence();
    let (session_id, expires_at_ledger) = f.create_open_session(500, 100);

    // 1. Extension <= current expiry
    let before = f.capture_state();
    let res = client.try_extend_session_expiry(&session_id, &expires_at_ledger);
    assert_eq!(res, Err(Ok(ContractError::ExpiredSession)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_extend_session_expiry(&session_id, &(expires_at_ledger - 1));
    assert_eq!(res, Err(Ok(ContractError::ExpiredSession)));
    f.assert_unchanged(&before);

    // 2. Extension > MAX_SESSION_DURATION
    let before = f.capture_state();
    let res = client.try_extend_session_expiry(
        &session_id,
        &(current_ledger + validation::MAX_SESSION_DURATION_LEDGERS + 1),
    );
    assert_eq!(res, Err(Ok(ContractError::SessionDurationTooLong)));
    f.assert_unchanged(&before);

    // 3. Non-existent session
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    let res = client.try_extend_session_expiry(&random_id, &(expires_at_ledger + 50));
    assert_eq!(res, Err(Ok(ContractError::SessionNotFound)));
    f.assert_unchanged(&before);

    // 4. Finalized session (settled)
    let usage_hash = BytesN::from_array(&f.env, &[7; 32]);
    let (session_id_2, exp2) = f.create_open_session(500, 100);
    client.settle(&session_id_2, &200, &usage_hash);
    let before = f.capture_state();
    let res = client.try_extend_session_expiry(&session_id_2, &(exp2 + 50));
    assert_eq!(res, Err(Ok(ContractError::SessionAlreadySettled)));
    f.assert_unchanged(&before);

    // 5. Finalized session (cancelled)
    let (session_id_3, exp3) = f.create_open_session(500, 100);
    client.cancel(&session_id_3);
    let before = f.capture_state();
    let res = client.try_extend_session_expiry(&session_id_3, &(exp3 + 50));
    assert_eq!(res, Err(Ok(ContractError::SessionCancelled)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_reduce_session_cap() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let (session_id, _) = f.create_open_session(500, 100);

    // 1. New cap <= 0
    let before = f.capture_state();
    let res = client.try_reduce_session_cap(&session_id, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidCapReduction)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_reduce_session_cap(&session_id, &-10);
    assert_eq!(res, Err(Ok(ContractError::InvalidCapReduction)));
    f.assert_unchanged(&before);

    // 2. New cap >= current cap (500)
    let before = f.capture_state();
    let res = client.try_reduce_session_cap(&session_id, &500);
    assert_eq!(res, Err(Ok(ContractError::InvalidCapReduction)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_reduce_session_cap(&session_id, &600);
    assert_eq!(res, Err(Ok(ContractError::InvalidCapReduction)));
    f.assert_unchanged(&before);

    // 3. Non-existent session
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    let res = client.try_reduce_session_cap(&random_id, &250);
    assert_eq!(res, Err(Ok(ContractError::SessionNotFound)));
    f.assert_unchanged(&before);

    // 4. Finalized session
    let (session_id_2, _) = f.create_open_session(500, 100);
    client.cancel(&session_id_2);
    let before = f.capture_state();
    let res = client.try_reduce_session_cap(&session_id_2, &250);
    assert_eq!(res, Err(Ok(ContractError::SessionCancelled)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_reject() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let usage_hash = BytesN::from_array(&f.env, &[7; 32]);

    // 1. Non-existent session
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    let res = client.try_reject(&random_id);
    assert_eq!(res, Err(Ok(ContractError::SessionNotFound)));
    f.assert_unchanged(&before);

    // 2. Finalized session (settled)
    let (session_id, _) = f.create_open_session(500, 100);
    client.settle(&session_id, &200, &usage_hash);
    let before = f.capture_state();
    let res = client.try_reject(&session_id);
    assert_eq!(res, Err(Ok(ContractError::SessionAlreadySettled)));
    f.assert_unchanged(&before);

    // 3. Finalized session (cancelled)
    let (session_id_2, _) = f.create_open_session(500, 100);
    client.cancel(&session_id_2);
    let before = f.capture_state();
    let res = client.try_reject(&session_id_2);
    assert_eq!(res, Err(Ok(ContractError::SessionCancelled)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_recover_expired() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let (session_id, expires_at_ledger) = f.create_open_session(500, 100);

    // 1. Session not expired
    let before = f.capture_state();
    let res = client.try_recover_expired(&session_id);
    assert_eq!(res, Err(Ok(ContractError::SessionNotExpired)));
    f.assert_unchanged(&before);

    // 2. Non-existent session
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    let res = client.try_recover_expired(&random_id);
    assert_eq!(res, Err(Ok(ContractError::SessionNotFound)));
    f.assert_unchanged(&before);

    // 3. Already recovered
    f.env.ledger().set_sequence_number(expires_at_ledger + 1);
    client.recover_expired(&session_id);
    let before = f.capture_state();
    let res = client.try_recover_expired(&session_id);
    assert_eq!(res, Err(Ok(ContractError::SessionExpired)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_activate_asset() {
    let f = PanicFreeFixture::new();
    let client = f.client();

    // 1. Duplicate asset (already active)
    let before = f.capture_state();
    let res = client.try_activate_asset(&AssetPolicy {
        asset: f.asset.clone(),
        max_session_amount: 1_000,
        min_session_amount: 1,
        max_asset_exposure: 100_000,
        max_buyer_exposure: 100_000,
    });
    assert_eq!(res, Err(Ok(ContractError::AssetAlreadyActive)));
    f.assert_unchanged(&before);

    // 2. Zero / invalid cap
    let new_token = create_local_test_token(&f.env, &f.buyer, 1_000);
    let before = f.capture_state();
    let res = client.try_activate_asset(&AssetPolicy {
        asset: new_token.clone(),
        max_session_amount: 0,
        min_session_amount: 1,
        max_asset_exposure: 100_000,
        max_buyer_exposure: 100_000,
    });
    assert_eq!(res, Err(Ok(ContractError::InvalidAssetCap)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_activate_asset(&AssetPolicy {
        asset: new_token.clone(),
        max_session_amount: -1,
        min_session_amount: 1,
        max_asset_exposure: 100_000,
        max_buyer_exposure: 100_000,
    });
    assert_eq!(res, Err(Ok(ContractError::InvalidAssetCap)));
    f.assert_unchanged(&before);

    // 3. Over limit: fill up to 16 assets, then 17th fails with AssetLimitReached
    for _ in 1..MAX_SUPPORTED_ASSETS {
        let t = create_local_test_token(&f.env, &f.buyer, 1_000);
        client.activate_asset(&AssetPolicy {
            asset: t,
            max_session_amount: 1_000,
            min_session_amount: 1,
            max_asset_exposure: 100_000,
            max_buyer_exposure: 100_000,
        });
    }

    let extra_token = create_local_test_token(&f.env, &f.buyer, 1_000);
    let before = f.capture_state();
    let res = client.try_activate_asset(&AssetPolicy {
        asset: extra_token,
        max_session_amount: 1_000,
        min_session_amount: 1,
        max_asset_exposure: 100_000,
        max_buyer_exposure: 100_000,
    });
    assert_eq!(res, Err(Ok(ContractError::AssetLimitReached)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_deprecate_asset() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let reason_hash = BytesN::from_array(&f.env, &[9; 32]);

    // 1. Unsupported asset
    let unsupported = Address::generate(&f.env);
    let before = f.capture_state();
    let res = client.try_deprecate_asset(&unsupported, &reason_hash);
    assert_eq!(res, Err(Ok(ContractError::AssetNotActive)));
    f.assert_unchanged(&before);

    // 2. Already deprecated
    client.deprecate_asset(&f.asset, &reason_hash);
    let before = f.capture_state();
    let res = client.try_deprecate_asset(&f.asset, &reason_hash);
    assert_eq!(res, Err(Ok(ContractError::AssetNotActive)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_remove_asset() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let reason_hash = BytesN::from_array(&f.env, &[9; 32]);

    // 1. Unsupported asset
    let unsupported = Address::generate(&f.env);
    let before = f.capture_state();
    let res = client.try_remove_asset(&unsupported, &reason_hash);
    assert_eq!(res, Err(Ok(ContractError::AssetNotDeprecated)));
    f.assert_unchanged(&before);

    // 2. Not deprecated (active)
    let before = f.capture_state();
    let res = client.try_remove_asset(&f.asset, &reason_hash);
    assert_eq!(res, Err(Ok(ContractError::AssetNotDeprecated)));
    f.assert_unchanged(&before);

    // 3. Liability > 0: create session, deprecate asset, then remove fails with AssetLiabilityOutstanding
    f.create_open_session(500, 100);
    client.deprecate_asset(&f.asset, &reason_hash);
    let before = f.capture_state();
    let res = client.try_remove_asset(&f.asset, &reason_hash);
    assert_eq!(res, Err(Ok(ContractError::AssetLiabilityOutstanding)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_set_asset_max_session_amount() {
    let f = PanicFreeFixture::new();
    let client = f.client();

    // 1. Amount 0
    let before = f.capture_state();
    let res = client.try_set_asset_max_session_amount(&f.asset, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidAssetCap)));
    f.assert_unchanged(&before);

    // 2. Negative amount
    let before = f.capture_state();
    let res = client.try_set_asset_max_session_amount(&f.asset, &-1);
    assert_eq!(res, Err(Ok(ContractError::InvalidAssetCap)));
    f.assert_unchanged(&before);

    // 3. Unconfigured asset
    let unsupported = Address::generate(&f.env);
    let before = f.capture_state();
    let res = client.try_set_asset_max_session_amount(&unsupported, &500);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedAsset)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_exposure_and_session_count_limits() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let unsupported = Address::generate(&f.env);

    // set_asset_exposure_ceiling
    let before = f.capture_state();
    let res = client.try_set_asset_exposure_ceiling(&f.asset, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidExposureCeiling)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_set_asset_exposure_ceiling(&f.asset, &-1);
    assert_eq!(res, Err(Ok(ContractError::InvalidExposureCeiling)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_set_asset_exposure_ceiling(&unsupported, &1_000);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedAsset)));
    f.assert_unchanged(&before);

    // set_buyer_exposure_ceiling
    let before = f.capture_state();
    let res = client.try_set_buyer_exposure_ceiling(&f.asset, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidExposureCeiling)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_set_buyer_exposure_ceiling(&f.asset, &-1);
    assert_eq!(res, Err(Ok(ContractError::InvalidExposureCeiling)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_set_buyer_exposure_ceiling(&unsupported, &1_000);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedAsset)));
    f.assert_unchanged(&before);

    // set_seller_exposure_ceiling
    let before = f.capture_state();
    let res = client.try_set_seller_exposure_ceiling(&f.asset, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidAssetCap)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_set_seller_exposure_ceiling(&f.asset, &-1);
    assert_eq!(res, Err(Ok(ContractError::InvalidAssetCap)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_set_seller_exposure_ceiling(&unsupported, &1_000);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedAsset)));
    f.assert_unchanged(&before);

    // set_max_buyer_open_sessions
    let before = f.capture_state();
    let res = client.try_set_max_buyer_open_sessions(&0);
    assert_eq!(res, Err(Ok(ContractError::InvalidSessionCountLimit)));
    f.assert_unchanged(&before);

    // set_max_seller_open_sessions
    let before = f.capture_state();
    let res = client.try_set_max_seller_open_sessions(&0);
    assert_eq!(res, Err(Ok(ContractError::InvalidSessionCountLimit)));
    f.assert_unchanged(&before);

    // uninitialized for open sessions limits
    let fresh_env = Env::default();
    fresh_env.mock_all_auths_allowing_non_root_auth();
    let uninit_id = fresh_env.register(UptoSessionContract, ());
    let uninit_client = UptoSessionContractClient::new(&fresh_env, &uninit_id);

    let res = uninit_client.try_set_max_buyer_open_sessions(&10);
    assert_eq!(res, Err(Ok(ContractError::NotInitialized)));

    let res = uninit_client.try_set_max_seller_open_sessions(&10);
    assert_eq!(res, Err(Ok(ContractError::NotInitialized)));
}

#[test]
fn panic_free_recover_surplus() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let recipient = Address::generate(&f.env);

    // 1. Amount <= 0
    let before = f.capture_state();
    let res = client.try_recover_surplus(&f.asset, &recipient, &0);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    f.assert_unchanged(&before);

    let before = f.capture_state();
    let res = client.try_recover_surplus(&f.asset, &recipient, &-1);
    assert_eq!(res, Err(Ok(ContractError::InvalidAmount)));
    f.assert_unchanged(&before);

    // 2. Amount > surplus (contract balance == liability == 0, so surplus == 0)
    let before = f.capture_state();
    let res = client.try_recover_surplus(&f.asset, &recipient, &100);
    assert_eq!(res, Err(Ok(ContractError::EscrowUnderfunded)));
    f.assert_unchanged(&before);
}

#[test]
fn panic_free_queries() {
    let f = PanicFreeFixture::new();
    let client = f.client();
    let unsupported = Address::generate(&f.env);

    let fresh_env = Env::default();
    let uninit_id = fresh_env.register(UptoSessionContract, ());
    let uninit_client = UptoSessionContractClient::new(&fresh_env, &uninit_id);
    let uninit_asset = Address::generate(&fresh_env);

    // get_configuration uninitialized
    assert_eq!(
        uninit_client.try_get_configuration(),
        Err(Ok(ContractError::NotInitialized))
    );

    // get_escrow_liability
    assert_eq!(
        uninit_client.try_get_escrow_liability(&uninit_asset),
        Err(Ok(ContractError::NotInitialized))
    );
    let before = f.capture_state();
    assert_eq!(
        client.try_get_escrow_liability(&unsupported),
        Err(Ok(ContractError::UnsupportedAsset))
    );
    f.assert_unchanged(&before);

    // get_solvency
    assert_eq!(
        uninit_client.try_get_solvency(&vec![&fresh_env, uninit_asset]),
        Err(Ok(ContractError::NotInitialized))
    );
    let before = f.capture_state();
    assert_eq!(
        client.try_get_solvency(&vec![&f.env]),
        Err(Ok(ContractError::InvalidBatchSize))
    );
    f.assert_unchanged(&before);

    // get_session not found
    let random_id = BytesN::from_array(&f.env, &[99; 32]);
    let before = f.capture_state();
    assert_eq!(
        client.try_get_session(&random_id),
        Err(Ok(ContractError::SessionNotFound))
    );
    f.assert_unchanged(&before);
}
