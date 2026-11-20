// Storage lifecycle boundary tests. Every test name starts with the rule it verifies; the rules
// (SR-1 .. SR-10) are defined in docs/storage-lifecycle.md, which maps each rule back to these
// tests.
//
// Host semantics relied on (soroban-env-host 28): `extend_ttl(threshold, extend_to)` sets
// `live_until = sequence + extend_to` only when `live_until - sequence <= threshold`, and the
// test `get_ttl()` helpers return `live_until - sequence`. The unit-test host does not model
// archival of an entry whose TTL has run out, so a lapsed entry is modelled by removing it. These
// tests make no claim that archived entries can be restored.

use super::*;
use crate::storage::{DataKey, DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT};
use crate::validation::MAX_SESSION_DURATION_LEDGERS;
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};

const CAP: i128 = 1_000;
const EXPIRY: u32 = 500;
/// Ledger at which an entry extended at ledger 0 has exactly `DEFAULT_TTL_THRESHOLD` left.
const AT_THRESHOLD: u32 = TTL_EXTENSION_AMOUNT - DEFAULT_TTL_THRESHOLD;

// SR-5: an open session reaches logical expiry before its storage TTL reaches the threshold.
const _: () = assert!(MAX_SESSION_DURATION_LEDGERS < AT_THRESHOLD);

struct Lifecycle {
    env: Env,
    contract_id: Address,
    buyer: Address,
    seller: Address,
    asset: Address,
    next_idempotency: core::cell::Cell<u8>,
}

impl Lifecycle {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let default_min_ttl = env.ledger().get().min_persistent_entry_ttl;
        let contract_id = env.register(UptoSessionContract, ());
        let admin = Address::generate(&env);
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);

        // The host refuses to extend an entry that is no longer live, and the token contract
        // extends its own instance and balance entries on use. Create those entries with a TTL
        // that outlives every ledger these tests reach, then restore the default so the escrow
        // contract's entries follow the TTL rules under test.
        env.ledger()
            .set_min_persistent_entry_ttl(2 * TTL_EXTENSION_AMOUNT);
        let asset = create_test_asset(&env, &buyer, CAP * 10);
        let token_admin = token::StellarAssetClient::new(&env, &asset);
        token_admin.mint(&seller, &1);
        token_admin.mint(&contract_id, &1);
        env.ledger().set_min_persistent_entry_ttl(default_min_ttl);

        let client = UptoSessionContractClient::new(&env, &contract_id);
        initialize_for_asset(&env, &client, &admin, &asset);
        Self {
            env,
            contract_id,
            buyer,
            seller,
            asset,
            next_idempotency: core::cell::Cell::new(1),
        }
    }

    fn client(&self) -> UptoSessionContractClient<'_> {
        UptoSessionContractClient::new(&self.env, &self.contract_id)
    }

    fn next_key(&self) -> BytesN<32> {
        let n = self.next_idempotency.get();
        self.next_idempotency.set(n.wrapping_add(1).max(1));
        BytesN::from_array(&self.env, &[n; 32])
    }

    fn create(&self, expires_at_ledger: u32) -> BytesN<32> {
        self.client().create_session(
            &self.buyer,
            &self.seller,
            &self.asset,
            &CAP,
            &expires_at_ledger,
            &BytesN::from_array(&self.env, &[7; 32]),
            &self.next_key(),
        )
    }

    fn usage_hash(&self) -> BytesN<32> {
        BytesN::from_array(&self.env, &[8; 32])
    }

    fn ledger(&self, sequence: u32) {
        self.env.ledger().set_sequence_number(sequence);
    }

    /// Re-sets the existing cap. This is the idempotent admin call operators use to keep the
    /// instance alive.
    fn admin_touch(&self) {
        self.client()
            .set_asset_max_session_amount(&self.asset, &i128::MAX);
    }

    fn instance_ttl(&self) -> u32 {
        self.env.as_contract(&self.contract_id, || {
            self.env.storage().instance().get_ttl()
        })
    }

    fn persistent_ttl(&self, key: &DataKey) -> u32 {
        self.env.as_contract(&self.contract_id, || {
            self.env.storage().persistent().get_ttl(key)
        })
    }

    fn remove_persistent(&self, key: &DataKey) {
        self.env.as_contract(&self.contract_id, || {
            self.env.storage().persistent().remove(key)
        });
    }

    fn session_key(&self, id: &BytesN<32>) -> DataKey {
        DataKey::Session(id.clone())
    }

    /// The four aggregate keys every open session of this harness touches.
    fn counter_keys(&self) -> [DataKey; 4] {
        [
            DataKey::Liability(self.asset.clone()),
            DataKey::SellerExposure(self.seller.clone(), self.asset.clone()),
            DataKey::BuyerOpenSessions(self.buyer.clone()),
            DataKey::SellerOpenSessions(self.seller.clone()),
        ]
    }

    fn session_and_counter_ttls(&self, id: &BytesN<32>) -> [u32; 5] {
        let [a, b, c, d] = self.counter_keys();
        [
            self.persistent_ttl(&self.session_key(id)),
            self.persistent_ttl(&a),
            self.persistent_ttl(&b),
            self.persistent_ttl(&c),
            self.persistent_ttl(&d),
        ]
    }

    fn balances(&self) -> (i128, i128, i128) {
        let token = token::Client::new(&self.env, &self.asset);
        (
            token.balance(&self.buyer),
            token.balance(&self.seller),
            token.balance(&self.contract_id),
        )
    }

    fn events(&self) -> usize {
        contract_event_count(&self.env, &self.contract_id)
    }
}

