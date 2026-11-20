use crate::{
    ids, AdminRotation, AssetStatus, CapIncreaseProposal, CommitmentKind, ContractError,
    OperationalState, Session, SessionStateCounts, SessionStatus,
};
use soroban_sdk::{contracttype, Address, BytesN, Env, Vec};

pub const STORAGE_LAYOUT_VERSION: u32 = 3;
// Every `extend_ttl` in this contract uses these two values: an entry whose remaining TTL is at or
// below `DEFAULT_TTL_THRESHOLD` ledgers is extended to `TTL_EXTENSION_AMOUNT` ledgers. Key
// lifetimes, extension responsibility, and missing-state behavior are specified in
// docs/storage-lifecycle.md (rules SR-1 to SR-10).
pub const DEFAULT_TTL_THRESHOLD: u32 = 100_000;
pub const TTL_EXTENSION_AMOUNT: u32 = 1_000_000;

// Instance keys share the single contract-instance TTL, extended only by `initialize` and admin
// entry points (SR-1, SR-2). Persistent keys have their own TTL, extended by session writes and
// `extend_ttl` (SR-3 to SR-6). No temporary storage is used.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    // Instance.
    Admin,
    // Instance.
    AdminRotation,
    // Instance. Lives with the rest of the config, so it cannot lapse on its own.
    NextSessionSequence,
    // Persistent, one per session (open and finalized).
    Session(BytesN<32>),
    // Instance.
    StorageLayoutVersion,
    // Instance, one per configured asset.
    SupportedAsset(Address),
    // Instance, one per configured asset.
    AssetMaxSessionAmount(Address),
    // Instance, one per configured asset.
    AssetMinSessionAmount(Address),
    // Instance, present only while a cap increase is pending.
    CapIncreaseProposal(Address),
    // Instance.
    SupportedAssets,
    // Instance.
    OperationalState,
    // Instance. Immutable digest captured during initialization.
    InitConfigDigest,
    // Instance. One bounded counter per lifecycle state.
    SessionStateCount(SessionStatus),
    // Persistent aggregate per asset; extended only alongside a session of that asset.
    Liability(Address),
    // Instance, optional.
    SellerExposureCeiling(Address),
    // Persistent aggregate per (seller, asset); extended only alongside a matching session.
    SellerExposure(Address, Address),
    // Instance, optional.
    MaxBuyerOpenSessions,
    // Persistent aggregate per buyer; extended only alongside that buyer's session.
    BuyerOpenSessions(Address),
    // Persistent monotonic count of every session created by this buyer.
    BuyerTotalSessions(Address),
    // Instance, optional.
    MaxSellerOpenSessions,
    // Persistent aggregate per seller; extended only alongside that seller's session.
    SellerOpenSessions(Address),
    // Persistent aggregate per (seller, lifecycle state).
    SellerSessionStateCount(Address, SessionStatus),
    AssetExposureCeiling(Address),
    BuyerExposureCeiling(Address),
    BuyerExposure(Address, Address),
    Idempotency(Address, BytesN<32>),
    // Persistent bounded list keyed by a versioned, domain-separated digest.
    CommitmentIndex(BytesN<32>),
}

pub const MAX_COMMITMENT_INDEX_SESSIONS: u32 = 8;
pub const SESSION_STATE_COUNTS_RESPONSE_VERSION: u32 = 1;

pub fn write_storage_layout_version(env: &Env) {
    env.storage()
        .instance()
        .set(&DataKey::StorageLayoutVersion, &STORAGE_LAYOUT_VERSION);
}

pub fn has_admin(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

pub fn write_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn write_admin_rotation(env: &Env, rotation: &AdminRotation) {
    env.storage()
        .instance()
        .set(&DataKey::AdminRotation, rotation);
}

pub fn read_admin_rotation(env: &Env) -> Option<AdminRotation> {
    env.storage().instance().get(&DataKey::AdminRotation)
}

pub fn remove_admin_rotation(env: &Env) {
    env.storage().instance().remove(&DataKey::AdminRotation);
}

pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);
}

