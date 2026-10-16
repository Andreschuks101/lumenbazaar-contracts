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