#[test]
fn sr1_instance_keys_share_one_ttl_set_by_initialize() {
    let h = Lifecycle::new();
    h.client().propose_admin(&Address::generate(&h.env), &1_000);
    h.create(EXPIRY);

    h.env.as_contract(&h.contract_id, || {
        let instance = h.env.storage().instance();
        let persistent = h.env.storage().persistent();
        for key in [
            DataKey::Admin,
            DataKey::AdminRotation,
            DataKey::NextSessionSequence,
            DataKey::StorageLayoutVersion,
            DataKey::SupportedAsset(h.asset.clone()),
            DataKey::AssetMaxSessionAmount(h.asset.clone()),
            DataKey::AssetMinSessionAmount(h.asset.clone()),
            DataKey::SupportedAssets,
            DataKey::OperationalState,
        ] {
            assert!(instance.has(&key), "missing instance key {key:?}");
            assert!(!persistent.has(&key), "{key:?} must not be persistent");
        }
    });
    assert_eq!(h.instance_ttl(), TTL_EXTENSION_AMOUNT);
}

#[test]
fn sr1_admin_call_extends_instance_only_at_threshold() {
    let h = Lifecycle::new();
    assert_eq!(h.instance_ttl(), TTL_EXTENSION_AMOUNT);

    h.ledger(AT_THRESHOLD - 1);
    assert_eq!(h.instance_ttl(), DEFAULT_TTL_THRESHOLD + 1);
    h.admin_touch();
    assert_eq!(h.instance_ttl(), DEFAULT_TTL_THRESHOLD + 1);

    h.ledger(AT_THRESHOLD);
    assert_eq!(h.instance_ttl(), DEFAULT_TTL_THRESHOLD);
    h.admin_touch();
    assert_eq!(h.instance_ttl(), TTL_EXTENSION_AMOUNT);
}

#[test]
fn sr2_session_paths_never_extend_instance_ttl() {
    let h = Lifecycle::new();
    let open_for_expiry = h.create(EXPIRY);
    h.ledger(AT_THRESHOLD);
    assert_eq!(h.instance_ttl(), DEFAULT_TTL_THRESHOLD);

    let expiry = AT_THRESHOLD + EXPIRY;
    let settled = h.create(expiry);
    let cancelled = h.create(expiry);
    let extended = h.create(expiry);
    h.client().settle(&settled, &(CAP / 2), &h.usage_hash());
    h.client().cancel(&cancelled);
    h.client().extend_ttl(&extended);
    h.client().recover_expired(&open_for_expiry);
    h.client().get_session(&extended);

    assert_eq!(h.instance_ttl(), DEFAULT_TTL_THRESHOLD);
    h.env.as_contract(&h.contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&h.env), 4);
    });
}

#[test]
fn sr3_create_extends_session_and_counters_to_full_ttl() {
    let h = Lifecycle::new();
    let id = h.create(EXPIRY);
    assert_eq!(h.session_and_counter_ttls(&id), [TTL_EXTENSION_AMOUNT; 5]);

    // A later session that touches the same counters extends them again once they reach the
    // threshold, so shared counters live as long as their most recently written session.
    h.ledger(AT_THRESHOLD);
    let later = h.create(AT_THRESHOLD + EXPIRY);
    assert_eq!(
        h.session_and_counter_ttls(&later),
        [TTL_EXTENSION_AMOUNT; 5]
    );
    assert_eq!(h.persistent_ttl(&h.session_key(&id)), DEFAULT_TTL_THRESHOLD);
}

#[test]
fn sr4_open_session_extend_ttl_boundary_is_exact() {
    let h = Lifecycle::new();
    let id = h.create(EXPIRY);

    h.ledger(AT_THRESHOLD - 1);
    h.client().extend_ttl(&id);
    assert_eq!(
        h.session_and_counter_ttls(&id),
        [DEFAULT_TTL_THRESHOLD + 1; 5]
    );

    h.ledger(AT_THRESHOLD);
    let events = h.events();
    h.client().extend_ttl(&id);
    assert_eq!(h.session_and_counter_ttls(&id), [TTL_EXTENSION_AMOUNT; 5]);
    assert_eq!(h.events(), events, "TTL extension emits no event");
    assert_eq!(h.client().get_session(&id).status, SessionStatus::Open);
}

