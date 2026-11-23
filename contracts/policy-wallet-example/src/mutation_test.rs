/// Mutation tests for security-critical guards in the policy-wallet-example contract.
///
/// Each test names the specific guard it kills so that a surviving mutant is
/// immediately identifiable.  Explicit `mock_auths` (no `mock_all_auths`) are
/// used for every authorization check so that a removed `require_auth` call is
/// detectable.
///
/// Out of scope: `example_only`, `get_config`, `get_policy`, `spent_today`,
/// `check_payment` (read-only), and the `set_policy` validation unit tests
/// that already live in the existing test module.
extern crate std;

use crate::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    vec, Address, BytesN, Env, IntoVal,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn register_and_init(env: &Env) -> (PolicyWalletExampleContractClient<'_>, Address, Address) {
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(env, &contract_id);
    let owner = Address::generate(env);
    let upto_session = Address::generate(env);

    // Use explicit mock_auth so auth-removal mutations are detectable.
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    (client, contract_id, owner)
}

#[allow(clippy::too_many_arguments)]
fn make_policy(
    env: &Env,
    agent: &Address,
    seller: &Address,
    asset: &Address,
    resource_hash: &BytesN<32>,
    max_per_payment: i128,
    max_per_day: i128,
    valid_until_ledger: u32,
) -> SpendingPolicy {
    SpendingPolicy {
        agent: agent.clone(),
        max_amount_per_payment: max_per_payment,
        max_amount_per_day: max_per_day,
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

fn set_policy_with_owner_auth(
    env: &Env,
    client: &PolicyWalletExampleContractClient<'_>,
    contract_id: &Address,
    owner: &Address,
    policy: SpendingPolicy,
) {
    client
        .mock_auths(&[MockAuth {
            address: owner,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);
}

#[allow(clippy::too_many_arguments)]
fn authorize_payment_with_agent_auth(
    env: &Env,
    client: &PolicyWalletExampleContractClient<'_>,
    contract_id: &Address,
    agent: &Address,
    seller: &Address,
    asset: &Address,
    amount: i128,
    resource_hash: &BytesN<32>,
) {
    client
        .mock_auths(&[MockAuth {
            address: agent,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name: "authorize_payment",
                args: (
                    agent.clone(),
                    seller.clone(),
                    asset.clone(),
                    amount,
                    resource_hash.clone(),
                )
                    .into_val(env),
                sub_invokes: &[],
            },
        }])
        .authorize_payment(agent, seller, asset, &amount, resource_hash);
}

// ── MUTATION TARGET A: owner.require_auth() in initialize ────────────────────
//
// Mutant: removal of `owner.require_auth()` → anyone can initialize the
//         wallet, setting themselves as owner and taking control.

#[test]
fn mutation_owner_auth_removal_in_initialize_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let upto_session = Address::generate(&env);

    // No mock_auths — the contract must enforce owner.require_auth() itself.
    let result = client.try_initialize(&owner, &upto_session);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: owner.require_auth() in initialize was not enforced; \
         wallet was initialized without owner authorization"
    );
    // Storage must remain uninitialized.
    assert_eq!(
        client.try_get_config(),
        Err(Ok(PolicyWalletError::NotInitialized)),
        "config was written despite no authorization"
    );
}

/// A third party providing their own auth for another owner's initialization must fail.
#[test]
fn mutation_owner_auth_wrong_party_in_initialize_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let attacker = Address::generate(&env);
    let upto_session = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_initialize(&owner, &upto_session);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: attacker authorization was accepted for initialize"
    );
    assert_eq!(
        client.try_get_config(),
        Err(Ok(PolicyWalletError::NotInitialized))
    );
}

// ── MUTATION TARGET B: owner.require_auth() in set_policy (via require_owner_auth) ─
//
// Mutant: removal of `owner.require_auth()` inside `require_owner_auth` →
//         any caller can install an arbitrary spending policy for any agent.

#[test]
fn mutation_owner_auth_removal_in_set_policy_is_caught() {
    let env = Env::default();
    let (client, _contract_id, _owner) = register_and_init(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xb1; 32]);
    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 100);

    // No mock_auths — the contract must enforce owner.require_auth().
    let result = client.try_set_policy(&policy, &0);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: owner.require_auth() in set_policy was not enforced; \
         policy was written without owner authorization"
    );
    // Policy must not be stored.
    assert_eq!(
        client.try_get_policy(&agent),
        Err(Ok(PolicyWalletError::PolicyNotFound)),
        "policy was stored despite no authorization"
    );
}

