// Deterministic ledger-boundary tests for contract storage TTL.
//
// Soroban semantics relied on here (documented in docs/threat-model.md):
//
// * An entry's remaining TTL is `live_until_ledger - current_ledger`. `extend_ttl(threshold,
//   extend_to)` raises `live_until_ledger` to `current_ledger + extend_to` only when the remaining
//   TTL is at or below `threshold`.
// * The unit-test host does not archive expired persistent entries, so a lapsed entry cannot be
//   produced by advancing the ledger. Lapse is modelled by removing the entry, which is the
//   observable condition the contract must fail closed on. These tests make no claim about
//   restoring archived entries; that is a network-level operation this contract neither
//   implements nor relies on.

use super::*;
use crate::storage::{DataKey, DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT};
use crate::validation::{MAX_ADMIN_ROTATION_DURATION_LEDGERS, MAX_SESSION_DURATION_LEDGERS};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};

/// Ledger at which an entry written at ledger 0 has exactly `DEFAULT_TTL_THRESHOLD` ledgers left.
const AT_THRESHOLD: u32 = TTL_EXTENSION_AMOUNT - DEFAULT_TTL_THRESHOLD;
const SESSION_EXPIRY: u32 = 500;
const CAP: i128 = 1_000;

// A session cannot logically expire after its TTL passes the extension threshold, so every
// finalization that must happen before expiry runs while the session TTL is above the threshold.
const _: () = assert!(MAX_SESSION_DURATION_LEDGERS < AT_THRESHOLD);

struct Harness {
    env: Env,
    contract_id: Address,
    buyer: Address,
    seller: Address,
    asset: Address,
    next_idempotency: core::cell::Cell<u8>,
}

impl Harness {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let contract_id = env.register(UptoSessionContract, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = create_test_asset(&env, &buyer, CAP * 10);
        let client = UptoSessionContractClient::new(&env, &contract_id);
        initialize_for_asset(&env, &client, &buyer, &asset);
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

    fn instance_ttl(&self) -> u32 {
        self.env
            .as_contract(&self.contract_id, || self.env.storage().instance().get_ttl())
    }

    fn session_ttl(&self, id: &BytesN<32>) -> u32 {
        self.env.as_contract(&self.contract_id, || {
            self.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::Session(id.clone()))
        })
    }

