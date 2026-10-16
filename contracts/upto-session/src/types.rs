use soroban_sdk::{contracttype, Address, BytesN, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetPolicy {
    pub asset: Address,
    pub max_session_amount: i128,
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Open,
    Settled,
    Cancelled,
    Expired,
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
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRecoveryResult {
    pub session_id: BytesN<32>,
    pub status: BatchRecoveryStatus,
    pub refunded_amount: i128,
}