#[test]
fn sr4_extend_ttl_after_logical_expiry_keeps_escrow_recoverable() {
    let h = Lifecycle::new();
    let id = h.create(EXPIRY);
    let (buyer_before, _, _) = h.balances();

    // Past logical expiry the session can no longer settle or cancel, but `extend_ttl` still
    // keeps it readable so the escrow can be recovered later.
    h.ledger(AT_THRESHOLD);
    assert_eq!(
        h.client().try_settle(&id, &1, &h.usage_hash()),
        Err(Ok(ContractError::ExpiredSession))
    );
    h.client().extend_ttl(&id);
    assert_eq!(h.persistent_ttl(&h.session_key(&id)), TTL_EXTENSION_AMOUNT);

    h.ledger(AT_THRESHOLD + 10);
    h.client().recover_expired(&id);
    assert_eq!(h.balances().0, buyer_before + CAP);
    assert_eq!(h.client().get_session(&id).status, SessionStatus::Expired);
}

#[test]
fn sr5_max_duration_session_expires_before_ttl_threshold() {
    let h = Lifecycle::new();
    let id = h.create(MAX_SESSION_DURATION_LEDGERS);

    h.ledger(MAX_SESSION_DURATION_LEDGERS - 1);
    assert!(h.persistent_ttl(&h.session_key(&id)) > DEFAULT_TTL_THRESHOLD);
    h.client().settle(&id, &CAP, &h.usage_hash());
    assert_eq!(h.client().get_session(&id).status, SessionStatus::Settled);

    h.ledger(0);
    let recover = h.create(MAX_SESSION_DURATION_LEDGERS);
    h.ledger(MAX_SESSION_DURATION_LEDGERS);
    assert!(h.persistent_ttl(&h.session_key(&recover)) > DEFAULT_TTL_THRESHOLD);
    h.client().recover_expired(&recover);
    assert_eq!(
        h.client().get_session(&recover).status,
        SessionStatus::Expired
    );
}

#[test]
fn sr6_finalizing_write_is_the_last_extension() {
    let h = Lifecycle::new();
    let id = h.create(EXPIRY);

    // Recovery at the threshold ledger is a finalizing write, which extends once.
    h.ledger(AT_THRESHOLD);
    h.client().recover_expired(&id);
    assert_eq!(h.persistent_ttl(&h.session_key(&id)), TTL_EXTENSION_AMOUNT);

    // After that, nothing extends it: TTL only counts down until the entry lapses. The admin
    // call keeps the contract instance itself alive for the rest of the test.
    h.admin_touch();
    h.ledger(AT_THRESHOLD * 2);
    let ttl = h.persistent_ttl(&h.session_key(&id));
    assert_eq!(ttl, DEFAULT_TTL_THRESHOLD);
    assert_eq!(
        h.client().try_extend_ttl(&id),
        Err(Ok(ContractError::TtlExtensionFailed))
    );
    assert_eq!(
        h.client().try_recover_expired(&id),
        Err(Ok(ContractError::SessionExpired))
    );
    assert_eq!(h.persistent_ttl(&h.session_key(&id)), ttl);
}

#[test]
fn sr6_finalized_sessions_reject_extension_for_every_terminal_status() {
    let h = Lifecycle::new();
    let settled = h.create(EXPIRY);
    let cancelled = h.create(EXPIRY);
    let recovered = h.create(EXPIRY);
    h.client().settle(&settled, &CAP, &h.usage_hash());
    h.client().cancel(&cancelled);
    h.ledger(EXPIRY);
    h.client().recover_expired(&recovered);

    h.ledger(AT_THRESHOLD);
    for id in [&settled, &cancelled, &recovered] {
        let ttl = h.persistent_ttl(&h.session_key(id));
        let session = h.client().get_session(id);
        let balances = h.balances();
        let events = h.events();

        assert_eq!(
            h.client().try_extend_ttl(id),
            Err(Ok(ContractError::TtlExtensionFailed))
        );
        assert_eq!(h.persistent_ttl(&h.session_key(id)), ttl);
        assert_eq!(h.client().get_session(id), session);
        assert_eq!(h.balances(), balances);
        assert_eq!(h.events(), events);
    }
}

