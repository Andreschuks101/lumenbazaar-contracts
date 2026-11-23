use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env, Vec,
};
use upto_session::{
    ids, storage, AssetPolicy, CommitmentKind, ContractError, Session, SessionStatus,
    UptoSessionContract,
};

fn session(env: &Env, id_byte: u8, seller: &Address, status: SessionStatus) -> Session {
    Session {
        id: BytesN::from_array(env, &[id_byte; 32]),
        buyer: Address::generate(env),
        seller: seller.clone(),
        asset: Address::generate(env),
        max_amount: 100,
        escrowed_amount: if status == SessionStatus::Open {
            100
        } else {
            0
        },
        settled_amount: if status == SessionStatus::Settled {
            100
        } else {
            0
        },
        expires_at_ledger: 100,
        resource_hash: BytesN::from_array(env, &[id_byte.wrapping_add(1); 32]),
        usage_hash: None,
        status,
        creation_ledger: 1,
        finalized_ledger: None,
        resource_scheme_version: 1,
        usage_scheme_version: None,
    }
}

#[test]
fn counters_follow_model_transitions_without_double_counting() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let seller = Address::generate(&env);

    env.as_contract(&contract, || {
        let mut first = session(&env, 1, &seller, SessionStatus::Open);
        let second = session(&env, 2, &seller, SessionStatus::Open);
        storage::write_session(&env, &first).unwrap();
        storage::write_session(&env, &second).unwrap();
        storage::write_session(&env, &first).unwrap();

        let open = storage::read_session_state_counts(&env);
        assert_eq!(open.open, 2);
        assert_eq!(open.settled, 0);
        assert_eq!(
            storage::read_seller_session_state_counts(&env, &seller),
            open
        );

        first.status = SessionStatus::Settled;
        first.escrowed_amount = 0;
        first.settled_amount = 100;
        storage::write_session(&env, &first).unwrap();
        storage::write_session(&env, &first).unwrap();

        let counts = storage::read_session_state_counts(&env);
        assert_eq!(counts.open, 1);
        assert_eq!(counts.settled, 1);
        assert_eq!(counts.cancelled, 0);
        assert_eq!(
            storage::read_seller_session_state_counts(&env, &seller),
            counts
        );
    });
}

proptest! {
    #[test]
    fn aggregate_counters_match_a_transition_model(transitions in prop::collection::vec(0_u8..5, 1..64)) {
        let env = Env::default();
        let contract = env.register(UptoSessionContract, ());
        let seller = Address::generate(&env);
        env.as_contract(&contract, || {
            let mut stored = session(&env, 9, &seller, SessionStatus::Open);
            storage::write_session(&env, &stored).unwrap();
            let mut expected = [1_u64, 0, 0, 0, 0];

            for target in transitions {
                let next = match target {
                    0 => SessionStatus::Open,
                    1 => SessionStatus::Settled,
                    2 => SessionStatus::Cancelled,
                    3 => SessionStatus::Expired,
                    _ => SessionStatus::Rejected,
                };
                let previous_index = match stored.status {
                    SessionStatus::Open => 0,
                    SessionStatus::Settled => 1,
                    SessionStatus::Cancelled => 2,
                    SessionStatus::Expired => 3,
                    SessionStatus::Rejected => 4,
                };
                let next_index = usize::from(target);
                if previous_index != next_index {
                    expected[previous_index] -= 1;
                    expected[next_index] += 1;
                }
                stored.status = next;
                storage::write_session(&env, &stored).unwrap();
                let actual = storage::read_session_state_counts(&env);
                prop_assert_eq!(
                    [actual.open, actual.settled, actual.cancelled, actual.expired, actual.rejected],
                    expected
                );
            }
            Ok(())
        })?;
    }
}

#[test]
fn commitment_index_is_kind_separated_deterministic_and_bounded() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let commitment = BytesN::from_array(&env, &[42; 32]);

    env.as_contract(&contract, || {
        for byte in 0..storage::MAX_COMMITMENT_INDEX_SESSIONS {
            let id = BytesN::from_array(&env, &[byte as u8; 32]);
            storage::index_session_commitment(&env, &CommitmentKind::Resource, &commitment, &id)
                .unwrap();
        }
        let first = BytesN::from_array(&env, &[0; 32]);
        storage::index_session_commitment(&env, &CommitmentKind::Resource, &commitment, &first)
            .unwrap();
        let resource =
            storage::read_sessions_by_commitment(&env, &CommitmentKind::Resource, &commitment);
        assert_eq!(resource.len(), storage::MAX_COMMITMENT_INDEX_SESSIONS);
        assert_eq!(resource.get(0), Some(first));
        assert!(
            storage::read_sessions_by_commitment(&env, &CommitmentKind::Usage, &commitment,)
                .is_empty()
        );

        let overflow = BytesN::from_array(&env, &[99; 32]);
        assert_eq!(
            storage::index_session_commitment(
                &env,
                &CommitmentKind::Resource,
                &commitment,
                &overflow,
            ),
            Err(ContractError::CommitmentIndexFull)
        );
    });
}

