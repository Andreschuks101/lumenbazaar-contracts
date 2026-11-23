//! `policy` target: arbitrary spending policies for the example policy wallet, followed by up to
//! eight `check_payment` / `authorize_payment` steps. Byte layout is in `fuzz/README.md`.

use crate::defined;
use crate::input::Input;
use policy_wallet_example::{
    PolicyWalletError, PolicyWalletExampleContract, PolicyWalletExampleContractClient,
    ResourceSpendingLimit, SellerSpendingLimit, SpendingPolicy, LEDGERS_PER_DAY,
};
use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Ledger as _},
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, Vec,
};

const START_LEDGER: u32 = LEDGERS_PER_DAY * 3 - 3;
const STEPS_OFFSET: usize = 54;
const STEP_BYTES: usize = 8;
const MAX_STEPS: usize = 8;

fn mask(byte: u8, width: u32) -> u8 {
    match byte {
        0 => 1,
        byte => byte % (1 << width),
    }
}

fn snapshot(env: &Env, wallet: &Address) -> Bytes {
    env.as_contract(wallet, || env.storage().instance().all().to_xdr(env))
}

/// The documented `set_policy` rules, in the order the contract reports them.
fn expected_policy(policy: &SpendingPolicy, ledger: u32) -> Result<(), PolicyWalletError> {
    if policy.max_amount_per_payment <= 0
        || policy.max_amount_per_day <= 0
        || policy.max_authorizations_per_day == 0
        || policy.max_amount_per_payment > policy.max_amount_per_day
        || policy.valid_from_ledger > policy.valid_until_ledger
        || policy.allowed_sellers.is_empty()
        || policy.allowed_assets.is_empty()
        || policy.allowed_resource_hashes.is_empty()
    {
        return Err(PolicyWalletError::InvalidPolicy);
    }
    if policy.valid_until_ledger <= ledger {
        return Err(PolicyWalletError::PolicyExpired);
    }
    let limit_ok = |payment: i128, day: i128| {
        payment > 0
            && day > 0
            && payment <= day
            && payment <= policy.max_amount_per_payment
            && day <= policy.max_amount_per_day
    };
    for limit in policy.seller_limits.iter() {
        if !policy.allowed_sellers.contains(&limit.seller)
            || !limit_ok(limit.max_amount_per_payment, limit.max_amount_per_day)
        {
            return Err(PolicyWalletError::InvalidPolicy);
        }
    }
    for limit in policy.resource_limits.iter() {
        if !policy
            .allowed_resource_hashes
            .contains(&limit.resource_hash)
            || !limit_ok(limit.max_amount_per_payment, limit.max_amount_per_day)
        {
            return Err(PolicyWalletError::InvalidPolicy);
        }
    }
    Ok(())
}

