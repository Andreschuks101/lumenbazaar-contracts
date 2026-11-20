/// Mutation tests for security-critical guards in the upto-session contract.
///
/// Each test is a *specification* of a killed mutant: the test should fail if
/// and only if the named security check is removed or inverted.  The suite is
/// deliberately narrow in scope – every test uses explicit `mock_auths` (not
/// `mock_all_auths`) so that a removed `require_auth` call can be detected
/// without any SDK-level bypass.
///
/// Out of scope: generated bindings, test-only helpers, resource-usage probes,
/// and property-test infrastructure.  Those files are excluded from any
/// cargo-mutants run via the configuration in `.cargo-mutants.toml`.
extern crate std;

use crate::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    token, Address, BytesN, Env, IntoVal,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn stellar_asset(env: &Env, owner: &Address, amount: i128) -> Address {
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
                args: (owner.clone(), amount).into_val(env),
                sub_invokes: &[],
            },
        }])
        .mint(owner, &amount);
    asset
}

fn initialized_contract<'a>(
    env: &'a Env,
    buyer: &Address,
    asset: &Address,
    cap: i128,
) -> (Address, UptoSessionContractClient<'a>) {
    let admin = buyer.clone(); // admin == buyer keeps explicit auth trees simple
    let contract = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(env, &contract);
    let policies = soroban_sdk::vec![
        env,
        AssetPolicy {
            asset: asset.clone(),
            max_session_amount: cap,
            min_session_amount: 1,
            max_asset_exposure: i128::MAX,
            max_buyer_exposure: i128::MAX,
        }
    ];
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "initialize",
                args: (admin.clone(), policies.clone()).into_val(env),
                sub_invokes: &[],
            },
        }])
        .initialize(&admin, &policies);
    (contract, client)
}

/// Create a session using an explicit buyer authorization tree (no mock_all_auths).
#[allow(clippy::too_many_arguments)]
fn create_with_buyer_auth(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    buyer: &Address,
    seller: &Address,
    asset: &Address,
    cap: i128,
    expires: u32,
    resource_hash: &BytesN<32>,
) -> BytesN<32> {
    let idempotency_key = resource_hash.clone();
    client
        .mock_auths(&[MockAuth {
            address: buyer,
            invoke: &MockAuthInvoke {
                contract,
                fn_name: "create_session",
                args: (
                    buyer.clone(),
                    seller.clone(),
                    asset.clone(),
                    cap,
                    expires,
                    resource_hash.clone(),
                    idempotency_key.clone(),
                )
                    .into_val(env),
                sub_invokes: &[MockAuthInvoke {
                    contract: asset,
                    fn_name: "transfer",
                    args: (buyer.clone(), contract.clone(), cap).into_val(env),
                    sub_invokes: &[],
                }],
            },
        }])
        .create_session(
            buyer,
            seller,
            asset,
            &cap,
            &expires,
            resource_hash,
            &idempotency_key,
        )
}

/// Settle a session using an explicit seller authorization tree.
fn settle_with_seller_auth(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    seller: &Address,
    session_id: &BytesN<32>,
    amount: i128,
    usage_hash: &BytesN<32>,
) {
    client
        .mock_auths(&[MockAuth {
            address: seller,
            invoke: &MockAuthInvoke {
                contract,
                fn_name: "settle",
                args: (session_id.clone(), amount, usage_hash.clone()).into_val(env),
                sub_invokes: &[],
            },
        }])
        .settle(session_id, &amount, usage_hash);
}

/// Cancel a session using an explicit buyer authorization tree.
fn cancel_with_buyer_auth(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    buyer: &Address,
    session_id: &BytesN<32>,
) {
    client
        .mock_auths(&[MockAuth {
            address: buyer,
            invoke: &MockAuthInvoke {
                contract,
                fn_name: "cancel",
                args: (session_id.clone(),).into_val(env),
                sub_invokes: &[],
            },
        }])
        .cancel(session_id);
}

// ── MUTATION TARGET A: buyer.require_auth() in create_session ─────────────────
//
// Mutant: removal of `buyer.require_auth()` → any caller can escrow tokens on
//         behalf of an arbitrary buyer without their consent.
//
// Killing strategy: call create_session with no auths at all and assert failure.
// A surviving mutant would create the session and drain the buyer's balance.