pub fn write_supported_asset(env: &Env, asset: &Address) {
    env.storage().instance().set(
        &DataKey::SupportedAsset(asset.clone()),
        &AssetStatus::Active,
    );
}

pub fn is_supported_asset(env: &Env, asset: &Address) -> bool {
    read_asset_status(env, asset) == Some(AssetStatus::Active)
}

pub fn read_asset_status(env: &Env, asset: &Address) -> Option<AssetStatus> {
    env.storage()
        .instance()
        .get(&DataKey::SupportedAsset(asset.clone()))
}

pub fn write_asset_status(env: &Env, asset: &Address, status: &AssetStatus) {
    env.storage()
        .instance()
        .set(&DataKey::SupportedAsset(asset.clone()), status);
}

pub fn remove_asset_status(env: &Env, asset: &Address) {
    env.storage()
        .instance()
        .remove(&DataKey::SupportedAsset(asset.clone()));
}

pub fn write_asset_max_session_amount(env: &Env, asset: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::AssetMaxSessionAmount(asset.clone()), &amount);
}

pub fn read_asset_max_session_amount(env: &Env, asset: &Address) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::AssetMaxSessionAmount(asset.clone()))
}

pub fn write_asset_min_session_amount(env: &Env, asset: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::AssetMinSessionAmount(asset.clone()), &amount);
}

pub fn read_asset_min_session_amount(env: &Env, asset: &Address) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::AssetMinSessionAmount(asset.clone()))
}

pub fn write_cap_increase_proposal(env: &Env, asset: &Address, proposal: &CapIncreaseProposal) {
    env.storage()
        .instance()
        .set(&DataKey::CapIncreaseProposal(asset.clone()), proposal);
}

pub fn read_cap_increase_proposal(env: &Env, asset: &Address) -> Option<CapIncreaseProposal> {
    env.storage()
        .instance()
        .get(&DataKey::CapIncreaseProposal(asset.clone()))
}

pub fn remove_cap_increase_proposal(env: &Env, asset: &Address) {
    env.storage()
        .instance()
        .remove(&DataKey::CapIncreaseProposal(asset.clone()));
}

pub fn remove_asset_max_session_amount(env: &Env, asset: &Address) {
    env.storage()
        .instance()
        .remove(&DataKey::AssetMaxSessionAmount(asset.clone()));
}

/// Deletes the zero-liability record of a removed asset. Callers must have
/// checked that the recorded liability is exactly zero.
pub fn remove_liability(env: &Env, asset: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Liability(asset.clone()));
}

/// Configured assets (active or deprecated) in activation order. Entries whose
/// lifecycle record no longer exists, such as assets removed before removal
/// cleaned the list, are skipped so they never read as supported.
pub fn read_configured_assets(env: &Env) -> Vec<Address> {
    let mut configured = Vec::new(env);
    for asset in read_supported_assets(env).iter() {
        if read_asset_status(env, &asset).is_some() {
            configured.push_back(asset);
        }
    }
    configured
}

/// Drops `asset` from the ordered asset list, keeping the order of the others.
/// Stale entries left by earlier removals are dropped in the same pass.
pub fn remove_from_supported_assets(env: &Env, asset: &Address) {
    let mut remaining = Vec::new(env);
    for configured in read_supported_assets(env).iter() {
        if &configured != asset && read_asset_status(env, &configured).is_some() {
            remaining.push_back(configured);
        }
    }
    write_supported_assets(env, &remaining);
}

pub fn write_supported_assets(env: &Env, assets: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::SupportedAssets, assets);
}

pub fn read_supported_assets(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::SupportedAssets)
        .unwrap_or(Vec::new(env))
}

pub fn write_operational_state(env: &Env, state: &OperationalState) {
    env.storage()
        .instance()
        .set(&DataKey::OperationalState, state);
}

pub fn write_initialization_configuration_digest(env: &Env, digest: &BytesN<32>) {
    env.storage()
        .instance()
        .set(&DataKey::InitConfigDigest, digest);
}

