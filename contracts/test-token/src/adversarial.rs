use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    BytesN, Env, Error, IntoVal, String,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdversarialBehavior {
    Honest,
    FailTransfer,
    FalseBalance,
    PartialTransfer,
    NestedAttempt,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Balance(Address),
    Behavior,
    CallbackTarget,
    CallbackSession,
    CallbackAmount,
    CallbackUsageHash,
    CallbackRejected,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AdversarialTokenError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
    BalanceOverflow = 5,
    TransferFailed = 6,
    CallbackNotConfigured = 7,
    NestedInvocationSucceeded = 8,
}

#[contract]
pub struct AdversarialTokenContract;

#[contractimpl]
impl AdversarialTokenContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), AdversarialTokenError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(AdversarialTokenError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Behavior, &AdversarialBehavior::Honest);
        Ok(())
    }

    pub fn mint(env: Env, to: Address, amount: i128) -> Result<(), AdversarialTokenError> {
        let admin = read_admin(&env)?;
        validate_positive_amount(amount)?;
        admin.require_auth();
        credit(&env, &to, amount)
    }

    pub fn set_behavior(
        env: Env,
        behavior: AdversarialBehavior,
    ) -> Result<(), AdversarialTokenError> {
        let admin = read_admin(&env)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Behavior, &behavior);
        Ok(())
    }

    pub fn configure_nested_attempt(
        env: Env,
        target: Address,
        session_id: BytesN<32>,
        actual_amount: i128,
        usage_hash: BytesN<32>,
    ) -> Result<(), AdversarialTokenError> {
        let admin = read_admin(&env)?;
        validate_positive_amount(actual_amount)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::CallbackTarget, &target);
        env.storage()
            .instance()
            .set(&DataKey::CallbackSession, &session_id);
        env.storage()
            .instance()
            .set(&DataKey::CallbackAmount, &actual_amount);
        env.storage()
            .instance()
            .set(&DataKey::CallbackUsageHash, &usage_hash);
        env.storage()
            .instance()
            .set(&DataKey::Behavior, &AdversarialBehavior::NestedAttempt);
        Ok(())
    }

    pub fn transfer(
        env: Env,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), AdversarialTokenError> {
        validate_positive_amount(amount)?;
        from.require_auth();
        let behavior = read_behavior(&env);
        if behavior == AdversarialBehavior::FailTransfer {
            panic_with_error!(&env, AdversarialTokenError::TransferFailed);
        }

        debit(&env, &from, amount)?;
        let credited = if behavior == AdversarialBehavior::PartialTransfer {
            amount - 1
        } else {
            amount
        };
        if credited > 0 {
            credit(&env, &to, credited)?;
        }

        if behavior == AdversarialBehavior::NestedAttempt {
            attempt_nested_settlement(&env)?;
        }
        Ok(())
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        if read_behavior(&env) == AdversarialBehavior::FalseBalance {
            return 0;
        }
        read_balance(&env, &id)
    }

    pub fn decimals() -> u32 {
        7
    }

    pub fn name(env: Env) -> String {
        String::from_str(&env, "LumenBazaar Adversarial Token")
    }

    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "LAT")
    }

    pub fn raw_balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    pub fn callback_rejected(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::CallbackRejected)
            .unwrap_or(false)
    }
}

fn attempt_nested_settlement(env: &Env) -> Result<(), AdversarialTokenError> {
    let target: Address = env
        .storage()
        .instance()
        .get(&DataKey::CallbackTarget)
        .ok_or(AdversarialTokenError::CallbackNotConfigured)?;
    let session_id: BytesN<32> = env
        .storage()
        .instance()
        .get(&DataKey::CallbackSession)
        .ok_or(AdversarialTokenError::CallbackNotConfigured)?;
    let amount: i128 = env
        .storage()
        .instance()
        .get(&DataKey::CallbackAmount)
        .ok_or(AdversarialTokenError::CallbackNotConfigured)?;
    let usage_hash: BytesN<32> = env
        .storage()
        .instance()
        .get(&DataKey::CallbackUsageHash)
        .ok_or(AdversarialTokenError::CallbackNotConfigured)?;
    let result = env.try_invoke_contract::<(), Error>(
        &target,
        &symbol_short!("settle"),
        (session_id, amount, usage_hash).into_val(env),
    );
    let rejected = result.is_err() || matches!(result, Ok(Err(_)));
    env.storage()
        .instance()
        .set(&DataKey::CallbackRejected, &rejected);
    if !rejected {
        return Err(AdversarialTokenError::NestedInvocationSucceeded);
    }
    Ok(())
}

fn read_admin(env: &Env) -> Result<Address, AdversarialTokenError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(AdversarialTokenError::NotInitialized)
}

fn read_behavior(env: &Env) -> AdversarialBehavior {
    env.storage()
        .instance()
        .get(&DataKey::Behavior)
        .unwrap_or(AdversarialBehavior::Honest)
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

fn validate_positive_amount(amount: i128) -> Result<(), AdversarialTokenError> {
    if amount <= 0 {
        return Err(AdversarialTokenError::InvalidAmount);
    }
    Ok(())
}

fn credit(env: &Env, account: &Address, amount: i128) -> Result<(), AdversarialTokenError> {
    let updated = read_balance(env, account)
        .checked_add(amount)
        .ok_or(AdversarialTokenError::BalanceOverflow)?;
    write_balance(env, account, updated);
    Ok(())
}

fn debit(env: &Env, account: &Address, amount: i128) -> Result<(), AdversarialTokenError> {
    let balance = read_balance(env, account);
    if balance < amount {
        return Err(AdversarialTokenError::InsufficientBalance);
    }
    write_balance(env, account, balance - amount);
    Ok(())
}