#[test]
fn mutation_buyer_auth_removal_in_create_session_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa1; 32]);

    // No mock_auths — the contract must enforce buyer.require_auth() itself.
    let result = client.try_create_session(
        &buyer,
        &seller,
        &asset,
        &100,
        &50,
        &resource_hash,
        &BytesN::from_array(&env, &[0xb0; 32]),
    );

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: buyer.require_auth() in create_session was not enforced; \
         a session was created without buyer authorization"
    );
    // No funds should have moved and no session sequence should have advanced.
    assert_eq!(
        token::Client::new(&env, &asset).balance(&buyer),
        500,
        "buyer balance was modified without authorization"
    );
    assert_eq!(
        token::Client::new(&env, &asset).balance(&contract),
        0,
        "escrow received funds without authorization"
    );
    env.as_contract(&contract, || {
        assert_eq!(
            storage::read_next_session_sequence(&env),
            0,
            "session sequence advanced without authorization"
        );
    });
}

/// A wrong party presenting buyer's authorization should also fail.
#[test]
fn mutation_buyer_auth_wrong_party_in_create_session_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let attacker = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa2; 32]);

    let idempotency_key = BytesN::from_array(&env, &[0xb1; 32]);

    // Attacker provides their own auth for buyer's create_session — must fail.
    let result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "create_session",
                args: (
                    buyer.clone(),
                    seller.clone(),
                    asset.clone(),
                    100_i128,
                    50_u32,
                    resource_hash.clone(),
                    idempotency_key.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_create_session(
            &buyer,
            &seller,
            &asset,
            &100,
            &50,
            &resource_hash,
            &idempotency_key,
        );

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: attacker authorization was accepted for buyer's create_session"
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 500);
    assert_eq!(token::Client::new(&env, &asset).balance(&contract), 0);
}

// ── MUTATION TARGET B: seller.require_auth() in settle ───────────────────────
//
// Mutant: removal of `session.seller.require_auth()` → anyone can settle a
//         session and redirect funds.
//
// Killing strategy: inject an Open session directly into storage (bypassing
// create_session so we isolate the settle auth path), then call settle with
// no auths at all.

#[test]
fn mutation_seller_auth_removal_in_settle_is_caught() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let session_id = BytesN::from_array(&env, &[0xb1; 32]);
    let resource_hash = BytesN::from_array(&env, &[0xb2; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xb3; 32]);

    // Write a fully valid Open session directly into storage.
    env.as_contract(&contract, || {
        let _ = storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer: buyer.clone(),
                seller: seller.clone(),
                asset: asset.clone(),
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 1,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        );
    });

    // No mock_auths — the contract must enforce seller.require_auth().
    let result = client.try_settle(&session_id, &50, &usage_hash);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: seller.require_auth() in settle was not enforced; \
         settle succeeded without seller authorization"
    );
    // Session must remain Open.
    let session = client.get_session(&session_id);
    assert_eq!(
        session.status,
        SessionStatus::Open,
        "session status changed without seller authorization"
    );
    assert_eq!(
        session.settled_amount, 0,
        "settled_amount changed without seller authorization"
    );
}

/// Wrong seller providing authorization must fail, and session must stay open.
#[test]
fn mutation_seller_auth_wrong_party_in_settle_is_caught() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let wrong_seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let session_id = BytesN::from_array(&env, &[0xb4; 32]);
    let resource_hash = BytesN::from_array(&env, &[0xb5; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xb6; 32]);

    env.as_contract(&contract, || {
        let _ = storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer: buyer.clone(),
                seller: seller.clone(),
                asset: asset.clone(),
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 1,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        );
    });

    let result = client
        .mock_auths(&[MockAuth {
            address: &wrong_seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 50_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &50, &usage_hash);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: wrong seller's authorization was accepted for settle"
    );
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);
    assert_eq!(client.get_session(&session_id).settled_amount, 0);
}

// ── MUTATION TARGET C: buyer.require_auth() in cancel ────────────────────────
//
// Mutant: removal of `session.buyer.require_auth()` → seller or third party
//         can cancel an open session, recovering the buyer's escrowed funds
//         to the buyer (griefing) or the cancellation can be triggered at will.

