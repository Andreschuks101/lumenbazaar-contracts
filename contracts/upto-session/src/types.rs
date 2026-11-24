use soroban_sdk::{contracttype, Address, BytesN, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetPolicy {
    pub asset: Address,
    pub max_session_amount: i128,
    pub min_session_amount: i128,
    /// Ceiling on the sum of open escrow for this asset across all sessions.
    pub max_asset_exposure: i128,
    /// Ceiling on the sum of open escrow for this asset held by any one buyer.
    pub max_buyer_exposure: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotation {
    pub pending_admin: Address,
    pub expires_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetStatus {
    Active,
    Deprecated,
}

// Lifecycle of an asset as reported by `get_asset_policy`. `NotConfigured`
// covers both addresses that were never configured and assets that were
// removed; removal deletes every policy record, so the two are identical.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetLifecycle {
    Active,
    Deprecated,
    NotConfigured,
}

// Read-only, versioned view of one asset's policy.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetPolicyResponse {
    pub response_version: u32,
    pub asset: Address,
    pub lifecycle: AssetLifecycle,
    // The cap applied to new sessions; `None` unless the asset is active.
    pub max_session_amount: Option<i128>,
    pub accepts_new_sessions: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Open,
    Settled,
    Cancelled,
    Expired,
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitmentKind {
    Resource,
    Usage,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStateCounts {
    pub response_version: u32,
    pub open: u64,
    pub settled: u64,
    pub cancelled: u64,
    pub expired: u64,
    pub rejected: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuyerSessionCount {
    pub response_version: u32,
    pub open: u32,
    pub total: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapIncreaseStatus {
    Missing,
    Waiting,
    Ready,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapIncreaseStatusResponse {
    pub status: CapIncreaseStatus,
    pub proposed_max_amount: Option<i128>,
    pub active_at_ledger: Option<u32>,
    pub expires_at_ledger: Option<u32>,
    pub is_ready: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapIncreaseProposal {
    pub proposed_max_amount: i128,
    pub active_at_ledger: u32,
    pub expires_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub id: BytesN<32>,
    pub buyer: Address,
    pub seller: Address,
    pub asset: Address,
    pub max_amount: i128,
    pub escrowed_amount: i128,
    pub settled_amount: i128,
    pub expires_at_ledger: u32,
    pub resource_hash: BytesN<32>,
    pub usage_hash: Option<BytesN<32>>,
    pub status: SessionStatus,
    pub creation_ledger: u32,
    pub finalized_ledger: Option<u32>,
    pub resource_scheme_version: u32,
    pub usage_scheme_version: Option<u32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvidence {
    Nonterminal,
    Terminal(TerminalEvidenceDigest),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalEvidenceDigest {
    pub evidence_version: u32,
    pub status: SessionStatus,
    pub finalized_ledger: u32,
    pub usage_hash: Option<BytesN<32>>,
    pub digest: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationalState {
    Active,
    Paused,
    Deprecated,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchRecoveryStatus {
    Recovered,
    NotFound,
    NotExpired,
    AlreadySettled,
    AlreadyCancelled,
    AlreadyRecovered,
    AlreadyRejected,
    Duplicate,
    EscrowUnavailable,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractConfiguration {
    pub response_version: u32,
    pub interface_version: u32,
    pub storage_layout_version: u32,
    pub admin: Address,
    pub supported_assets: Vec<Address>,
    pub max_supported_assets: u32,
    pub operational_state: OperationalState,
    pub instance_ttl_threshold: u32,
    pub instance_ttl_extension: u32,
    pub max_session_duration_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SolvencyStatus {
    /// Token balance equals tracked liability.
    Collateralized,
    /// Token balance exceeds tracked liability; `surplus` is positive.
    Surplus,
    /// Token balance is below tracked liability; `deficit` is positive.
    Underfunded,
    /// The token balance call failed or returned a value that cannot be a
    /// balance. Never healthy.
    BalanceUnavailable,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetSolvencySnapshot {
    pub response_version: u32,
    pub asset: Address,
    pub asset_status: AssetStatus,
    /// Balance reported by the token for this contract, or `None` when the
    /// balance call failed.
    pub token_balance: Option<i128>,
    pub liability: i128,
    /// `token_balance - liability` when positive, otherwise `0`.
    pub surplus: i128,
    /// `liability - token_balance` when positive, otherwise `0`.
    pub deficit: i128,
    pub status: SolvencyStatus,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRecoveryResult {
    pub session_id: BytesN<32>,
    pub status: BatchRecoveryStatus,
    pub refunded_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettlementRejectionReason {
    None,
    NotFound,
    AlreadySettled,
    AlreadyCancelled,
    AlreadyExpired,
    AlreadyRejected,
    InvalidAmount,
    AmountExceedsCap,
    Expired,
    EscrowUnderfunded,
    UnsupportedScheme,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementPreview {
    pub eligible: bool,
    pub actual_amount: i128,
    pub seller_payout: i128,
    pub buyer_refund: i128,
    pub is_expired: bool,
    pub rejection_reason: SettlementRejectionReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CancellationRejectionReason {
    None,
    NotFound,
    AlreadySettled,
    AlreadyCancelled,
    AlreadyExpired,
    AlreadyRejected,
    Expired,
    EscrowUnderfunded,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancellationPreview {
    pub eligible: bool,
    pub refundable_amount: i128,
    pub is_expired: bool,
    pub rejection_reason: CancellationRejectionReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoverabilityStatus {
    NotFound,
    NotExpired,
    Recoverable,
    AlreadySettled,
    AlreadyCancelled,
    AlreadyRecovered,
    AlreadyRejected,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecoverability {
    pub response_version: u32,
    pub status: RecoverabilityStatus,
    pub expires_at_ledger: u32,
    pub current_ledger: u32,
    pub recipient: Option<Address>,
    pub asset: Option<Address>,
    pub refundable_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentDomainMetadata {
    pub response_version: u32,
    pub interface_version: u32,
    pub receipt_schema_version: u32,
    pub session_id_domain: BytesN<32>,
    pub usage_commitment_domain: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SolvencyAssetState {
    Unknown,
    Active,
    Deprecated,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetSolvency {
    pub asset: Address,
    pub state: SolvencyAssetState,
    pub balance: i128,
    pub liability: i128,
    pub surplus: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStatusResponse {
    pub exists: bool,
    pub status: SessionStatus,
    pub expires_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum BatchSessionStatus {
    Found(Session),
    NotFound,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuditIndexCategory {
    ResourceCommitment,
    UsageCommitment,
    SellerStateCount,
    BuyerOpenSessions,
    SellerOpenSessions,
    SessionNotFound,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionIndexMismatch {
    pub session_id: BytesN<32>,
    pub category: AuditIndexCategory,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionIndexAuditResult {
    pub audited_count: u32,
    pub mismatches: Vec<SessionIndexMismatch>,
    pub is_consistent: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchSessionResult {
    pub session_id: BytesN<32>,
    pub status: BatchSessionStatus,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetExposureHeadroom {
    pub ceiling: i128,
    pub exposure: i128,
    pub headroom: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuyerExposureHeadroom {
    pub ceiling: i128,
    pub exposure: i128,
    pub headroom: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SellerExposureHeadroom {
    pub ceiling: Option<i128>,
    pub exposure: i128,
    pub headroom: Option<i128>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminRotationStatus {
    NoProposal,
    Active {
        pending_admin: Address,
        expires_at_ledger: u32,
    },
    Expired {
        pending_admin: Address,
        expires_at_ledger: u32,
    },
}