pub fn read_initialization_configuration_digest(env: &Env) -> Option<BytesN<32>> {
    env.storage().instance().get(&DataKey::InitConfigDigest)
}

pub fn read_session_state_count(env: &Env, status: &SessionStatus) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::SessionStateCount(status.clone()))
        .unwrap_or(0)
}

pub fn read_seller_session_state_count(env: &Env, seller: &Address, status: &SessionStatus) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::SellerSessionStateCount(
            seller.clone(),
            status.clone(),
        ))
        .unwrap_or(0)
}

pub fn read_session_state_counts(env: &Env) -> SessionStateCounts {
    counts_for(env, None)
}

pub fn read_seller_session_state_counts(env: &Env, seller: &Address) -> SessionStateCounts {
    counts_for(env, Some(seller))
}

fn counts_for(env: &Env, seller: Option<&Address>) -> SessionStateCounts {
    let read = |status: SessionStatus| match seller {
        Some(address) => read_seller_session_state_count(env, address, &status),
        None => read_session_state_count(env, &status),
    };
    SessionStateCounts {
        response_version: SESSION_STATE_COUNTS_RESPONSE_VERSION,
        open: read(SessionStatus::Open),
        settled: read(SessionStatus::Settled),
        cancelled: read(SessionStatus::Cancelled),
        expired: read(SessionStatus::Expired),
        rejected: read(SessionStatus::Rejected),
    }
}

fn update_count(
    env: &Env,
    seller: Option<&Address>,
    status: &SessionStatus,
    increment: bool,
) -> Result<(), ContractError> {
    let current = match seller {
        Some(address) => read_seller_session_state_count(env, address, status),
        None => read_session_state_count(env, status),
    };
    let updated = if increment {
        current
            .checked_add(1)
            .ok_or(ContractError::CounterOverflow)?
    } else {
        current
            .checked_sub(1)
            .ok_or(ContractError::CounterUnderflow)?
    };
    match seller {
        Some(address) => {
            let key = DataKey::SellerSessionStateCount(address.clone(), status.clone());
            if updated == 0 {
                env.storage().persistent().remove(&key);
            } else {
                env.storage().persistent().set(&key, &updated);
                env.storage().persistent().extend_ttl(
                    &key,
                    DEFAULT_TTL_THRESHOLD,
                    TTL_EXTENSION_AMOUNT,
                );
            }
        }
        None => env
            .storage()
            .instance()
            .set(&DataKey::SessionStateCount(status.clone()), &updated),
    }
    Ok(())
}

fn transition_session_counts(
    env: &Env,
    previous: Option<&Session>,
    next: &Session,
) -> Result<(), ContractError> {
    if let Some(prior) = previous {
        if prior.status == next.status && prior.seller == next.seller {
            return Ok(());
        }
        update_count(env, None, &prior.status, false)?;
        update_count(env, Some(&prior.seller), &prior.status, false)?;
    }
    update_count(env, None, &next.status, true)?;
    update_count(env, Some(&next.seller), &next.status, true)
}

pub fn read_operational_state(env: &Env) -> OperationalState {
    env.storage()
        .instance()
        .get(&DataKey::OperationalState)
        .unwrap_or(OperationalState::Active)
}

pub fn read_liability(env: &Env, asset: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Liability(asset.clone()))
        .unwrap_or(0)
}

pub fn increase_liability(env: &Env, asset: &Address, amount: i128) -> Result<i128, ContractError> {
    let updated = read_liability(env, asset)
        .checked_add(amount)
        .ok_or(ContractError::LiabilityOverflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::Liability(asset.clone()), &updated);
    Ok(updated)
}

pub fn decrease_liability(env: &Env, asset: &Address, amount: i128) -> Result<i128, ContractError> {
    let updated = read_liability(env, asset)
        .checked_sub(amount)
        .filter(|value| *value >= 0)
        .ok_or(ContractError::LiabilityUnderflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::Liability(asset.clone()), &updated);
    Ok(updated)
}

