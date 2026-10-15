extern crate std;

use crate::*;
use proptest::{
    collection,
    prelude::*,
    prop_oneof,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    token, Address, BytesN, Env, IntoVal,
};

const PROPERTY_CASES: u32 = 128;
const PROPERTY_SEED: u64 = 0x4c55_4d45_4e42_5a52;
const EXPIRY_LEDGER: u32 = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelStatus {
    Open,
    Settled,
    Cancelled,
    Expired,
}

#[derive(Clone, Copy, Debug)]
struct ModelSession {
    cap: i128,
    settled: i128,
    status: ModelStatus,
}

fn property_config() -> ProptestConfig {
    let cases = std::env::var("LUMENBAZAAR_PROPERTY_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(PROPERTY_CASES);
    ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        max_shrink_iters: 10_000,
        ..ProptestConfig::default()
    }
}

fn cap_strategy() -> impl Strategy<Value = (i128, i128)> {
    let cap = prop_oneof![
        Just(1_i128),
        Just(2_i128),
        Just(10_000_000_i128),
        (1_u32..1_000_000_u32).prop_map(i128::from),
    ];
    (cap.clone(), cap)
}

fn action_strategy() -> impl Strategy<Value = std::vec::Vec<(u8, u8, u8)>> {
    collection::vec((0_u8..6, 0_u8..2, 0_u8..5), 1..24)
}