#[test]
fn mutation_buyer_auth_removal_in_cancel_is_caught() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let session_id = BytesN::from_array(&env, &[0xc1; 32]);
    let resource_hash = BytesN::from_array(&env, &[0xc2; 32]);

    env.as_contract(&contract, || {
        let _ = storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer: buyer.clone(),
                seller: seller.clone(),
                asset: asset.clone(),
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 1,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        );
    });

    // No mock_auths — the contract must enforce buyer.require_auth().
    let result = client.try_cancel(&session_id);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: buyer.require_auth() in cancel was not enforced; \
         cancel succeeded without buyer authorization"
    );
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Open,
        "session status changed without buyer authorization"
    );
}

/// Seller providing auth for cancel must be rejected.
#[test]
fn mutation_buyer_auth_seller_cannot_cancel_session() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let session_id = BytesN::from_array(&env, &[0xc3; 32]);
    let resource_hash = BytesN::from_array(&env, &[0xc4; 32]);

    env.as_contract(&contract, || {
        let _ = storage::write_session(
            &env,
            &Session {
                id: session_id.clone(),
                buyer: buyer.clone(),
                seller: seller.clone(),
                asset: asset.clone(),
                max_amount: 100,
                escrowed_amount: 100,
                settled_amount: 0,
                expires_at_ledger: 50,
                resource_hash,
                usage_hash: None,
                status: SessionStatus::Open,
                creation_ledger: 1,
                finalized_ledger: None,
                resource_scheme_version: 1,
                usage_scheme_version: None,
            },
        );
    });

    let result = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "cancel",
                args: (session_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_cancel(&session_id);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: seller was allowed to cancel a session on the buyer's behalf"
    );
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);
}

// ── MUTATION TARGET D: asset-cap check in create_session ─────────────────────
//
// Mutant: inversion of `max_amount > asset_cap` to `max_amount <= asset_cap`
//         (or removal of the check) → a session exceeding the configured cap
//         is allowed.
//
// Killing strategy: create a session with max_amount == cap + 1 and assert
// AmountExceedsCap is returned.

#[test]
fn mutation_asset_cap_inversion_in_create_session_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 1_000);
    let cap: i128 = 250;
    let (contract, client) = initialized_contract(&env, &buyer, &asset, cap);
    let resource_hash = BytesN::from_array(&env, &[0xd1; 32]);

    // Attempt exactly cap + 1.
    let result = client.try_create_session(
        &buyer,
        &seller,
        &asset,
        &(cap + 1),
        &50,
        &resource_hash,
        &BytesN::from_array(&env, &[0xd0; 32]),
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::AmountExceedsCap)),
        "MUTATION SURVIVED: cap check in create_session did not reject max_amount > asset_cap"
    );
    // Buyer's balance must be untouched — no funds should have moved.
    assert_eq!(
        token::Client::new(&env, &asset).balance(&buyer),
        1_000,
        "buyer funds were taken despite cap violation"
    );
    assert_eq!(
        token::Client::new(&env, &asset).balance(&contract),
        0,
        "escrow received funds despite cap violation"
    );
}

/// Exactly at the cap boundary must succeed (guards the boundary from off-by-one mutation).
#[test]
fn mutation_asset_cap_exact_boundary_succeeds() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let cap: i128 = 200;
    let (contract, client) = initialized_contract(&env, &buyer, &asset, cap);
    let resource_hash = BytesN::from_array(&env, &[0xd2; 32]);

    // Exactly at the cap — must succeed.
    let session_id = create_with_buyer_auth(
        &env,
        &client,
        &contract,
        &buyer,
        &seller,
        &asset,
        cap,
        50,
        &resource_hash,
    );
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Open,
        "session at exact cap was rejected — off-by-one mutation may have been introduced"
    );
}

// ── MUTATION TARGET E: settlement amount cap in validate_settlement_amount ────
//
// Mutant: inversion of `actual_amount > session.max_amount` → settlement
//         above max_amount is permitted, letting the seller overdraw escrow.