pub fn write_seller_exposure_ceiling(env: &Env, asset: &Address, ceiling: i128) {
    env.storage()
        .instance()
        .set(&DataKey::SellerExposureCeiling(asset.clone()), &ceiling);
}

pub fn read_seller_exposure_ceiling(env: &Env, asset: &Address) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::SellerExposureCeiling(asset.clone()))
}

pub fn read_seller_exposure(env: &Env, seller: &Address, asset: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::SellerExposure(seller.clone(), asset.clone()))
        .unwrap_or(0)
}

pub fn increase_seller_exposure(
    env: &Env,
    seller: &Address,
    asset: &Address,
    amount: i128,
) -> Result<i128, ContractError> {
    let current = read_seller_exposure(env, seller, asset);
    let updated = current
        .checked_add(amount)
        .ok_or(ContractError::LiabilityOverflow)?;
    env.storage().persistent().set(
        &DataKey::SellerExposure(seller.clone(), asset.clone()),
        &updated,
    );
    Ok(updated)
}

pub fn decrease_seller_exposure(
    env: &Env,
    seller: &Address,
    asset: &Address,
    amount: i128,
) -> Result<i128, ContractError> {
    let current = read_seller_exposure(env, seller, asset);
    let updated = current
        .checked_sub(amount)
        .filter(|value| *value >= 0)
        .ok_or(ContractError::LiabilityUnderflow)?;
    env.storage().persistent().set(
        &DataKey::SellerExposure(seller.clone(), asset.clone()),
        &updated,
    );
    Ok(updated)
}

pub fn write_max_buyer_open_sessions(env: &Env, limit: u32) {
    env.storage()
        .instance()
        .set(&DataKey::MaxBuyerOpenSessions, &limit);
}

pub fn read_max_buyer_open_sessions(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::MaxBuyerOpenSessions)
}

pub fn read_buyer_open_sessions(env: &Env, buyer: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::BuyerOpenSessions(buyer.clone()))
        .unwrap_or(0)
}

pub fn increment_buyer_open_sessions(env: &Env, buyer: &Address) -> Result<u32, ContractError> {
    let current = read_buyer_open_sessions(env, buyer);
    let updated = current
        .checked_add(1)
        .ok_or(ContractError::LiabilityOverflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::BuyerOpenSessions(buyer.clone()), &updated);
    Ok(updated)
}

pub fn read_buyer_total_sessions(env: &Env, buyer: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::BuyerTotalSessions(buyer.clone()))
        .unwrap_or(0)
}

pub fn increment_buyer_total_sessions(env: &Env, buyer: &Address) -> Result<u32, ContractError> {
    let updated = read_buyer_total_sessions(env, buyer)
        .checked_add(1)
        .ok_or(ContractError::LiabilityOverflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::BuyerTotalSessions(buyer.clone()), &updated);
    Ok(updated)
}

pub fn decrement_buyer_open_sessions(env: &Env, buyer: &Address) -> Result<u32, ContractError> {
    let current = read_buyer_open_sessions(env, buyer);
    let updated = current
        .checked_sub(1)
        .ok_or(ContractError::LiabilityUnderflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::BuyerOpenSessions(buyer.clone()), &updated);
    Ok(updated)
}

pub fn write_max_seller_open_sessions(env: &Env, limit: u32) {
    env.storage()
        .instance()
        .set(&DataKey::MaxSellerOpenSessions, &limit);
}

pub fn read_max_seller_open_sessions(env: &Env) -> Option<u32> {
    env.storage()
        .instance()
        .get(&DataKey::MaxSellerOpenSessions)
}

pub fn read_seller_open_sessions(env: &Env, seller: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::SellerOpenSessions(seller.clone()))
        .unwrap_or(0)
}

pub fn increment_seller_open_sessions(env: &Env, seller: &Address) -> Result<u32, ContractError> {
    let current = read_seller_open_sessions(env, seller);
    let updated = current
        .checked_add(1)
        .ok_or(ContractError::LiabilityOverflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::SellerOpenSessions(seller.clone()), &updated);
    Ok(updated)
}

