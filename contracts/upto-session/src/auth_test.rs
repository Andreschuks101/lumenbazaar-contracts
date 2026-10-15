extern crate std;

use crate::*;
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke},
    token, Address, Env, IntoVal, Symbol, Val, Vec as SorobanVec,
};

#[contract]
struct EscrowAuthorizationHarness;

#[contractimpl]
impl EscrowAuthorizationHarness {
    pub fn fund(env: Env, buyer: Address, asset: Address, amount: i128) {
        buyer.require_auth();
        token::Client::new(&env, &asset).transfer(&buyer, env.current_contract_address(), &amount);
    }

    pub fn settle(
        env: Env,
        _session_id: BytesN<32>,
        seller: Address,
        buyer: Address,
        asset: Address,
        actual_amount: i128,
        refund_amount: i128,
    ) {
        seller.require_auth();
        let escrow = env.current_contract_address();
        let token = token::Client::new(&env, &asset);
        token.transfer(&escrow, &seller, &actual_amount);
        if refund_amount > 0 {
            token.transfer(&escrow, &buyer, &refund_amount);
        }
    }
}

struct TestIdentities {
    admin: Address,
    buyer: Address,
    seller: Address,
    attacker: Address,
}

impl TestIdentities {
    fn generate(env: &Env) -> Self {
        Self {
            admin: Address::generate(env),
            buyer: Address::generate(env),
            seller: Address::generate(env),
            attacker: Address::generate(env),
        }
    }

    fn assert_distinct(&self) {
        assert_ne!(self.admin, self.buyer);
        assert_ne!(self.admin, self.seller);
        assert_ne!(self.admin, self.attacker);
        assert_ne!(self.buyer, self.seller);
        assert_ne!(self.buyer, self.attacker);
        assert_ne!(self.seller, self.attacker);
    }
}

fn create_test_asset(env: &Env, owner: &Address, amount: i128) -> Address {
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

fn contract_invocation(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: SorobanVec<Val>,
    sub_invocations: std::vec::Vec<AuthorizedInvocation>,
) -> AuthorizedInvocation {
    AuthorizedInvocation {
        function: AuthorizedFunction::Contract((contract.clone(), Symbol::new(env, fn_name), args)),
        sub_invocations,
    }
}

fn token_transfer_invocation(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) -> AuthorizedInvocation {
    contract_invocation(
        env,
        asset,
        "transfer",
        (from.clone(), to.clone(), amount).into_val(env),
        std::vec![],
    )
}

struct SettlementExpectation<'a> {
    contract: &'a Address,
    session_id: &'a BytesN<32>,
    seller: &'a Address,
    buyer: &'a Address,
    asset: &'a Address,
    actual_amount: i128,
    refund_amount: i128,
}

fn settlement_invocation(env: &Env, expected: SettlementExpectation<'_>) -> AuthorizedInvocation {
    contract_invocation(
        env,
        expected.contract,
        "settle",
        (
            expected.session_id.clone(),
            expected.seller.clone(),
            expected.buyer.clone(),
            expected.asset.clone(),
            expected.actual_amount,
            expected.refund_amount,
        )
            .into_val(env),
        std::vec![],
    )
}

fn assert_single_authorization(env: &Env, authorizer: &Address, expected: AuthorizedInvocation) {
    assert_eq!(env.auths(), std::vec![(authorizer.clone(), expected)]);
}

#[test]
fn escrow_funding_uses_exact_buyer_and_token_transfer_tree() {
    let env = Env::default();
    let identities = TestIdentities::generate(&env);
    identities.assert_distinct();
    let contract_id = env.register(EscrowAuthorizationHarness, ());
    let client = EscrowAuthorizationHarnessClient::new(&env, &contract_id);
    let asset = create_test_asset(&env, &identities.buyer, 500);

    client
        .mock_auths(&[MockAuth {
            address: &identities.buyer,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "fund",
                args: (identities.buyer.clone(), asset.clone(), 500_i128).into_val(&env),
                sub_invokes: &[MockAuthInvoke {
                    contract: &asset,
                    fn_name: "transfer",
                    args: (identities.buyer.clone(), contract_id.clone(), 500_i128).into_val(&env),
                    sub_invokes: &[],
                }],
            },
        }])
        .fund(&identities.buyer, &asset, &500);

    assert_single_authorization(
        &env,
        &identities.buyer,
        contract_invocation(
            &env,
            &contract_id,
            "fund",
            (identities.buyer.clone(), asset.clone(), 500_i128).into_val(&env),
            std::vec![token_transfer_invocation(
                &env,
                &asset,
                &identities.buyer,
                &contract_id,
                500,
            )],
        ),
    );
    assert_eq!(
        token::Client::new(&env, &asset).balance(&identities.buyer),
        0
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 500);
}

