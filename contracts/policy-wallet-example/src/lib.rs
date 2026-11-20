#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env, Vec,
};

pub const LEDGERS_PER_DAY: u32 = 17_280;
pub const MAX_ALLOWED_SELLERS: u32 = 16;
pub const MAX_ALLOWED_ASSETS: u32 = 16;
pub const MAX_ALLOWED_RESOURCE_HASHES: u32 = 16;

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
    OwnerRotation,
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

    pub fn get_policy_revision(env: Env, agent: Address) -> Result<u32, PolicyWalletError> {
        Ok(read_policy(&env, &agent)?.revision)
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
        let auth_count = read_daily_auth_count(&env, &agent, ledger_day);
        let next_auth_count = auth_count
            .checked_add(1)
            .ok_or(PolicyWalletError::AuthorizationLimitExceeded)?;
        env.storage().instance().set(
            &DataKey::DailyAuthCount(agent.clone(), ledger_day),
            &next_auth_count,
        );

        let spent = read_daily_spend(&env, &agent, ledger_day);
        let next_spent = spent
            .checked_add(amount)
            .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
        env.storage()
            .instance()
            .set(&DataKey::DailySpend(agent.clone(), ledger_day), &next_spent);

        let seller_spent = read_daily_seller_spend(&env, &agent, &seller, ledger_day);
        let next_seller_spent = seller_spent
            .checked_add(amount)
            .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
        env.storage().instance().set(
            &DataKey::DailySellerSpend(agent.clone(), seller, ledger_day),
            &next_seller_spent,
        );

        let resource_spent = read_daily_resource_spend(&env, &agent, &resource_hash, ledger_day);
        let next_resource_spent = resource_spent
            .checked_add(amount)
            .ok_or(PolicyWalletError::AmountExceedsDailyCap)?;
        env.storage().instance().set(
            &DataKey::DailyResourceSpend(agent, resource_hash, ledger_day),
            &next_resource_spent,
        );
        env.storage()
            .instance()
            .extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);

        Ok(())
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
        let item = items.get(i).unwrap();
        for j in (i + 1)..len {
            if item == items.get(j).unwrap() {
                return true;
            }
        }
    }
    false
}

fn has_duplicate_hashes(items: &Vec<BytesN<32>>) -> bool {
    let len = items.len();
    for i in 0..len {
        let item = items.get(i).unwrap();
        for j in (i + 1)..len {
            if item == items.get(j).unwrap() {
                return true;
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
        let limit = policy.seller_limits.get(i).unwrap();
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
            let other = policy.seller_limits.get(j).unwrap();
            if limit.seller == other.seller {
                return Err(PolicyWalletError::InvalidPolicy);
            }
        }
    }

    let resource_limits_len = policy.resource_limits.len();
    for i in 0..resource_limits_len {
        let limit = policy.resource_limits.get(i).unwrap();
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
            let other = policy.resource_limits.get(j).unwrap();
            if limit.resource_hash == other.resource_hash {
                return Err(PolicyWalletError::InvalidPolicy);
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

    for i in 0..policy.seller_limits.len() {
        let seller_limit = policy.seller_limits.get(i).unwrap();
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

    for i in 0..policy.resource_limits.len() {
        let resource_limit = policy.resource_limits.get(i).unwrap();
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

#[cfg(test)]
mod test;
/// Reads the daily spend for an agent on a given ledger day.
/// Returns 0 if no record exists.
fn read_daily_spend(env: &Env, agent: &Address, ledger_day: u32) -> i128 {
    let key = DataKey::DailySpend(agent.clone(), ledger_day);
    env.storage().instance().get(&key).unwrap_or(0)
}
