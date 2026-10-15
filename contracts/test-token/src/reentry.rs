use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Error, IntoVal,
    Symbol,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReentryAction {
    None,
    Create,
    Settle,
    Cancel,
    Recover,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Balance(Address),
    Target,
    Action,
    Buyer,
    Seller,
    SessionId,
    Amount,
    ExpiresAtLedger,
    ResourceHash,
    UsageHash,
    CallbackRejected,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ReentryTokenError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
    BalanceOverflow = 5,
    CallbackNotConfigured = 6,
    NestedInvocationSucceeded = 7,
}

#[contract]
pub struct ReentryTokenContract;

#[contractimpl]
impl ReentryTokenContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), ReentryTokenError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(ReentryTokenError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Action, &ReentryAction::None);
        Ok(())
    }

    pub fn mint(env: Env, to: Address, amount: i128) -> Result<(), ReentryTokenError> {
        let admin = read_admin(&env)?;
        validate_positive(amount)?;
        admin.require_auth();
        credit(&env, &to, amount)
    }

    pub fn configure_create(
        env: Env,
        target: Address,
        buyer: Address,
        seller: Address,
        amount: i128,
        expires_at_ledger: u32,
        resource_hash: BytesN<32>,
    ) -> Result<(), ReentryTokenError> {
        require_admin(&env)?;
        validate_positive(amount)?;
        env.storage().instance().set(&DataKey::Target, &target);
        env.storage().instance().set(&DataKey::Buyer, &buyer);
        env.storage().instance().set(&DataKey::Seller, &seller);
        env.storage().instance().set(&DataKey::Amount, &amount);
        env.storage()
            .instance()
            .set(&DataKey::ExpiresAtLedger, &expires_at_ledger);
        env.storage()
            .instance()
            .set(&DataKey::ResourceHash, &resource_hash);
        set_action(&env, ReentryAction::Create);
        Ok(())
    }

    pub fn configure_terminal(
        env: Env,
        target: Address,
        action: ReentryAction,
        session_id: BytesN<32>,
        amount: i128,
        usage_hash: BytesN<32>,
    ) -> Result<(), ReentryTokenError> {
        require_admin(&env)?;
        if !matches!(
            action,
            ReentryAction::Settle | ReentryAction::Cancel | ReentryAction::Recover
        ) {
            return Err(ReentryTokenError::CallbackNotConfigured);
        }
        env.storage().instance().set(&DataKey::Target, &target);
        env.storage()
            .instance()
            .set(&DataKey::SessionId, &session_id);
        env.storage().instance().set(&DataKey::Amount, &amount);
        env.storage()
            .instance()
            .set(&DataKey::UsageHash, &usage_hash);
        set_action(&env, action);
        Ok(())
    }

    pub fn transfer(
        env: Env,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), ReentryTokenError> {
        validate_positive(amount)?;
        from.require_auth();
        debit(&env, &from, amount)?;
        credit(&env, &to, amount)?;
        attempt_callback(&env)?;
        Ok(())
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    pub fn callback_rejected(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::CallbackRejected)
            .unwrap_or(false)
    }
}

fn set_action(env: &Env, action: ReentryAction) {
    env.storage().instance().set(&DataKey::Action, &action);
    env.storage()
        .instance()
        .set(&DataKey::CallbackRejected, &false);
}

fn attempt_callback(env: &Env) -> Result<(), ReentryTokenError> {
    let action: ReentryAction = env
        .storage()
        .instance()
        .get(&DataKey::Action)
        .unwrap_or(ReentryAction::None);
    if action == ReentryAction::None {
        return Ok(());
    }
    let target: Address = required(env, &DataKey::Target)?;
    let rejected = match action {
        ReentryAction::None => true,
        ReentryAction::Create => {
            let buyer: Address = required(env, &DataKey::Buyer)?;
            let seller: Address = required(env, &DataKey::Seller)?;
            let amount: i128 = required(env, &DataKey::Amount)?;
            let expires_at: u32 = required(env, &DataKey::ExpiresAtLedger)?;
            let resource_hash: BytesN<32> = required(env, &DataKey::ResourceHash)?;
            let result = env.try_invoke_contract::<BytesN<32>, Error>(
                &target,
                &Symbol::new(env, "create_session"),
                (
                    buyer,
                    seller,
                    env.current_contract_address(),
                    amount,
                    expires_at,
                    resource_hash,
                )
                    .into_val(env),
            );
            result.is_err() || matches!(result, Ok(Err(_)))
        }
        ReentryAction::Settle => {
            let session_id: BytesN<32> = required(env, &DataKey::SessionId)?;
            let amount: i128 = required(env, &DataKey::Amount)?;
            let usage_hash: BytesN<32> = required(env, &DataKey::UsageHash)?;
            rejected_unit(
                env,
                &target,
                "settle",
                (session_id, amount, usage_hash).into_val(env),
            )
        }
        ReentryAction::Cancel => {
            let session_id: BytesN<32> = required(env, &DataKey::SessionId)?;
            rejected_unit(env, &target, "cancel", (session_id,).into_val(env))
        }
        ReentryAction::Recover => {
            let session_id: BytesN<32> = required(env, &DataKey::SessionId)?;
            rejected_unit(env, &target, "recover_expired", (session_id,).into_val(env))
        }
    };
    env.storage()
        .instance()
        .set(&DataKey::CallbackRejected, &rejected);
    if rejected {
        Ok(())
    } else {
        Err(ReentryTokenError::NestedInvocationSucceeded)
    }
}

fn rejected_unit(
    env: &Env,
    target: &Address,
    method: &str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) -> bool {
    let result = env.try_invoke_contract::<(), Error>(target, &Symbol::new(env, method), args);
    result.is_err() || matches!(result, Ok(Err(_)))
}

fn required<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<T, ReentryTokenError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(ReentryTokenError::CallbackNotConfigured)
}

fn require_admin(env: &Env) -> Result<Address, ReentryTokenError> {
    let admin = read_admin(env)?;
    admin.require_auth();
    Ok(admin)
}

fn read_admin(env: &Env) -> Result<Address, ReentryTokenError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(ReentryTokenError::NotInitialized)
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

fn validate_positive(amount: i128) -> Result<(), ReentryTokenError> {
    if amount <= 0 {
        return Err(ReentryTokenError::InvalidAmount);
    }
    Ok(())
}

fn credit(env: &Env, account: &Address, amount: i128) -> Result<(), ReentryTokenError> {
    let updated = read_balance(env, account)
        .checked_add(amount)
        .ok_or(ReentryTokenError::BalanceOverflow)?;
    write_balance(env, account, updated);
    Ok(())
}

fn debit(env: &Env, account: &Address, amount: i128) -> Result<(), ReentryTokenError> {
    let balance = read_balance(env, account);
    if balance < amount {
        return Err(ReentryTokenError::InsufficientBalance);
    }
    write_balance(env, account, balance - amount);
    Ok(())
}