#[test]
fn initialization_digest_is_deterministic_and_domain_separated() {
    let env = Env::default();
    env.ledger().set_network_id([7; 32]);
    let first_contract = env.register(UptoSessionContract, ());
    let second_contract = env.register(UptoSessionContract, ());
    let admin = Address::generate(&env);
    let asset = Address::generate(&env);
    let policies = Vec::from_array(
        &env,
        [AssetPolicy {
            asset,
            max_session_amount: 1_000,
            min_session_amount: 10,
            max_asset_exposure: 10_000,
            max_buyer_exposure: 2_000,
        }],
    );

    let first = env.as_contract(&first_contract, || {
        ids::initialization_configuration_digest(&env, &admin, &policies)
    });
    let same = env.as_contract(&first_contract, || {
        ids::initialization_configuration_digest(&env, &admin, &policies)
    });
    let other_contract = env.as_contract(&second_contract, || {
        ids::initialization_configuration_digest(&env, &admin, &policies)
    });
    assert_eq!(first, same);
    assert_ne!(first, other_contract);
    println!("initialization-digest={:02x?}", first.to_array());
}

#[test]
fn session_indexes_and_counters_ttl_lifecycle_at_threshold_boundaries() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let asset = Address::generate(&env);
    let commitment = BytesN::from_array(&env, &[15; 32]);
    let usage_commitment = BytesN::from_array(&env, &[16; 32]);

    env.as_contract(&contract, || {
        let session_id = BytesN::from_array(&env, &[1; 32]);
        let mut sess = Session {
            id: session_id.clone(),
            buyer: buyer.clone(),
            seller: seller.clone(),
            asset: asset.clone(),
            max_amount: 100,
            escrowed_amount: 100,
            settled_amount: 0,
            expires_at_ledger: 500,
            resource_hash: commitment.clone(),
            usage_hash: None,
            status: SessionStatus::Open,
            creation_ledger: 0,
            finalized_ledger: None,
            resource_scheme_version: 1,
            usage_scheme_version: None,
        };

        // Write open session: creates and extends all session indexes and counters
        storage::write_session(&env, &sess).unwrap();
        storage::index_session_commitment(
            &env,
            &CommitmentKind::Resource,
            &commitment,
            &session_id,
        )
        .unwrap();

        let resource_key = storage::DataKey::CommitmentIndex(ids::commitment_index_key(
            &env,
            &CommitmentKind::Resource,
            &commitment,
        ));
        let seller_state_key =
            storage::DataKey::SellerSessionStateCount(seller.clone(), SessionStatus::Open);
        let buyer_key = storage::DataKey::BuyerOpenSessions(buyer.clone());
        let buyer_total_key = storage::DataKey::BuyerTotalSessions(buyer.clone());
        let seller_key = storage::DataKey::SellerOpenSessions(seller.clone());
        let liability_key = storage::DataKey::Liability(asset.clone());
        let seller_exposure_key =
            storage::DataKey::SellerExposure(seller.clone(), asset.clone());
        let buyer_exposure_key =
            storage::DataKey::BuyerExposure(buyer.clone(), asset.clone());

        // Verify all indexes and aggregate counters start with full extension TTL
        for key in [
            &resource_key,
            &seller_state_key,
            &buyer_key,
            &buyer_total_key,
            &seller_key,
            &liability_key,
            &seller_exposure_key,
            &buyer_exposure_key,
        ] {
            assert_eq!(
                env.storage().persistent().get_ttl(key),
                storage::TTL_EXTENSION_AMOUNT
            );
        }

        // Advance ledger to 1 ledger before threshold: TTL is DEFAULT_TTL_THRESHOLD + 1
        let at_threshold = storage::TTL_EXTENSION_AMOUNT - storage::DEFAULT_TTL_THRESHOLD;
        env.ledger().set_sequence_number(at_threshold - 1);
        storage::extend_session_and_liability_ttl(&env, &sess);

        for key in [
            &resource_key,
            &seller_state_key,
            &buyer_key,
            &buyer_total_key,
            &seller_key,
            &liability_key,
            &seller_exposure_key,
            &buyer_exposure_key,
        ] {
            assert_eq!(
                env.storage().persistent().get_ttl(key),
                storage::DEFAULT_TTL_THRESHOLD + 1
            );
        }

        // Advance ledger to exact threshold: TTL reaches DEFAULT_TTL_THRESHOLD
        env.ledger().set_sequence_number(at_threshold);
        storage::extend_session_and_liability_ttl(&env, &sess);

        // All indexes and aggregate counters are extended back to full TTL_EXTENSION_AMOUNT
        for key in [
            &resource_key,
            &seller_state_key,
            &buyer_key,
            &buyer_total_key,
            &seller_key,
            &liability_key,
            &seller_exposure_key,
            &buyer_exposure_key,
        ] {
            assert_eq!(
                env.storage().persistent().get_ttl(key),
                storage::TTL_EXTENSION_AMOUNT
            );
        }

        // Terminal transition: settle the session
        sess.status = SessionStatus::Settled;
        sess.escrowed_amount = 0;
        sess.settled_amount = 100;
        sess.usage_hash = Some(usage_commitment.clone());
        sess.finalized_ledger = Some(at_threshold);
        storage::write_session(&env, &sess).unwrap();
        storage::index_session_commitment(
            &env,
            &CommitmentKind::Usage,
            &usage_commitment,
            &session_id,
        )
        .unwrap();

        // Terminal transitions preserve lookup availability
        let found_by_resource = storage::read_sessions_by_commitment(
            &env,
            &CommitmentKind::Resource,
            &commitment,
        );
        assert_eq!(found_by_resource.len(), 1);
        assert_eq!(found_by_resource.get(0), Some(session_id.clone()));

        let found_by_usage = storage::read_sessions_by_commitment(
            &env,
            &CommitmentKind::Usage,
            &usage_commitment,
        );
        assert_eq!(found_by_usage.len(), 1);
        assert_eq!(found_by_usage.get(0), Some(session_id));

        // State counts reflect settled status
        let counts = storage::read_session_state_counts(&env);
        assert_eq!(counts.open, 0);
        assert_eq!(counts.settled, 1);
        assert_eq!(
            storage::read_seller_session_state_count(&env, &seller, &SessionStatus::Settled),
            1
        );
    });
}

