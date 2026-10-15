#![no_std]

#[cfg(feature = "testutils")]
pub mod asset_probe;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Balance(Address),
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
}

#[contract]
pub struct TestTokenContract;

#[contractimpl]
impl TestTokenContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), TokenError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(TokenError::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);

        Ok(())
    }

    pub fn mint(env: Env, to: Address, amount: i128) -> Result<(), TokenError> {
        let admin = read_admin(&env)?;

        validate_positive_amount(amount)?;
        admin.require_auth();
        credit(&env, &to, amount)
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
