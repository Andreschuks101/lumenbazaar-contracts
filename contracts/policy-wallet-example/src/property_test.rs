// Randomized invariant properties for the example policy wallet. Each property (P1 .. P7) is
// listed with the test that proves it in docs/invariants.md. The wallet is example code and these
// properties do not make it audited or production ready.
//
// Every generated case runs a sequence of `check_payment` / `authorize_payment` calls against the
// real contract and a reference model of the documented rules, across ledger-day boundaries.

extern crate std;

use super::*;
use proptest::{
    collection,
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use soroban_sdk::{testutils::storage::Instance as _, xdr::ToXdr, Bytes};
use std::collections::BTreeMap;

const POLICY_CASES: u32 = 64;
const POLICY_SEED: u64 = 0x4c55_4d45_504f_4c49;
/// Three ledgers before a day boundary, so short sequences already roll over.
const START_LEDGER: u32 = LEDGERS_PER_DAY * 3 - 3;
const SELLERS: usize = 3;
const ASSETS: usize = 2;
const RESOURCES: usize = 3;

fn property_config(default_cases: u32) -> ProptestConfig {
    let cases = std::env::var("LUMENBAZAAR_POLICY_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default_cases);
    let seed = std::env::var("LUMENBAZAAR_POLICY_SEED")
        .ok()
        .and_then(|value| u64::from_str_radix(value.trim_start_matches("0x"), 16).ok())
        .unwrap_or(POLICY_SEED);
    std::println!("policy campaign seed=0x{seed:016x} cases={cases}; replay: LUMENBAZAAR_POLICY_SEED=0x{seed:016x} LUMENBAZAAR_POLICY_CASES={cases} cargo test -p policy-wallet-example property_test -- --nocapture --test-threads=1");
    ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(seed),
        max_shrink_iters: 2_000,
        ..ProptestConfig::default()
    }
}

#[derive(Clone, Debug)]
struct PolicySpec {
    per_payment: i128,
    per_day: i128,
    max_authorizations: u32,
    valid_from: u32,
    valid_until: u32,
    seller_mask: u8,
    asset_mask: u8,
    resource_mask: u8,
    seller_limit: Option<(i128, i128)>,
    resource_limit: Option<(i128, i128)>,
}

#[derive(Clone, Copy, Debug)]
struct Step {
    advance: u8,
    advance_by: u32,
    seller: u8,
    asset: u8,
    resource: u8,
    amount: u8,
    raw_amount: i128,
    authorize: bool,
}

/// Synthetic actors. Index `SELLERS`, `ASSETS`, and `RESOURCES` are never in any allowlist.
struct Actors {
    agent: Address,
    sellers: std::vec::Vec<Address>,
    assets: std::vec::Vec<Address>,
    resources: std::vec::Vec<BytesN<32>>,
}

fn cap_value() -> impl Strategy<Value = i128> {
    prop_oneof![
        Just(1_i128),
        Just(2_i128),
        Just(100_i128),
        1_i128..=10_000,
        Just(i128::MAX - 1),
        Just(i128::MAX),
    ]
}

fn limit_within(per_payment: i128, per_day: i128) -> impl Strategy<Value = Option<(i128, i128)>> {
    proptest::option::of((cap_value(), cap_value())).prop_map(move |limit| {
        limit.map(|(a, b)| {
            let day = a.max(b).min(per_day);
            let payment = a.min(b).min(per_payment).min(day);
            (payment, day)
        })
    })
}

fn policy_strategy() -> impl Strategy<Value = PolicySpec> {
    (
        cap_value(),
        cap_value(),
        prop_oneof![1_u32..=4, Just(u32::MAX)],
        -3_i64..=3,
        prop_oneof![1_u32..=4, 1_u32..=(LEDGERS_PER_DAY * 2)],
        1_u8..8,
        1_u8..4,
        1_u8..8,
    )
        .prop_flat_map(
            |(a, b, max_authorizations, from_offset, until_offset, sellers, assets, resources)| {
                let per_payment = a.min(b);
                let per_day = a.max(b);
                let valid_from = (i64::from(START_LEDGER) + from_offset) as u32;
                let valid_until = (START_LEDGER + until_offset).max(valid_from);
                (
                    limit_within(per_payment, per_day),
                    limit_within(per_payment, per_day),
                )
                    .prop_map(move |(seller_limit, resource_limit)| PolicySpec {
                        per_payment,
                        per_day,
                        max_authorizations,
                        valid_from,
                        valid_until,
                        seller_mask: sellers,
                        asset_mask: assets,
                        resource_mask: resources,
                        seller_limit,
                        resource_limit,
                    })
            },
        )
}

fn steps_strategy() -> impl Strategy<Value = std::vec::Vec<Step>> {
    let step = (
        0_u8..6,
        1_u32..50,
        0_u8..=(SELLERS as u8),
        0_u8..=(ASSETS as u8),
        0_u8..=(RESOURCES as u8),
        0_u8..10,
        any::<i128>(),
        any::<bool>(),
    )
        .prop_map(
            |(advance, advance_by, seller, asset, resource, amount, raw_amount, authorize)| Step {
                advance,
                advance_by,
                seller,
                asset,
                resource,
                amount,
                raw_amount,
                authorize,
            },
        );
    collection::vec(step, 1..24)
}

fn first_bit(mask: u8) -> usize {
    mask.trailing_zeros() as usize
}

fn allowed(mask: u8, index: usize) -> bool {
    index < 8 && mask & (1 << index) != 0
}

fn build_policy(env: &Env, actors: &Actors, spec: &PolicySpec) -> SpendingPolicy {
    let mut sellers = Vec::new(env);
    for (i, seller) in actors.sellers.iter().enumerate().take(SELLERS) {
        if allowed(spec.seller_mask, i) {
            sellers.push_back(seller.clone());
        }
    }
    let mut assets = Vec::new(env);
    for (i, asset) in actors.assets.iter().enumerate().take(ASSETS) {
        if allowed(spec.asset_mask, i) {
            assets.push_back(asset.clone());
        }
    }
    let mut resources = Vec::new(env);
    for (i, resource) in actors.resources.iter().enumerate().take(RESOURCES) {
        if allowed(spec.resource_mask, i) {
            resources.push_back(resource.clone());
        }
    }
    let mut seller_limits = Vec::new(env);
    if let Some((payment, day)) = spec.seller_limit {
        seller_limits.push_back(SellerSpendingLimit {
            seller: actors.sellers[first_bit(spec.seller_mask)].clone(),
            max_amount_per_payment: payment,
            max_amount_per_day: day,
        });
    }
    let mut resource_limits = Vec::new(env);
    if let Some((payment, day)) = spec.resource_limit {
        resource_limits.push_back(ResourceSpendingLimit {
            resource_hash: actors.resources[first_bit(spec.resource_mask)].clone(),
            max_amount_per_payment: payment,
            max_amount_per_day: day,
        });
    }
    SpendingPolicy {
        agent: actors.agent.clone(),
        max_amount_per_payment: spec.per_payment,
        max_amount_per_day: spec.per_day,
        max_authorizations_per_day: spec.max_authorizations,
        valid_from_ledger: spec.valid_from,
        valid_until_ledger: spec.valid_until,
        allowed_sellers: sellers,
        allowed_assets: assets,
        allowed_resource_hashes: resources,
        seller_limits,
        resource_limits,
        revision: 0,
    }
}

/// Reference model: spend recorded per ledger day, keyed exactly like the contract.
#[derive(Default)]
struct Model {
    spent: BTreeMap<u32, i128>,
    seller_spent: BTreeMap<(usize, u32), i128>,
    resource_spent: BTreeMap<(usize, u32), i128>,
    authorizations: BTreeMap<u32, u32>,
}

impl Model {
    /// The documented rules, in the order the contract reports them.
    fn expected(
        &self,
        spec: &PolicySpec,
        ledger: u32,
        seller: usize,
        asset: usize,
        resource: usize,
        amount: i128,
    ) -> Result<(), PolicyWalletError> {
        let day = ledger / LEDGERS_PER_DAY;
        if ledger < spec.valid_from {
            return Err(PolicyWalletError::PolicyNotActive);
        }
        if ledger > spec.valid_until {
            return Err(PolicyWalletError::PolicyExpired);
        }
        if amount <= 0 {
            return Err(PolicyWalletError::InvalidAmount);
        }
        if amount > spec.per_payment {
            return Err(PolicyWalletError::AmountExceedsPaymentCap);
        }
        if !allowed(spec.seller_mask, seller) || seller >= SELLERS {
            return Err(PolicyWalletError::SellerNotAllowed);
        }
        if !allowed(spec.asset_mask, asset) || asset >= ASSETS {
            return Err(PolicyWalletError::AssetNotAllowed);
        }
        if !allowed(spec.resource_mask, resource) || resource >= RESOURCES {
            return Err(PolicyWalletError::ResourceHashNotAllowed);
        }
        if self.authorizations.get(&day).copied().unwrap_or(0) >= spec.max_authorizations {
            return Err(PolicyWalletError::AuthorizationLimitExceeded);
        }
        let within = |spent: i128, cap: i128| spent.checked_add(amount).is_some_and(|n| n <= cap);
        if !within(self.spent.get(&day).copied().unwrap_or(0), spec.per_day) {
            return Err(PolicyWalletError::AmountExceedsDailyCap);
        }
        if let Some((payment, cap)) = spec.seller_limit {
            if seller == first_bit(spec.seller_mask) {
                if amount > payment {
                    return Err(PolicyWalletError::AmountExceedsPaymentCap);
                }
                let spent = self.seller_spent.get(&(seller, day)).copied().unwrap_or(0);
                if !within(spent, cap) {
                    return Err(PolicyWalletError::AmountExceedsDailyCap);
                }
            }
        }
        if let Some((payment, cap)) = spec.resource_limit {
            if resource == first_bit(spec.resource_mask) {
                if amount > payment {
                    return Err(PolicyWalletError::AmountExceedsPaymentCap);
                }
                let spent = self
                    .resource_spent
                    .get(&(resource, day))
                    .copied()
                    .unwrap_or(0);
                if !within(spent, cap) {
                    return Err(PolicyWalletError::AmountExceedsDailyCap);
                }
            }
        }
        Ok(())
    }

    fn record(&mut self, ledger: u32, seller: usize, resource: usize, amount: i128) {
        let day = ledger / LEDGERS_PER_DAY;
        *self.spent.entry(day).or_default() += amount;
        *self.seller_spent.entry((seller, day)).or_default() += amount;
        *self.resource_spent.entry((resource, day)).or_default() += amount;
        *self.authorizations.entry(day).or_default() += 1;
    }
}

/// Full wallet instance storage (config, policies, every spend counter), encoded as XDR so two
/// snapshots compare byte for byte.
fn snapshot(env: &Env, wallet: &Address) -> Bytes {
    env.as_contract(wallet, || env.storage().instance().all().to_xdr(env))
}

/// P7: every call returns `Ok` or a defined `PolicyWalletError`; a host panic or an undecodable
/// result fails the property.
fn defined<E: core::fmt::Debug, I: core::fmt::Debug>(
    result: Result<Result<(), E>, Result<PolicyWalletError, I>>,
) -> Result<(), PolicyWalletError> {
    match result {
        Ok(Ok(())) => Ok(()),
        Err(Ok(error)) => Err(error),
        other => panic!("call did not return a defined result: {other:?}"),
    }
}

fn setup(env: &Env) -> (PolicyWalletExampleContractClient<'_>, Actors) {
    env.mock_all_auths();
    env.ledger().set_sequence_number(START_LEDGER);
    let wallet = env.register(PolicyWalletExampleContract, ());
    let client = PolicyWalletExampleContractClient::new(env, &wallet);
    client.initialize(&Address::generate(env), &Address::generate(env));
    let actors = Actors {
        agent: Address::generate(env),
        sellers: (0..=SELLERS).map(|_| Address::generate(env)).collect(),
        assets: (0..=ASSETS).map(|_| Address::generate(env)).collect(),
        resources: (0..=RESOURCES)
            .map(|i| BytesN::from_array(env, &[0x40 + i as u8; 32]))
            .collect(),
    };
    (client, actors)
}

fn next_day_start(ledger: u32) -> u32 {
    (ledger / LEDGERS_PER_DAY + 1) * LEDGERS_PER_DAY
}

fn advance(env: &Env, step: &Step) {
    let ledger = env.ledger().sequence();
    let next = match step.advance {
        1 => ledger + 1,
        2 => ledger + step.advance_by,
        3 => next_day_start(ledger) - 1,
        4 => next_day_start(ledger),
        5 => next_day_start(ledger) + 1,
        _ => ledger,
    };
    env.ledger().set_sequence_number(next);
}

fn amount_for(step: &Step, spec: &PolicySpec, model: &Model, ledger: u32) -> i128 {
    let spent = model
        .spent
        .get(&(ledger / LEDGERS_PER_DAY))
        .copied()
        .unwrap_or(0);
    match step.amount {
        0 => 0,
        1 => 1,
        2 => -1,
        3 => i128::MIN,
        4 => i128::MAX,
        5 => spec.per_payment,
        6 => spec.per_payment.saturating_add(1),
        7 => (spec.per_day - spent).max(1),
        8 => step.raw_amount.rem_euclid(10_000) + 1,
        _ => step.raw_amount,
    }
}

/// One observed step: the shared result of check and authorize, and today's recorded totals.
type Trace = std::vec::Vec<(u32, Result<(), PolicyWalletError>, i128, u32)>;

/// Runs one generated case and asserts P1, P2, P3, P5, P6, and P7 after every step.
fn run_case(spec: &PolicySpec, steps: &[Step]) -> Trace {
    let env = Env::default();
    let (client, actors) = setup(&env);
    client.set_policy(&build_policy(&env, &actors, spec), &0);
    let wallet = client.address.clone();
    let mut model = Model::default();
    let mut trace = Trace::new();

    for step in steps {
        advance(&env, step);
        let ledger = env.ledger().sequence();
        let (seller, asset, resource) = (
            step.seller as usize,
            step.asset as usize,
            step.resource as usize,
        );
        let amount = amount_for(step, spec, &model, ledger);
        let expected = model.expected(spec, ledger, seller, asset, resource, amount);
        let args = (
            &actors.agent,
            &actors.sellers[seller],
            &actors.assets[asset],
            &amount,
            &actors.resources[resource],
        );

        let before = snapshot(&env, &wallet);
        let checked = defined(client.try_check_payment(args.0, args.1, args.2, args.3, args.4));
        // P5: the result is exactly the documented composition of every constraint.
        assert_eq!(
            checked, expected,
            "check_payment at ledger {ledger}, amount {amount}"
        );
        // P3: check never mutates state, accepted or rejected.
        assert_eq!(
            snapshot(&env, &wallet),
            before,
            "check_payment mutated state"
        );

        let mut result = checked;
        if step.authorize {
            let authorized =
                defined(client.try_authorize_payment(args.0, args.1, args.2, args.3, args.4));
            // P6: authorize agrees with the check made in the same state.
            assert_eq!(authorized, checked, "check and authorize disagree");
            match authorized {
                Ok(()) => {
                    // P1: an authorized payment is within every applicable per-payment cap.
                    assert!(amount > 0 && amount <= spec.per_payment);
                    if let Some((payment, _)) = spec.seller_limit {
                        assert!(seller != first_bit(spec.seller_mask) || amount <= payment);
                    }
                    if let Some((payment, _)) = spec.resource_limit {
                        assert!(resource != first_bit(spec.resource_mask) || amount <= payment);
                    }
                    model.record(ledger, seller, resource, amount);
                }
                // P3: a rejected authorization leaves every counter untouched.
                Err(_) => assert_eq!(
                    snapshot(&env, &wallet),
                    before,
                    "rejected authorize mutated state"
                ),
            }
            result = authorized;
        }

        let day = ledger / LEDGERS_PER_DAY;
        let spent = client.spent_today(&actors.agent);
        let count = client.authorized_count_today(&actors.agent);
        assert_eq!(spent, model.spent.get(&day).copied().unwrap_or(0));
        assert_eq!(count, model.authorizations.get(&day).copied().unwrap_or(0));
        // P2: cumulative authorized spend today never exceeds any daily cap.
        assert!(spent <= spec.per_day);
        assert!(count <= spec.max_authorizations);
        if let Some((_, cap)) = spec.seller_limit {
            let seller = &actors.sellers[first_bit(spec.seller_mask)];
            assert!(client.spent_today_seller(&actors.agent, seller) <= cap);
        }
        if let Some((_, cap)) = spec.resource_limit {
            let resource = &actors.resources[first_bit(spec.resource_mask)];
            assert!(client.spent_today_resource(&actors.agent, resource) <= cap);
        }
        trace.push((ledger, result, spent, count));
    }
    trace
}

proptest! {
    #![proptest_config(property_config(POLICY_CASES))]

    /// P1, P2, P3, P5, P6, P7 over random policies, payments, and day boundaries.
    #[test]
    fn p1_p2_p3_p5_p6_p7_random_policies_hold_every_invariant(
        spec in policy_strategy(),
        steps in steps_strategy(),
    ) {
        run_case(&spec, &steps);
    }
}

proptest! {
    #![proptest_config(property_config(POLICY_CASES / 4))]

    /// P4: identical policy, ledger sequence, and inputs give identical results and totals.
    #[test]
    fn p4_identical_inputs_give_identical_results(
        spec in policy_strategy(),
        steps in steps_strategy(),
    ) {
        prop_assert_eq!(run_case(&spec, &steps), run_case(&spec, &steps));
    }
}

fn pinned_policy(env: &Env, actors: &Actors, per_day: i128, valid_until: u32) -> SpendingPolicy {
    build_policy(
        env,
        actors,
        &PolicySpec {
            per_payment: per_day,
            per_day,
            max_authorizations: 2,
            valid_from: START_LEDGER,
            valid_until,
            seller_mask: 1,
            asset_mask: 1,
            resource_mask: 1,
            seller_limit: None,
            resource_limit: None,
        },
    )
}

/// P4: the day boundary is `sequence / LEDGERS_PER_DAY`; spend and authorization counts reset
/// exactly at its first ledger and not one ledger earlier or later.
#[test]
fn p4_day_rollover_is_pinned_at_the_boundary() {
    let env = Env::default();
    let (client, actors) = setup(&env);
    let boundary = next_day_start(START_LEDGER);
    client.set_policy(
        &pinned_policy(&env, &actors, 100, boundary + LEDGERS_PER_DAY),
        &0,
    );
    let pay = |amount: i128| {
        defined(client.try_authorize_payment(
            &actors.agent,
            &actors.sellers[0],
            &actors.assets[0],
            &amount,
            &actors.resources[0],
        ))
    };

    env.ledger().set_sequence_number(boundary - 1);
    assert_eq!(pay(100), Ok(()));
    assert_eq!(pay(1), Err(PolicyWalletError::AmountExceedsDailyCap));
    assert_eq!(client.spent_today(&actors.agent), 100);

    env.ledger().set_sequence_number(boundary);
    assert_eq!(client.spent_today(&actors.agent), 0);
    assert_eq!(client.authorized_count_today(&actors.agent), 0);
    assert_eq!(pay(60), Ok(()));
    assert_eq!(pay(40), Ok(()));
    assert_eq!(pay(1), Err(PolicyWalletError::AuthorizationLimitExceeded));

    env.ledger().set_sequence_number(boundary + 1);
    assert_eq!(client.spent_today(&actors.agent), 100);
    assert_eq!(pay(1), Err(PolicyWalletError::AuthorizationLimitExceeded));

    // Going back to the previous day reads that day's record, unchanged by later days.
    env.ledger().set_sequence_number(boundary - 1);
    assert_eq!(client.spent_today(&actors.agent), 100);
}

/// P5: the validity window is inclusive at both ends, including expiry exactly at the ledger.
#[test]
fn p5_validity_window_is_inclusive_at_both_ends() {
    let env = Env::default();
    let (client, actors) = setup(&env);
    let valid_until = START_LEDGER + 10;
    let mut policy = pinned_policy(&env, &actors, 100, valid_until);
    policy.valid_from_ledger = START_LEDGER + 2;
    client.set_policy(&policy, &0);
    let check = || {
        defined(client.try_check_payment(
            &actors.agent,
            &actors.sellers[0],
            &actors.assets[0],
            &1,
            &actors.resources[0],
        ))
    };

    env.ledger().set_sequence_number(START_LEDGER + 1);
    assert_eq!(check(), Err(PolicyWalletError::PolicyNotActive));
    env.ledger().set_sequence_number(START_LEDGER + 2);
    assert_eq!(check(), Ok(()));
    env.ledger().set_sequence_number(valid_until);
    assert_eq!(check(), Ok(()));
    env.ledger().set_sequence_number(valid_until + 1);
    assert_eq!(check(), Err(PolicyWalletError::PolicyExpired));
}

/// P5: degenerate policies (zero or negative caps, zero authorizations, inverted caps or windows,
/// empty allowlists, limits for unlisted entries, already expired) are rejected and change
/// nothing.
#[test]
fn p5_degenerate_policies_are_rejected_without_state_change() {
    let env = Env::default();
    let (client, actors) = setup(&env);
    let wallet = client.address.clone();
    let valid = pinned_policy(&env, &actors, 100, START_LEDGER + 10);
    let unlisted_seller = actors.sellers[SELLERS].clone();
    let unlisted_resource = actors.resources[RESOURCES].clone();
    type Mutation = fn(&mut SpendingPolicy, &Env, &Address, &BytesN<32>);
    let mutations: [(&str, Mutation); 12] = [
        ("zero payment cap", |p, _, _, _| {
            p.max_amount_per_payment = 0
        }),
        ("zero daily cap", |p, _, _, _| p.max_amount_per_day = 0),
        ("negative caps", |p, _, _, _| {
            p.max_amount_per_payment = -1;
            p.max_amount_per_day = -1;
        }),
        ("zero authorizations", |p, _, _, _| {
            p.max_authorizations_per_day = 0
        }),
        ("payment cap above daily cap", |p, _, _, _| {
            p.max_amount_per_payment = 101
        }),
        ("inverted window", |p, _, _, _| {
            p.valid_from_ledger = p.valid_until_ledger + 1
        }),
        ("already expired", |p, _, _, _| {
            p.valid_until_ledger = START_LEDGER
        }),
        ("no sellers", |p, env, _, _| {
            p.allowed_sellers = Vec::new(env)
        }),
        ("no assets", |p, env, _, _| p.allowed_assets = Vec::new(env)),
        ("no resources", |p, env, _, _| {
            p.allowed_resource_hashes = Vec::new(env)
        }),
        ("limit for unlisted seller", |p, env, seller, _| {
            p.seller_limits = soroban_sdk::vec![
                env,
                SellerSpendingLimit {
                    seller: seller.clone(),
                    max_amount_per_payment: 1,
                    max_amount_per_day: 1,
                }
            ]
        }),
        ("limit for unlisted resource", |p, env, _, resource| {
            p.resource_limits = soroban_sdk::vec![
                env,
                ResourceSpendingLimit {
                    resource_hash: resource.clone(),
                    max_amount_per_payment: 1,
                    max_amount_per_day: 1,
                }
            ]
        }),
    ];

    for (name, mutate) in mutations {
        let mut policy = valid.clone();
        mutate(&mut policy, &env, &unlisted_seller, &unlisted_resource);
        let before = snapshot(&env, &wallet);
        let result = defined(client.try_set_policy(&policy, &0));
        assert!(
            matches!(
                result,
                Err(PolicyWalletError::InvalidPolicy | PolicyWalletError::PolicyExpired)
            ),
            "{name}: {result:?}"
        );
        assert_eq!(snapshot(&env, &wallet), before, "{name} changed state");
    }
}

/// P7: amounts at the arithmetic extremes return defined rejections and never overflow.
#[test]
fn p7_extreme_amounts_return_defined_errors() {
    let env = Env::default();
    let (client, actors) = setup(&env);
    let mut policy = pinned_policy(&env, &actors, i128::MAX, START_LEDGER + 10);
    policy.max_authorizations_per_day = u32::MAX;
    client.set_policy(&policy, &0);
    let pay = |amount: i128| {
        defined(client.try_authorize_payment(
            &actors.agent,
            &actors.sellers[0],
            &actors.assets[0],
            &amount,
            &actors.resources[0],
        ))
    };

    for amount in [0, -1, i128::MIN, i128::MIN + 1] {
        assert_eq!(pay(amount), Err(PolicyWalletError::InvalidAmount));
    }
    assert_eq!(pay(i128::MAX - 1), Ok(()));
    assert_eq!(pay(2), Err(PolicyWalletError::AmountExceedsDailyCap));
    assert_eq!(pay(1), Ok(()));
    assert_eq!(client.spent_today(&actors.agent), i128::MAX);
    // `i128::MAX + 1` would overflow; the checked addition reports the daily cap instead.
    assert_eq!(pay(1), Err(PolicyWalletError::AmountExceedsDailyCap));
    assert_eq!(
        pay(i128::MAX),
        Err(PolicyWalletError::AmountExceedsDailyCap)
    );
    assert_eq!(client.spent_today(&actors.agent), i128::MAX);
}