pub fn policy(data: &[u8]) {
    let input = Input::new(data);
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(START_LEDGER);
    let wallet = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(&env, &wallet);
    client.initialize(&Address::generate(&env), &Address::generate(&env));

    // Index 3 (sellers, resources) and 2 (assets) are never allowlisted.
    let agent = Address::generate(&env);
    let sellers: [Address; 4] = core::array::from_fn(|_| Address::generate(&env));
    let assets: [Address; 3] = core::array::from_fn(|_| Address::generate(&env));
    let resources: [BytesN<32>; 4] =
        core::array::from_fn(|i| BytesN::from_array(&env, &[0x40 + i as u8; 32]));

    let per_payment = input.amount(0, 75);
    let per_day = input.amount(17, 100);
    let max_authorizations = match input.u8(34) % 4 {
        0 => 10,
        1 => 0,
        2 => u32::MAX,
        _ => input.u32(35),
    };
    let valid_from = match input.u8(39) % 4 {
        0 => START_LEDGER,
        1 => START_LEDGER + 1,
        2 => input.u32(40),
        _ => 0,
    };
    let valid_until = match input.u8(44) % 5 {
        0 => START_LEDGER + 100,
        1 => START_LEDGER,
        2 => START_LEDGER - 1,
        3 => input.u32(45),
        _ => u32::MAX,
    };
    let (seller_mask, asset_mask, resource_mask) = (
        mask(input.u8(49), 3),
        mask(input.u8(50), 2),
        mask(input.u8(51), 3),
    );
    let pick = |mask: u8, i: usize| mask & (1 << i) != 0;

    let mut policy = SpendingPolicy {
        agent: agent.clone(),
        max_amount_per_payment: per_payment,
        max_amount_per_day: per_day,
        max_authorizations_per_day: max_authorizations,
        valid_from_ledger: valid_from,
        valid_until_ledger: valid_until,
        allowed_sellers: Vec::new(&env),
        allowed_assets: Vec::new(&env),
        allowed_resource_hashes: Vec::new(&env),
        seller_limits: Vec::new(&env),
        resource_limits: Vec::new(&env),
        revision: 0,
    };
    for (i, (seller, resource)) in sellers.iter().zip(&resources).take(3).enumerate() {
        if pick(seller_mask, i) {
            policy.allowed_sellers.push_back(seller.clone());
        }
        if pick(resource_mask, i) {
            policy.allowed_resource_hashes.push_back(resource.clone());
        }
    }
    for (i, asset) in assets.iter().take(2).enumerate() {
        if pick(asset_mask, i) {
            policy.allowed_assets.push_back(asset.clone());
        }
    }
    let half = |value: i128| (value / 2).max(1);
    // Limit selector: 0 none, 1 valid halves, 2 swapped caps, 3 for an unlisted entry.
    let limit_caps = |selector: u8| match selector % 4 {
        0 => None,
        2 => Some((per_day, per_payment)),
        _ => Some((
            half(per_payment).min(per_payment),
            half(per_day).min(per_day),
        )),
    };
    let first_seller = (0..3).find(|&i| pick(seller_mask, i));
    let first_resource = (0..3).find(|&i| pick(resource_mask, i));
    if let Some((payment, day)) = limit_caps(input.u8(52)) {
        let seller = match (input.u8(52) % 4, first_seller) {
            (3, _) | (_, None) => sellers[3].clone(),
            (_, Some(i)) => sellers[i].clone(),
        };
        policy.seller_limits.push_back(SellerSpendingLimit {
            seller,
            max_amount_per_payment: payment,
            max_amount_per_day: day,
        });
    }
    if let Some((payment, day)) = limit_caps(input.u8(53)) {
        let resource_hash = match (input.u8(53) % 4, first_resource) {
            (3, _) | (_, None) => resources[3].clone(),
            (_, Some(i)) => resources[i].clone(),
        };
        policy.resource_limits.push_back(ResourceSpendingLimit {
            resource_hash,
            max_amount_per_payment: payment,
            max_amount_per_day: day,
        });
    }

    let before = snapshot(&env, &wallet);
    let expected = expected_policy(&policy, START_LEDGER);
    let result = defined(client.try_set_policy(&policy, &0));
    assert_eq!(result, expected, "set_policy result");
    if result.is_err() {
        assert_eq!(
            snapshot(&env, &wallet),
            before,
            "rejected set_policy changed state"
        );
        return;
    }

    let steps = (data.len().saturating_sub(STEPS_OFFSET) / STEP_BYTES).clamp(1, MAX_STEPS);
    for step in 0..steps {
        let at = STEPS_OFFSET + step * STEP_BYTES;
        let ledger = env.ledger().sequence();
        let next_day = (ledger / LEDGERS_PER_DAY + 1) * LEDGERS_PER_DAY;
        let ledger = match input.u8(at) % 4 {
            1 => ledger + 1,
            2 => next_day - 1,
            3 => next_day,
            _ => ledger,
        };
        env.ledger().set_sequence_number(ledger);
        let seller = sellers[(input.u8(at + 1) % 4) as usize].clone();
        let asset = assets[(input.u8(at + 2) % 3) as usize].clone();
        let resource = resources[(input.u8(at + 3) % 4) as usize].clone();
        let amount = match input.u8(at + 4) % 6 {
            0 => 1,
            1 => 0,
            2 => -1,
            3 => i128::MAX,
            4 => per_payment,
            _ => i128::from(u16::from_be_bytes([input.u8(at + 5), input.u8(at + 6)])),
        };
        let authorize = input.u8(at + 7).is_multiple_of(2);

        let before = snapshot(&env, &wallet);
        let checked =
            defined(client.try_check_payment(&agent, &seller, &asset, &amount, &resource));
        assert_eq!(
            snapshot(&env, &wallet),
            before,
            "check_payment changed state"
        );
        if !authorize {
            continue;
        }
        let authorized =
            defined(client.try_authorize_payment(&agent, &seller, &asset, &amount, &resource));
        assert_eq!(
            authorized, checked,
            "check_payment and authorize_payment disagree"
        );
        if authorized.is_err() {
            assert_eq!(
                snapshot(&env, &wallet),
                before,
                "rejected authorize changed state"
            );
            continue;
        }
        assert!(
            amount > 0 && amount <= per_payment,
            "authorized amount outside cap"
        );
        assert!(client.spent_today(&agent) <= per_day, "daily cap exceeded");
        assert!(client.authorized_count_today(&agent) <= max_authorizations);
        for limit in policy.seller_limits.iter() {
            if limit.seller == seller {
                assert!(amount <= limit.max_amount_per_payment);
            }
            assert!(client.spent_today_seller(&agent, &limit.seller) <= limit.max_amount_per_day);
        }
        for limit in policy.resource_limits.iter() {
            if limit.resource_hash == resource {
                assert!(amount <= limit.max_amount_per_payment);
            }
            assert!(
                client.spent_today_resource(&agent, &limit.resource_hash)
                    <= limit.max_amount_per_day
            );
        }
    }
}
