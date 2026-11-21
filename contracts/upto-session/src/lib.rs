#![no_std]
#![allow(clippy::too_many_arguments)]

mod errors;
pub mod events;
pub mod ids;
pub mod storage;
mod types;
pub mod validation;

pub use errors::ContractError;
pub use types::{
    AdminRotation, AssetExposureHeadroom, AssetLifecycle, AssetPolicy, AssetPolicyResponse,
    AssetSolvency, AssetSolvencySnapshot, AssetStatus, BatchRecoveryResult, BatchRecoveryStatus,
    BatchSessionResult, BatchSessionStatus, BuyerExposureHeadroom, BuyerSessionCount,
    CapIncreaseProposal, CommitmentDomainMetadata, CommitmentKind, ContractConfiguration,
    OperationalState, RecoverabilityStatus, Session, SessionRecoverability, SessionStateCounts,
    SessionStatus, SessionStatusResponse, SettlementPreview, SettlementRejectionReason,
    SolvencyAssetState, SolvencyStatus, TerminalEvidence, TerminalEvidenceDigest,
};

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env, Vec};

#[contract]
pub struct UptoSessionContract;

pub const INTERFACE_VERSION: u32 = 3;
pub const CONFIGURATION_RESPONSE_VERSION: u32 = 2;
pub const RECOVERABILITY_RESPONSE_VERSION: u32 = 1;
pub const COMMITMENT_DOMAIN_METADATA_RESPONSE_VERSION: u32 = 1;
pub const RECEIPT_SCHEMA_VERSION: u32 = 1;
pub const ASSET_POLICY_RESPONSE_VERSION: u32 = 1;
pub const MAX_SUPPORTED_ASSETS: u32 = 16;
pub const MAX_RECOVERY_BATCH_SIZE: u32 = 8;
pub const MAX_SOLVENCY_QUERY_ASSETS: u32 = 16;
pub const SOLVENCY_RESPONSE_VERSION: u32 = 1;
pub const MAX_SESSION_QUERY_BATCH_SIZE: u32 = 8;
pub const TERMINAL_EVIDENCE_VERSION: u32 = 1;
pub const BUYER_SESSION_COUNT_RESPONSE_VERSION: u32 = 1;

#[contractimpl]
#[allow(clippy::too_many_arguments)]
impl UptoSessionContract {
    pub fn interface_version() -> u32 {
        INTERFACE_VERSION
    }

    pub fn max_session_duration_ledgers() -> u32 {
        validation::MAX_SESSION_DURATION_LEDGERS
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
            validation::validate_asset_policy(
                policy.min_session_amount,
                policy.max_session_amount,
            )?;
            validation::validate_exposure_ceiling(policy.max_asset_exposure)?;
            validation::validate_exposure_ceiling(policy.max_buyer_exposure)?;
            if supported_assets.iter().any(|asset| asset == policy.asset) {
                return Err(ContractError::InvalidSupportedAssets);
            }
            supported_assets.push_back(policy.asset);
        }
        for policy in asset_policies.iter() {
            let probe = validation::validate_supported_asset(&env, &policy.asset, &admin)?;
            storage::write_supported_asset(&env, &policy.asset);
            storage::write_asset_max_session_amount(&env, &policy.asset, policy.max_session_amount);
            storage::write_asset_min_session_amount(&env, &policy.asset, policy.min_session_amount);
            storage::write_asset_exposure_ceiling(&env, &policy.asset, policy.max_asset_exposure);
            storage::write_buyer_exposure_ceiling(&env, &policy.asset, policy.max_buyer_exposure);
            events::publish_asset_activated(&env, &policy.asset, probe.decimals);
        }
        let configuration_digest =
            ids::initialization_configuration_digest(&env, &admin, &asset_policies);
        storage::write_supported_assets(&env, &supported_assets);
        storage::write_operational_state(&env, &OperationalState::Active);
        storage::write_admin(&env, &admin);
        storage::write_initialization_configuration_digest(&env, &configuration_digest);
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

        let min_session_amount = storage::read_asset_min_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        validation::validate_asset_policy(min_session_amount, max_session_amount)?;
        let previous_max_session_amount = storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;

        if max_session_amount > previous_max_session_amount {
            return Err(ContractError::CapIncreasePending);
        }

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

    pub fn propose_asset_cap_increase(
        env: Env,
        asset: Address,
        proposed_max_amount: i128,
        active_at_ledger: u32,
        expires_at_ledger: u32,
    ) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        admin.require_auth();
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        let current_cap = storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        if storage::read_cap_increase_proposal(&env, &asset).is_some() {
            return Err(ContractError::CapIncreasePending);
        }
        validation::validate_cap_increase_proposal(
            &env,
            current_cap,
            proposed_max_amount,
            active_at_ledger,
            expires_at_ledger,
        )?;