fn create_asset(env: &Env, owner: &Address, amount: i128) -> Address {
    let admin = Address::generate(env);
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    token::StellarAssetClient::new(env, &asset)
        .mock_auths(&[MockAuth {
            address: &admin,
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

fn initialize(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    admin: &Address,
    assets: &soroban_sdk::Vec<Address>,
) {
    let mut policies = soroban_sdk::Vec::new(env);
    for asset in assets.iter() {
        policies.push_back(AssetPolicy {
            asset,
            max_session_amount: i128::MAX,
        });
    }
    client
        .mock_auths(&[MockAuth {
            address: admin,
            invoke: &MockAuthInvoke {
                contract,
                fn_name: "initialize",
                args: (admin.clone(), policies.clone()).into_val(env),
                sub_invokes: &[],
            },
        }])
        .initialize(admin, &policies);
}

struct CreateInput<'a> {
    buyer: &'a Address,
    seller: &'a Address,
    asset: &'a Address,
    cap: i128,
    resource_hash: &'a BytesN<32>,
}

fn create_session(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    input: CreateInput<'_>,
) -> BytesN<32> {
    client
        .mock_auths(&[MockAuth {
            address: input.buyer,
            invoke: &MockAuthInvoke {
                contract,
                fn_name: "create_session",
                args: (
                    input.buyer.clone(),
                    input.seller.clone(),
                    input.asset.clone(),
                    input.cap,
                    EXPIRY_LEDGER,
                    input.resource_hash.clone(),
                )
                    .into_val(env),
                sub_invokes: &[MockAuthInvoke {
                    contract: input.asset,
                    fn_name: "transfer",
                    args: (
                        input.buyer.clone(),
                        contract.clone(),
                        input.cap,
                    )
                        .into_val(env),
                    sub_invokes: &[],
                }],
            },
        }])
        .create_session(
            input.buyer,
            input.seller,
            input.asset,
            &input.cap,
            &EXPIRY_LEDGER,
            input.resource_hash,
        )
}

fn try_settle(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    seller: &Address,
    session_id: &BytesN<32>,
    amount: i128,
    usage_hash: &BytesN<32>,
) -> bool {
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
        .try_settle(session_id, &amount, usage_hash)
        .is_ok()
}

fn try_cancel(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    contract: &Address,
    buyer: &Address,
    session_id: &BytesN<32>,
) -> bool {
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
        .try_cancel(session_id)
        .is_ok()
}

fn selected_amount(selector: u8, cap: i128) -> i128 {
    match selector {
        0 => 0,
        1 => 1,
        2 => cap / 2 + cap % 2,
        3 => cap,
        _ => cap + 1,
    }
}

struct InvariantInput<'a> {
    contract: &'a Address,
    session_id: &'a BytesN<32>,
    buyer: &'a Address,
    seller: &'a Address,
    asset: &'a Address,
    model: ModelSession,
}

fn assert_session_invariants(
    env: &Env,
    client: &UptoSessionContractClient<'_>,
    input: InvariantInput<'_>,
) {
    let session = client.get_session(input.session_id);
    let token = token::Client::new(env, input.asset);
    let model = input.model;
    let (buyer_balance, seller_balance, contract_balance, escrowed, status) = match model.status {
        ModelStatus::Open => (0, 0, model.cap, model.cap, SessionStatus::Open),
        ModelStatus::Settled => (
            model.cap - model.settled,
            model.settled,
            0,
            0,
            SessionStatus::Settled,
        ),
        ModelStatus::Cancelled => (model.cap, 0, 0, 0, SessionStatus::Cancelled),
        ModelStatus::Expired => (model.cap, 0, 0, 0, SessionStatus::Expired),
    };

    assert_eq!(session.status, status);
    assert_eq!(session.settled_amount, model.settled);
    assert_eq!(session.escrowed_amount, escrowed);
    assert!(session.settled_amount <= session.max_amount);
    assert_eq!(token.balance(input.buyer), buyer_balance);
    assert_eq!(token.balance(input.seller), seller_balance);
    assert_eq!(token.balance(input.contract), contract_balance);
    assert_eq!(buyer_balance + seller_balance + contract_balance, model.cap);
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn generated_sequences_preserve_value_caps_and_isolation(
        (first_cap, second_cap) in cap_strategy(),
        actions in action_strategy(),
    ) {
        let env = Env::default();
        let contract = env.register(UptoSessionContract, ());
        let client = UptoSessionContractClient::new(&env, &contract);
        let admin = Address::generate(&env);
        let attacker = Address::generate(&env);
        let buyers = [Address::generate(&env), Address::generate(&env)];
        let sellers = [Address::generate(&env), Address::generate(&env)];
        let caps = [first_cap, second_cap];
        let assets = [
            create_asset(&env, &buyers[0], caps[0]),
            create_asset(&env, &buyers[1], caps[1]),
        ];
        let supported_assets = soroban_sdk::vec![&env, assets[0].clone(), assets[1].clone()];
        initialize(&env, &client, &contract, &admin, &supported_assets);
        let resource_hashes = [
            BytesN::from_array(&env, &[61; 32]),
            BytesN::from_array(&env, &[62; 32]),
        ];
        let session_ids = [
            create_session(
                &env,
                &client,
                &contract,
                CreateInput {
                    buyer: &buyers[0],
                    seller: &sellers[0],
                    asset: &assets[0],
                    cap: caps[0],
                    resource_hash: &resource_hashes[0],
                },
            ),
            create_session(
                &env,
                &client,
                &contract,
                CreateInput {
                    buyer: &buyers[1],
                    seller: &sellers[1],
                    asset: &assets[1],
                    cap: caps[1],
                    resource_hash: &resource_hashes[1],
                },
            ),
        ];
        let mut model = [
            ModelSession { cap: caps[0], settled: 0, status: ModelStatus::Open },
            ModelSession { cap: caps[1], settled: 0, status: ModelStatus::Open },
        ];
        let mut expired = false;

        for (step, (kind, raw_index, amount_selector)) in actions.into_iter().enumerate() {
            let index = usize::from(raw_index);
            let before = model[index];
            let usage_byte = u8::try_from(step % 190 + 63).expect("bounded usage byte");
            let usage_hash = BytesN::from_array(&env, &[usage_byte; 32]);

            match kind {
                0 => {
                    let amount = selected_amount(amount_selector, caps[index]);
                    let succeeded = try_settle(
                        &env,
                        &client,
                        &contract,
                        &sellers[index],
                        &session_ids[index],
                        amount,
                        &usage_hash,
                    );
                    let expected = before.status == ModelStatus::Open
                        && !expired
                        && amount > 0
                        && amount <= caps[index];
                    prop_assert_eq!(succeeded, expected);
                    if expected {
                        model[index].status = ModelStatus::Settled;
                        model[index].settled = amount;
                    }
                }
                1 => {
                    let succeeded = try_cancel(
                        &env,
                        &client,
                        &contract,
                        &buyers[index],
                        &session_ids[index],
                    );
                    let expected = before.status == ModelStatus::Open && !expired;
                    prop_assert_eq!(succeeded, expected);
                    if expected {
                        model[index].status = ModelStatus::Cancelled;
                    }
                }
                2 => {
                    env.ledger().set_sequence_number(EXPIRY_LEDGER);
                    expired = true;
                }
                3 => {
                    let succeeded = client.try_recover_expired(&session_ids[index]).is_ok();
                    let expected = before.status == ModelStatus::Open && expired;
                    prop_assert_eq!(succeeded, expected);
                    if expected {
                        model[index].status = ModelStatus::Expired;
                    }
                }
                4 => {
                    let first = try_settle(
                        &env,
                        &client,
                        &contract,
                        &sellers[index],
                        &session_ids[index],
                        1,
                        &usage_hash,
                    );
                    let expected = before.status == ModelStatus::Open && !expired;
                    prop_assert_eq!(first, expected);
                    if expected {
                        model[index].status = ModelStatus::Settled;
                        model[index].settled = 1;
                    }
                    prop_assert!(!try_settle(
                        &env,
                        &client,
                        &contract,
                        &sellers[index],
                        &session_ids[index],
                        1,
                        &usage_hash,
                    ));
                }
                _ => {
                    prop_assert!(!try_settle(
                        &env,
                        &client,
                        &contract,
                        &attacker,
                        &session_ids[index],
                        1,
                        &usage_hash,
                    ));
                }
            }

            for check_index in 0..2 {
                assert_session_invariants(
                    &env,
                    &client,
                    InvariantInput {
                        contract: &contract,
                        session_id: &session_ids[check_index],
                        buyer: &buyers[check_index],
                        seller: &sellers[check_index],
                        asset: &assets[check_index],
                        model: model[check_index],
                    },
                );
            }
        }
    }
}
