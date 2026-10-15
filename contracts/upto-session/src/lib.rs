#![no_std]

mod errors;
pub mod events;
pub mod ids;
pub mod storage;
mod types;
pub mod validation;

pub use errors::ContractError;
pub use types::{Session, SessionStatus};

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env, Vec};

#[contract]
pub struct UptoSessionContract;

pub const INTERFACE_VERSION: u32 = 2;

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
        let buyer_balance_before = escrow.balance(&buyer);
        let contract_balance_before = escrow.balance(&contract);
        escrow.transfer(&buyer, &contract, &max_amount);
        verify_debit(buyer_balance_before, escrow.balance(&buyer), max_amount)?;
        verify_credit(
            contract_balance_before,
            escrow.balance(&contract),
            max_amount,
        )?;
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
        let contract_balance_before = asset.balance(&contract);
        let seller_balance_before = asset.balance(&session.seller);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, escrowed_amount)?;
        asset.transfer(&contract, &session.seller, &actual_amount);
        if refunded_amount > 0 {
            asset.transfer(&contract, &session.buyer, &refunded_amount);
        }
        verify_debit(
            contract_balance_before,
            asset.balance(&contract),
            escrowed_amount,
        )?;
        verify_credit(
            seller_balance_before,
            asset.balance(&session.seller),
            actual_amount,
        )?;
        verify_credit(
            buyer_balance_before,
            asset.balance(&session.buyer),
            refunded_amount,
        )?;
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
        let contract_balance_before = asset.balance(&contract);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, refunded_amount)?;
        asset.transfer(&contract, &session.buyer, &refunded_amount);
        verify_debit(
            contract_balance_before,
            asset.balance(&contract),
            refunded_amount,
        )?;
        verify_credit(
            buyer_balance_before,
            asset.balance(&session.buyer),
            refunded_amount,
        )?;
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

        let refunded_amount = session.escrowed_amount;
        let asset = token::Client::new(&env, &session.asset);
        let contract = env.current_contract_address();
        if refunded_amount != session.max_amount
            || asset.balance(&contract) < storage::read_liability(&env, &session.asset)
        {
            return Err(ContractError::EscrowUnderfunded);
        }
        let contract_balance_before = asset.balance(&contract);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, refunded_amount)?;
        asset.transfer(&contract, &session.buyer, &refunded_amount);
        verify_debit(
            contract_balance_before,
            asset.balance(&contract),
            refunded_amount,
        )?;
        verify_credit(
            buyer_balance_before,
            asset.balance(&session.buyer),
            refunded_amount,
        )?;
        session.escrowed_amount = 0;
        session.status = SessionStatus::Expired;
        storage::write_session(&env, &session);
        events::publish_session_recovered(&env, &session, refunded_amount);

        Ok(())
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

fn verify_debit(before: i128, after: i128, amount: i128) -> Result<(), ContractError> {
    if before.checked_sub(amount) != Some(after) {
        return Err(ContractError::TokenInvariantViolation);
    }
    Ok(())
}

fn verify_credit(before: i128, after: i128, amount: i128) -> Result<(), ContractError> {
    if before.checked_add(amount) != Some(after) {
        return Err(ContractError::TokenInvariantViolation);
    }
    Ok(())
}

#[cfg(test)]
mod test;
