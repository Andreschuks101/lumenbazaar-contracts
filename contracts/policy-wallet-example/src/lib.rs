#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, xdr::ToXdr, Address,
    Bytes, BytesN, Env, Vec,
};

pub const LEDGERS_PER_DAY: u32 = 17_280;
pub const MAX_ALLOWED_SELLERS: u32 = 16;
pub const MAX_ALLOWED_ASSETS: u32 = 16;
pub const MAX_ALLOWED_RESOURCE_HASHES: u32 = 16;
pub const MAX_STORED_AGENT_POLICIES: u32 = 32;
pub const MAX_POLICY_PAGE_SIZE: u32 = 10;

pub const POLICY_CONFIGURATION_DOMAIN: &[u8] = b"lumenbazaar/policy-configuration";
pub const POLICY_CONFIGURATION_VERSION: u32 = 1;
pub const AUTHORIZATION_NONCE_TTL_LEDGERS: u32 = 17_280 * 30; // 30 days retention

// TTL management constants for instance storage keys
pub const DEFAULT_TTL_THRESHOLD: u32 = 100_000;
pub const TTL_EXTENSION_AMOUNT: u32 = 1_000_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Config,
    Policy(Address),
    DailySpend(Address, u32),
    DailySellerSpend(Address, Address, u32),
    DailyResourceSpend(Address, BytesN<32>, u32),
    DailyAuthCount(Address, u32),
    /// Ledger at which the owner revoked the agent's policy.
    RevokedPolicy(Address),
    AgentPolicyIndex(Address),
    OwnerRotation,
    AuthorizationNonce(Address, BytesN<32>),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyDenialReason {
    None,
    PolicyNotFound,
    PolicyRevoked,
    PolicyNotActive,
    PolicyExpired,
    InvalidAmount,
    AmountExceedsPaymentCap,
    SellerNotAllowed,
    AssetNotAllowed,
    ResourceHashNotAllowed,
    AuthorizationLimitExceeded,
    AmountExceedsDailyCap,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyPreviewResult {
    pub allowed: bool,
    pub denial_reason: PolicyDenialReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SellerSpendingLimit {
    pub seller: Address,
    pub max_amount_per_payment: i128,
    pub max_amount_per_day: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceSpendingLimit {
    pub resource_hash: BytesN<32>,
    pub max_amount_per_payment: i128,
    pub max_amount_per_day: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerRotation {
    pub pending_owner: Address,
    pub expires_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WalletConfig {
    pub owner: Address,
    pub upto_session_contract: Address,
    pub initialized_at_ledger: u32,
    pub example_only: bool,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PolicyStatus {
    Pending,
    Active,
    Expired,
    Revoked,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyActiveWindowStatus {
    pub status: PolicyStatus,
    pub current_ledger: u32,
    pub valid_from_ledger: u32,
    pub valid_until_ledger: u32,
    pub next_boundary_ledger: Option<u32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpendingPolicy {
    pub agent: Address,
    pub max_amount_per_payment: i128,
    pub max_amount_per_day: i128,
    pub max_authorizations_per_day: u32,
    pub valid_from_ledger: u32,
    pub valid_until_ledger: u32,
    pub allowed_sellers: Vec<Address>,
    pub allowed_assets: Vec<Address>,
    pub allowed_resource_hashes: Vec<BytesN<32>>,
    pub seller_limits: Vec<SellerSpendingLimit>,
    pub resource_limits: Vec<ResourceSpendingLimit>,
    pub revision: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyPage {
    pub policy_ids: Vec<BytesN<32>>,
    pub next_cursor: Option<u32>,
    pub total_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRecord {
    pub request_hash: BytesN<32>,
    pub authorized_at_ledger: u32,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentAuthorizationOutcome {
    pub replayed: bool,
    pub authorized_at_ledger: u32,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PolicyWalletError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidPolicy = 3,
    PolicyNotFound = 4,
    PolicyExpired = 5,
    InvalidAmount = 6,
    AmountExceedsPaymentCap = 7,
    AmountExceedsDailyCap = 8,
    SellerNotAllowed = 9,
    AssetNotAllowed = 10,
    ResourceHashNotAllowed = 11,
    PolicyNotActive = 12,
    AuthorizationLimitExceeded = 13,
    PolicyRevoked = 14,
    InvalidOwner = 15,
    InvalidOwnerRotationExpiry = 16,
    OwnerRotationPending = 17,
    OwnerRotationNotFound = 18,
    OwnerRotationExpired = 19,
    StaleRevision = 20,
    RevisionOverflow = 21,
    ReplayConflict = 22,
    InvalidPageLimit = 23,
    PolicyIndexFull = 24,
}

/// Emitted once when the owner revokes an agent's policy.
#[contractevent]
pub struct AgentPolicyRevoked {
    #[topic]
    pub agent: Address,
    pub owner: Address,
    pub revoked_at_ledger: u32,
}

#[contract]
pub struct PolicyWalletExampleContract;

#[contractimpl]
impl PolicyWalletExampleContract {
    pub fn initialize(
        env: Env,
        owner: Address,
        upto_session_contract: Address,
    ) -> Result<(), PolicyWalletError> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(PolicyWalletError::AlreadyInitialized);
        }

        owner.require_auth();

        let config = WalletConfig {
            owner,
            upto_session_contract,
            initialized_at_ledger: env.ledger().sequence(),
            example_only: true,
        };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage()
            .instance()
            .extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);

        Ok(())
    }

    pub fn get_config(env: Env) -> Result<WalletConfig, PolicyWalletError> {
        read_config(&env)
    }

    pub fn require_owner(env: Env) -> Result<(), PolicyWalletError> {
        require_owner_auth(&env)?;
        Ok(())
    }

    pub fn set_policy(
        env: Env,
        mut policy: SpendingPolicy,
        expected_revision: u32,
    ) -> Result<(), PolicyWalletError> {
        require_owner_auth(&env)?;
        validate_policy(&env, &policy)?;

        let agent = policy.agent.clone();
        let current_revision = env
            .storage()
            .instance()
            .get::<_, SpendingPolicy>(&DataKey::Policy(agent.clone()))
            .map(|existing| existing.revision)
            .unwrap_or(0);

        if current_revision != expected_revision {
            return Err(PolicyWalletError::StaleRevision);
        }

        let next_revision = current_revision
            .checked_add(1)
            .ok_or(PolicyWalletError::RevisionOverflow)?;

        policy.revision = next_revision;

        let policy_id = compute_policy_configuration_digest(&env, &read_config(&env)?.owner, &policy);
        update_agent_policy_index_on_upsert(&env, &agent, &policy_id)?;

        // Setting a policy is an explicit re-grant and clears a revocation.
        env.storage()
            .instance()
            .remove(&DataKey::RevokedPolicy(agent.clone()));
        env.storage()
            .instance()
            .set(&DataKey::Policy(agent), &policy);
        env.storage()
            .instance()
            .extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);

        Ok(())
    }

    /// Owner-only: removes the agent's policy so every later check and
    /// authorization fails with `PolicyRevoked`. Revoking an agent that never
    /// had a policy fails with `PolicyNotFound`; revoking twice fails with
    /// `PolicyRevoked`. Daily-spend records and other agents are untouched.
    pub fn revoke_policy(env: Env, agent: Address) -> Result<(), PolicyWalletError> {
        let owner = read_config(&env)?.owner;
        owner.require_auth();
        if env
            .storage()
            .instance()
            .has(&DataKey::RevokedPolicy(agent.clone()))
        {
            return Err(PolicyWalletError::PolicyRevoked);
        }
        let policy_key = DataKey::Policy(agent.clone());
        if !env.storage().instance().has(&policy_key) {
            return Err(PolicyWalletError::PolicyNotFound);
        }

        let revoked_at_ledger = env.ledger().sequence();
        env.storage().instance().remove(&policy_key);
        env.storage()
            .instance()
            .set(&DataKey::RevokedPolicy(agent.clone()), &revoked_at_ledger);
        update_agent_policy_index_on_revoke(&env, &agent);
        AgentPolicyRevoked {
            agent,
            owner,
            revoked_at_ledger,
        }
        .publish(&env);
        Ok(())
    }

    /// Ledger at which the agent's policy was revoked, if it is revoked.
    pub fn get_policy_revocation(env: Env, agent: Address) -> Option<u32> {
        env.storage().instance().get(&DataKey::RevokedPolicy(agent))
    }

    pub fn create_policy(env: Env, policy: SpendingPolicy) -> Result<(), PolicyWalletError> {
        Self::set_policy(env, policy, 0)
    }

    pub fn get_policy(env: Env, agent: Address) -> Result<SpendingPolicy, PolicyWalletError> {
        read_policy(&env, &agent)
    }

    /// Queries the versioned, domain-separated cryptographic digest of an agent's active policy configuration.
    /// The digest binds policy owner, limits, validity windows, allowlists, and contract identity.
    pub fn get_policy_configuration_digest(
        env: Env,
        agent: Address,
    ) -> Result<BytesN<32>, PolicyWalletError> {
        let config = read_config(&env)?;
        let policy = read_policy(&env, &agent)?;
        Ok(compute_policy_configuration_digest(&env, &config.owner, &policy))
    }

    pub fn get_policy_revision(env: Env, agent: Address) -> Result<u32, PolicyWalletError> {
        Ok(read_policy(&env, &agent)?.revision)
    }

    /// Returns a paginated view of policy IDs assigned to a specific agent.
    /// Results are deterministically ordered and bounded by MAX_POLICY_PAGE_SIZE.
    pub fn get_agent_policies(
        env: Env,
        agent: Address,
        cursor: Option<u32>,
        limit: Option<u32>,
    ) -> Result<PolicyPage, PolicyWalletError> {
        let page_limit = match limit {
            Some(l) => {
                if l == 0 || l > MAX_POLICY_PAGE_SIZE {
                    return Err(PolicyWalletError::InvalidPageLimit);
                }
                l
            }
            None => MAX_POLICY_PAGE_SIZE,
        };

        let stored_ids: Vec<BytesN<32>> = env
            .storage()
            .instance()
            .get(&DataKey::AgentPolicyIndex(agent))
            .unwrap_or(Vec::new(&env));

        let total_count = stored_ids.len();
        let start = cursor.unwrap_or(0);

        let mut page_ids = Vec::new(&env);
        if start < total_count {
            let end = (start + page_limit).min(total_count);
            for i in start..end {
                page_ids.push_back(stored_ids.get(i).unwrap());
            }
            let next_cursor = if end < total_count { Some(end) } else { None };
            Ok(PolicyPage {
                policy_ids: page_ids,
                next_cursor,
                total_count,
            })
        } else {
            Ok(PolicyPage {
                policy_ids: page_ids,
                next_cursor: None,
                total_count,
            })
        }
    }

    /// Read-only status query reporting whether an agent's policy is Pending, Active,
    /// Expired, or Revoked at the current ledger sequence, alongside the policy's
    /// active window and the next ledger boundary. Revocation takes precedence.
    pub fn get_policy_status(
        env: Env,
        agent: Address,
    ) -> Result<PolicyActiveWindowStatus, PolicyWalletError> {
        let current_ledger = env.ledger().sequence();
        if env
            .storage()
            .instance()
            .has(&DataKey::RevokedPolicy(agent.clone()))
        {
            let policy_opt = env
                .storage()
                .instance()
                .get::<_, SpendingPolicy>(&DataKey::Policy(agent));
            let (from, until) = match policy_opt {
                Some(p) => (p.valid_from_ledger, p.valid_until_ledger),
                None => (0, 0),
            };
            return Ok(PolicyActiveWindowStatus {
                status: PolicyStatus::Revoked,
                current_ledger,
                valid_from_ledger: from,
                valid_until_ledger: until,
                next_boundary_ledger: None,
            });
        }

        let policy = env
            .storage()
            .instance()
            .get::<_, SpendingPolicy>(&DataKey::Policy(agent))
            .ok_or(PolicyWalletError::PolicyNotFound)?;

        let from = policy.valid_from_ledger;
        let until = policy.valid_until_ledger;

        if current_ledger < from {
            Ok(PolicyActiveWindowStatus {
                status: PolicyStatus::Pending,
                current_ledger,
                valid_from_ledger: from,
                valid_until_ledger: until,
                next_boundary_ledger: Some(from),
            })
        } else if current_ledger <= until {
            let next_boundary = if until < u32::MAX {
                Some(until + 1)
            } else {
                None
            };
            Ok(PolicyActiveWindowStatus {
                status: PolicyStatus::Active,
                current_ledger,
                valid_from_ledger: from,
                valid_until_ledger: until,
                next_boundary_ledger: next_boundary,
            })
        } else {
            Ok(PolicyActiveWindowStatus {
                status: PolicyStatus::Expired,
                current_ledger,
                valid_from_ledger: from,
                valid_until_ledger: until,
                next_boundary_ledger: None,
            })
        }
    }

    pub fn check_payment(
        env: Env,
        agent: Address,
        seller: Address,
        asset: Address,
        amount: i128,
        resource_hash: BytesN<32>,
    ) -> Result<(), PolicyWalletError> {
        validate_payment(&env, &agent, &seller, &asset, amount, &resource_hash)?;
        Ok(())
    }

    /// Read-only decision explaining whether a proposed payment would satisfy
    /// the current wallet policy without mutating daily counters or extending TTL.
    pub fn preview_payment(
        env: Env,
        agent: Address,
        seller: Address,
        asset: Address,
        amount: i128,
        resource_hash: BytesN<32>,
    ) -> PolicyPreviewResult {
        match validate_payment(&env, &agent, &seller, &asset, amount, &resource_hash) {
            Ok(()) => PolicyPreviewResult {
                allowed: true,
                denial_reason: PolicyDenialReason::None,
            },
            Err(err) => {
                let denial_reason = match err {
                    PolicyWalletError::PolicyNotFound => PolicyDenialReason::PolicyNotFound,
                    PolicyWalletError::PolicyRevoked => PolicyDenialReason::PolicyRevoked,
                    PolicyWalletError::PolicyNotActive => PolicyDenialReason::PolicyNotActive,
                    PolicyWalletError::PolicyExpired => PolicyDenialReason::PolicyExpired,
                    PolicyWalletError::InvalidAmount => PolicyDenialReason::InvalidAmount,
                    PolicyWalletError::AmountExceedsPaymentCap => {
                        PolicyDenialReason::AmountExceedsPaymentCap
                    }
                    PolicyWalletError::SellerNotAllowed => PolicyDenialReason::SellerNotAllowed,
                    PolicyWalletError::AssetNotAllowed => PolicyDenialReason::AssetNotAllowed,
                    PolicyWalletError::ResourceHashNotAllowed => {
                        PolicyDenialReason::ResourceHashNotAllowed
                    }
                    PolicyWalletError::AuthorizationLimitExceeded => {
                        PolicyDenialReason::AuthorizationLimitExceeded
                    }
                    PolicyWalletError::AmountExceedsDailyCap => {
                        PolicyDenialReason::AmountExceedsDailyCap
                    }
                    _ => PolicyDenialReason::None,
                };
                PolicyPreviewResult {
                    allowed: false,
                    denial_reason,
                }
            }
        }
    }

    pub fn authorize_payment(
        env: Env,
        agent: Address,
        seller: Address,
        asset: Address,
        amount: i128,
        resource_hash: BytesN<32>,
    ) -> Result<(), PolicyWalletError> {
        agent.require_auth();
        validate_payment(&env, &agent, &seller, &asset, amount, &resource_hash)?;

        let ledger_day = current_ledger_day(&env);
        record_payment_spend(&env, &agent, &seller, &resource_hash, amount, ledger_day)?;
        extend_instance_ttl(&env);

        Ok(())
    }

    /// Authorizes a payment while binding a caller-provided nonce to prevent replay attacks
    /// or duplicate consumption.
    ///
    /// - If the nonce was not previously seen for this agent, validates and consumes policy allowances,
    ///   recording the authorization under the nonce. Returns `PaymentAuthorizationOutcome { replayed: false, ... }`.
    /// - If the exact same request is resubmitted with the same nonce, returns `PaymentAuthorizationOutcome { replayed: true, ... }`
    ///   deterministically without re-deducting spend or authorization counts.
    /// - If the nonce is reused for a distinct request (different seller, asset, amount, or resource hash),
    ///   fails with `PolicyWalletError::ReplayConflict`.
    pub fn authorize_payment_with_nonce(
        env: Env,
        agent: Address,
        seller: Address,
        asset: Address,
        amount: i128,
        resource_hash: BytesN<32>,
        nonce: BytesN<32>,
    ) -> Result<PaymentAuthorizationOutcome, PolicyWalletError> {
        agent.require_auth();

        let request_hash = compute_request_hash(&env, &seller, &asset, amount, &resource_hash);
        let nonce_key = DataKey::AuthorizationNonce(agent.clone(), nonce);

        if let Some(record) = env.storage().instance().get::<_, AuthorizationRecord>(&nonce_key) {
            if record.request_hash != request_hash {
                return Err(PolicyWalletError::ReplayConflict);
            }
            extend_instance_ttl(&env);
            return Ok(PaymentAuthorizationOutcome {
                replayed: true,
                authorized_at_ledger: record.authorized_at_ledger,
            });
        }

        validate_payment(&env, &agent, &seller, &asset, amount, &resource_hash)?;

        let ledger_day = current_ledger_day(&env);
        record_payment_spend(&env, &agent, &seller, &resource_hash, amount, ledger_day)?;

        let current_ledger = env.ledger().sequence();
        let record = AuthorizationRecord {
            request_hash,
            authorized_at_ledger: current_ledger,
            amount,
        };
        env.storage().instance().set(&nonce_key, &record);
        extend_instance_ttl(&env);

        Ok(PaymentAuthorizationOutcome {
            replayed: false,
            authorized_at_ledger: current_ledger,
        })
    }

    /// Queries the recorded replay outcome for a given agent and authorization nonce.
    pub fn get_authorization_nonce(
        env: Env,
        agent: Address,
        nonce: BytesN<32>,
    ) -> Option<AuthorizationRecord> {
        env.storage()
            .instance()
            .get(&DataKey::AuthorizationNonce(agent, nonce))
    }

    pub fn spent_today(env: Env, agent: Address) -> i128 {
        read_daily_spend(&env, &agent, current_ledger_day(&env))
    }

    pub fn spent_today_seller(env: Env, agent: Address, seller: Address) -> i128 {
        read_daily_seller_spend(&env, &agent, &seller, current_ledger_day(&env))
    }

    pub fn spent_today_resource(env: Env, agent: Address, resource_hash: BytesN<32>) -> i128 {
        read_daily_resource_spend(&env, &agent, &resource_hash, current_ledger_day(&env))
    }

    pub fn authorized_count_today(env: Env, agent: Address) -> u32 {
        read_daily_auth_count(&env, &agent, current_ledger_day(&env))
    }

    pub fn propose_owner(
        env: Env,
        pending_owner: Address,
        expires_at_ledger: u32,
    ) -> Result<(), PolicyWalletError> {
        let config = read_config(&env)?;
        config.owner.require_auth();

        if pending_owner == config.owner {
            return Err(PolicyWalletError::InvalidOwner);
        }

        if expires_at_ledger <= env.ledger().sequence() {
            return Err(PolicyWalletError::InvalidOwnerRotationExpiry);
        }

        if env.storage().instance().has(&DataKey::OwnerRotation) {
            return Err(PolicyWalletError::OwnerRotationPending);
        }

        let rotation = OwnerRotation {
            pending_owner,
            expires_at_ledger,
        };
        env.storage()
            .instance()
            .set(&DataKey::OwnerRotation, &rotation);

        Ok(())
    }

    pub fn accept_owner(env: Env) -> Result<(), PolicyWalletError> {
        let mut config = read_config(&env)?;
        let rotation: OwnerRotation = env
            .storage()
            .instance()
            .get(&DataKey::OwnerRotation)
            .ok_or(PolicyWalletError::OwnerRotationNotFound)?;

        if env.ledger().sequence() >= rotation.expires_at_ledger {
            return Err(PolicyWalletError::OwnerRotationExpired);
        }

        rotation.pending_owner.require_auth();

        config.owner = rotation.pending_owner;
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().remove(&DataKey::OwnerRotation);

        Ok(())
    }

    pub fn cancel_owner_rotation(env: Env) -> Result<(), PolicyWalletError> {
        let config = read_config(&env)?;
        config.owner.require_auth();

        if !env.storage().instance().has(&DataKey::OwnerRotation) {
            return Err(PolicyWalletError::OwnerRotationNotFound);
        }

        env.storage().instance().remove(&DataKey::OwnerRotation);

        Ok(())
    }

    pub fn get_owner_rotation(env: Env) -> Option<OwnerRotation> {
        env.storage().instance().get(&DataKey::OwnerRotation)
    }

    pub fn get_owner(env: Env) -> Result<Address, PolicyWalletError> {
        Ok(read_config(&env)?.owner)
    }

    pub fn example_only(_env: Env) -> bool {
        true
    }
}

fn read_config(env: &Env) -> Result<WalletConfig, PolicyWalletError> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(PolicyWalletError::NotInitialized)
}

fn require_owner_auth(env: &Env) -> Result<(), PolicyWalletError> {
    read_config(env)?.owner.require_auth();
    Ok(())
}

fn read_policy(env: &Env, agent: &Address) -> Result<SpendingPolicy, PolicyWalletError> {
    if env
        .storage()
        .instance()
        .has(&DataKey::RevokedPolicy(agent.clone()))
    {
        return Err(PolicyWalletError::PolicyRevoked);
    }
    env.storage()
        .instance()
        .get(&DataKey::Policy(agent.clone()))
        .ok_or(PolicyWalletError::PolicyNotFound)
}

fn has_duplicate_addresses(items: &Vec<Address>) -> bool {
    let len = items.len();
    for i in 0..len {
        if let Some(item) = items.get(i) {
            for j in (i + 1)..len {
                if let Some(other) = items.get(j) {
                    if item == other {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn has_duplicate_hashes(items: &Vec<BytesN<32>>) -> bool {
    let len = items.len();
    for i in 0..len {
        if let Some(item) = items.get(i) {
            for j in (i + 1)..len {
                if let Some(other) = items.get(j) {
                    if item == other {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn validate_policy(env: &Env, policy: &SpendingPolicy) -> Result<(), PolicyWalletError> {
    if policy.max_amount_per_payment <= 0
        || policy.max_amount_per_day <= 0
        || policy.max_authorizations_per_day == 0
        || policy.max_amount_per_payment > policy.max_amount_per_day
        || policy.valid_from_ledger > policy.valid_until_ledger
        || policy.allowed_sellers.is_empty()
        || policy.allowed_sellers.len() > MAX_ALLOWED_SELLERS
        || has_duplicate_addresses(&policy.allowed_sellers)
        || policy.allowed_assets.is_empty()
        || policy.allowed_assets.len() > MAX_ALLOWED_ASSETS
        || has_duplicate_addresses(&policy.allowed_assets)
        || policy.allowed_resource_hashes.is_empty()
        || policy.allowed_resource_hashes.len() > MAX_ALLOWED_RESOURCE_HASHES
        || has_duplicate_hashes(&policy.allowed_resource_hashes)
    {
        return Err(PolicyWalletError::InvalidPolicy);
    }

    if policy.valid_until_ledger <= env.ledger().sequence() {
        return Err(PolicyWalletError::PolicyExpired);
    }

    let seller_limits_len = policy.seller_limits.len();
    for i in 0..seller_limits_len {
        let limit = match policy.seller_limits.get(i) {
            Some(l) => l,
            None => return Err(PolicyWalletError::InvalidPolicy),
        };
        if !policy.allowed_sellers.contains(&limit.seller) {
            return Err(PolicyWalletError::InvalidPolicy);
        }
        if limit.max_amount_per_payment <= 0
            || limit.max_amount_per_day <= 0
            || limit.max_amount_per_payment > limit.max_amount_per_day
            || limit.max_amount_per_payment > policy.max_amount_per_payment
            || limit.max_amount_per_day > policy.max_amount_per_day
        {
            return Err(PolicyWalletError::InvalidPolicy);
        }
        for j in (i + 1)..seller_limits_len {
            if let Some(other) = policy.seller_limits.get(j) {
                if limit.seller == other.seller {
                    return Err(PolicyWalletError::InvalidPolicy);
                }
            }
        }
    }

    let resource_limits_len = policy.resource_limits.len();
    for i in 0..resource_limits_len {
        let limit = match policy.resource_limits.get(i) {
            Some(l) => l,
            None => return Err(PolicyWalletError::InvalidPolicy),
        };
        if !policy
            .allowed_resource_hashes
            .contains(&limit.resource_hash)
        {
            return Err(PolicyWalletError::InvalidPolicy);
        }
        if limit.max_amount_per_payment <= 0
            || limit.max_amount_per_day <= 0
            || limit.max_amount_per_payment > limit.max_amount_per_day
            || limit.max_amount_per_payment > policy.max_amount_per_payment
            || limit.max_amount_per_day > policy.max_amount_per_day
        {
            return Err(PolicyWalletError::InvalidPolicy);
        }
        for j in (i + 1)..resource_limits_len {
            if let Some(other) = policy.resource_limits.get(j) {
                if limit.resource_hash == other.resource_hash {
                    return Err(PolicyWalletError::InvalidPolicy);
                }
            }
        }
    }

    Ok(())
}

fn validate_payment(
    env: &Env,
    agent: &Address,
    seller: &Address,
    asset: &Address,
    amount: i128,
    resource_hash: &BytesN<32>,
) -> Result<(), PolicyWalletError> {
    let policy = read_policy(env, agent)?;

    if env.ledger().sequence() < policy.valid_from_ledger {
        return Err(PolicyWalletError::PolicyNotActive);
    }
    if env.ledger().sequence() > policy.valid_until_ledger {
        return Err(PolicyWalletError::PolicyExpired);
    }
    if amount <= 0 {
        return Err(PolicyWalletError::InvalidAmount);
    }
    if amount > policy.max_amount_per_payment {
        return Err(PolicyWalletError::AmountExceedsPaymentCap);
    }
    if !policy.allowed_sellers.contains(seller) {
        return Err(PolicyWalletError::SellerNotAllowed);
    }
    if !policy.allowed_assets.contains(asset) {
        return Err(PolicyWalletError::AssetNotAllowed);
    }
    if !policy.allowed_resource_hashes.contains(resource_hash) {
        return Err(PolicyWalletError::ResourceHashNotAllowed);
    }

    let ledger_day = current_ledger_day(env);
    let auth_count = read_daily_auth_count(env, agent, ledger_day);
    if auth_count >= policy.max_authorizations_per_day {
        return Err(PolicyWalletError::AuthorizationLimitExceeded);
    }

    let spent = read_daily_spend(env, agent, ledger_day);
    let next_spent = spent
        .checked_add(amount)
        .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
    if next_spent > policy.max_amount_per_day {
        return Err(PolicyWalletError::AmountExceedsDailyCap);
    }

    for seller_limit in policy.seller_limits.iter() {
        if seller_limit.seller == *seller {
            if amount > seller_limit.max_amount_per_payment {
                return Err(PolicyWalletError::AmountExceedsPaymentCap);
            }
            let seller_spent = read_daily_seller_spend(env, agent, seller, ledger_day);
            let next_seller_spent = seller_spent
                .checked_add(amount)
                .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
            if next_seller_spent > seller_limit.max_amount_per_day {
                return Err(PolicyWalletError::AmountExceedsDailyCap);
            }
            break;
        }
    }

    for resource_limit in policy.resource_limits.iter() {
        if resource_limit.resource_hash == *resource_hash {
            if amount > resource_limit.max_amount_per_payment {
                return Err(PolicyWalletError::AmountExceedsPaymentCap);
            }
            let resource_spent = read_daily_resource_spend(env, agent, resource_hash, ledger_day);
            let next_resource_spent = resource_spent
                .checked_add(amount)
                .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
            if next_resource_spent > resource_limit.max_amount_per_day {
                return Err(PolicyWalletError::AmountExceedsDailyCap);
            }
            break;
        }
    }

    Ok(())
}

pub const POLICY_AUTHORIZATION_DOMAIN: &[u8] = b"lumenbazaar/policy-authorization";

fn compute_request_hash(
    env: &Env,
    seller: &Address,
    asset: &Address,
    amount: i128,
    resource_hash: &BytesN<32>,
) -> BytesN<32> {
    use soroban_sdk::xdr::ToXdr;
    use soroban_sdk::Bytes;
    let payload = (
        Bytes::from_slice(env, POLICY_AUTHORIZATION_DOMAIN),
        seller.clone(),
        asset.clone(),
        amount,
        resource_hash.clone(),
    );
    env.crypto().sha256(&payload.to_xdr(env)).to_bytes()
}

fn record_payment_spend(
    env: &Env,
    agent: &Address,
    seller: &Address,
    resource_hash: &BytesN<32>,
    amount: i128,
    ledger_day: u32,
) -> Result<(), PolicyWalletError> {
    let auth_count = read_daily_auth_count(env, agent, ledger_day);
    let next_auth_count = auth_count
        .checked_add(1)
        .ok_or(PolicyWalletError::AuthorizationLimitExceeded)?;
    env.storage().instance().set(
        &DataKey::DailyAuthCount(agent.clone(), ledger_day),
        &next_auth_count,
    );

    let spent = read_daily_spend(env, agent, ledger_day);
    let next_spent = spent
        .checked_add(amount)
        .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
    env.storage()
        .instance()
        .set(&DataKey::DailySpend(agent.clone(), ledger_day), &next_spent);

    let seller_spent = read_daily_seller_spend(env, agent, seller, ledger_day);
    let next_seller_spent = seller_spent
        .checked_add(amount)
        .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
    env.storage().instance().set(
        &DataKey::DailySellerSpend(agent.clone(), seller.clone(), ledger_day),
        &next_seller_spent,
    );

    let resource_spent = read_daily_resource_spend(env, agent, resource_hash, ledger_day);
    let next_resource_spent = resource_spent
        .checked_add(amount)
        .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
    env.storage().instance().set(
        &DataKey::DailyResourceSpend(agent.clone(), resource_hash.clone(), ledger_day),
        &next_resource_spent,
    );

    Ok(())
}

pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);
}

fn read_daily_seller_spend(env: &Env, agent: &Address, seller: &Address, ledger_day: u32) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::DailySellerSpend(
            agent.clone(),
            seller.clone(),
            ledger_day,
        ))
        .unwrap_or(0)
}

fn read_daily_resource_spend(
    env: &Env,
    agent: &Address,
    resource_hash: &BytesN<32>,
    ledger_day: u32,
) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::DailyResourceSpend(
            agent.clone(),
            resource_hash.clone(),
            ledger_day,
        ))
        .unwrap_or(0)
}

fn read_daily_auth_count(env: &Env, agent: &Address, ledger_day: u32) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::DailyAuthCount(agent.clone(), ledger_day))
        .unwrap_or(0)
}

fn current_ledger_day(env: &Env) -> u32 {
    env.ledger().sequence() / LEDGERS_PER_DAY
}

/// Computes the canonical, versioned, domain-separated SHA-256 digest
/// over the wallet owner and policy configuration fields (owner, limits, windows, allowlists).
pub fn compute_policy_configuration_digest(
    env: &Env,
    owner: &Address,
    policy: &SpendingPolicy,
) -> BytesN<32> {
    let domain = (
        Bytes::from_slice(env, POLICY_CONFIGURATION_DOMAIN),
        POLICY_CONFIGURATION_VERSION,
        env.ledger().network_id(),
        env.current_contract_address(),
    )
        .to_xdr(env);
    let payload = (
        domain,
        owner.clone(),
        policy.agent.clone(),
        policy.max_amount_per_payment,
        policy.max_amount_per_day,
        policy.max_authorizations_per_day,
        policy.valid_from_ledger,
        policy.valid_until_ledger,
        policy.allowed_sellers.clone(),
        policy.allowed_assets.clone(),
        policy.allowed_resource_hashes.clone(),
        policy.seller_limits.clone(),
        policy.resource_limits.clone(),
    );
    env.crypto().sha256(&payload.to_xdr(env)).to_bytes()
}

fn update_agent_policy_index_on_upsert(
    env: &Env,
    agent: &Address,
    policy_id: &BytesN<32>,
) -> Result<(), PolicyWalletError> {
    let key = DataKey::AgentPolicyIndex(agent.clone());
    let mut stored: Vec<BytesN<32>> = env.storage().instance().get(&key).unwrap_or(Vec::new(env));

    // If ID already present (e.g. replacing identical configuration or re-granting same config), maintain idempotence
    for i in 0..stored.len() {
        if stored.get(i).unwrap() == *policy_id {
            return Ok(());
        }
    }

    if stored.len() >= MAX_STORED_AGENT_POLICIES {
        return Err(PolicyWalletError::PolicyIndexFull);
    }

    stored.push_back(policy_id.clone());
    env.storage().instance().set(&key, &stored);
    env.storage().instance().extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);
    Ok(())
}

fn update_agent_policy_index_on_revoke(env: &Env, agent: &Address) {
    let key = DataKey::AgentPolicyIndex(agent.clone());
    env.storage().instance().remove(&key);
}

#[cfg(test)]
mod test;
/// Reads the daily spend for an agent on a given ledger day.
/// Returns 0 if no record exists.
fn read_daily_spend(env: &Env, agent: &Address, ledger_day: u32) -> i128 {
    let key = DataKey::DailySpend(agent.clone(), ledger_day);
    env.storage().instance().get(&key).unwrap_or(0)
}