/// An attacker providing their own auth for set_policy must fail.
#[test]
fn mutation_owner_auth_wrong_party_in_set_policy_is_caught() {
    let env = Env::default();
    let (client, contract_id, _owner) = register_and_init(&env);
    let attacker = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xb2; 32]);
    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 100);

    let result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_policy(&policy, &0);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: attacker was allowed to set a policy without being the owner"
    );
    assert_eq!(
        client.try_get_policy(&agent),
        Err(Ok(PolicyWalletError::PolicyNotFound))
    );
}

// ── MUTATION TARGET C: agent.require_auth() in authorize_payment ─────────────
//
// Mutant: removal of `agent.require_auth()` → any caller can authorize a
//         payment on an agent's behalf, draining the policy's daily budget
//         or triggering unintended upto-session calls.

#[test]
fn mutation_agent_auth_removal_in_authorize_payment_is_caught() {
    let env = Env::default();
    env.mock_all_auths(); // allow initialize and set_policy to proceed unobstructed
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xc1; 32]);

    client.initialize(&owner, &Address::generate(&env));
    client.set_policy(
        &make_policy(
            &env,
            &agent,
            &seller,
            &asset,
            &resource_hash,
            50,
            100,
            100,
        ),
        &0,
    );

    // Drop mock_all_auths and test with no auth for authorize_payment.
    let env2 = Env::default();
    // Re-register in a fresh env to isolate from the mock_all_auths above.
    let contract_id2 = env2.register(PolicyWalletExampleContract, ());
    let client2 = PolicyWalletExampleContractClient::new(&env2, &contract_id2);
    let owner2 = Address::generate(&env2);
    let agent2 = Address::generate(&env2);
    let seller2 = Address::generate(&env2);
    let asset2 = Address::generate(&env2);
    let rh2 = BytesN::from_array(&env2, &[0xc2; 32]);

    let upto_session2 = Address::generate(&env2);
    // Initialize and set policy using explicit auths.
    client2
        .mock_auths(&[MockAuth {
            address: &owner2,
            invoke: &MockAuthInvoke {
                contract: &contract_id2,
                fn_name: "initialize",
                args: (owner2.clone(), upto_session2.clone()).into_val(&env2),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner2, &upto_session2);

    let policy2 = make_policy(&env2, &agent2, &seller2, &asset2, &rh2, 50, 100, 100);
    client2
        .mock_auths(&[MockAuth {
            address: &owner2,
            invoke: &MockAuthInvoke {
                contract: &contract_id2,
                fn_name: "set_policy",
                args: (policy2.clone(), 0_u32).into_val(&env2),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy2, &0);

    // Call authorize_payment with NO mock_auths — must fail.
    let result =
        client2.try_authorize_payment(&agent2, &seller2, &asset2, &30, &rh2);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: agent.require_auth() in authorize_payment was not enforced; \
         payment was authorized without agent authorization"
    );
    // Daily spend must remain zero.
    assert_eq!(
        client2.spent_today(&agent2),
        0,
        "daily spend was recorded despite no authorization"
    );
}

/// A third party providing their own auth for another agent's authorize_payment must fail.
#[test]
fn mutation_agent_auth_wrong_party_in_authorize_payment_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let attacker = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xc3; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 100);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // Attacker provides their own auth for agent's authorize_payment — must fail.
    let result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "authorize_payment",
                args: (
                    agent.clone(),
                    seller.clone(),
                    asset.clone(),
                    30_i128,
                    resource_hash.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_authorize_payment(&agent, &seller, &asset, &30, &resource_hash);

    assert!(
        result.is_err(),
        "MUTATION SURVIVED: attacker was allowed to authorize a payment on the agent's behalf"
    );
    assert_eq!(client.spent_today(&agent), 0);
}

// ── MUTATION TARGET D: payment-cap check (max_amount_per_payment) ────────────
//
// Mutant: inversion of `amount > policy.max_amount_per_payment` →
//         payments exceeding the per-payment cap are allowed.

#[test]
fn mutation_payment_cap_inversion_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xd1; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 75, 200, 100);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // amount == max_amount_per_payment + 1 must be rejected.
    let result = client.try_check_payment(&agent, &seller, &asset, &76, &resource_hash);

    assert_eq!(
        result,
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap)),
        "MUTATION SURVIVED: per-payment cap check did not reject amount > max_amount_per_payment"
    );

    // Exact boundary must succeed (kills off-by-one mutant on >=).
    assert!(
        client
            .try_check_payment(&agent, &seller, &asset, &75, &resource_hash)
            .is_ok(),
        "per-payment cap incorrectly rejected exact boundary amount"
    );
}

