extern crate std;

use std::borrow::ToOwned;
use crate::*;
use soroban_sdk::{
    contract, contractimpl,
    testutils::{
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, Ledger as _, MockAuth,
        MockAuthInvoke,
    },
    token, Address, Env, IntoVal, Symbol, TryFromVal, Val, Vec as SorobanVec,
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
    let idempotency_key = BytesN::from_array(&env, &[43; 32]);
    let asset_policies = soroban_sdk::vec![
        &env,
        AssetPolicy {
            asset: asset.clone(),
            max_session_amount: 500,
            min_session_amount: 1,
            max_asset_exposure: 500,
            max_buyer_exposure: 500,
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
                    idempotency_key.clone(),
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
            &idempotency_key,
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


// ---------------------------------------------------------------------------
// Authorization tree snapshots for every public `upto-session` mutation.
//
// Each case drives the real contract through `try_invoke_contract` with an
// explicit `mock_auths` tree and then compares `env.auths()` against the golden
// fixture entry in `artifacts/interface/authorization-trees.json`. Unauthorized
// attempts (missing signer, wrong signer, role confusion, wrong function, wrong
// arguments, missing sub-invocation) must fail with zero ledger, balance,
// liability and event change.
// ---------------------------------------------------------------------------

const AUTH_FIXTURE: &str = include_str!("../../../artifacts/interface/authorization-trees.json");
const UPTO_CONTRACT: &str = "upto-session";
const UPTO_MUTATIONS: [&str; 28] = [
    "initialize",
    "propose_admin",
    "accept_admin",
    "cancel_admin_rotation",
    "pause_creation",
    "resume_creation",
    "activate_asset",
    "deprecate_asset",
    "remove_asset",
    "set_asset_max_session_amount",
    "set_asset_exposure_ceiling",
    "set_buyer_exposure_ceiling",
    "set_seller_exposure_ceiling",
    "set_max_buyer_open_sessions",
    "set_max_seller_open_sessions",
    "propose_asset_cap_increase",
    "accept_asset_cap_increase",
    "cancel_asset_cap_increase",
    "recover_surplus",
    "create_session",
    "settle",
    "cancel",
    "reduce_session_cap",
    "reject",
    "extend_session_expiry",
    "recover_expired",
    "recover_expired_batch",
    "extend_ttl",
];

fn fixture_mutations() -> std::vec::Vec<serde_json::Value> {
    let doc: serde_json::Value = serde_json::from_str(AUTH_FIXTURE).expect("fixture is JSON");
    doc["mutations"].as_array().expect("mutations array").clone()
}

fn fixture_entry(contract: &str, method: &str) -> serde_json::Value {
    fixture_mutations()
        .into_iter()
        .find(|entry| entry["contract"] == contract && entry["method"] == method)
        .unwrap_or_else(|| panic!("missing fixture entry {contract}.{method}"))
}

#[derive(Clone)]
struct Node {
    contract: Address,
    fn_name: &'static str,
    args: SorobanVec<Val>,
    sub: Option<std::boxed::Box<Node>>,
}

impl Node {
    fn expected(&self, env: &Env) -> AuthorizedInvocation {
        contract_invocation(
            env,
            &self.contract,
            self.fn_name,
            self.args.clone(),
            self.sub
                .iter()
                .map(|sub| sub.expected(env))
                .collect::<std::vec::Vec<_>>(),
        )
    }
}

struct Case {
    method: &'static str,
    args: SorobanVec<Val>,
    roles: std::vec::Vec<&'static str>,
    sub: Option<Node>,
}

struct Harness {
    env: Env,
    id: Address,
    admin: Address,
    pending_admin: Address,
    buyer: Address,
    seller: Address,
    attacker: Address,
    recipient: Address,
    token_admin: Address,
    asset: Address,
    next_key: u8,
}

const SESSION_AMOUNT: i128 = 500;
const SESSION_EXPIRY: u32 = 50;

impl Harness {
    fn uninitialized() -> Self {
        let env = Env::default();
        let id = env.register(UptoSessionContract, ());
        let token_admin = Address::generate(&env);
        let asset = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let harness = Self {
            id,
            admin: Address::generate(&env),
            pending_admin: Address::generate(&env),
            buyer: Address::generate(&env),
            seller: Address::generate(&env),
            attacker: Address::generate(&env),
            recipient: Address::generate(&env),
            token_admin,
            asset,
            next_key: 1,
            env,
        };
        harness.mint(&harness.buyer, 10_000);
        harness
    }

    fn initialized() -> Self {
        let harness = Self::uninitialized();
        let args = harness.args((harness.admin.clone(), harness.policies()));
        let root = harness.node("initialize", args.clone(), None);
        harness.must("initialize", &args, &[(harness.admin.clone(), root)]);
        harness
    }

    fn args<T: IntoVal<Env, SorobanVec<Val>>>(&self, tuple: T) -> SorobanVec<Val> {
        tuple.into_val(&self.env)
    }

    fn no_args(&self) -> SorobanVec<Val> {
        SorobanVec::new(&self.env)
    }

    fn policy_for(&self, asset: &Address) -> AssetPolicy {
        AssetPolicy {
            asset: asset.clone(),
            max_session_amount: SESSION_AMOUNT,
            min_session_amount: 1,
            max_asset_exposure: 10_000,
            max_buyer_exposure: 10_000,
        }
    }

    fn policies(&self) -> SorobanVec<AssetPolicy> {
        soroban_sdk::vec![&self.env, self.policy_for(&self.asset)]
    }

    fn mint(&self, to: &Address, amount: i128) {
        token::StellarAssetClient::new(&self.env, &self.asset)
            .mock_auths(&[MockAuth {
                address: &self.token_admin,
                invoke: &MockAuthInvoke {
                    contract: &self.asset,
                    fn_name: "mint",
                    args: (to.clone(), amount).into_val(&self.env),
                    sub_invokes: &[],
                },
            }])
            .mint(to, &amount);
    }

    fn new_asset(&self) -> Address {
        self.env
            .register_stellar_asset_contract_v2(Address::generate(&self.env))
            .address()
    }

    fn balance(&self, asset: &Address, who: &Address) -> i128 {
        token::Client::new(&self.env, asset).balance(who)
    }

    fn role(&self, role: &str) -> Address {
        match role {
            "admin" => self.admin.clone(),
            "pending_admin" => self.pending_admin.clone(),
            "buyer" => self.buyer.clone(),
            "seller" => self.seller.clone(),
            other => panic!("unknown role {other}"),
        }
    }

    fn identities(&self) -> std::vec::Vec<Address> {
        std::vec![
            self.admin.clone(),
            self.pending_admin.clone(),
            self.buyer.clone(),
            self.seller.clone(),
            self.attacker.clone(),
        ]
    }

    fn node(&self, fn_name: &'static str, args: SorobanVec<Val>, sub: Option<Node>) -> Node {
        Node {
            contract: self.id.clone(),
            fn_name,
            args,
            sub: sub.map(std::boxed::Box::new),
        }
    }

    fn case(&self, method: &'static str, roles: &[&'static str], args: SorobanVec<Val>) -> Case {
        Case {
            method,
            args,
            roles: roles.to_vec(),
            sub: None,
        }
    }

    fn call(
        &self,
        method: &str,
        args: &SorobanVec<Val>,
        auths: &[(Address, Node)],
    ) -> Option<Val> {
        let subs: std::vec::Vec<std::vec::Vec<MockAuthInvoke>> = auths
            .iter()
            .map(|(_, node)| {
                node.sub
                    .iter()
                    .map(|sub| MockAuthInvoke {
                        contract: &sub.contract,
                        fn_name: sub.fn_name,
                        args: sub.args.clone(),
                        sub_invokes: &[],
                    })
                    .collect()
            })
            .collect();
        let invokes: std::vec::Vec<MockAuthInvoke> = auths
            .iter()
            .zip(subs.iter())
            .map(|((_, node), sub_invokes)| MockAuthInvoke {
                contract: &node.contract,
                fn_name: node.fn_name,
                args: node.args.clone(),
                sub_invokes,
            })
            .collect();
        let mocks: std::vec::Vec<MockAuth> = auths
            .iter()
            .zip(invokes.iter())
            .map(|((address, _), invoke)| MockAuth { address, invoke })
            .collect();
        self.env.mock_auths(&mocks);
        match self.env.try_invoke_contract::<Val, soroban_sdk::Error>(
            &self.id,
            &Symbol::new(&self.env, method),
            args.clone(),
        ) {
            Ok(Ok(value)) => Some(value),
            _ => None,
        }
    }

    fn must(&self, method: &str, args: &SorobanVec<Val>, auths: &[(Address, Node)]) -> Val {
        self.call(method, args, auths)
            .unwrap_or_else(|| panic!("authorized setup call {method} failed"))
    }

    /// Runs `case` with its exact authorization tree as setup.
    fn run_authorized(&self, case: &Case) -> Val {
        let root = self.node(case.method, case.args.clone(), case.sub.clone());
        let auths: std::vec::Vec<(Address, Node)> = case
            .roles
            .iter()
            .map(|role| (self.role(role), root.clone()))
            .collect();
        self.must(case.method, &case.args, &auths)
    }

    fn snapshot(&self) -> std::string::String {
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
        std::format!("{:?}", filtered)
    }

    fn event_count(&self) -> usize {
        self.env.events().all().events().len()
    }

    fn create_session(&mut self) -> BytesN<32> {
        let key = BytesN::from_array(&self.env, &[self.next_key; 32]);
        self.next_key += 1;
        let case = self.create_session_case(&key);
        let value = self.run_authorized(&case);
        BytesN::<32>::try_from_val(&self.env, &value).expect("session id")
    }

    fn create_session_case(&self, key: &BytesN<32>) -> Case {
        let resource_hash = BytesN::from_array(&self.env, &[41; 32]);
        let mut case = self.case(
            "create_session",
            &["buyer"],
            self.args((
                self.buyer.clone(),
                self.seller.clone(),
                self.asset.clone(),
                SESSION_AMOUNT,
                SESSION_EXPIRY,
                resource_hash,
                key.clone(),
            )),
        );
        case.sub = Some(Node {
            contract: self.asset.clone(),
            fn_name: "transfer",
            args: self.args((self.buyer.clone(), self.id.clone(), SESSION_AMOUNT)),
            sub: None,
        });
        case
    }

    fn set_ledger(&self, sequence: u32) {
        self.env.ledger().with_mut(|ledger| {
            ledger.sequence_number = sequence;
        });
    }
}

fn assert_matches_fixture(h: &Harness, case: &Case) {
    let entry = fixture_entry(UPTO_CONTRACT, case.method);
    let roles: std::vec::Vec<&str> = entry["required_signers"]
        .as_array()
        .expect("required_signers array")
        .iter()
        .map(|role| role.as_str().expect("role string"))
        .collect();
    let expected_roles: std::vec::Vec<&str> = if case.roles.is_empty() {
        std::vec!["permissionless"]
    } else {
        case.roles.clone()
    };
    assert_eq!(roles, expected_roles, "{} signer roles", case.method);
    assert_eq!(entry["tree"]["root"]["contract"], UPTO_CONTRACT);
    assert_eq!(entry["tree"]["root"]["function"], case.method);
    let subs: std::vec::Vec<&str> = entry["tree"]["sub_invocations"]
        .as_array()
        .expect("sub_invocations array")
        .iter()
        .map(|sub| sub.as_str_function())
        .collect();
    let expected_subs: std::vec::Vec<&str> = case.sub.iter().map(|sub| sub.fn_name).collect();
    assert_eq!(subs, expected_subs, "{} sub-invocations", case.method);
    assert_eq!(
        entry["unauthorized_behavior"],
        "fails_closed_zero_state_change"
    );
    let _ = h;
}

trait SubFunction {
    fn as_str_function(&self) -> &str;
}

impl SubFunction for serde_json::Value {
    fn as_str_function(&self) -> &str {
        self["function"].as_str().expect("function string")
    }
}

/// Core assertion: pins the exact authorization tree and proves every
/// unauthorized variant fails closed before the authorized call succeeds.
fn assert_auth_tree(h: &Harness, case: &Case) {
    assert_matches_fixture(h, case);
    let root = h.node(case.method, case.args.clone(), case.sub.clone());
    let authorized: std::vec::Vec<(Address, Node)> = case
        .roles
        .iter()
        .map(|role| (h.role(role), root.clone()))
        .collect();

    if !authorized.is_empty() {
        let mut attempts: std::vec::Vec<(&str, std::vec::Vec<(Address, Node)>)> =
            std::vec![("missing signer", std::vec::Vec::new())];
        for candidate in h.identities() {
            if authorized.iter().any(|(signer, _)| *signer == candidate) {
                continue;
            }
            attempts.push((
                "wrong signer",
                authorized
                    .iter()
                    .map(|_| (candidate.clone(), root.clone()))
                    .collect(),
            ));
        }
        if authorized.len() > 1 {
            for auth in &authorized {
                attempts.push(("partial signers", std::vec![auth.clone()]));
            }
        }
        let mut wrong_function = root.clone();
        wrong_function.fn_name = "not_the_invoked_function";
        attempts.push((
            "wrong function",
            authorized
                .iter()
                .map(|(signer, _)| (signer.clone(), wrong_function.clone()))
                .collect(),
        ));
        let mut wrong_args = root.clone();
        wrong_args.args.push_back(7_u32.into_val(&h.env));
        attempts.push((
            "wrong arguments",
            authorized
                .iter()
                .map(|(signer, _)| (signer.clone(), wrong_args.clone()))
                .collect(),
        ));
        if let Some(sub) = root.sub.as_deref() {
            let mut missing_sub = root.clone();
            missing_sub.sub = None;
            attempts.push((
                "missing sub-invocation",
                authorized
                    .iter()
                    .map(|(signer, _)| (signer.clone(), missing_sub.clone()))
                    .collect(),
            ));
            let mut wrong_sub = root.clone();
            let mut altered = sub.clone();
            altered.args.push_back(7_u32.into_val(&h.env));
            wrong_sub.sub = Some(std::boxed::Box::new(altered));
            attempts.push((
                "wrong sub-invocation arguments",
                authorized
                    .iter()
                    .map(|(signer, _)| (signer.clone(), wrong_sub.clone()))
                    .collect(),
            ));
        }

        for (label, auths) in attempts {
            let before = h.snapshot();
            assert!(
                h.call(case.method, &case.args, &auths).is_none(),
                "{}: {label} must be rejected",
                case.method
            );
            assert_eq!(
                before,
                h.snapshot(),
                "{}: {label} changed ledger state",
                case.method
            );
            assert_eq!(
                h.event_count(),
                0,
                "{}: {label} emitted events",
                case.method
            );
        }
    }

    let before = h.snapshot();
    h.must(case.method, &case.args, &authorized);
    assert_eq!(
        h.env.auths(),
        authorized
            .iter()
            .map(|(signer, node)| (signer.clone(), node.expected(&h.env)))
            .collect::<std::vec::Vec<_>>(),
        "{}: recorded authorization tree",
        case.method
    );
    if !case.roles.is_empty() {
        assert_ne!(before, h.snapshot(), "{} wrote no state", case.method);
    }
}

fn assert_permissionless(h: &Harness, case: &Case) {
    assert!(case.roles.is_empty());
    assert_matches_fixture(h, case);
    let root = h.node(case.method, case.args.clone(), None);
    // An attacker authorization the call does not need must never be consumed.
    h.must(case.method, &case.args, &[]);
    assert!(
        h.env.auths().is_empty(),
        "{} must not require_auth",
        case.method
    );
    let _ = root;
}

#[test]
fn fixture_inventory_matches_the_public_mutation_list_exactly() {
    let upto: std::vec::Vec<std::string::String> = fixture_mutations()
        .iter()
        .filter(|entry| entry["contract"] == UPTO_CONTRACT)
        .map(|entry| entry["method"].as_str().expect("method").to_owned())
        .collect();
    let mut expected: std::vec::Vec<std::string::String> = UPTO_MUTATIONS
        .iter()
        .map(|method| (*method).to_owned())
        .collect();
    let mut actual = upto.clone();
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected);
    assert_eq!(upto.len(), UPTO_MUTATIONS.len(), "no duplicate entries");
    for entry in fixture_mutations() {
        assert_eq!(
            entry["unauthorized_behavior"],
            "fails_closed_zero_state_change"
        );
        let roles = entry["required_signers"].as_array().expect("roles");
        assert!(!roles.is_empty());
    }
}

#[test]
fn initialize_requires_admin_auth_tree() {
    let h = Harness::uninitialized();
    let case = h.case(
        "initialize",
        &["admin"],
        h.args((h.admin.clone(), h.policies())),
    );
    assert_auth_tree(&h, &case);
    assert_eq!(UptoSessionContractClient::new(&h.env, &h.id).get_admin(), h.admin);
}

#[test]
fn propose_admin_requires_admin_auth_tree() {
    let h = Harness::initialized();
    let case = h.case(
        "propose_admin",
        &["admin"],
        h.args((h.pending_admin.clone(), 100_u32)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn accept_admin_requires_pending_admin_auth_tree() {
    let h = Harness::initialized();
    h.run_authorized(&h.case(
        "propose_admin",
        &["admin"],
        h.args((h.pending_admin.clone(), 100_u32)),
    ));
    let case = h.case("accept_admin", &["pending_admin"], h.no_args());
    assert_auth_tree(&h, &case);
    let client = UptoSessionContractClient::new(&h.env, &h.id);
    assert_eq!(client.get_admin(), h.pending_admin);
}

#[test]
fn cancel_admin_rotation_requires_admin_auth_tree() {
    let h = Harness::initialized();
    h.run_authorized(&h.case(
        "propose_admin",
        &["admin"],
        h.args((h.pending_admin.clone(), 100_u32)),
    ));
    let case = h.case("cancel_admin_rotation", &["admin"], h.no_args());
    assert_auth_tree(&h, &case);
}

#[test]
fn pause_creation_requires_admin_auth_tree() {
    let h = Harness::initialized();
    assert_auth_tree(&h, &h.case("pause_creation", &["admin"], h.no_args()));
}

#[test]
fn resume_creation_requires_admin_auth_tree() {
    let h = Harness::initialized();
    h.run_authorized(&h.case("pause_creation", &["admin"], h.no_args()));
    assert_auth_tree(&h, &h.case("resume_creation", &["admin"], h.no_args()));
}

#[test]
fn activate_asset_requires_admin_auth_tree() {
    let h = Harness::initialized();
    let second_asset = h.new_asset();
    let case = h.case(
        "activate_asset",
        &["admin"],
        h.args((h.policy_for(&second_asset),)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn deprecate_asset_requires_admin_auth_tree() {
    let h = Harness::initialized();
    let reason = BytesN::from_array(&h.env, &[7; 32]);
    let case = h.case(
        "deprecate_asset",
        &["admin"],
        h.args((h.asset.clone(), reason)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn remove_asset_requires_admin_auth_tree() {
    let h = Harness::initialized();
    let reason = BytesN::from_array(&h.env, &[7; 32]);
    h.run_authorized(&h.case(
        "deprecate_asset",
        &["admin"],
        h.args((h.asset.clone(), reason.clone())),
    ));
    let case = h.case(
        "remove_asset",
        &["admin"],
        h.args((h.asset.clone(), reason)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn asset_cap_and_ceiling_setters_require_admin_auth_trees() {
    let h = Harness::initialized();
    assert_auth_tree(
        &h,
        &h.case(
            "set_asset_max_session_amount",
            &["admin"],
            h.args((h.asset.clone(), 400_i128)),
        ),
    );
    for method in [
        "set_asset_exposure_ceiling",
        "set_buyer_exposure_ceiling",
        "set_seller_exposure_ceiling",
    ] {
        assert_auth_tree(
            &h,
            &h.case(method, &["admin"], h.args((h.asset.clone(), 5_000_i128))),
        );
    }
}

#[test]
fn open_session_limit_setters_require_admin_auth_trees() {
    let h = Harness::initialized();
    for method in ["set_max_buyer_open_sessions", "set_max_seller_open_sessions"] {
        assert_auth_tree(&h, &h.case(method, &["admin"], h.args((3_u32,))));
    }
}

#[test]
fn propose_asset_cap_increase_requires_admin_auth_tree() {
    let h = Harness::initialized();
    let case = h.case(
        "propose_asset_cap_increase",
        &["admin"],
        h.args((h.asset.clone(), 1_000_i128, 200_u32, 300_u32)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn accept_asset_cap_increase_requires_admin_auth_tree() {
    let h = Harness::initialized();
    h.run_authorized(&h.case(
        "propose_asset_cap_increase",
        &["admin"],
        h.args((h.asset.clone(), 1_000_i128, 200_u32, 300_u32)),
    ));
    h.set_ledger(250);
    let case = h.case(
        "accept_asset_cap_increase",
        &["admin"],
        h.args((h.asset.clone(),)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn cancel_asset_cap_increase_requires_admin_auth_tree() {
    let h = Harness::initialized();
    h.run_authorized(&h.case(
        "propose_asset_cap_increase",
        &["admin"],
        h.args((h.asset.clone(), 1_000_i128, 200_u32, 300_u32)),
    ));
    let case = h.case(
        "cancel_asset_cap_increase",
        &["admin"],
        h.args((h.asset.clone(),)),
    );
    assert_auth_tree(&h, &case);
}

#[test]
fn recover_surplus_requires_admin_auth_tree_and_moves_no_funds_when_unauthorized() {
    let h = Harness::initialized();
    h.mint(&h.id, 50);
    let case = h.case(
        "recover_surplus",
        &["admin"],
        h.args((h.asset.clone(), h.recipient.clone(), 50_i128)),
    );
    assert_auth_tree(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.recipient), 50);
    assert_eq!(h.balance(&h.asset, &h.id), 0);
}

#[test]
fn create_session_requires_buyer_auth_with_asset_transfer_sub_invocation() {
    let h = Harness::initialized();
    let key = BytesN::from_array(&h.env, &[200; 32]);
    let case = h.create_session_case(&key);
    assert_auth_tree(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.buyer), 9_500);
    assert_eq!(h.balance(&h.asset, &h.id), SESSION_AMOUNT);
    let client = UptoSessionContractClient::new(&h.env, &h.id);
    assert_eq!(client.get_escrow_liability(&h.asset), SESSION_AMOUNT);
}

#[test]
fn unauthorized_create_session_leaves_balances_and_liability_untouched() {
    let h = Harness::initialized();
    let key = BytesN::from_array(&h.env, &[201; 32]);
    let case = h.create_session_case(&key);
    let root = h.node(case.method, case.args.clone(), case.sub.clone());
    let attacker_auths = [(h.attacker.clone(), root)];
    assert!(h.call(case.method, &case.args, &attacker_auths).is_none());
    let client = UptoSessionContractClient::new(&h.env, &h.id);
    assert_eq!(h.balance(&h.asset, &h.buyer), 10_000);
    assert_eq!(h.balance(&h.asset, &h.id), 0);
    assert_eq!(client.get_escrow_liability(&h.asset), 0);
    assert_eq!(client.get_session_state_counts().open, 0);
}

#[test]
fn settle_requires_seller_auth_tree() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    let usage_hash = BytesN::from_array(&h.env, &[42; 32]);
    let case = h.case(
        "settle",
        &["seller"],
        h.args((session_id.clone(), 125_i128, usage_hash)),
    );
    assert_auth_tree(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.seller), 125);
    assert_eq!(h.balance(&h.asset, &h.buyer), 9_875);
    assert_eq!(h.balance(&h.asset, &h.id), 0);
}

#[test]
fn cancel_requires_buyer_auth_tree_and_rejects_the_seller() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    let case = h.case("cancel", &["buyer"], h.args((session_id,)));
    // `assert_auth_tree` includes the seller among the rejected wrong signers.
    assert_auth_tree(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.buyer), 10_000);
    assert_eq!(h.balance(&h.asset, &h.id), 0);
}

#[test]
fn reduce_session_cap_requires_buyer_auth_tree() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    let case = h.case(
        "reduce_session_cap",
        &["buyer"],
        h.args((session_id, 100_i128)),
    );
    assert_auth_tree(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.id), 100);
}

#[test]
fn reject_requires_seller_auth_tree() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    let case = h.case("reject", &["seller"], h.args((session_id,)));
    assert_auth_tree(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.buyer), 10_000);
}

#[test]
fn extend_session_expiry_requires_both_buyer_and_seller_auth_tree() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    let case = h.case(
        "extend_session_expiry",
        &["buyer", "seller"],
        h.args((session_id.clone(), 80_u32)),
    );
    assert_auth_tree(&h, &case);
    let client = UptoSessionContractClient::new(&h.env, &h.id);
    assert_eq!(client.get_session(&session_id).expires_at_ledger, 80);
}

#[test]
fn recover_expired_is_permissionless() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    h.set_ledger(SESSION_EXPIRY + 10);
    let case = h.case("recover_expired", &[], h.args((session_id,)));
    assert_permissionless(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.buyer), 10_000);
    assert_eq!(h.balance(&h.asset, &h.id), 0);
}

#[test]
fn recover_expired_batch_is_permissionless() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    h.set_ledger(SESSION_EXPIRY + 10);
    let ids = soroban_sdk::vec![&h.env, session_id];
    let case = h.case("recover_expired_batch", &[], h.args((ids,)));
    assert_permissionless(&h, &case);
    assert_eq!(h.balance(&h.asset, &h.buyer), 10_000);
}

#[test]
fn extend_ttl_is_permissionless() {
    let mut h = Harness::initialized();
    let session_id = h.create_session();
    let case = h.case("extend_ttl", &[], h.args((session_id,)));
    assert_permissionless(&h, &case);
}

#[test]
fn mismatched_expected_trees_are_not_normalized_away() {
    let h = Harness::initialized();
    let key = BytesN::from_array(&h.env, &[202; 32]);
    let case = h.create_session_case(&key);
    h.run_authorized(&case);
    let recorded = h.env.auths();
    let root = h.node(case.method, case.args.clone(), case.sub.clone());
    let exact = std::vec![(h.buyer.clone(), root.expected(&h.env))];
    assert_eq!(recorded, exact);

    let mut without_sub = root.clone();
    without_sub.sub = None;
    assert_ne!(
        recorded,
        std::vec![(h.buyer.clone(), without_sub.expected(&h.env))],
        "dropping the token transfer sub-invocation must be detected"
    );
    assert_ne!(
        recorded,
        std::vec![(h.seller.clone(), root.expected(&h.env))],
        "a different signer must be detected"
    );
    let mut extra = exact.clone();
    extra.push((h.attacker.clone(), root.expected(&h.env)));
    assert_ne!(recorded, extra, "an unexpected extra authorization must be detected");
    let mut wrong_function = root;
    wrong_function.fn_name = "settle";
    assert_ne!(
        recorded,
        std::vec![(h.buyer.clone(), wrong_function.expected(&h.env))],
        "a different function must be detected"
    );
}