#[test]
fn sr7_missing_session_fails_closed_everywhere() {
    let h = Lifecycle::new();
    let lapsed = h.create(EXPIRY);
    h.remove_persistent(&h.session_key(&lapsed));
    let never_created = BytesN::from_array(&h.env, &[9; 32]);
    let balances = h.balances();
    let events = h.events();
    let client = h.client();

    for id in [&lapsed, &never_created] {
        assert_eq!(
            client.try_get_session(id),
            Err(Ok(ContractError::SessionNotFound))
        );
        assert_eq!(
            client.try_get_session_status(id),
            Err(Ok(ContractError::SessionNotFound))
        );
        assert_eq!(
            client.try_settle(id, &1, &h.usage_hash()),
            Err(Ok(ContractError::SessionNotFound))
        );
        assert_eq!(
            client.try_cancel(id),
            Err(Ok(ContractError::SessionNotFound))
        );
        assert_eq!(
            client.try_extend_ttl(id),
            Err(Ok(ContractError::SessionNotFound))
        );
    }
    h.ledger(EXPIRY);
    for id in [&lapsed, &never_created] {
        assert_eq!(
            client.try_recover_expired(id),
            Err(Ok(ContractError::SessionNotFound))
        );
    }
    let batch = client.recover_expired_batch(&soroban_sdk::vec![
        &h.env,
        lapsed.clone(),
        never_created.clone()
    ]);
    for result in batch.iter() {
        assert_eq!(result.status, BatchRecoveryStatus::NotFound);
        assert_eq!(result.refunded_amount, 0);
    }

    // The escrow of the lapsed session stays in the contract; nothing is paid out on a guess.
    assert_eq!(h.balances(), balances);
    assert_eq!(h.events(), events);
    assert_eq!(client.get_escrow_liability(&h.asset), CAP);
}

#[test]
fn sr8_missing_aggregate_counter_fails_closed_without_payout() {
    for action in ["settle", "cancel", "recover_expired"] {
        for counter in 0..4 {
            let h = Lifecycle::new();
            let id = h.create(EXPIRY);
            let key = h.counter_keys()[counter].clone();
            h.remove_persistent(&key);
            if action == "recover_expired" {
                h.ledger(EXPIRY);
            }
            let balances = h.balances();
            let events = h.events();
            let client = h.client();

            let result = match action {
                "settle" => client.try_settle(&id, &CAP, &h.usage_hash()),
                "cancel" => client.try_cancel(&id),
                _ => client.try_recover_expired(&id),
            };
            assert_eq!(
                result,
                Err(Ok(ContractError::LiabilityUnderflow)),
                "{action} with {key:?} missing"
            );
            assert_eq!(h.balances(), balances, "{action} moved funds");
            assert_eq!(h.events(), events, "{action} emitted an event");
            assert_eq!(client.get_session(&id).status, SessionStatus::Open);
            for other in h.counter_keys() {
                if other != key {
                    h.env.as_contract(&h.contract_id, || {
                        assert!(h.env.storage().persistent().has(&other));
                    });
                }
            }
        }
    }
}

#[test]
fn sr9_missing_instance_config_blocks_creation_and_admin_but_not_exits() {
    let h = Lifecycle::new();
    let cancelled = h.create(EXPIRY);
    let settled = h.create(EXPIRY);
    h.env.as_contract(&h.contract_id, || {
        h.env.storage().instance().remove(&DataKey::Admin)
    });
    let client = h.client();

    assert_eq!(
        client.try_create_session(
            &h.buyer,
            &h.seller,
            &h.asset,
            &CAP,
            &EXPIRY,
            &BytesN::from_array(&h.env, &[7; 32]),
            &h.next_key(),
        ),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_get_configuration(),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_pause_creation(),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_set_asset_max_session_amount(&h.asset, &i128::MAX),
        Err(Ok(ContractError::NotInitialized))
    );

    // Existing escrow can still leave through the session's own record.
    client.cancel(&cancelled);
    client.settle(&settled, &CAP, &h.usage_hash());
    h.env.as_contract(&h.contract_id, || {
        assert_eq!(storage::read_liability(&h.env, &h.asset), 0);
    });
    assert_eq!(
        client.try_get_escrow_liability(&h.asset),
        Err(Ok(ContractError::NotInitialized))
    );
}

#[test]
fn sr10_reads_never_extend_ttl() {
    let h = Lifecycle::new();
    let id = h.create(EXPIRY);
    h.ledger(AT_THRESHOLD);
    let ttls = h.session_and_counter_ttls(&id);
    let instance = h.instance_ttl();
    assert_eq!(ttls, [DEFAULT_TTL_THRESHOLD; 5]);
    assert_eq!(instance, DEFAULT_TTL_THRESHOLD);

    let client = h.client();
    client.get_session(&id);
    client.get_session_status(&id);
    client.get_escrow_liability(&h.asset);
    client.get_configuration();
    client.get_buyer_open_sessions(&h.buyer);
    client.get_seller_exposure(&h.seller, &h.asset);

    assert_eq!(h.session_and_counter_ttls(&id), ttls);
    assert_eq!(h.instance_ttl(), instance);
}