#[test]
fn mutation_settlement_cap_inversion_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xe1; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xe2; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // Attempt to settle for max_amount + 1 (501 > 500).
    let result = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 501_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &501, &usage_hash);

    assert_eq!(
        result,
        Err(Ok(ContractError::AmountExceedsCap)),
        "MUTATION SURVIVED: settlement cap check did not reject actual_amount > max_amount"
    );
    // Session must remain open and untouched.
    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Open);
    assert_eq!(session.settled_amount, 0);
    // Funds must still be escrowed.
    assert_eq!(token::Client::new(&env, &asset).balance(&contract), 500);
    assert_eq!(token::Client::new(&env, &asset).balance(&seller), 0);
}

/// Settling for exactly max_amount must succeed (kills off-by-one mutants on >=).
#[test]
fn mutation_settlement_cap_exact_boundary_succeeds() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xe3; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xe4; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    settle_with_seller_auth(&env, &client, &contract, &seller, &session_id, 500, &usage_hash);

    let session = client.get_session(&session_id);
    assert_eq!(
        session.status,
        SessionStatus::Settled,
        "settlement at exact cap was rejected"
    );
    assert_eq!(session.settled_amount, 500);
    assert_eq!(token::Client::new(&env, &asset).balance(&seller), 500);
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 0);
}

// ── MUTATION TARGET F: settlement expiry check in validate_settlement_amount ──
//
// Mutant: inversion of `ledger >= expires_at_ledger` to `ledger < expires_at_ledger`
//         → settlement is blocked before expiry and allowed at/after expiry.

#[test]
fn mutation_settle_expiry_check_inversion_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xf1; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xf2; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // Advance ledger to the expiry boundary — settle must be rejected here.
    env.ledger().set_sequence_number(50);
    let result = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 100_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &100, &usage_hash);

    assert_eq!(
        result,
        Err(Ok(ContractError::ExpiredSession)),
        "MUTATION SURVIVED: settle expiry check did not fire at ledger == expires_at_ledger"
    );
    // Session must remain Open so recover_expired can clean it up.
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Open
    );
}

/// One ledger before expiry, settle must succeed — kills the inverted mutant
/// that blocks settlement before expiry.
#[test]
fn mutation_settle_expiry_check_before_boundary_succeeds() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xf3; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xf4; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // One ledger before expiry — must still succeed.
    env.ledger().set_sequence_number(49);
    settle_with_seller_auth(&env, &client, &contract, &seller, &session_id, 100, &usage_hash);

    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Settled,
        "settle one ledger before expiry was incorrectly rejected"
    );
}

// ── MUTATION TARGET G: cancel expiry check in cancel ─────────────────────────
//
// Mutant: inversion of `ledger >= expires_at_ledger` to `ledger < expires_at_ledger`
//         → cancel is blocked before expiry and allowed at/after expiry,
//         which would let a buyer cancel an already-expired session and
//         override permissionless recovery.

#[test]
fn mutation_cancel_expiry_check_inversion_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x71; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // At expiry boundary, cancel must be rejected.
    env.ledger().set_sequence_number(50);
    let result = client
        .mock_auths(&[MockAuth {
            address: &buyer,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "cancel",
                args: (session_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_cancel(&session_id);

    assert_eq!(
        result,
        Err(Ok(ContractError::ExpiredSession)),
        "MUTATION SURVIVED: cancel expiry check did not fire at ledger == expires_at_ledger"
    );
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Open
    );
}

/// One ledger before expiry, cancel must succeed — kills the inverted mutant.
#[test]
fn mutation_cancel_expiry_check_before_boundary_succeeds() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x72; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // One ledger before expiry — cancel must succeed.
    env.ledger().set_sequence_number(49);
    cancel_with_buyer_auth(&env, &client, &contract, &buyer, &session_id);

    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Cancelled,
        "cancel one ledger before expiry was incorrectly rejected"
    );
}

// ── MUTATION TARGET H: not-expired check in recover_expired ──────────────────
//
// Mutant: inversion of `ledger < expires_at_ledger` to `ledger >= expires_at_ledger`
//         → recovery is allowed before expiry, letting anyone prematurely
//         terminate an Open session.