    fn liability_ttl(&self) -> u32 {
        self.env.as_contract(&self.contract_id, || {
            self.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::Liability(self.asset.clone()))
        })
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
fn ttl_constants_are_the_documented_boundary_values() {
    let harness = Harness::new();
    let configuration = harness.client().get_configuration();

    assert_eq!(DEFAULT_TTL_THRESHOLD, 100_000);
    assert_eq!(TTL_EXTENSION_AMOUNT, 1_000_000);
    assert_eq!(configuration.instance_ttl_threshold, DEFAULT_TTL_THRESHOLD);
    assert_eq!(configuration.instance_ttl_extension, TTL_EXTENSION_AMOUNT);
    assert_eq!(AT_THRESHOLD, 900_000);
}

#[test]
fn every_instance_key_exists_and_shares_the_single_instance_ttl() {
    let harness = Harness::new();
    let client = harness.client();
    let new_admin = Address::generate(&harness.env);
    client.propose_admin(&new_admin, &1_000);
    harness.create(SESSION_EXPIRY);

    // Config keys (Admin, StorageLayoutVersion, SupportedAsset, AssetMaxSessionAmount,
    // SupportedAssets, OperationalState, AdminRotation) and the sequence key
    // (NextSessionSequence) all live in instance storage.
    harness.env.as_contract(&harness.contract_id, || {
        let instance = harness.env.storage().instance();
        for key in [
            DataKey::Admin,
            DataKey::AdminRotation,
            DataKey::NextSessionSequence,
            DataKey::StorageLayoutVersion,
            DataKey::SupportedAsset(harness.asset.clone()),
            DataKey::AssetMaxSessionAmount(harness.asset.clone()),
            DataKey::SupportedAssets,
            DataKey::OperationalState,
        ] {
            assert!(instance.has(&key), "missing instance key {key:?}");
            assert!(
                !harness.env.storage().persistent().has(&key),
                "instance key {key:?} must not also be persistent"
            );
        }
    });
    // Session and Liability keys are persistent, not instance.
    harness.env.as_contract(&harness.contract_id, || {
        let instance = harness.env.storage().instance();
        assert!(!instance.has(&DataKey::Liability(harness.asset.clone())));
    });
    assert_eq!(harness.instance_ttl(), TTL_EXTENSION_AMOUNT);
}

#[test]
fn instance_ttl_extension_boundary_is_exact_for_config_changes() {
    let harness = Harness::new();
    let client = harness.client();
    assert_eq!(harness.instance_ttl(), TTL_EXTENSION_AMOUNT);

    // One ledger above the threshold: the admin call must not extend.
    harness.ledger(AT_THRESHOLD - 1);
    assert_eq!(harness.instance_ttl(), DEFAULT_TTL_THRESHOLD + 1);
    client.set_asset_max_session_amount(&harness.asset, &(CAP * 2));
    assert_eq!(harness.instance_ttl(), DEFAULT_TTL_THRESHOLD + 1);

    // Exactly at the threshold: the same call extends to the full amount.
    harness.ledger(AT_THRESHOLD);
    assert_eq!(harness.instance_ttl(), DEFAULT_TTL_THRESHOLD);
    client.set_asset_max_session_amount(&harness.asset, &CAP);
    assert_eq!(harness.instance_ttl(), TTL_EXTENSION_AMOUNT);
    assert_eq!(
        client.get_asset_max_session_amount(&harness.asset),
        CAP
    );
}

#[test]
fn session_creation_advances_sequence_but_does_not_extend_instance_ttl() {
    let harness = Harness::new();
    harness.ledger(AT_THRESHOLD);
    assert_eq!(harness.instance_ttl(), DEFAULT_TTL_THRESHOLD);

    let first = harness.create(AT_THRESHOLD + SESSION_EXPIRY);
    let second = harness.create(AT_THRESHOLD + SESSION_EXPIRY);

    harness.env.as_contract(&harness.contract_id, || {
        assert_eq!(storage::read_next_session_sequence(&harness.env), 2);
    });
    assert_ne!(first, second);
    // Documented limitation: only admin entry points extend instance storage, which holds the
    // config and the session sequence. Operators must keep it alive with an admin call.
    assert_eq!(harness.instance_ttl(), DEFAULT_TTL_THRESHOLD);
    // New persistent entries get a full TTL regardless of the instance TTL.
    assert_eq!(harness.session_ttl(&first), TTL_EXTENSION_AMOUNT);
    assert_eq!(harness.liability_ttl(), TTL_EXTENSION_AMOUNT);
}

#[test]
fn session_and_liability_ttl_extension_boundary_is_exact() {
    let harness = Harness::new();
    let client = harness.client();
    let id = harness.create(SESSION_EXPIRY);
    assert_eq!(harness.session_ttl(&id), TTL_EXTENSION_AMOUNT);
    assert_eq!(harness.liability_ttl(), TTL_EXTENSION_AMOUNT);

    harness.ledger(AT_THRESHOLD - 1);
    assert_eq!(harness.session_ttl(&id), DEFAULT_TTL_THRESHOLD + 1);
    client.extend_ttl(&id);
    assert_eq!(harness.session_ttl(&id), DEFAULT_TTL_THRESHOLD + 1);
    assert_eq!(harness.liability_ttl(), DEFAULT_TTL_THRESHOLD + 1);
    assert_eq!(harness.events(), 0);

    harness.ledger(AT_THRESHOLD);
    assert_eq!(harness.session_ttl(&id), DEFAULT_TTL_THRESHOLD);
    client.extend_ttl(&id);
    assert_eq!(harness.session_ttl(&id), TTL_EXTENSION_AMOUNT);
    assert_eq!(harness.liability_ttl(), TTL_EXTENSION_AMOUNT);
    // Extension never changes session state, balances, or emits an event.
    assert_eq!(client.get_session(&id).status, SessionStatus::Open);
    assert_eq!(harness.balances(), (CAP * 9, 0, CAP));
    assert_eq!(harness.events(), 0);
}

#[test]
fn logical_expiry_boundary_is_independent_of_storage_ttl() {
    let harness = Harness::new();
    let client = harness.client();
    let id = harness.create(SESSION_EXPIRY);
    let usage_hash = harness.usage_hash();

    // Pre-expiry (ledger 499): every action on an open session is permitted.
    harness.ledger(SESSION_EXPIRY - 1);
    assert!(client.try_extend_ttl(&id).is_ok());
    assert_eq!(
        client.try_recover_expired(&id),
        Err(Ok(ContractError::SessionNotExpired))
    );

    // At expiry (ledger 500): settle and cancel fail closed; recovery is the only exit. Extending
    // storage TTL is not gated by logical expiry so recovery data stays available.
    harness.ledger(SESSION_EXPIRY);
    assert_eq!(
        client.try_settle(&id, &1, &usage_hash),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert_eq!(
        client.try_cancel(&id),
        Err(Ok(ContractError::ExpiredSession))
    );
    assert!(client.try_extend_ttl(&id).is_ok());
    assert_eq!(client.get_session(&id).status, SessionStatus::Open);
    assert_eq!(harness.balances(), (CAP * 9, 0, CAP));

    // Post-expiry (ledger 501): still recoverable exactly once, then extension is rejected.
    harness.ledger(SESSION_EXPIRY + 1);
    client.recover_expired(&id);
    assert_eq!(client.get_session(&id).status, SessionStatus::Expired);
    assert_eq!(harness.balances(), (CAP * 10, 0, 0));
    assert_eq!(
        client.try_extend_ttl(&id),
        Err(Ok(ContractError::TtlExtensionFailed))
    );
    assert_eq!(
        client.try_recover_expired(&id),
        Err(Ok(ContractError::SessionExpired))
    );
}

#[test]
fn ttl_extension_cannot_revive_finalized_sessions() {
    let harness = Harness::new();
    let client = harness.client();
    let settled = harness.create(SESSION_EXPIRY);
    let cancelled = harness.create(SESSION_EXPIRY);
    let expired = harness.create(SESSION_EXPIRY);
    client.settle(&settled, &400, &harness.usage_hash());
    client.cancel(&cancelled);
    harness.ledger(SESSION_EXPIRY);
    client.recover_expired(&expired);

    // At the exact ledger where extension would apply to a live entry.
    harness.ledger(AT_THRESHOLD);
    let balances_before = harness.balances();
    let liability_before = client.get_escrow_liability(&harness.asset);
    for (id, status) in [
        (&settled, SessionStatus::Settled),
        (&cancelled, SessionStatus::Cancelled),
        (&expired, SessionStatus::Expired),
    ] {
        let ttl_before = harness.session_ttl(id);
        assert_eq!(ttl_before, DEFAULT_TTL_THRESHOLD);
        assert_eq!(
            client.try_extend_ttl(id),
            Err(Ok(ContractError::TtlExtensionFailed))
        );
        assert_eq!(harness.events(), 0);
        assert_eq!(harness.session_ttl(id), ttl_before);
        assert_eq!(client.get_session(id).status, status);
    }
    assert_eq!(harness.liability_ttl(), DEFAULT_TTL_THRESHOLD);
    assert_eq!(harness.balances(), balances_before);
    assert_eq!(client.get_escrow_liability(&harness.asset), liability_before);
    assert_eq!(client.get_session(&settled).settled_amount, 400);
    assert_eq!(client.get_session(&settled).escrowed_amount, 0);
}

#[test]
fn recovery_extends_ttl_only_inside_the_finalizing_transition_at_the_threshold() {
    let harness = Harness::new();
    let client = harness.client();
    let before_boundary = harness.create(SESSION_EXPIRY);
    let at_boundary = harness.create(SESSION_EXPIRY);

    harness.ledger(AT_THRESHOLD - 1);
    client.recover_expired(&before_boundary);
    assert_eq!(harness.session_ttl(&before_boundary), DEFAULT_TTL_THRESHOLD + 1);

    harness.ledger(AT_THRESHOLD);
    client.recover_expired(&at_boundary);
    assert_eq!(harness.session_ttl(&at_boundary), TTL_EXTENSION_AMOUNT);
    assert_eq!(
        client.get_session(&at_boundary).status,
        SessionStatus::Expired
    );

    // The finalized state is retained, not reopened: later extension and recovery both fail.
    assert_eq!(
        client.try_extend_ttl(&at_boundary),
        Err(Ok(ContractError::TtlExtensionFailed))
    );
    assert_eq!(
        client.try_recover_expired(&at_boundary),
        Err(Ok(ContractError::SessionExpired))
    );
    assert_eq!(harness.balances(), (CAP * 10, 0, 0));
}

#[test]
fn lapsed_session_entry_fails_closed_without_moving_funds() {
    let harness = Harness::new();
    let client = harness.client();
    let open = harness.create(SESSION_EXPIRY);
    let balances_before = harness.balances();
    let liability_before = client.get_escrow_liability(&harness.asset);

    harness.env.as_contract(&harness.contract_id, || {
        harness
            .env
            .storage()
            .persistent()
            .remove(&DataKey::Session(open.clone()));
    });
    harness.ledger(SESSION_EXPIRY + 1);

    let usage_hash = harness.usage_hash();
    assert_eq!(
        client.try_get_session(&open),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(
        client.try_settle(&open, &1, &usage_hash),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(client.try_cancel(&open), Err(Ok(ContractError::SessionNotFound)));
    assert_eq!(
        client.try_recover_expired(&open),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(
        client.try_extend_ttl(&open),
        Err(Ok(ContractError::SessionNotFound))
    );
    let batch = client.recover_expired_batch(&soroban_sdk::vec![&harness.env, open.clone()]);
    assert_eq!(
        batch.get(0).expect("one result").status,
        BatchRecoveryStatus::NotFound
    );
    assert_eq!(harness.events(), 0);
    assert_eq!(harness.balances(), balances_before);
    assert_eq!(client.get_escrow_liability(&harness.asset), liability_before);
}

#[test]
fn lapsed_session_entry_cannot_be_replaced_by_extension_of_a_finalized_session() {
    let harness = Harness::new();
    let client = harness.client();
    let settled = harness.create(SESSION_EXPIRY);
    client.settle(&settled, &CAP, &harness.usage_hash());
    harness.env.as_contract(&harness.contract_id, || {
        harness
            .env
            .storage()
            .persistent()
            .remove(&DataKey::Session(settled.clone()));
    });
    harness.ledger(AT_THRESHOLD);

    assert_eq!(
        client.try_extend_ttl(&settled),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(
        client.try_settle(&settled, &CAP, &harness.usage_hash()),
        Err(Ok(ContractError::SessionNotFound))
    );
    assert_eq!(harness.balances(), (CAP * 9, CAP, 0));
}

#[test]
fn lapsed_liability_entry_fails_closed_and_keeps_escrow_in_the_contract() {
    let harness = Harness::new();
    let client = harness.client();
    let id = harness.create(SESSION_EXPIRY);
    let balances_before = harness.balances();
    harness.env.as_contract(&harness.contract_id, || {
        harness
            .env
            .storage()
            .persistent()
            .remove(&DataKey::Liability(harness.asset.clone()));
    });

    assert_eq!(
        client.try_settle(&id, &1, &harness.usage_hash()),
        Err(Ok(ContractError::LiabilityUnderflow))
    );
    assert_eq!(
        client.try_cancel(&id),
        Err(Ok(ContractError::LiabilityUnderflow))
    );
    harness.ledger(SESSION_EXPIRY);
    assert_eq!(
        client.try_recover_expired(&id),
        Err(Ok(ContractError::LiabilityUnderflow))
    );
    let batch = client.recover_expired_batch(&soroban_sdk::vec![&harness.env, id.clone()]);
    assert_eq!(
        batch.get(0).expect("one result").status,
        BatchRecoveryStatus::EscrowUnavailable
    );
    assert_eq!(client.get_session(&id).status, SessionStatus::Open);
    assert_eq!(client.get_session(&id).escrowed_amount, CAP);
    assert_eq!(harness.events(), 0);
    assert_eq!(harness.balances(), balances_before);
}

#[test]
fn lapsed_instance_config_fails_closed_for_admin_and_new_sessions_only() {
    let harness = Harness::new();
    let client = harness.client();
    let open = harness.create(SESSION_EXPIRY);
    let balances_before = harness.balances();

    harness.env.as_contract(&harness.contract_id, || {
        let instance = harness.env.storage().instance();
        for key in [
            DataKey::Admin,
            DataKey::AdminRotation,
            DataKey::NextSessionSequence,
            DataKey::StorageLayoutVersion,
            DataKey::SupportedAsset(harness.asset.clone()),
            DataKey::AssetMaxSessionAmount(harness.asset.clone()),
            DataKey::SupportedAssets,
            DataKey::OperationalState,
        ] {
            instance.remove(&key);
        }
    });

    assert_eq!(client.try_get_admin(), Err(Ok(ContractError::NotInitialized)));
    assert_eq!(
        client.try_get_configuration(),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_get_escrow_liability(&harness.asset),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_set_asset_max_session_amount(&harness.asset, &CAP),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(
        client.try_create_session(
            &harness.buyer,
            &harness.seller,
            &harness.asset,
            &CAP,
            &SESSION_EXPIRY,
            &BytesN::from_array(&harness.env, &[9; 32]),
            &harness.next_key(),
        ),
        Err(Ok(ContractError::NotInitialized))
    );
    assert_eq!(harness.balances(), balances_before);

    // Existing escrow is never trapped by config loss: the stored buyer can still cancel.
    client.cancel(&open);
    assert_eq!(harness.balances(), (CAP * 10, 0, 0));
}

#[test]
fn admin_rotation_expiry_boundary_is_exact_and_expired_state_fails_closed() {
    let harness = Harness::new();
    let client = harness.client();
    let pending = Address::generate(&harness.env);
    let expires_at = 100;
    client.propose_admin(&pending, &expires_at);

    harness.ledger(expires_at - 1);
    assert_eq!(client.get_admin_rotation().map(|r| r.expires_at_ledger), Some(expires_at));

    harness.ledger(expires_at);
    assert_eq!(
        client.try_accept_admin(),
        Err(Ok(ContractError::AdminRotationExpired))
    );
    // Expired proposals are retained until cancelled; they never promote the pending admin.
    assert_eq!(client.get_admin(), harness.buyer);
    assert!(client.get_admin_rotation().is_some());
    assert_eq!(
        client.try_propose_admin(&pending, &(expires_at + 10)),
        Err(Ok(ContractError::AdminRotationPending))
    );
    client.cancel_admin_rotation();
    assert!(client.get_admin_rotation().is_none());
}

#[test]
fn admin_rotation_accepts_on_the_last_valid_ledger() {
    let harness = Harness::new();
    let client = harness.client();
    let pending = Address::generate(&harness.env);
    client.propose_admin(&pending, &100);

    harness.ledger(99);
    client.accept_admin();
    assert_eq!(client.get_admin(), pending);
    assert!(client.get_admin_rotation().is_none());
}

#[test]
fn creation_and_rotation_expiry_windows_have_exact_bounds() {
    let harness = Harness::new();
    let client = harness.client();
    harness.ledger(1_000);
    let hash = BytesN::from_array(&harness.env, &[9; 32]);
    let create = |expires_at_ledger: u32| {
        client.try_create_session(
            &harness.buyer,
            &harness.seller,
            &harness.asset,
            &1,
            &expires_at_ledger,
            &hash,
            &harness.next_key(),
        )
    };

    assert_eq!(create(1_000), Err(Ok(ContractError::ExpiredSession)));
    assert!(create(1_001).is_ok());
    assert!(create(1_000 + MAX_SESSION_DURATION_LEDGERS).is_ok());
    assert_eq!(
        create(1_000 + MAX_SESSION_DURATION_LEDGERS + 1),
        Err(Ok(ContractError::SessionDurationTooLong))
    );

    let pending = Address::generate(&harness.env);
    assert_eq!(
        client.try_propose_admin(&pending, &1_000),
        Err(Ok(ContractError::InvalidAdminRotationExpiry))
    );
    assert_eq!(
        client.try_propose_admin(&pending, &(1_000 + MAX_ADMIN_ROTATION_DURATION_LEDGERS + 1)),
        Err(Ok(ContractError::InvalidAdminRotationExpiry))
    );
    assert!(client
        .try_propose_admin(&pending, &(1_000 + MAX_ADMIN_ROTATION_DURATION_LEDGERS))
        .is_ok());
}