// ── MUTATION TARGET E: daily-cap check (max_amount_per_day) ──────────────────
//
// Mutant: inversion of `next_spent > policy.max_amount_per_day` →
//         the daily spending limit is not enforced.

#[test]
fn mutation_daily_cap_inversion_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xe1; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    // max_per_payment = 60, max_per_day = 100.
    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 60, 100, 100);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // First payment: 60 → total 60, within daily cap.
    authorize_payment_with_agent_auth(
        &env, &client, &contract_id, &agent, &seller, &asset, 60, &resource_hash,
    );
    assert_eq!(client.spent_today(&agent), 60);

    // Second payment: 60 → would total 120, exceeds daily cap of 100.
    let result = client
        .mock_auths(&[MockAuth {
            address: &agent,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "authorize_payment",
                args: (
                    agent.clone(),
                    seller.clone(),
                    asset.clone(),
                    60_i128,
                    resource_hash.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_authorize_payment(&agent, &seller, &asset, &60, &resource_hash);

    assert_eq!(
        result,
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap)),
        "MUTATION SURVIVED: daily cap check did not reject payment that would exceed max_amount_per_day"
    );
    // Spent amount must be unchanged.
    assert_eq!(
        client.spent_today(&agent),
        60,
        "daily spend was updated despite cap violation"
    );
}

/// Exactly at the daily cap (cumulative == max_per_day) must succeed.
#[test]
fn mutation_daily_cap_exact_boundary_succeeds() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xe2; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 100, 100, 100);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // Exactly at daily cap — must succeed.
    authorize_payment_with_agent_auth(
        &env, &client, &contract_id, &agent, &seller, &asset, 100, &resource_hash,
    );
    assert_eq!(
        client.spent_today(&agent),
        100,
        "payment at exact daily cap was rejected"
    );
}

// ── MUTATION TARGET F: policy expiry check in validate_payment ───────────────
//
// Mutant: inversion of `ledger > policy.valid_until_ledger` to `ledger <= …`
//         → expired policies are still accepted.
//
// Note: the contract uses strict `>` (not `>=`), so valid_until_ledger is the
// last valid ledger.  The mutation test verifies the boundary from both sides.

#[test]
fn mutation_policy_expiry_check_inversion_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xf1; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    // Policy valid until ledger 10.
    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 10);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // At ledger 11 (one past valid_until_ledger) — must be rejected.
    env.ledger().set_sequence_number(11);
    let result = client.try_check_payment(&agent, &seller, &asset, &30, &resource_hash);

    assert_eq!(
        result,
        Err(Ok(PolicyWalletError::PolicyExpired)),
        "MUTATION SURVIVED: policy expiry check did not fire at ledger > valid_until_ledger"
    );
}

/// At the valid_until_ledger boundary (ledger == valid_until), check_payment must succeed.
#[test]
fn mutation_policy_expiry_check_at_boundary_succeeds() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xf2; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 10);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // At valid_until_ledger itself — must still succeed (contract uses strict >).
    env.ledger().set_sequence_number(10);
    assert!(
        client
            .try_check_payment(&agent, &seller, &asset, &30, &resource_hash)
            .is_ok(),
        "check_payment was incorrectly rejected at ledger == valid_until_ledger"
    );
}

// ── MUTATION TARGET G: authorize_payment records spend / daily-cap state ─────
//
// Mutant: removal of the storage write in authorize_payment → daily spend is
//         never accumulated, allowing unbounded payments within a single day.

#[test]
fn mutation_authorize_payment_must_record_daily_spend() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0x71; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 60, 100, 100);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    authorize_payment_with_agent_auth(
        &env, &client, &contract_id, &agent, &seller, &asset, 60, &resource_hash,
    );

    // The daily spend MUST be 60 after one payment.  If the storage write was
    // removed (a surviving mutant), spent_today returns 0 and the assertion fails.
    assert_eq!(
        client.spent_today(&agent),
        60,
        "MUTATION SURVIVED: authorize_payment did not record daily spend; \
         spent_today returned 0 after a 60-unit payment"
    );
}

// ── End-to-end confirmation ──────────────────────────────────────────────────
//
// Runs the full initialize → set_policy → authorize_payment lifecycle with
// explicit auth trees and verifies every cap and auth boundary together.