pub fn decrement_seller_open_sessions(env: &Env, seller: &Address) -> Result<u32, ContractError> {
    let current = read_seller_open_sessions(env, seller);
    let updated = current
        .checked_sub(1)
        .ok_or(ContractError::LiabilityUnderflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::SellerOpenSessions(seller.clone()), &updated);
    Ok(updated)
}

pub fn write_asset_exposure_ceiling(env: &Env, asset: &Address, ceiling: i128) {
    env.storage()
        .instance()
        .set(&DataKey::AssetExposureCeiling(asset.clone()), &ceiling);
}

pub fn read_asset_exposure_ceiling(env: &Env, asset: &Address) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::AssetExposureCeiling(asset.clone()))
}

pub fn write_buyer_exposure_ceiling(env: &Env, asset: &Address, ceiling: i128) {
    env.storage()
        .instance()
        .set(&DataKey::BuyerExposureCeiling(asset.clone()), &ceiling);
}

pub fn read_buyer_exposure_ceiling(env: &Env, asset: &Address) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::BuyerExposureCeiling(asset.clone()))
}

pub fn read_buyer_exposure(env: &Env, buyer: &Address, asset: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::BuyerExposure(buyer.clone(), asset.clone()))
        .unwrap_or(0)
}

/// Returns the buyer exposure after adding `amount`, without writing it.
pub fn checked_buyer_exposure_increase(
    env: &Env,
    buyer: &Address,
    asset: &Address,
    amount: i128,
) -> Result<i128, ContractError> {
    read_buyer_exposure(env, buyer, asset)
        .checked_add(amount)
        .ok_or(ContractError::LiabilityOverflow)
}

/// Returns the buyer exposure after removing `amount`, without writing it.
pub fn checked_buyer_exposure_decrease(
    env: &Env,
    buyer: &Address,
    asset: &Address,
    amount: i128,
) -> Result<i128, ContractError> {
    read_buyer_exposure(env, buyer, asset)
        .checked_sub(amount)
        .filter(|value| *value >= 0)
        .ok_or(ContractError::BuyerExposureUnderflow)
}

/// Stores the buyer exposure. A zero exposure removes the entry so a buyer
/// without open sessions holds no rent-bearing state.
pub fn write_buyer_exposure(env: &Env, buyer: &Address, asset: &Address, exposure: i128) {
    let key = DataKey::BuyerExposure(buyer.clone(), asset.clone());
    if exposure == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &exposure);
    }
}

pub fn read_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Admin)
}

pub fn read_storage_layout_version(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::StorageLayoutVersion)
}

/// The maximum valid session sequence that can be assigned.
/// Sequences are 0-indexed and advance monotonically up to this terminal value.
/// When the sequence counter reaches `u64::MAX`, session sequence capacity is exhausted.
/// All subsequent session creation requests fail deterministically with
/// `ContractError::SequenceExhausted` before authorization, transfers, or writes.
pub const LAST_VALID_SESSION_SEQUENCE: u64 = u64::MAX - 1;

pub fn read_next_session_sequence(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::NextSessionSequence)
        .unwrap_or(0)
}

pub fn check_session_sequence_available(env: &Env) -> Result<u64, ContractError> {
    let sequence = read_next_session_sequence(env);
    if sequence == u64::MAX {
        return Err(ContractError::SequenceExhausted);
    }
    Ok(sequence)
}

pub fn take_next_session_sequence(env: &Env) -> Result<u64, ContractError> {
    let sequence = read_next_session_sequence(env);
    let next = sequence
        .checked_add(1)
        .ok_or(ContractError::SequenceExhausted)?;
    env.storage()
        .instance()
        .set(&DataKey::NextSessionSequence, &next);
    Ok(sequence)
}