        let proposal = CapIncreaseProposal {
            proposed_max_amount,
            active_at_ledger,
            expires_at_ledger,
        };
        storage::write_cap_increase_proposal(&env, &asset, &proposal);
        storage::extend_instance_ttl(&env);
        events::publish_cap_increase_proposed(
            &env,
            &asset,
            &admin,
            proposed_max_amount,
            active_at_ledger,
            expires_at_ledger,
        );
        Ok(())
    }

    pub fn set_seller_exposure_ceiling(
        env: Env,
        asset: Address,
        ceiling: i128,
    ) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        validation::validate_asset_cap(ceiling)?;
        admin.require_auth();

        let previous_ceiling = storage::read_seller_exposure_ceiling(&env, &asset).unwrap_or(0);
        storage::write_seller_exposure_ceiling(&env, &asset, ceiling);
        storage::extend_instance_ttl(&env);
        events::publish_seller_exposure_ceiling_updated(
            &env,
            &asset,
            &admin,
            previous_ceiling,
            ceiling,
        );
        Ok(())
    }

    pub fn accept_asset_cap_increase(env: Env, asset: Address) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        admin.require_auth();
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        let previous_max_session_amount = storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        let proposal = storage::read_cap_increase_proposal(&env, &asset)
            .ok_or(ContractError::CapIncreaseNotFound)?;

        let current_ledger = env.ledger().sequence();
        if current_ledger < proposal.active_at_ledger {
            return Err(ContractError::CapIncreaseNotReady);
        }
        if current_ledger >= proposal.expires_at_ledger {
            return Err(ContractError::CapIncreaseExpired);
        }

        storage::write_asset_max_session_amount(&env, &asset, proposal.proposed_max_amount);
        storage::remove_cap_increase_proposal(&env, &asset);
        storage::extend_instance_ttl(&env);
        events::publish_asset_session_cap_updated(
            &env,
            &asset,
            &admin,
            previous_max_session_amount,
            proposal.proposed_max_amount,
        );
        Ok(())
    }

    pub fn cancel_asset_cap_increase(env: Env, asset: Address) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        admin.require_auth();
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        let proposal = storage::read_cap_increase_proposal(&env, &asset)
            .ok_or(ContractError::CapIncreaseNotFound)?;

        storage::remove_cap_increase_proposal(&env, &asset);
        storage::extend_instance_ttl(&env);
        events::publish_cap_increase_cancelled(&env, &asset, &admin, proposal.proposed_max_amount);
        Ok(())
    }

    pub fn get_asset_cap_increase(env: Env, asset: Address) -> Option<CapIncreaseProposal> {
        storage::read_cap_increase_proposal(&env, &asset)
    }

    pub fn get_seller_exposure_ceiling(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        storage::read_seller_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::SellerExposureCeilingNotConfigured)
    }

    pub fn get_seller_exposure(
        env: Env,
        seller: Address,
        asset: Address,
    ) -> Result<i128, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        Ok(storage::read_seller_exposure(&env, &seller, &asset))
    }

    pub fn set_max_buyer_open_sessions(env: Env, limit: u32) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        if limit == 0 {
            return Err(ContractError::InvalidSessionCountLimit);
        }
        admin.require_auth();

        let previous_limit = storage::read_max_buyer_open_sessions(&env).unwrap_or(0);
        storage::write_max_buyer_open_sessions(&env, limit);
        storage::extend_instance_ttl(&env);
        events::publish_max_buyer_open_sessions_updated(&env, &admin, previous_limit, limit);
        Ok(())
    }

    pub fn get_max_buyer_open_sessions(env: Env) -> Option<u32> {
        storage::read_max_buyer_open_sessions(&env)
    }

    pub fn get_buyer_open_sessions(env: Env, buyer: Address) -> Result<u32, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(storage::read_buyer_open_sessions(&env, &buyer))
    }

    pub fn get_buyer_session_count(
        env: Env,
        buyer: Address,
    ) -> Result<BuyerSessionCount, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(BuyerSessionCount {
            response_version: BUYER_SESSION_COUNT_RESPONSE_VERSION,
            open: storage::read_buyer_open_sessions(&env, &buyer),
            total: storage::read_buyer_total_sessions(&env, &buyer),
        })
    }

    pub fn set_max_seller_open_sessions(env: Env, limit: u32) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        if limit == 0 {
            return Err(ContractError::InvalidSessionCountLimit);
        }
        admin.require_auth();

        let previous_limit = storage::read_max_seller_open_sessions(&env).unwrap_or(0);
        storage::write_max_seller_open_sessions(&env, limit);
        storage::extend_instance_ttl(&env);
        events::publish_max_seller_open_sessions_updated(&env, &admin, previous_limit, limit);
        Ok(())
    }

    pub fn get_max_seller_open_sessions(env: Env) -> Option<u32> {
        storage::read_max_seller_open_sessions(&env)
    }

    pub fn get_seller_open_sessions(env: Env, seller: Address) -> Result<u32, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(storage::read_seller_open_sessions(&env, &seller))
    }

    // Activates one reviewed asset after initialization. Only the current
    // admin may call it. The asset must be new (or fully removed earlier),
    // pass the same SEP-41 probe as `initialize`, carry a positive cap, and
    // fit within `MAX_SUPPORTED_ASSETS`. A deprecated asset is never
    // reactivated here. Every check runs before any storage write.
    pub fn activate_asset(env: Env, policy: AssetPolicy) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        validation::validate_asset_cap(policy.max_session_amount)?;
        if policy.asset == env.current_contract_address() || policy.asset == admin {
            return Err(ContractError::InvalidAsset);
        }
        match storage::read_asset_status(&env, &policy.asset) {
            None => {}
            Some(AssetStatus::Active) => return Err(ContractError::AssetAlreadyActive),
            Some(AssetStatus::Deprecated) => return Err(ContractError::AssetDeprecated),
        }
        let mut configured = storage::read_configured_assets(&env);
        if configured.len() >= MAX_SUPPORTED_ASSETS {
            return Err(ContractError::AssetLimitReached);
        }
        let probe = validation::validate_supported_asset(&env, &policy.asset, &admin)?;

        storage::write_supported_asset(&env, &policy.asset);
        storage::write_asset_max_session_amount(&env, &policy.asset, policy.max_session_amount);
        configured.push_back(policy.asset.clone());
        storage::write_supported_assets(&env, &configured);
        storage::extend_instance_ttl(&env);
        events::publish_asset_activated(&env, &policy.asset, probe.decimals);
        Ok(())
    }

    // Authoritative, read-only lifecycle view of one asset. Needs no
    // authorization and never writes. Unknown and removed assets both report
    // `NotConfigured` with no cap.
    pub fn get_asset_policy(
        env: Env,
        asset: Address,
    ) -> Result<AssetPolicyResponse, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        let (lifecycle, max_session_amount) = match storage::read_asset_status(&env, &asset) {
            Some(AssetStatus::Active) => (
                AssetLifecycle::Active,
                Some(
                    storage::read_asset_max_session_amount(&env, &asset)
                        .ok_or(ContractError::AssetCapNotConfigured)?,
                ),
            ),
            Some(AssetStatus::Deprecated) => (AssetLifecycle::Deprecated, None),
            None => (AssetLifecycle::NotConfigured, None),
        };
        Ok(AssetPolicyResponse {
            response_version: ASSET_POLICY_RESPONSE_VERSION,
            asset,
            accepts_new_sessions: lifecycle == AssetLifecycle::Active,
            lifecycle,
            max_session_amount,
        })
    }

    pub fn get_asset_max_session_amount(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)
    }

    pub fn get_asset_min_session_amount(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        storage::read_asset_min_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)
    }

    /// Admin-only: sets the ceiling on aggregate open escrow for `asset`.
    /// Lowering it below the current liability is allowed; existing sessions
    /// are unaffected and new sessions are rejected until capacity returns.
    pub fn set_asset_exposure_ceiling(
        env: Env,
        asset: Address,
        ceiling: i128,
    ) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        validation::validate_exposure_ceiling(ceiling)?;
        admin.require_auth();

        let previous_ceiling = storage::read_asset_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)?;
        storage::write_asset_exposure_ceiling(&env, &asset, ceiling);
        storage::extend_instance_ttl(&env);
        events::publish_asset_exposure_ceiling_updated(
            &env,
            &asset,
            &admin,
            previous_ceiling,
            ceiling,
            storage::read_liability(&env, &asset),
        );
        Ok(())
    }

    pub fn get_asset_exposure_ceiling(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        storage::read_asset_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)
    }

    pub fn get_asset_exposure_headroom(
        env: Env,
        asset: Address,
    ) -> Result<AssetExposureHeadroom, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        let ceiling = storage::read_asset_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)?;
        let exposure = storage::read_liability(&env, &asset);
        let headroom = ceiling.saturating_sub(exposure);
        Ok(AssetExposureHeadroom {
            ceiling,
            exposure,
            headroom,
        })
    }

    /// Admin-only: sets the ceiling on open escrow any single buyer may hold
    /// for `asset`. Each buyer has an independent counter.
    pub fn set_buyer_exposure_ceiling(
        env: Env,
        asset: Address,
        ceiling: i128,
    ) -> Result<(), ContractError> {
        let admin = storage::read_admin(&env).ok_or(ContractError::NotInitialized)?;
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        validation::validate_exposure_ceiling(ceiling)?;
        admin.require_auth();

        let previous_ceiling = storage::read_buyer_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)?;
        storage::write_buyer_exposure_ceiling(&env, &asset, ceiling);
        storage::extend_instance_ttl(&env);
        events::publish_buyer_exposure_ceiling_updated(
            &env,
            &asset,
            &admin,
            previous_ceiling,
            ceiling,
        );
        Ok(())
    }

    pub fn get_buyer_exposure_ceiling(env: Env, asset: Address) -> Result<i128, ContractError> {
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        storage::read_buyer_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)
    }

    /// Open escrow currently held by `buyer` for `asset`.
    pub fn get_buyer_exposure(
        env: Env,
        buyer: Address,
        asset: Address,
    ) -> Result<i128, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        match storage::read_asset_status(&env, &asset) {
            Some(AssetStatus::Active) | Some(AssetStatus::Deprecated) => {
                Ok(storage::read_buyer_exposure(&env, &buyer, &asset))
            }
            None => Err(ContractError::UnsupportedAsset),
        }
    }

    pub fn get_buyer_exposure_headroom(
        env: Env,
        buyer: Address,
        asset: Address,
    ) -> Result<BuyerExposureHeadroom, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        let ceiling = storage::read_buyer_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)?;
        let exposure = storage::read_buyer_exposure(&env, &buyer, &asset);
        let headroom = ceiling.saturating_sub(exposure);
        Ok(BuyerExposureHeadroom {
            ceiling,
            exposure,
            headroom,
        })
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
            supported_assets: storage::read_configured_assets(&env),
            max_supported_assets: MAX_SUPPORTED_ASSETS,
            operational_state: storage::read_operational_state(&env),
            instance_ttl_threshold: storage::DEFAULT_TTL_THRESHOLD,
            instance_ttl_extension: storage::TTL_EXTENSION_AMOUNT,
            max_session_duration_ledgers: validation::MAX_SESSION_DURATION_LEDGERS,
        })
    }

    pub fn get_commitment_domain_metadata(env: Env) -> CommitmentDomainMetadata {
        CommitmentDomainMetadata {
            response_version: COMMITMENT_DOMAIN_METADATA_RESPONSE_VERSION,
            interface_version: INTERFACE_VERSION,
            receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            session_id_domain: ids::session_id_domain_hash(&env),
            usage_commitment_domain: ids::usage_commitment_domain_hash(&env),
        }
    }

    pub fn get_initialization_digest(env: Env) -> Result<BytesN<32>, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        storage::read_initialization_configuration_digest(&env)
            .ok_or(ContractError::NotInitialized)
    }

    pub fn get_session_state_counts(env: Env) -> Result<SessionStateCounts, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(storage::read_session_state_counts(&env))
    }

    pub fn get_seller_session_counts(
        env: Env,
        seller: Address,
    ) -> Result<SessionStateCounts, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(storage::read_seller_session_state_counts(&env, &seller))
    }

    pub fn get_sessions_by_commitment(
        env: Env,
        kind: CommitmentKind,
        commitment: BytesN<32>,
    ) -> Result<Vec<BytesN<32>>, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(storage::read_sessions_by_commitment(
            &env,
            &kind,
            &commitment,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        env: Env,
        buyer: Address,
        seller: Address,
        asset: Address,
        max_amount: i128,
        expires_at_ledger: u32,
        resource_hash: BytesN<32>,
        idempotency_key: BytesN<32>,
    ) -> Result<BytesN<32>, ContractError> {
        validation::validate_commitment_scheme(
            validation::SUPPORTED_RESOURCE_SCHEME_VERSION,
            validation::SUPPORTED_RESOURCE_SCHEME_VERSION,
        )?;
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
        if storage::read_operational_state(&env) != OperationalState::Active {
            return Err(ContractError::CreationPaused);
        }
        if !storage::is_supported_asset(&env, &asset) {
            return Err(ContractError::UnsupportedAsset);
        }
        let asset_cap = storage::read_asset_max_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        let asset_min = storage::read_asset_min_session_amount(&env, &asset)
            .ok_or(ContractError::AssetCapNotConfigured)?;
        if max_amount < asset_min {
            return Err(ContractError::InvalidAmount);
        }
        if max_amount > asset_cap {
            return Err(ContractError::AmountExceedsCap);
        }
        let asset_ceiling = storage::read_asset_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)?;
        let next_liability = storage::read_liability(&env, &asset)
            .checked_add(max_amount)
            .ok_or(ContractError::LiabilityOverflow)?;
        if next_liability > asset_ceiling {
            return Err(ContractError::AssetExposureCeilingExceeded);
        }
        let buyer_ceiling = storage::read_buyer_exposure_ceiling(&env, &asset)
            .ok_or(ContractError::ExposureCeilingNotConfigured)?;
        let next_buyer_exposure =
            storage::checked_buyer_exposure_increase(&env, &buyer, &asset, max_amount)?;
        if next_buyer_exposure > buyer_ceiling {
            return Err(ContractError::BuyerExposureCeilingExceeded);
        }

        if let Some(ceiling) = storage::read_seller_exposure_ceiling(&env, &asset) {
            let current_exposure = storage::read_seller_exposure(&env, &seller, &asset);
            let projected_exposure = current_exposure
                .checked_add(max_amount)
                .ok_or(ContractError::LiabilityOverflow)?;
            if projected_exposure > ceiling {
                return Err(ContractError::SellerExposureCeilingExceeded);
            }
        }

        if let Some(limit) = storage::read_max_buyer_open_sessions(&env) {
            let current_sessions = storage::read_buyer_open_sessions(&env, &buyer);
            if current_sessions >= limit {
                return Err(ContractError::BuyerOpenSessionsExceeded);
            }
        }

        if let Some(limit) = storage::read_max_seller_open_sessions(&env) {
            let current_sessions = storage::read_seller_open_sessions(&env, &seller);
            if current_sessions >= limit {
                return Err(ContractError::SellerOpenSessionsExceeded);
            }
        }

        storage::check_session_sequence_available(&env)?;

        if let Some(existing_session_id) =
            storage::read_idempotency_key(&env, &buyer, &idempotency_key)
        {
            let session = storage::read_session(&env, &existing_session_id)
                .ok_or(ContractError::SessionNotFound)?;
            if session.seller != seller
                || session.asset != asset
                || session.max_amount != max_amount
                || session.expires_at_ledger != expires_at_ledger
                || session.resource_hash != resource_hash
            {
                return Err(ContractError::IdempotencyConflict);
            }
            buyer.require_auth();
            return Ok(existing_session_id);
        }

        buyer.require_auth();
        let sequence = storage::take_next_session_sequence(&env)?;
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
                idempotency_key: &idempotency_key,
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
        storage::increase_seller_exposure(&env, &seller, &asset, max_amount)?;
        storage::increment_buyer_open_sessions(&env, &buyer)?;
        storage::increment_buyer_total_sessions(&env, &buyer)?;
        storage::increment_seller_open_sessions(&env, &seller)?;
        if escrow.balance(&contract) < liability {
            return Err(ContractError::EscrowUnderfunded);
        }
        // Re-derived after the transfer so a token callback cannot have raced
        // the counter past the ceiling.
        let buyer_exposure =
            storage::checked_buyer_exposure_increase(&env, &buyer, &asset, max_amount)?;
        if liability > asset_ceiling {
            return Err(ContractError::AssetExposureCeilingExceeded);
        }
        if buyer_exposure > buyer_ceiling {
            return Err(ContractError::BuyerExposureCeilingExceeded);
        }
        storage::write_buyer_exposure(&env, &buyer, &asset, buyer_exposure);

        let current_ledger = env.ledger().sequence();
        let session = Session {
            id: session_id.clone(),
            buyer: buyer.clone(),
            seller,
            asset,
            max_amount,
            escrowed_amount: max_amount,
            settled_amount: 0,
            expires_at_ledger,
            resource_hash,
            usage_hash: None,
            status: SessionStatus::Open,
            creation_ledger: current_ledger,
            finalized_ledger: None,
            resource_scheme_version: validation::SUPPORTED_RESOURCE_SCHEME_VERSION,
            usage_scheme_version: None,
        };

        storage::write_session(&env, &session)?;
        storage::index_session_commitment(
            &env,
            &CommitmentKind::Resource,
            &session.resource_hash,
            &session_id,
        )?;
        storage::write_idempotency_key(&env, &buyer, &idempotency_key, &session_id);
        events::publish_session_created(&env, &session);

        Ok(session_id)
    }

    pub fn deprecate_asset(
        env: Env,
        asset: Address,
        reason_commitment: BytesN<32>,
    ) -> Result<(), ContractError> {
        validation::validate_reason_commitment(&env, &reason_commitment)?;
        let admin = require_admin(&env)?;
        match storage::read_asset_status(&env, &asset) {
            Some(AssetStatus::Active) => {}
            Some(AssetStatus::Deprecated) | None => return Err(ContractError::AssetNotActive),
        }

        storage::write_asset_status(&env, &asset, &AssetStatus::Deprecated);
        storage::extend_instance_ttl(&env);
        events::publish_asset_deprecated(&env, &asset, &admin, &reason_commitment);
        Ok(())
    }

    pub fn remove_asset(
        env: Env,
        asset: Address,
        reason_commitment: BytesN<32>,
    ) -> Result<(), ContractError> {
        validation::validate_reason_commitment(&env, &reason_commitment)?;
        let admin = require_admin(&env)?;
        if storage::read_asset_status(&env, &asset) != Some(AssetStatus::Deprecated) {
            return Err(ContractError::AssetNotDeprecated);
        }
        if storage::read_liability(&env, &asset) != 0 {
            return Err(ContractError::AssetLiabilityOutstanding);
        }

        // Delete every policy record so nothing about the asset can be read as
        // active configuration; session history is untouched.
        storage::remove_from_supported_assets(&env, &asset);
        storage::remove_asset_status(&env, &asset);
        storage::remove_asset_max_session_amount(&env, &asset);
        storage::remove_liability(&env, &asset);
        storage::extend_instance_ttl(&env);
        events::publish_asset_removed(&env, &asset, &admin, &reason_commitment);
        Ok(())
    }

    pub fn pause_creation(env: Env) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        match storage::read_operational_state(&env) {
            OperationalState::Active => {}
            OperationalState::Paused | OperationalState::Deprecated => {
                return Err(ContractError::OperationalStateUnchanged);
            }
        }
        storage::write_operational_state(&env, &OperationalState::Paused);
        storage::extend_instance_ttl(&env);
        events::publish_creation_paused(&env, &admin);
        Ok(())
    }

    pub fn resume_creation(env: Env) -> Result<(), ContractError> {
        let admin = require_admin(&env)?;
        match storage::read_operational_state(&env) {
            OperationalState::Paused => {}
            OperationalState::Active | OperationalState::Deprecated => {
                return Err(ContractError::OperationalStateUnchanged);
            }
        }
        storage::write_operational_state(&env, &OperationalState::Active);
        storage::extend_instance_ttl(&env);
        events::publish_creation_resumed(&env, &admin);
        Ok(())
    }

    pub fn recover_surplus(
        env: Env,
        asset: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        let _admin = require_admin(&env)?;
        if storage::read_asset_status(&env, &asset).is_none() {
            return Err(ContractError::UnsupportedAsset);
        }
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let escrow = token::Client::new(&env, &asset);
        let contract = env.current_contract_address();
        let balance_before = escrow.balance(&contract);
        let liability = storage::read_liability(&env, &asset);
        if balance_before < liability {
            return Err(ContractError::EscrowUnderfunded);
        }
        let surplus = balance_before - liability;
        if amount > surplus {
            return Err(ContractError::EscrowUnderfunded);
        }

        let recipient_balance_before = escrow.balance(&recipient);
        escrow.transfer(&contract, &recipient, &amount);
        verify_debit(balance_before, escrow.balance(&contract), amount)?;
        verify_credit(recipient_balance_before, escrow.balance(&recipient), amount)?;
        if escrow.balance(&contract) < liability {
            return Err(ContractError::EscrowUnderfunded);
        }

        events::publish_surplus_recovered(&env, &asset, &recipient, amount);
        storage::extend_instance_ttl(&env);
        Ok(())
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
            SessionStatus::Rejected => return Err(ContractError::SessionRejected),
        }

        validation::validate_commitment_scheme(
            validation::SUPPORTED_USAGE_SCHEME_VERSION,
            validation::SUPPORTED_USAGE_SCHEME_VERSION,
        )?;
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
        let buyer_exposure = storage::checked_buyer_exposure_decrease(
            &env,
            &session.buyer,
            &session.asset,
            escrowed_amount,
        )?;
        let contract_balance_before = asset.balance(&contract);
        let seller_balance_before = asset.balance(&session.seller);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, escrowed_amount)?;
        storage::decrease_seller_exposure(&env, &session.seller, &session.asset, escrowed_amount)?;
        storage::decrement_buyer_open_sessions(&env, &session.buyer)?;
        storage::decrement_seller_open_sessions(&env, &session.seller)?;
        storage::write_buyer_exposure(&env, &session.buyer, &session.asset, buyer_exposure);
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
        session.finalized_ledger = Some(env.ledger().sequence());
        session.usage_scheme_version = Some(validation::SUPPORTED_USAGE_SCHEME_VERSION);
        storage::write_session(&env, &session)?;
        storage::index_session_commitment(
            &env,
            &CommitmentKind::Usage,
            &usage_hash,
            &session_id,
        )?;
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

    pub fn preview_settle(
        env: Env,
        session_id: BytesN<32>,
        actual_amount: i128,
    ) -> SettlementPreview {
        let session = match storage::read_session(&env, &session_id) {
            Some(s) => s,
            None => {
                return SettlementPreview {
                    eligible: false,
                    actual_amount,
                    seller_payout: 0,
                    buyer_refund: 0,
                    is_expired: false,
                    rejection_reason: SettlementRejectionReason::NotFound,
                };
            }
        };

        let is_expired = env.ledger().sequence() >= session.expires_at_ledger;

        let rejection_reason = match &session.status {
            SessionStatus::Settled => SettlementRejectionReason::AlreadySettled,
            SessionStatus::Cancelled => SettlementRejectionReason::AlreadyCancelled,
            SessionStatus::Expired => SettlementRejectionReason::AlreadyExpired,
            SessionStatus::Rejected => SettlementRejectionReason::AlreadyRejected,
            SessionStatus::Open => {
                if actual_amount <= 0 {
                    SettlementRejectionReason::InvalidAmount
                } else if actual_amount > session.max_amount {
                    SettlementRejectionReason::AmountExceedsCap
                } else if is_expired {
                    SettlementRejectionReason::Expired
                } else {
                    let contract = env.current_contract_address();
                    let asset = token::Client::new(&env, &session.asset);
                    if session.escrowed_amount != session.max_amount
                        || asset.balance(&contract) < storage::read_liability(&env, &session.asset)
                    {
                        SettlementRejectionReason::EscrowUnderfunded
                    } else {
                        SettlementRejectionReason::None
                    }
                }
            }
        };

        let eligible = rejection_reason == SettlementRejectionReason::None;
        let (seller_payout, buyer_refund) = if eligible {
            (actual_amount, session.escrowed_amount - actual_amount)
        } else {
            (0, 0)
        };

        SettlementPreview {
            eligible,
            actual_amount,
            seller_payout,
            buyer_refund,
            is_expired,
            rejection_reason,
        }
    }

    pub fn cancel(env: Env, session_id: BytesN<32>) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
            SessionStatus::Rejected => return Err(ContractError::SessionRejected),
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
        let buyer_exposure = storage::checked_buyer_exposure_decrease(
            &env,
            &session.buyer,
            &session.asset,
            refunded_amount,
        )?;
        let contract_balance_before = asset.balance(&contract);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, refunded_amount)?;
        storage::decrease_seller_exposure(&env, &session.seller, &session.asset, refunded_amount)?;
        storage::decrement_buyer_open_sessions(&env, &session.buyer)?;
        storage::decrement_seller_open_sessions(&env, &session.seller)?;
        storage::write_buyer_exposure(&env, &session.buyer, &session.asset, buyer_exposure);
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
        session.finalized_ledger = Some(env.ledger().sequence());
        storage::write_session(&env, &session)?;
        events::publish_session_cancelled(
            &env,
            &session_id,
            &session.buyer,
            &session.asset,
            refunded_amount,
        );

        Ok(())
    }

    pub fn reduce_session_cap(
        env: Env,
        session_id: BytesN<32>,
        new_max_amount: i128,
    ) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
            SessionStatus::Rejected => return Err(ContractError::SessionRejected),
        }

        if env.ledger().sequence() >= session.expires_at_ledger {
            return Err(ContractError::ExpiredSession);
        }
        if new_max_amount <= 0 || new_max_amount >= session.max_amount {
            return Err(ContractError::InvalidCapReduction);
        }

        session.buyer.require_auth();

        let previous_max_amount = session.max_amount;
        let refund_difference = previous_max_amount - new_max_amount;
        let asset = token::Client::new(&env, &session.asset);
        let contract = env.current_contract_address();

        if session.escrowed_amount != previous_max_amount
            || asset.balance(&contract) < storage::read_liability(&env, &session.asset)
        {
            return Err(ContractError::EscrowUnderfunded);
        }
        let contract_balance_before = asset.balance(&contract);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, refund_difference)?;
        storage::decrease_seller_exposure(
            &env,
            &session.seller,
            &session.asset,
            refund_difference,
        )?;
        asset.transfer(&contract, &session.buyer, &refund_difference);

        verify_debit(
            contract_balance_before,
            asset.balance(&contract),
            refund_difference,
        )?;
        verify_credit(
            buyer_balance_before,
            asset.balance(&session.buyer),
            refund_difference,
        )?;

        session.max_amount = new_max_amount;
        session.escrowed_amount = new_max_amount;
        storage::write_session(&env, &session)?;

        events::publish_session_cap_reduced(
            &env,
            &session_id,
            &session.buyer,
            &session.asset,
            previous_max_amount,
            new_max_amount,
            refund_difference,
        );

        Ok(())
    }

    pub fn reject(env: Env, session_id: BytesN<32>) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
            SessionStatus::Rejected => return Err(ContractError::SessionRejected),
        }
        if env.ledger().sequence() >= session.expires_at_ledger {
            return Err(ContractError::ExpiredSession);
        }

        session.seller.require_auth();
        let refunded_amount = session.escrowed_amount;
        let asset = token::Client::new(&env, &session.asset);
        let contract = env.current_contract_address();
        if refunded_amount != session.max_amount
            || asset.balance(&contract) < storage::read_liability(&env, &session.asset)
        {
            return Err(ContractError::EscrowUnderfunded);
        }

        let buyer_exposure = storage::checked_buyer_exposure_decrease(
            &env,
            &session.buyer,
            &session.asset,
            refunded_amount,
        )?;
        let contract_balance_before = asset.balance(&contract);
        let buyer_balance_before = asset.balance(&session.buyer);
        storage::decrease_liability(&env, &session.asset, refunded_amount)?;
        storage::decrease_seller_exposure(&env, &session.seller, &session.asset, refunded_amount)?;
        storage::decrement_buyer_open_sessions(&env, &session.buyer)?;
        storage::decrement_seller_open_sessions(&env, &session.seller)?;
        storage::write_buyer_exposure(&env, &session.buyer, &session.asset, buyer_exposure);
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
        session.status = SessionStatus::Rejected;
        session.finalized_ledger = Some(env.ledger().sequence());
        storage::write_session(&env, &session)?;
        events::publish_session_rejected(
            &env,
            &session_id,
            &session.seller,
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
            SessionStatus::Rejected => return Err(ContractError::SessionRejected),
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
                SessionStatus::Rejected => BatchRecoveryStatus::AlreadyRejected,
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
                            ContractError::EscrowUnderfunded
                            | ContractError::LiabilityUnderflow
                            | ContractError::BuyerExposureUnderflow,
                        ) => BatchRecoveryStatus::EscrowUnavailable,
                        Err(error) => return Err(error),
                    }
                }
            };
            results.push_back(batch_result(session_id, status, 0));
        }

        Ok(results)
    }

    pub fn get_recoverability(env: Env, session_id: BytesN<32>) -> SessionRecoverability {
        let current_ledger = env.ledger().sequence();
        let session = match storage::read_session(&env, &session_id) {
            Some(session) => session,
            None => {
                return SessionRecoverability {
                    response_version: RECOVERABILITY_RESPONSE_VERSION,
                    status: RecoverabilityStatus::NotFound,
                    expires_at_ledger: 0,
                    current_ledger,
                    recipient: None,
                    asset: None,
                    refundable_amount: 0,
                };
            }
        };

        let status = match session.status {
            SessionStatus::Settled => RecoverabilityStatus::AlreadySettled,
            SessionStatus::Cancelled => RecoverabilityStatus::AlreadyCancelled,
            SessionStatus::Expired => RecoverabilityStatus::AlreadyRecovered,
            SessionStatus::Rejected => RecoverabilityStatus::AlreadyRejected,
            SessionStatus::Open if current_ledger < session.expires_at_ledger => {
                RecoverabilityStatus::NotExpired
            }
            SessionStatus::Open => RecoverabilityStatus::Recoverable,
        };
        let refundable_amount = if status == RecoverabilityStatus::Recoverable {
            session.escrowed_amount
        } else {
            0
        };

        SessionRecoverability {
            response_version: RECOVERABILITY_RESPONSE_VERSION,
            status,
            expires_at_ledger: session.expires_at_ledger,
            current_ledger,
            recipient: Some(session.buyer),
            asset: Some(session.asset),
            refundable_amount,
        }
    }

    /// Read-only solvency snapshot for a caller-supplied list of one to
    /// `MAX_SOLVENCY_QUERY_ASSETS` assets. Results preserve input order.
    /// Duplicates are evaluated independently and returned once per
    /// occurrence. Assets unknown to this deployment are reported as
    /// `Unknown` with zero values and are never called, so arbitrary
    /// contracts cannot be invoked. `surplus` is `balance - liability`
    /// (negative when underfunded).
    pub fn get_solvency(
        env: Env,
        assets: Vec<Address>,
    ) -> Result<Vec<AssetSolvency>, ContractError> {
        if assets.is_empty() || assets.len() > MAX_SOLVENCY_QUERY_ASSETS {
            return Err(ContractError::InvalidBatchSize);
        }
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }

        let contract = env.current_contract_address();
        let mut results = Vec::new(&env);
        for asset in assets.iter() {
            let state = match storage::read_asset_status(&env, &asset) {
                Some(AssetStatus::Active) => SolvencyAssetState::Active,
                Some(AssetStatus::Deprecated) => SolvencyAssetState::Deprecated,
                None => {
                    results.push_back(AssetSolvency {
                        asset,
                        state: SolvencyAssetState::Unknown,
                        balance: 0,
                        liability: 0,
                        surplus: 0,
                    });
                    continue;
                }
            };
            let balance = token::Client::new(&env, &asset).balance(&contract);
            let liability = storage::read_liability(&env, &asset);
            results.push_back(AssetSolvency {
                asset,
                state,
                balance,
                liability,
                surplus: balance.saturating_sub(liability),
            });
        }
        Ok(results)
    }

    pub fn get_session(env: Env, session_id: BytesN<32>) -> Result<Session, ContractError> {
        storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)
    }

    pub fn get_terminal_evidence(
        env: Env,
        session_id: BytesN<32>,
    ) -> Result<TerminalEvidence, ContractError> {
        let session = storage::read_session(&env, &session_id)
            .ok_or(ContractError::SessionNotFound)?;
        if session.status == SessionStatus::Open {
            return Ok(TerminalEvidence::Nonterminal);
        }
        let finalized_ledger = session.finalized_ledger
            .ok_or(ContractError::InvalidStateTransition)?;
        let digest = ids::terminal_evidence_digest(
            &env,
            &session.id,
            &session.status,
            finalized_ledger,
            &session.usage_hash,
        );
        Ok(TerminalEvidence::Terminal(TerminalEvidenceDigest {
            evidence_version: TERMINAL_EVIDENCE_VERSION,
            status: session.status,
            finalized_ledger,
            usage_hash: session.usage_hash,
            digest,
        }))
    }

    pub fn get_session_batch(
        env: Env,
        session_ids: Vec<BytesN<32>>,
    ) -> Result<Vec<BatchSessionResult>, ContractError> {
        if session_ids.is_empty() || session_ids.len() > MAX_SESSION_QUERY_BATCH_SIZE {
            return Err(ContractError::InvalidBatchSize);
        }

        let mut results = Vec::new(&env);
        for session_id in session_ids.iter() {
            let status = match storage::read_session(&env, &session_id) {
                Some(session) => BatchSessionStatus::Found(session),
                None => BatchSessionStatus::NotFound,
            };
            results.push_back(BatchSessionResult { session_id, status });
        }

        Ok(results)
    }

    pub fn get_session_status(
        env: Env,
        session_id: BytesN<32>,
    ) -> Result<SessionStatusResponse, ContractError> {
        let session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;
        Ok(SessionStatusResponse {
            exists: true,
            status: session.status,
            expires_at_ledger: session.expires_at_ledger,
        })
    }

    pub fn extend_session_expiry(
        env: Env,
        session_id: BytesN<32>,
        new_expires_at_ledger: u32,
    ) -> Result<(), ContractError> {
        let mut session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match &session.status {
            SessionStatus::Open => {}
            SessionStatus::Settled => return Err(ContractError::SessionAlreadySettled),
            SessionStatus::Cancelled => return Err(ContractError::SessionCancelled),
            SessionStatus::Expired => return Err(ContractError::SessionExpired),
            SessionStatus::Rejected => return Err(ContractError::SessionRejected),
        }

        validation::validate_expiry_extension(&env, &session, new_expires_at_ledger)?;

        session.buyer.require_auth();
        session.seller.require_auth();

        let previous_expires_at_ledger = session.expires_at_ledger;
        session.expires_at_ledger = new_expires_at_ledger;
        storage::write_session(&env, &session)?;

        events::publish_session_expiry_extended(
            &env,
            &session_id,
            &session.buyer,
            &session.seller,
            previous_expires_at_ledger,
            new_expires_at_ledger,
        );

        Ok(())
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

    /// Read-only, authorization-free solvency view for an active or
    /// deprecated asset. It never writes state or extends TTLs.
    pub fn get_asset_solvency(
        env: Env,
        asset: Address,
    ) -> Result<AssetSolvencySnapshot, ContractError> {
        if !storage::has_admin(&env) {
            return Err(ContractError::NotInitialized);
        }
        let asset_status =
            storage::read_asset_status(&env, &asset).ok_or(ContractError::UnsupportedAsset)?;
        let liability = storage::read_liability(&env, &asset);
        let token_balance = token::Client::new(&env, &asset)
            .try_balance(&env.current_contract_address())
            .ok()
            .and_then(|balance| balance.ok());
        let (surplus, deficit, status) = solvency_of(token_balance, liability);
        Ok(AssetSolvencySnapshot {
            response_version: SOLVENCY_RESPONSE_VERSION,
            asset,
            asset_status,
            token_balance,
            liability,
            surplus,
            deficit,
            status,
        })
    }

    pub fn extend_ttl(env: Env, session_id: BytesN<32>) -> Result<(), ContractError> {
        let session =
            storage::read_session(&env, &session_id).ok_or(ContractError::SessionNotFound)?;

        match session.status {
            SessionStatus::Open => {
                storage::extend_session_and_liability_ttl(&env, &session);
                Ok(())
            }
            SessionStatus::Settled
            | SessionStatus::Cancelled
            | SessionStatus::Expired
            | SessionStatus::Rejected => Err(ContractError::TtlExtensionFailed),
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
    let buyer_exposure = storage::checked_buyer_exposure_decrease(
        env,
        &session.buyer,
        &session.asset,
        refunded_amount,
    )?;
    let contract_balance_before = asset.balance(&contract);
    let buyer_balance_before = asset.balance(&session.buyer);
    storage::decrease_liability(env, &session.asset, refunded_amount)?;
    storage::decrease_seller_exposure(env, &session.seller, &session.asset, refunded_amount)?;
    storage::decrement_buyer_open_sessions(env, &session.buyer)?;
    storage::decrement_seller_open_sessions(env, &session.seller)?;
    storage::write_buyer_exposure(env, &session.buyer, &session.asset, buyer_exposure);
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
    session.finalized_ledger = Some(env.ledger().sequence());
    storage::write_session(env, session)?;
    events::publish_session_recovered(env, session, refunded_amount);
    Ok(())
}

/// Classifies a balance against a liability with checked arithmetic. A
/// missing, negative, or unrepresentable balance is `BalanceUnavailable`, and
/// a shortfall is never reported as surplus.
fn solvency_of(token_balance: Option<i128>, liability: i128) -> (i128, i128, SolvencyStatus) {
    let Some(balance) = token_balance.filter(|balance| *balance >= 0) else {
        return (0, 0, SolvencyStatus::BalanceUnavailable);
    };
    if liability < 0 {
        return (0, 0, SolvencyStatus::BalanceUnavailable);
    }
    match balance.checked_sub(liability) {
        Some(0) => (0, 0, SolvencyStatus::Collateralized),
        Some(surplus) if surplus > 0 => (surplus, 0, SolvencyStatus::Surplus),
        Some(shortfall) => match shortfall.checked_neg() {
            Some(deficit) => (0, deficit, SolvencyStatus::Underfunded),
            None => (0, 0, SolvencyStatus::BalanceUnavailable),
        },
        None => (0, 0, SolvencyStatus::BalanceUnavailable),
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

fn require_admin(env: &Env) -> Result<Address, ContractError> {
    let admin = storage::read_admin(env).ok_or(ContractError::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}

#[cfg(test)]
mod encoding_vectors_test;
#[cfg(test)]
mod exposure_test;
#[cfg(test)]
mod receipt_test;
#[cfg(test)]
mod test;