#[test]
fn mutation_recover_not_expired_check_inversion_is_caught() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x81; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // Ledger at 0 — session is not expired; recover must fail.
    let result = client.try_recover_expired(&session_id);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionNotExpired)),
        "MUTATION SURVIVED: recover_expired not-expired check did not fire before expiry"
    );
    // Session must remain Open.
    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Open
    );
    // Buyer's funds must still be escrowed.
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 0);
    assert_eq!(token::Client::new(&env, &asset).balance(&contract), 500);
}

/// At expiry boundary recovery must succeed — kills the inverted mutant.
#[test]
fn mutation_recover_at_expiry_boundary_succeeds() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x82; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    // Advance to exact expiry — recovery must succeed.
    env.ledger().set_sequence_number(50);
    client.recover_expired(&session_id);

    assert_eq!(
        client.get_session(&session_id).status,
        SessionStatus::Expired,
        "recovery at exact expiry boundary was rejected"
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&buyer), 500);
    assert_eq!(token::Client::new(&env, &asset).balance(&contract), 0);
}

// ── MUTATION TARGET I: finalized-state guard in settle ───────────────────────
//
// Mutant: removal or inversion of the `match session.status` guard in settle
//         → a Settled, Cancelled, or Expired session can be re-settled,
//         potentially double-paying the seller.

#[test]
fn mutation_finalized_guard_in_settle_rejects_settled_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x91; 32]);
    let usage_hash = BytesN::from_array(&env, &[0x92; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    settle_with_seller_auth(&env, &client, &contract, &seller, &session_id, 250, &usage_hash);

    // Second settle attempt must be rejected.
    let result = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 250_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &250, &usage_hash);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionAlreadySettled)),
        "MUTATION SURVIVED: finalized guard in settle allowed re-settling a Settled session"
    );
    // Balances must not change a second time.
    assert_eq!(
        token::Client::new(&env, &asset).balance(&seller),
        250,
        "seller received a second payment"
    );
    assert_eq!(
        token::Client::new(&env, &asset).balance(&buyer),
        250,
        "buyer balance was modified by a second settle"
    );
}

#[test]
fn mutation_finalized_guard_in_settle_rejects_cancelled_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x93; 32]);
    let usage_hash = BytesN::from_array(&env, &[0x94; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    cancel_with_buyer_auth(&env, &client, &contract, &buyer, &session_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 100_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &100, &usage_hash);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionCancelled)),
        "MUTATION SURVIVED: settle succeeded on a Cancelled session"
    );
}

#[test]
fn mutation_finalized_guard_in_settle_rejects_expired_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0x95; 32]);
    let usage_hash = BytesN::from_array(&env, &[0x96; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    env.ledger().set_sequence_number(50);
    client.recover_expired(&session_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 100_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &100, &usage_hash);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionExpired)),
        "MUTATION SURVIVED: settle succeeded on an Expired session"
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&seller), 0);
}

// ── MUTATION TARGET J: finalized-state guard in cancel ───────────────────────
//
// Mutant: removal or inversion of the `match session.status` guard in cancel
//         → a Settled or Expired session can be cancelled, potentially
//         re-refunding the buyer from an empty escrow.

#[test]
fn mutation_finalized_guard_in_cancel_rejects_settled_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa3; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xa4; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    settle_with_seller_auth(&env, &client, &contract, &seller, &session_id, 500, &usage_hash);

    let result = client
        .mock_auths(&[MockAuth {
            address: &buyer,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "cancel",
                args: (session_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_cancel(&session_id);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionAlreadySettled)),
        "MUTATION SURVIVED: cancel succeeded on a Settled session"
    );
}

#[test]
fn mutation_finalized_guard_in_cancel_rejects_already_cancelled_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa5; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    cancel_with_buyer_auth(&env, &client, &contract, &buyer, &session_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &buyer,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "cancel",
                args: (session_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_cancel(&session_id);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionCancelled)),
        "MUTATION SURVIVED: cancel succeeded on a Cancelled session"
    );
}

// ── MUTATION TARGET K: finalized-state guard in recover_expired ───────────────
//
// Mutant: removal or inversion of the `match session.status` guard in
//         recover_expired → an already-terminal session can be re-recovered.

#[test]
fn mutation_finalized_guard_in_recover_rejects_settled_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa6; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xa7; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    settle_with_seller_auth(&env, &client, &contract, &seller, &session_id, 100, &usage_hash);

    env.ledger().set_sequence_number(50);
    let result = client.try_recover_expired(&session_id);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionAlreadySettled)),
        "MUTATION SURVIVED: recover_expired succeeded on a Settled session"
    );
}