pub fn write_idempotency_key(
    env: &Env,
    buyer: &Address,
    key: &BytesN<32>,
    session_id: &BytesN<32>,
) {
    let storage_key = DataKey::Idempotency(buyer.clone(), key.clone());
    env.storage().persistent().set(&storage_key, session_id);
    env.storage().persistent().extend_ttl(
        &storage_key,
        DEFAULT_TTL_THRESHOLD,
        TTL_EXTENSION_AMOUNT,
    );
}

pub fn read_idempotency_key(env: &Env, buyer: &Address, key: &BytesN<32>) -> Option<BytesN<32>> {
    env.storage()
        .persistent()
        .get(&DataKey::Idempotency(buyer.clone(), key.clone()))
}

pub fn has_idempotency_key(env: &Env, buyer: &Address, key: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::Idempotency(buyer.clone(), key.clone()))
}

pub fn write_session(env: &Env, session: &Session) -> Result<(), ContractError> {
    let previous = read_session(env, &session.id);
    transition_session_counts(env, previous.as_ref(), session)?;
    env.storage()
        .persistent()
        .set(&DataKey::Session(session.id.clone()), session);
    extend_session_and_liability_ttl(env, session);
    Ok(())
}

pub fn index_session_commitment(
    env: &Env,
    kind: &CommitmentKind,
    commitment: &BytesN<32>,
    session_id: &BytesN<32>,
) -> Result<(), ContractError> {
    let digest = ids::commitment_index_key(env, kind, commitment);
    let key = DataKey::CommitmentIndex(digest);
    let mut sessions: Vec<BytesN<32>> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    if sessions.iter().any(|existing| existing == *session_id) {
        return Ok(());
    }
    if sessions.len() >= MAX_COMMITMENT_INDEX_SESSIONS {
        return Err(ContractError::CommitmentIndexFull);
    }
    sessions.push_back(session_id.clone());
    env.storage().persistent().set(&key, &sessions);
    env.storage()
        .persistent()
        .extend_ttl(&key, DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);
    Ok(())
}