#[test]
fn escrow_payout_needs_only_the_seller_authorization() {
    let env = Env::default();
    let identities = TestIdentities::generate(&env);
    let contract_id = env.register(EscrowAuthorizationHarness, ());
    let client = EscrowAuthorizationHarnessClient::new(&env, &contract_id);
    let asset = create_test_asset(&env, &contract_id, 500);
    let session_id = BytesN::from_array(&env, &[43; 32]);

    client
        .mock_auths(&[MockAuth {
            address: &identities.seller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle",
                args: (
                    session_id.clone(),
                    identities.seller.clone(),
                    identities.buyer.clone(),
                    asset.clone(),
                    125_i128,
                    375_i128,
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .settle(
            &session_id,
            &identities.seller,
            &identities.buyer,
            &asset,
            &125,
            &375,
        );

    assert_single_authorization(
        &env,
        &identities.seller,
        settlement_invocation(
            &env,
            SettlementExpectation {
                contract: &contract_id,
                session_id: &session_id,
                seller: &identities.seller,
                buyer: &identities.buyer,
                asset: &asset,
                actual_amount: 125,
                refund_amount: 375,
            },
        ),
    );
    let token = token::Client::new(&env, &asset);
    assert_eq!(token.balance(&identities.seller), 125);
    assert_eq!(token.balance(&identities.buyer), 375);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn production_escrow_funds_and_settles_with_exact_authorization_trees() {
    let env = Env::default();
    let identities = TestIdentities::generate(&env);
    let contract_id = env.register(UptoSessionContract, ());
    let client = UptoSessionContractClient::new(&env, &contract_id);
    let asset = create_test_asset(&env, &identities.buyer, 500);
    let resource_hash = BytesN::from_array(&env, &[41; 32]);
    let usage_hash = BytesN::from_array(&env, &[42; 32]);
    let asset_policies = soroban_sdk::vec![
        &env,
        AssetPolicy {
            asset: asset.clone(),
            max_session_amount: 500,
        }
    ];

    client
        .mock_auths(&[MockAuth {
            address: &identities.admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "initialize",
                args: (identities.admin.clone(), asset_policies.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .initialize(&identities.admin, &asset_policies);

    let session_id = client
        .mock_auths(&[MockAuth {
            address: &identities.buyer,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "create_session",
                args: (
                    identities.buyer.clone(),
                    identities.seller.clone(),
                    asset.clone(),
                    500_i128,
                    50_u32,
                    resource_hash.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[MockAuthInvoke {
                    contract: &asset,
                    fn_name: "transfer",
                    args: (
                        identities.buyer.clone(),
                        contract_id.clone(),
                        500_i128,
                    )
                        .into_val(&env),
                    sub_invokes: &[],
                }],
            },
        }])
        .create_session(
            &identities.buyer,
            &identities.seller,
            &asset,
            &500,
            &50,
            &resource_hash,
        );

    client
        .mock_auths(&[MockAuth {
            address: &identities.seller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle",
                args: (session_id.clone(), 125_i128, usage_hash.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .settle(&session_id, &125, &usage_hash);

    let session = client.get_session(&session_id);
    assert_eq!(session.status, SessionStatus::Settled);
    assert_eq!(session.settled_amount, 125);
    assert_eq!(session.escrowed_amount, 0);
    let token = token::Client::new(&env, &asset);
    assert_eq!(token.balance(&identities.buyer), 375);
    assert_eq!(token.balance(&identities.seller), 125);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn escrow_funding_rejects_an_attacker_authorization() {
    let env = Env::default();
    let identities = TestIdentities::generate(&env);
    let contract_id = env.register(EscrowAuthorizationHarness, ());
    let client = EscrowAuthorizationHarnessClient::new(&env, &contract_id);
    let asset = create_test_asset(&env, &identities.buyer, 500);

    let result = client
        .mock_auths(&[MockAuth {
            address: &identities.attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "fund",
                args: (identities.buyer.clone(), asset.clone(), 500_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_fund(&identities.buyer, &asset, &500);

    assert!(result.is_err());
    assert_eq!(
        token::Client::new(&env, &asset).balance(&identities.buyer),
        500
    );
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 0);
}
