#![no_std]

mod errors;
pub mod events;
pub mod ids;
pub mod storage;
mod types;
pub mod validation;

pub use errors::ContractError;
pub use types::{BatchRecoveryResult, BatchRecoveryStatus, Session, SessionStatus};

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env, Vec};

#[contract]
pub struct UptoSessionContract;

pub const INTERFACE_VERSION: u32 = 3;
pub const MAX_RECOVERY_BATCH_SIZE: u32 = 8;

#[contractimpl]
impl UptoSessionContract {
    pub fn interface_version() -> u32 {
        INTERFACE_VERSION
    }

    pub fn initialize(
        env: Env,
        admin: Address,
        supported_assets: Vec<Address>,
    ) -> Result<(), ContractError> {
        if storage::has_admin(&env) {
            return Err(ContractError::AlreadyInitialized);
        }

        admin.require_auth();
        if supported_assets.is_empty() {
            return Err(ContractError::InvalidSupportedAssets);
        }
        for asset in supported_assets.iter() {
            if storage::is_supported_asset(&env, &asset) {
                return Err(ContractError::InvalidSupportedAssets);
            }
            storage::write_supported_asset(&env, &asset);
        }
        storage::write_admin(&env, &admin);
        storage::write_storage_layout_version(&env);
        storage::extend_instance_ttl(&env);

        Ok(())
    }

    pub fn create_session(
        env: Env,
        buyer: Address,
        seller: Address,
        asset: Address,
        max_amount: i128,
        expires_at_ledger: u32,
        resource_hash: BytesN<32>,
    ) -> Result<BytesN<32>, ContractError> {
        validation::validate_create_session(
            &env,
            &buyer,
            &seller,
            &asset,
            max_amount,
            expires_at_ledger,
            &resource_hash,
        )?;
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }

        buyer.require_auth();
        let sequence = storage::take_next_session_sequence(&env);
        let session_id = ids::derive_session_id(
            &env,
            &ids::SessionIdInput {
                buyer: &buyer,
                seller: &seller,
                asset: &asset,
                max_amount,
                expires_at_ledger,
                resource_hash: &resource_hash,
                sequence,
            },
        );

        let escrow = token::Client::new(&env, &asset);
        let contract = env.current_contract_address();
        escrow.transfer(&buyer, &contract, &max_amount);
        let liability = storage::increase_liability(&env, &asset, max_amount)?;
        if escrow.balance(&contract) < liability {
            return Err(ContractError::EscrowUnderfunded);
        }

        let session = Session {
            id: session_id.clone(),
            buyer,
            seller,
            asset,
            max_amount,
            escrowed_amount: max_amount,
            settled_amount: 0,
            expires_at_ledger,
            resource_hash,
            usage_hash: None,
            status: SessionStatus::Open,
        };

        storage::write_session(&env, &session);
        events::publish_session_created(&env, &session);