pub fn read_sessions_by_commitment(
    env: &Env,
    kind: &CommitmentKind,
    commitment: &BytesN<32>,
) -> Vec<BytesN<32>> {
    let key = DataKey::CommitmentIndex(ids::commitment_index_key(env, kind, commitment));
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn extend_session_and_liability_ttl(env: &Env, session: &Session) {
    extend_session_ttl(env, &session.id);
    let liability_key = DataKey::Liability(session.asset.clone());
    if env.storage().persistent().has(&liability_key) {
        env.storage().persistent().extend_ttl(
            &liability_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    let seller_exposure_key =
        DataKey::SellerExposure(session.seller.clone(), session.asset.clone());
    if env.storage().persistent().has(&seller_exposure_key) {
        env.storage().persistent().extend_ttl(
            &seller_exposure_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    let buyer_exposure_key = DataKey::BuyerExposure(session.buyer.clone(), session.asset.clone());
    if env.storage().persistent().has(&buyer_exposure_key) {
        env.storage().persistent().extend_ttl(
            &buyer_exposure_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    let buyer_key = DataKey::BuyerOpenSessions(session.buyer.clone());
    if env.storage().persistent().has(&buyer_key) {
        env.storage().persistent().extend_ttl(
            &buyer_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    let buyer_total_key = DataKey::BuyerTotalSessions(session.buyer.clone());
    if env.storage().persistent().has(&buyer_total_key) {
        env.storage().persistent().extend_ttl(
            &buyer_total_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    let seller_key = DataKey::SellerOpenSessions(session.seller.clone());
    if env.storage().persistent().has(&seller_key) {
        env.storage().persistent().extend_ttl(
            &seller_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    let seller_state_key =
        DataKey::SellerSessionStateCount(session.seller.clone(), session.status.clone());
    if env.storage().persistent().has(&seller_state_key) {
        env.storage().persistent().extend_ttl(
            &seller_state_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
    for (kind, commitment) in [
        (
            CommitmentKind::Resource,
            Some(session.resource_hash.clone()),
        ),
        (CommitmentKind::Usage, session.usage_hash.clone()),
    ] {
        if let Some(value) = commitment {
            let key = DataKey::CommitmentIndex(ids::commitment_index_key(env, &kind, &value));
            if env.storage().persistent().has(&key) {
                env.storage().persistent().extend_ttl(
                    &key,
                    DEFAULT_TTL_THRESHOLD,
                    TTL_EXTENSION_AMOUNT,
                );
            }
        }
    }
}

pub fn read_session(env: &Env, session_id: &BytesN<32>) -> Option<Session> {
    env.storage()
        .persistent()
        .get(&DataKey::Session(session_id.clone()))
}

pub fn has_session(env: &Env, session_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::Session(session_id.clone()))
}

pub fn extend_session_ttl(env: &Env, session_id: &BytesN<32>) {
    env.storage().persistent().extend_ttl(
        &DataKey::Session(session_id.clone()),
        DEFAULT_TTL_THRESHOLD,
        TTL_EXTENSION_AMOUNT,
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::SessionStatus;
    use soroban_sdk::{testutils::Address as _, Address, BytesN, Env};

    fn sample_session(env: &Env) -> Session {
        let id = BytesN::from_array(env, &[1; 32]);
        Session {
            id,
            buyer: Address::generate(env),
            seller: Address::generate(env),
            asset: Address::generate(env),
            max_amount: 100,
            escrowed_amount: 100,
            settled_amount: 0,
            expires_at_ledger: 50,
            resource_hash: BytesN::from_array(env, &[2; 32]),
            usage_hash: None,
            status: SessionStatus::Open,
            creation_ledger: 10,
            finalized_ledger: None,
            resource_scheme_version: 1,
            usage_scheme_version: None,
        }
    }

    #[test]
    fn storage_layout_version_round_trips() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            assert_eq!(read_storage_layout_version(&env), None);
            write_storage_layout_version(&env);

            assert_eq!(
                read_storage_layout_version(&env),
                Some(STORAGE_LAYOUT_VERSION)
            );
        });
    }

    #[test]
    fn session_sequence_increments() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            assert_eq!(take_next_session_sequence(&env).unwrap(), 0);
            assert_eq!(take_next_session_sequence(&env).unwrap(), 1);
            assert_eq!(read_next_session_sequence(&env), 2);
        });
    }

    #[test]
    fn session_sequence_exhaustion_behavior() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            // Set sequence counter to last valid sequence
            env.storage()
                .instance()
                .set(&DataKey::NextSessionSequence, &LAST_VALID_SESSION_SEQUENCE);

            // Last valid sequence succeeds and advances to u64::MAX
            assert_eq!(
                take_next_session_sequence(&env).unwrap(),
                LAST_VALID_SESSION_SEQUENCE
            );
            assert_eq!(read_next_session_sequence(&env), u64::MAX);

            // At terminal capacity, check fails without changing state
            assert_eq!(
                check_session_sequence_available(&env),
                Err(ContractError::SequenceExhausted)
            );
            assert_eq!(
                take_next_session_sequence(&env),
                Err(ContractError::SequenceExhausted)
            );
            assert_eq!(read_next_session_sequence(&env), u64::MAX);
        });
    }

    #[test]
    fn idempotency_storage_round_trips() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            let buyer = Address::generate(&env);
            let key = BytesN::from_array(&env, &[42; 32]);
            let session_id = BytesN::from_array(&env, &[99; 32]);

            assert!(!has_idempotency_key(&env, &buyer, &key));
            assert_eq!(read_idempotency_key(&env, &buyer, &key), None);

            write_idempotency_key(&env, &buyer, &key, &session_id);

            assert!(has_idempotency_key(&env, &buyer, &key));
            assert_eq!(read_idempotency_key(&env, &buyer, &key), Some(session_id));
        });
    }

    #[test]
    fn session_state_round_trips() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            let session = sample_session(&env);

            assert!(!has_session(&env, &session.id));
            write_session(&env, &session).unwrap();

            assert!(has_session(&env, &session.id));
            assert_eq!(read_session(&env, &session.id), Some(session));
        });
    }
}