#[test]
fn mutation_end_to_end_policy_wallet_guards_are_all_present() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let attacker = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0x01; 32]);

    // 1. Initialize requires owner auth.
    assert!(
        client.try_initialize(&owner, &Address::generate(&env)).is_err(),
        "owner auth not enforced for initialize"
    );
    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    // 2. set_policy requires owner auth.
    let policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 100);
    assert!(
        client.try_set_policy(&policy, &0).is_err(),
        "owner auth not enforced for set_policy"
    );
    set_policy_with_owner_auth(&env, &client, &contract_id, &owner, policy.clone());

    // 3. Payment cap is enforced.
    assert_eq!(
        client.try_check_payment(&agent, &seller, &asset, &51, &resource_hash),
        Err(Ok(PolicyWalletError::AmountExceedsPaymentCap)),
        "per-payment cap not enforced"
    );

    // 4. authorize_payment requires agent auth.
    assert!(
        client
            .try_authorize_payment(&agent, &seller, &asset, &30, &resource_hash)
            .is_err(),
        "agent auth not enforced for authorize_payment"
    );

    // 5. Attacker cannot authorize on behalf of agent.
    assert!(
        client
            .mock_auths(&[MockAuth {
                address: &attacker,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "authorize_payment",
                    args: (
                        agent.clone(),
                        seller.clone(),
                        asset.clone(),
                        30_i128,
                        resource_hash.clone(),
                    )
                        .into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .try_authorize_payment(&agent, &seller, &asset, &30, &resource_hash)
            .is_err(),
        "attacker was allowed to authorize on behalf of agent"
    );

    // 6. Correct agent auth succeeds and records spend.
    authorize_payment_with_agent_auth(
        &env, &client, &contract_id, &agent, &seller, &asset, 50, &resource_hash,
    );
    assert_eq!(client.spent_today(&agent), 50);

    // Authorize another 50 to reach the exact daily cap of 100.
    authorize_payment_with_agent_auth(
        &env, &client, &contract_id, &agent, &seller, &asset, 50, &resource_hash,
    );
    assert_eq!(client.spent_today(&agent), 100);

    // 7. Daily cap prevents overspend.
    // 100 spent; daily cap 100; another 1 would total 101 > 100.
    let over_daily = client
        .mock_auths(&[MockAuth {
            address: &agent,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "authorize_payment",
                args: (
                    agent.clone(),
                    seller.clone(),
                    asset.clone(),
                    1_i128,
                    resource_hash.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_authorize_payment(&agent, &seller, &asset, &1, &resource_hash);
    assert_eq!(
        over_daily,
        Err(Ok(PolicyWalletError::AmountExceedsDailyCap)),
        "daily cap not enforced after partial spend"
    );
    // Spend must not have changed.
    assert_eq!(client.spent_today(&agent), 100);
}

/// A payment before `valid_from_ledger` must be rejected with PolicyNotActive.
#[test]
fn mutation_policy_activation_check_before_boundary_is_caught() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xfa; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let mut policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 100);
    policy.valid_from_ledger = 50;
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // At ledger 49 (< valid_from_ledger 50) — must be rejected.
    env.ledger().set_sequence_number(49);
    let result = client.try_check_payment(&agent, &seller, &asset, &30, &resource_hash);

    assert_eq!(
        result,
        Err(Ok(PolicyWalletError::PolicyNotActive)),
        "MUTATION SURVIVED: policy activation check did not fire at ledger < valid_from_ledger"
    );
}

/// At valid_from_ledger itself, check_payment must succeed.
#[test]
fn mutation_policy_activation_check_at_boundary_succeeds() {
    let env = Env::default();
    let contract_id = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let resource_hash = BytesN::from_array(&env, &[0xfb; 32]);

    let upto_session = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (owner.clone(), upto_session.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&owner, &upto_session);

    let mut policy = make_policy(&env, &agent, &seller, &asset, &resource_hash, 50, 100, 100);
    policy.valid_from_ledger = 50;
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_policy",
                args: (policy.clone(), 0_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_policy(&policy, &0);

    // At valid_from_ledger itself — must succeed (inclusive boundary).
    env.ledger().set_sequence_number(50);
    assert!(
        client
            .try_check_payment(&agent, &seller, &asset, &30, &resource_hash)
            .is_ok(),
        "check_payment was incorrectly rejected at ledger == valid_from_ledger"
    );
}