#[test]
fn session_index_consistency_audit_detects_mismatches_and_valid_state() {
    let env = Env::default();
    let contract = env.register(UptoSessionContract, ());
    let admin = Address::generate(&env);
    let seller = Address::generate(&env);
    let token_contract = env.register(
        test_token::TestTokenContract,
        (),
    );
    let token_client = test_token::TestTokenContractClient::new(&env, &token_contract);
    token_client.mock_all_auths().initialize(&admin);

    let policies = Vec::from_array(
        &env,
        [AssetPolicy {
            asset: token_contract.clone(),
            max_session_amount: 1_000,
            min_session_amount: 10,
            max_asset_exposure: 10_000,
            max_buyer_exposure: 2_000,
        }],
    );

    let client = upto_session::UptoSessionContractClient::new(&env, &contract);
    client.mock_all_auths().initialize(&admin, &policies);

    let open_session = session(&env, 10, &seller, SessionStatus::Open);
    env.as_contract(&contract, || {
        storage::write_session(&env, &open_session).unwrap();
        let _ = storage::increment_buyer_open_sessions(&env, &open_session.buyer);
        let _ = storage::increment_seller_open_sessions(&env, &open_session.seller);
    });

    // Audit before indexing resource commitment detects mismatch
    let audit_res = client.audit_session_index_consistency(&Vec::from_array(&env, [open_session.id.clone()]));
    assert_eq!(audit_res.is_consistent, false);
    assert_eq!(audit_res.mismatches.len(), 1);
    assert_eq!(audit_res.mismatches.get(0).unwrap().category, upto_session::AuditIndexCategory::ResourceCommitment);

    // Index resource commitment
    env.as_contract(&contract, || {
        storage::index_session_commitment(
            &env,
            &CommitmentKind::Resource,
            &open_session.resource_hash,
            &open_session.id,
        ).unwrap();
    });

    // Audit after indexing passes
    let audit_res_ok = client.audit_session_index_consistency(&Vec::from_array(&env, [open_session.id.clone()]));
    assert_eq!(audit_res_ok.is_consistent, true);
    assert_eq!(audit_res_ok.mismatches.len(), 0);

    // Unknown session ID returns SessionNotFound
    let unknown_id = BytesN::from_array(&env, &[99; 32]);
    let audit_unknown = client.audit_session_index_consistency(&Vec::from_array(&env, [unknown_id]));
    assert_eq!(audit_unknown.is_consistent, false);
    assert_eq!(audit_unknown.mismatches.get(0).unwrap().category, upto_session::AuditIndexCategory::SessionNotFound);
}
