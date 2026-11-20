use crate::Session;
use soroban_sdk::{contractevent, Address, BytesN, Env};

#[contractevent]
pub struct AssetSessionCapUpdated {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub previous_max_session_amount: i128,
    pub new_max_session_amount: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct CapIncreaseProposed {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub proposed_max_amount: i128,
    pub active_at_ledger: u32,
    pub expires_at_ledger: u32,
    pub event_version: u32,
}

#[contractevent]
pub struct AssetExposureCeilingUpdated {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub previous_ceiling: i128,
    pub new_ceiling: i128,
    pub current_liability: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct SellerExposureCeilingUpdated {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub previous_ceiling: i128,
    pub new_ceiling: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct BuyerExposureCeilingUpdated {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub previous_ceiling: i128,
    pub new_ceiling: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct CapIncreaseCancelled {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub proposed_max_amount: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct MaxBuyerOpenSessionsUpdated {
    #[topic]
    pub admin: Address,
    pub previous_limit: u32,
    pub new_limit: u32,
    pub event_version: u32,
}

#[contractevent]
pub struct MaxSellerOpenSessionsUpdated {
    #[topic]
    pub admin: Address,
    pub previous_limit: u32,
    pub new_limit: u32,
    pub event_version: u32,
}

#[contractevent]
pub struct AssetDeprecated {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub reason_commitment: BytesN<32>,
    pub event_version: u32,
}

#[contractevent]
pub struct AdminRotationProposed {
    #[topic]
    pub pending_admin: Address,
    pub current_admin: Address,
    pub expires_at_ledger: u32,
    pub event_version: u32,
}

#[contractevent]
pub struct AdminRotationAccepted {
    #[topic]
    pub new_admin: Address,
    pub previous_admin: Address,
    pub event_version: u32,
}

#[contractevent]
pub struct AdminRotationCancelled {
    #[topic]
    pub pending_admin: Address,
    pub current_admin: Address,
    pub event_version: u32,
}

#[contractevent]
pub struct AssetActivated {
    #[topic]
    pub asset: Address,
    pub decimals: u32,
    pub event_version: u32,
}

#[contractevent]
pub struct AssetRemoved {
    #[topic]
    pub asset: Address,
    pub admin: Address,
    pub reason_commitment: BytesN<32>,
    pub event_version: u32,
}

#[contractevent]
pub struct CreationPaused {
    #[topic]
    pub admin: Address,
    pub event_version: u32,
}

#[contractevent]
pub struct CreationResumed {
    #[topic]
    pub admin: Address,
    pub event_version: u32,
}

#[contractevent]
pub struct SurplusRecovered {
    #[topic]
    pub asset: Address,
    pub recipient: Address,
    pub amount: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct SessionCreated {
    #[topic]
    pub session_id: BytesN<32>,
    pub buyer: Address,
    pub seller: Address,
    pub asset: Address,
    pub max_amount: i128,
    pub expires_at_ledger: u32,
    pub resource_hash: BytesN<32>,
    pub event_version: u32,
    pub escrowed_amount: i128,
}

#[contractevent]
pub struct SessionSettled {
    #[topic]
    pub session_id: BytesN<32>,
    pub seller: Address,
    pub asset: Address,
    pub actual_amount: i128,
    pub usage_hash: BytesN<32>,
    pub event_version: u32,
    pub refunded_amount: i128,
}

#[contractevent]
pub struct SessionCancelled {
    #[topic]
    pub session_id: BytesN<32>,
    pub buyer: Address,
    pub asset: Address,
    pub event_version: u32,
    pub refunded_amount: i128,
}

#[contractevent]
pub struct SessionRejected {
    #[topic]
    pub session_id: BytesN<32>,
    pub seller: Address,
    pub buyer: Address,
    pub asset: Address,
    pub event_version: u32,
    pub refunded_amount: i128,
}

#[contractevent]
pub struct SessionCapReduced {
    #[topic]
    pub session_id: BytesN<32>,
    pub buyer: Address,
    pub asset: Address,
    pub previous_max_amount: i128,
    pub new_max_amount: i128,
    pub refunded_amount: i128,
    pub event_version: u32,
}

#[contractevent]
pub struct SessionRecovered {
    #[topic]
    pub session_id: BytesN<32>,
    pub buyer: Address,
    pub asset: Address,
    pub event_version: u32,
    pub refunded_amount: i128,
    pub expired_at_ledger: u32,
    pub recovered_at_ledger: u32,
}

#[contractevent]
pub struct SessionExpiryExtended {
    #[topic]
    pub session_id: BytesN<32>,
    pub buyer: Address,
    pub seller: Address,
    pub previous_expires_at_ledger: u32,
    pub new_expires_at_ledger: u32,
    pub event_version: u32,
}

pub fn publish_session_created(env: &Env, session: &Session) {
    SessionCreated {
        session_id: session.id.clone(),
        buyer: session.buyer.clone(),
        seller: session.seller.clone(),
        asset: session.asset.clone(),
        max_amount: session.max_amount,
        expires_at_ledger: session.expires_at_ledger,
        resource_hash: session.resource_hash.clone(),
        event_version: 2,
        escrowed_amount: session.escrowed_amount,
    }
    .publish(env);
}

pub fn publish_asset_session_cap_updated(
    env: &Env,
    asset: &Address,
    admin: &Address,
    previous_max_session_amount: i128,
    new_max_session_amount: i128,
) {
    AssetSessionCapUpdated {
        asset: asset.clone(),
        admin: admin.clone(),
        previous_max_session_amount,
        new_max_session_amount,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_cap_increase_proposed(
    env: &Env,
    asset: &Address,
    admin: &Address,
    proposed_max_amount: i128,
    active_at_ledger: u32,
    expires_at_ledger: u32,
) {
    CapIncreaseProposed {
        asset: asset.clone(),
        admin: admin.clone(),
        proposed_max_amount,
        active_at_ledger,
        expires_at_ledger,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_asset_exposure_ceiling_updated(
    env: &Env,
    asset: &Address,
    admin: &Address,
    previous_ceiling: i128,
    new_ceiling: i128,
    current_liability: i128,
) {
    AssetExposureCeilingUpdated {
        asset: asset.clone(),
        admin: admin.clone(),
        previous_ceiling,
        new_ceiling,
        current_liability,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_seller_exposure_ceiling_updated(
    env: &Env,
    asset: &Address,
    admin: &Address,
    previous_ceiling: i128,
    new_ceiling: i128,
) {
    SellerExposureCeilingUpdated {
        asset: asset.clone(),
        admin: admin.clone(),
        previous_ceiling,
        new_ceiling,
        event_version: 1,
    }
    .publish(env);
}

pub fn publish_cap_increase_cancelled(
    env: &Env,
    asset: &Address,
    admin: &Address,
    proposed_max_amount: i128,
) {
    CapIncreaseCancelled {
        asset: asset.clone(),
        admin: admin.clone(),
        proposed_max_amount,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_buyer_exposure_ceiling_updated(
    env: &Env,
    asset: &Address,
    admin: &Address,
    previous_ceiling: i128,
    new_ceiling: i128,
) {
    BuyerExposureCeilingUpdated {
        asset: asset.clone(),
        admin: admin.clone(),
        previous_ceiling,
        new_ceiling,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_max_buyer_open_sessions_updated(
    env: &Env,
    admin: &Address,
    previous_limit: u32,
    new_limit: u32,
) {
    MaxBuyerOpenSessionsUpdated {
        admin: admin.clone(),
        previous_limit,
        new_limit,
        event_version: 1,
    }
    .publish(env);
}

pub fn publish_max_seller_open_sessions_updated(
    env: &Env,
    admin: &Address,
    previous_limit: u32,
    new_limit: u32,
) {
    MaxSellerOpenSessionsUpdated {
        admin: admin.clone(),
        previous_limit,
        new_limit,
        event_version: 1,
    }
    .publish(env);
}

pub fn publish_asset_deprecated(
    env: &Env,
    asset: &Address,
    admin: &Address,
    reason_commitment: &BytesN<32>,
) {
    AssetDeprecated {
        asset: asset.clone(),
        admin: admin.clone(),
        reason_commitment: reason_commitment.clone(),
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_admin_rotation_proposed(
    env: &Env,
    current_admin: &Address,
    pending_admin: &Address,
    expires_at_ledger: u32,
) {
    AdminRotationProposed {
        pending_admin: pending_admin.clone(),
        current_admin: current_admin.clone(),
        expires_at_ledger,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_admin_rotation_accepted(env: &Env, previous_admin: &Address, new_admin: &Address) {
    AdminRotationAccepted {
        new_admin: new_admin.clone(),
        previous_admin: previous_admin.clone(),
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_admin_rotation_cancelled(
    env: &Env,
    current_admin: &Address,
    pending_admin: &Address,
) {
    AdminRotationCancelled {
        pending_admin: pending_admin.clone(),
        current_admin: current_admin.clone(),
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_asset_activated(env: &Env, asset: &Address, decimals: u32) {
    AssetActivated {
        asset: asset.clone(),
        decimals,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_asset_removed(
    env: &Env,
    asset: &Address,
    admin: &Address,
    reason_commitment: &BytesN<32>,
) {
    AssetRemoved {
        asset: asset.clone(),
        admin: admin.clone(),
        reason_commitment: reason_commitment.clone(),
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_creation_paused(env: &Env, admin: &Address) {
    CreationPaused {
        admin: admin.clone(),
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_creation_resumed(env: &Env, admin: &Address) {
    CreationResumed {
        admin: admin.clone(),
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_session_settled(
    env: &Env,
    session_id: &BytesN<32>,
    seller: &Address,
    asset: &Address,
    actual_amount: i128,
    refunded_amount: i128,
    usage_hash: &BytesN<32>,
) {
    SessionSettled {
        session_id: session_id.clone(),
        seller: seller.clone(),
        asset: asset.clone(),
        actual_amount,
        usage_hash: usage_hash.clone(),
        event_version: 2,
        refunded_amount,
    }
    .publish(env);
}

pub fn publish_session_cancelled(
    env: &Env,
    session_id: &BytesN<32>,
    buyer: &Address,
    asset: &Address,
    refunded_amount: i128,
) {
    SessionCancelled {
        session_id: session_id.clone(),
        buyer: buyer.clone(),
        asset: asset.clone(),
        event_version: 2,
        refunded_amount,
    }
    .publish(env);
}

pub fn publish_session_rejected(
    env: &Env,
    session_id: &BytesN<32>,
    seller: &Address,
    buyer: &Address,
    asset: &Address,
    refunded_amount: i128,
) {
    SessionRejected {
        session_id: session_id.clone(),
        seller: seller.clone(),
        buyer: buyer.clone(),
        asset: asset.clone(),
        event_version: 2,
        refunded_amount,
    }
    .publish(env);
}

pub fn publish_session_cap_reduced(
    env: &Env,
    session_id: &BytesN<32>,
    buyer: &Address,
    asset: &Address,
    previous_max_amount: i128,
    new_max_amount: i128,
    refunded_amount: i128,
) {
    SessionCapReduced {
        session_id: session_id.clone(),
        buyer: buyer.clone(),
        asset: asset.clone(),
        previous_max_amount,
        new_max_amount,
        refunded_amount,
        event_version: 1,
    }
    .publish(env);
}

pub fn publish_session_recovered(env: &Env, session: &Session, refunded_amount: i128) {
    SessionRecovered {
        session_id: session.id.clone(),
        buyer: session.buyer.clone(),
        asset: session.asset.clone(),
        event_version: 2,
        refunded_amount,
        expired_at_ledger: session.expires_at_ledger,
        recovered_at_ledger: env.ledger().sequence(),
    }
    .publish(env);
}

pub fn publish_surplus_recovered(env: &Env, asset: &Address, recipient: &Address, amount: i128) {
    SurplusRecovered {
        asset: asset.clone(),
        recipient: recipient.clone(),
        amount,
        event_version: 3,
    }
    .publish(env);
}

pub fn publish_session_expiry_extended(
    env: &Env,
    session_id: &BytesN<32>,
    buyer: &Address,
    seller: &Address,
    previous_expires_at_ledger: u32,
    new_expires_at_ledger: u32,
) {
    SessionExpiryExtended {
        session_id: session_id.clone(),
        buyer: buyer.clone(),
        seller: seller.clone(),
        previous_expires_at_ledger,
        new_expires_at_ledger,
        event_version: 2,
    }
    .publish(env);
}
