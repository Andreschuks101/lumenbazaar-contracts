#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![no_std]

#[cfg(feature = "testutils")]
pub mod adversarial;
#[cfg(feature = "testutils")]
pub mod asset_probe;
#[cfg(feature = "testutils")]
pub mod conformance;
#[cfg(feature = "testutils")]
pub mod decimal_matrix;
#[cfg(feature = "testutils")]
pub mod reentry;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Balance(Address),
    MintCeiling,
    TotalSupply,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TokenError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
    BalanceOverflow = 5,
    MintCeilingExceeded = 6,
}

/// Default immutable total mint ceiling applied by [`TestTokenContract::initialize`].
///
/// The default is `i128::MAX` so legacy callers that only pass `admin`
/// (deployment scripts, rehearsal tooling, and `upto-session` test helpers)
/// keep working unchanged. This ceiling is test-only: it bounds accidental
/// testnet minting and is not an economic guarantee.
pub const DEFAULT_MINT_CEILING: i128 = i128::MAX;

#[contract]
pub struct TestTokenContract;

#[contractimpl]
impl TestTokenContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), TokenError> {
        Self::initialize_with_ceiling(env, admin, DEFAULT_MINT_CEILING)
    }

    /// One-time initialization with an immutable, positive total mint ceiling.
    ///
    /// The ceiling is bound at initialization and can never change. Passing a
    /// non-positive ceiling is rejected so a misconfiguration cannot silently
    /// disable minting or enable unbounded issuance.
    pub fn initialize_with_ceiling(
        env: Env,
        admin: Address,
        ceiling: i128,
    ) -> Result<(), TokenError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(TokenError::AlreadyInitialized);
        }
        if ceiling <= 0 {
            return Err(TokenError::InvalidAmount);
        }

        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::MintCeiling, &ceiling);
        env.storage().instance().set(&DataKey::TotalSupply, &0_i128);

        Ok(())
    }

    pub fn mint(env: Env, to: Address, amount: i128) -> Result<(), TokenError> {
        let admin = read_admin(&env)?;

        validate_positive_amount(amount)?;
        admin.require_auth();

        // Checked supply accounting: the ceiling and overflow are validated
        // before any balance write so a rejected mint leaves total supply and
        // every balance unchanged.
        let current_supply = read_total_supply(&env);
        let next_supply = current_supply
            .checked_add(amount)
            .ok_or(TokenError::MintCeilingExceeded)?;
        if next_supply > read_mint_ceiling(&env) {
            return Err(TokenError::MintCeilingExceeded);
        }

        credit(&env, &to, amount)?;
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &next_supply);

        Ok(())
    }

    /// Immutable test-only total mint ceiling configured at initialization.
    pub fn mint_ceiling(env: Env) -> i128 {
        read_mint_ceiling(&env)
    }

    /// Sum of all successful mint amounts since initialization.
    pub fn total_supply(env: Env) -> i128 {
        read_total_supply(&env)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), TokenError> {
        validate_positive_amount(amount)?;
        from.require_auth();
        debit(&env, &from, amount)?;
        credit(&env, &to, amount)
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    pub fn decimals() -> u32 {
        7
    }

    pub fn name(env: Env) -> String {
        String::from_str(&env, "LumenBazaar Test Token")
    }

    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "LBT")
    }

    pub fn utility_only(_env: Env) -> bool {
        true
    }
}

fn read_admin(env: &Env) -> Result<Address, TokenError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(TokenError::NotInitialized)
}

fn read_mint_ceiling(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::MintCeiling)
        .unwrap_or(DEFAULT_MINT_CEILING)
}

fn read_total_supply(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalSupply)
        .unwrap_or(0)
}

fn read_balance(env: &Env, account: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Balance(account.clone()))
        .unwrap_or(0)
}

fn write_balance(env: &Env, account: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&DataKey::Balance(account.clone()), &amount);
}

fn validate_positive_amount(amount: i128) -> Result<(), TokenError> {
    if amount <= 0 {
        return Err(TokenError::InvalidAmount);
    }

    Ok(())
}

fn credit(env: &Env, account: &Address, amount: i128) -> Result<(), TokenError> {
    let balance = read_balance(env, account);
    let updated = balance
        .checked_add(amount)
        .ok_or(TokenError::BalanceOverflow)?;

    write_balance(env, account, updated);
    Ok(())
}

fn debit(env: &Env, account: &Address, amount: i128) -> Result<(), TokenError> {
    let balance = read_balance(env, account);

    if balance < amount {
        return Err(TokenError::InsufficientBalance);
    }

    write_balance(env, account, balance - amount);
    Ok(())
}

#[cfg(test)]
mod test;