        Ok(session_id)
    }

    pub fn settle(
        env: Env,
        session_id: BytesN<32>,
        actual_amount: i128,
        usage_hash: BytesN<32>,
    ) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
        }

        validation::validate_settlement_amount(&env, &session, actual_amount)?;
        validation::validate_usage_hash(&env, &usage_hash)?;
        session.seller.require_auth();
        let escrowed_amount = session.escrowed_amount;
        let contract = env.current_contract_address();
        let asset = token::Client::new(&env, &session.asset);
        if escrowed_amount != session.max_amount
            || asset.balance(&contract) < storage::read_liability(&env, &session.asset)
        {
            return Err(ContractError::EscrowUnderfunded);
        }
        let refunded_amount = escrowed_amount - actual_amount;
        storage::decrease_liability(&env, &session.asset, escrowed_amount)?;
        asset.transfer(&contract, &session.seller, &actual_amount);
        if refunded_amount > 0 {
            asset.transfer(&contract, &session.buyer, &refunded_amount);
        }
        session.settled_amount = actual_amount;
        session.escrowed_amount = 0;
        session.usage_hash = Some(usage_hash.clone());
        session.status = SessionStatus::Settled;
        storage::write_session(&env, &session);
        events::publish_session_settled(
            &env,
            &session_id,
            &session.seller,
            &session.asset,
            actual_amount,
            refunded_amount,
            &usage_hash,
        );

        Ok(())
    }

    pub fn cancel(env: Env, session_id: BytesN<32>) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
        }

        if env.ledger().sequence() >= session.expires_at_ledger {
            return Err(ContractError::ExpiredSession);
        }
        session.buyer.require_auth();
        let refunded_amount = session.escrowed_amount;
        let asset = token::Client::new(&env, &session.asset);
        let contract = env.current_contract_address();
        if refunded_amount != session.max_amount
            || asset.balance(&contract) < storage::read_liability(&env, &session.asset)
        {
            return Err(ContractError::EscrowUnderfunded);
        }
        storage::decrease_liability(&env, &session.asset, refunded_amount)?;
        asset.transfer(&contract, &session.buyer, &refunded_amount);
        session.escrowed_amount = 0;
        session.status = SessionStatus::Cancelled;
        storage::write_session(&env, &session);
        events::publish_session_cancelled(
            &env,
            &session_id,
            &session.buyer,
            &session.asset,
            refunded_amount,
        );

        Ok(())
    }

    pub fn recover_expired(env: Env, session_id: BytesN<32>) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
        }
        if env.ledger().sequence() < session.expires_at_ledger {
            return Err(ContractError::SessionNotExpired);
        }

        recover_open_session(&env, &mut session)?;
        Ok(())
    }

    pub fn recover_expired_batch(
        env: Env,
        session_ids: Vec<BytesN<32>>,
    ) -> Result<Vec<BatchRecoveryResult>, ContractError> {
        if session_ids.is_empty() || session_ids.len() > MAX_RECOVERY_BATCH_SIZE {
            return Err(ContractError::InvalidBatchSize);
        }

        let mut seen = Vec::new(&env);
        let mut results = Vec::new(&env);
        for session_id in session_ids.iter() {
            if seen.iter().any(|seen_id| seen_id == session_id) {
                results.push_back(batch_result(session_id, BatchRecoveryStatus::Duplicate, 0));
                continue;
            }
            seen.push_back(session_id.clone());

            let Some(mut session) = storage::read_session(&env, &session_id) else {
                results.push_back(batch_result(session_id, BatchRecoveryStatus::NotFound, 0));
                continue;
            };

            let status = match session.status {
                SessionStatus::Settled => BatchRecoveryStatus::AlreadySettled,
                SessionStatus::Cancelled => BatchRecoveryStatus::AlreadyCancelled,
                SessionStatus::Expired => BatchRecoveryStatus::AlreadyRecovered,
                SessionStatus::Open if env.ledger().sequence() < session.expires_at_ledger => {
                    BatchRecoveryStatus::NotExpired
                }
                SessionStatus::Open => {
                    let refunded_amount = session.escrowed_amount;
                    match recover_open_session(&env, &mut session) {
                        Ok(()) => {
                            results.push_back(batch_result(
                                session_id,
                                BatchRecoveryStatus::Recovered,
                                refunded_amount,
                            ));
                            continue;
                        }
                        Err(
                            ContractError::EscrowUnderfunded | ContractError::LiabilityUnderflow,
                        ) => BatchRecoveryStatus::EscrowUnavailable,
                        Err(error) => return Err(error),
                    }
                }
            };
            results.push_back(batch_result(session_id, status, 0));
        }

        Ok(results)
    }

    pub fn get_session(env: Env, session_id: BytesN<32>) -> Result<Session, ContractError> {
        storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)
    }

    pub fn extend_ttl(env: Env, session_id: BytesN<32>) -> Result<(), ContractError> {
        let session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match session.status {
            SessionStatus::Open => {
                storage::extend_session_and_liability_ttl(&env, &session);
                Ok(())
            }
            SessionStatus::Settled | SessionStatus::Cancelled | SessionStatus::Expired => {
                Err(ContractError::TtlExtensionFailed)
            }
        }
    }
}

fn batch_result(
    session_id: BytesN<32>,
    status: BatchRecoveryStatus,
    refunded_amount: i128,
) -> BatchRecoveryResult {
    BatchRecoveryResult {
        session_id,
        status,
        refunded_amount,
    }
}

fn recover_open_session(env: &Env, session: &mut Session) -> Result<(), ContractError> {
    let refunded_amount = session.escrowed_amount;
    let asset = token::Client::new(env, &session.asset);
    let contract = env.current_contract_address();
    if refunded_amount != session.max_amount
        || asset.balance(&contract) < storage::read_liability(env, &session.asset)
    {
        return Err(ContractError::EscrowUnderfunded);
    }
    storage::decrease_liability(env, &session.asset, refunded_amount)?;
    asset.transfer(&contract, &session.buyer, &refunded_amount);
    session.escrowed_amount = 0;
    session.status = SessionStatus::Expired;
    storage::write_session(env, session);
    events::publish_session_recovered(env, session, refunded_amount);
    Ok(())
}

#[cfg(test)]
mod test;