#[test]
fn mutation_finalized_guard_in_recover_rejects_cancelled_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa8; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );
    cancel_with_buyer_auth(&env, &client, &contract, &buyer, &session_id);

    env.ledger().set_sequence_number(50);
    let result = client.try_recover_expired(&session_id);

    assert_eq!(
        result,
        Err(Ok(ContractError::SessionCancelled)),
        "MUTATION SURVIVED: recover_expired succeeded on a Cancelled session"
    );
}

#[test]
fn mutation_finalized_guard_in_recover_rejects_already_expired_session() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 500);
    let (contract, client) = initialized_contract(&env, &buyer, &asset, 500);
    let resource_hash = BytesN::from_array(&env, &[0xa9; 32]);

    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, 500, 50, &resource_hash,
    );

    env.ledger().set_sequence_number(50);
    client.recover_expired(&session_id);

    // Second recovery must fail.
    let result = client.try_recover_expired(&session_id);
    assert_eq!(
        result,
        Err(Ok(ContractError::SessionExpired)),
        "MUTATION SURVIVED: recover_expired succeeded on an already-Expired session"
    );
}

// ── End-to-end kill confirmation ──────────────────────────────────────────────
//
// This test runs the full create→settle lifecycle with explicit authorization
// trees and verifies that every intermediate auth boundary is enforced together.
// It is the definitive confirmation that no single security-critical guard was
// removed from the happy path.

#[test]
fn mutation_end_to_end_auth_and_cap_guards_are_all_present() {
    let env = Env::default();
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = stellar_asset(&env, &buyer, 1_000);
    let cap: i128 = 500;
    let (contract, client) = initialized_contract(&env, &buyer, &asset, cap);
    let resource_hash = BytesN::from_array(&env, &[0xaa; 32]);
    let usage_hash = BytesN::from_array(&env, &[0xab; 32]);

    // 1. Cap check fires before buyer auth (no auth needed to see the error).
    assert_eq!(
        client.try_create_session(
            &buyer,
            &seller,
            &asset,
            &(cap + 1),
            &50,
            &resource_hash,
            &BytesN::from_array(&env, &[0xac; 32]),
        ),
        Err(Ok(ContractError::AmountExceedsCap)),
        "cap check did not fire"
    );

    // 2. Buyer auth is required for a valid amount.
    assert!(
        client
            .try_create_session(
                &buyer,
                &seller,
                &asset,
                &cap,
                &50,
                &resource_hash,
                &BytesN::from_array(&env, &[0xad; 32]),
            )
            .is_err(),
        "buyer auth was not enforced"
    );

    // 3. Full create with correct buyer auth succeeds.
    let session_id = create_with_buyer_auth(
        &env, &client, &contract, &buyer, &seller, &asset, cap, 50, &resource_hash,
    );
    assert_eq!(client.get_session(&session_id).status, SessionStatus::Open);

    // 4. Settle fails without seller auth.
    assert!(
        client.try_settle(&session_id, &200, &usage_hash).is_err(),
        "seller auth was not enforced for settle"
    );

    // 5. Settle with correct seller auth succeeds.
    settle_with_seller_auth(&env, &client, &contract, &seller, &session_id, 200, &usage_hash);
    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Settled);
    assert_eq!(session.settled_amount, 200);

    // 6. Double-settle is blocked.
    let second_settle = client
        .mock_auths(&[MockAuth {
            address: &seller,
            invoke: &MockAuthInvoke {
                contract: &contract,
                fn_name: "settle",
                args: (session_id.clone(), 200_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle(&session_id, &200, &usage_hash);
    assert_eq!(
        second_settle,
        Err(Ok(ContractError::SessionAlreadySettled)),
        "double-settle was not blocked"
    );

    // 7. Token balances are correct.
    let token = token::Client::new(&env, &asset);
    assert_eq!(token.balance(&buyer), 800);   // refunded 300
    assert_eq!(token.balance(&seller), 200);   // paid 200
    assert_eq!(token.balance(&contract), 0);  // escrow cleared
}
