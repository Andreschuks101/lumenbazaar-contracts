#![no_std]

mod errors;
pub mod events;
pub mod ids;
pub mod storage;
mod types;
pub mod validation;

pub use errors::ContractError;
pub use types::{
    AdminRotation, AssetPolicy, BatchRecoveryResult, BatchRecoveryStatus, ContractConfiguration,
    OperationalState, Session, SessionStatus,
};

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env, Vec};

#[contract]
pub struct UptoSessionContract;

pub const INTERFACE_VERSION: u32 = 3;
pub const CONFIGURATION_RESPONSE_VERSION: u32 = 1;
pub const MAX_SUPPORTED_ASSETS: u32 = 16;
pub const MAX_RECOVERY_BATCH_SIZE: u32 = 8;

#[contractimpl]
impl UptoSessionContract {
    pub fn interface_version() -> u32 {
        INTERFACE_VERSION
    }

    pub fn initialize(
        env: Env,
        admin: Address,
        asset_policies: Vec<AssetPolicy>,
    ) -> Result<(), ContractError> {
        if storage::has_admin(&env) {
            return Err(ContractError::AlreadyInitialized);
        }

        admin.require_auth();
        if asset_policies.is_empty() || asset_policies.len() > MAX_SUPPORTED_ASSETS {
            return Err(ContractError::InvalidSupportedAssets);
        }
        let mut supported_assets = Vec::new(&env);
        for policy in asset_policies.iter() {
            validation::validate_asset_cap(policy.max_session_amount)?;
            if supported_assets.iter().any(|asset| asset == policy.asset) {
                return Err(ContractError::InvalidSupportedAssets);
            }
            supported_assets.push_back(policy.asset);
        }
        for policy in asset_policies.iter() {
            let probe = validation::validate_supported_asset(&env, &policy.asset, &admin)?;
            storage::write_supported_asset(&env, &policy.asset);
            storage::write_asset_max_session_amount(&env, &policy.asset, policy.max_session_amount);
            events::publish_asset_activated(&env, &policy.asset, probe.decimals);
        }
        storage::write_supported_assets(&env, &supported_assets);
        storage::write_operational_state(&env, &OperationalState::Active);
        storage::write_admin(&env, &admin);
        storage::write_storage_layout_version(&env);
        storage::extend_instance_ttl(&env);

        Ok(())
    }

    pub fn set_asset_max_session_amount(
        env: Env,
        asset: Address,
        max_session_amount: i128,
    ) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        validation::validate_asset_cap(max_session_amount)?;
        admin.require_auth();

        let previous_max_session_amount = storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        storage::write_asset_max_session_amount(&env, &asset, max_session_amount);
        storage::extend_instance_ttl(&env);
        events::publish_asset_session_cap_updated(
            &env,
            &asset,
            &admin,
            previous_max_session_amount,
            max_session_amount,
        );
        Ok(())
    }

    pub fn get_asset_max_session_amount(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)
    }

    pub fn propose_admin(
        env: Env,
        pending_admin: Address,
        expires_at_ledger: u32,
    ) -> Result<(), ContractError> {
        let current_admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        current_admin.require_auth();
        if storage::read_admin_rotation(&env).is_some() {
            return Err(ContractError::AdminRotationPending);
        }
        validation::validate_admin_rotation(
            &env,
            &current_admin,
            &pending_admin,
            expires_at_ledger,
        )?;

        storage::write_admin_rotation(
            &env,
            &AdminRotation {
                pending_admin: pending_admin.clone(),
                expires_at_ledger,
            },
        );
        storage::extend_instance_ttl(&env);
        events::publish_admin_rotation_proposed(
            &env,
            &current_admin,
            &pending_admin,
            expires_at_ledger,
        );
        Ok(())
    }

    pub fn accept_admin(env: Env) -> Result<(), ContractError> {
        let current_admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        let rotation =
            storage::read_admin_rotation(&env).ok_or(ContractError::AdminRotationNotFound)?;
        if env.ledger().sequence() >= rotation.expires_at_ledger {
            return Err(ContractError::AdminRotationExpired);
        }
        rotation.pending_admin.require_auth();

        storage::write_admin(&env, &rotation.pending_admin);
        storage::remove_admin_rotation(&env);
        storage::extend_instance_ttl(&env);
        events::publish_admin_rotation_accepted(&env, &current_admin, &rotation.pending_admin);
        Ok(())
    }

    pub fn cancel_admin_rotation(env: Env) -> Result<(), ContractError> {
        let current_admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        current_admin.require_auth();
        let rotation =
            storage::read_admin_rotation(&env).ok_or(ContractError::AdminRotationNotFound)?;

        storage::remove_admin_rotation(&env);
        storage::extend_instance_ttl(&env);
        events::publish_admin_rotation_cancelled(&env, &current_admin, &rotation.pending_admin);
        Ok(())
    }

    pub fn get_admin(env: Env) -> Result<Address, ContractError> {
        storage::read_admin(&env).ok_or(ContractError::NotInitialized)
    }

    pub fn get_admin_rotation(env: Env) -> Option<AdminRotation> {
        storage::read_admin_rotation(&env)
    }

    pub fn get_configuration(env: Env) -> Result<ContractConfiguration, ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        Ok(ContractConfiguration {
            response_version: CONFIGURATION_RESPONSE_VERSION,
            interface_version: INTERFACE_VERSION,
            storage_layout_version: storage::read_storage_layout_version(&env)
                .unwrap_or(storage::STORAGE_LAYOUT_VERSION),
            admin,
            supported_assets: storage::read_supported_assets(&env),
            max_supported_assets: MAX_SUPPORTED_ASSETS,
            operational_state: storage::read_operational_state(&env),
            instance_ttl_threshold: storage::DEFAULT_TTL_THRESHOLD,
            instance_ttl_extension: storage::TTL_EXTENSION_AMOUNT,
        })
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
        let asset_cap = storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        if max_amount > asset_cap {
            return Err(ContractError::AmountExceedsCap);
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

    pub fn get_escrow_liability(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        Ok(storage::read_liability(&env, &asset))
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
    let contract_balance_before = asset.balance(&contract);
    let buyer_balance_before = asset.balance(&session.buyer);
    storage::decrease_liability(env, &session.asset, refunded_amount)?;
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
    storage::write_session(env, session);
    events::publish_session_recovered(env, session, refunded_amount);
    Ok(())
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
