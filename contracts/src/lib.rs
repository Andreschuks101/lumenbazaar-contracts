#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Val, Vec};

#[contracttype]
pub enum DataKey {
    Admin,
    Config,
    Allowance(AllowanceKey),
}

#[contracttype]
pub struct AllowanceKey {
    pub owner: Address,
    pub spender: Address,
}

#[contracttype]
pub struct Config {
    pub max_supply: i128,
    pub burn_fee_bps: u32,
}

// Event definitions and decoders
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContractEvent {
    ConfigUpdated { admin: Address, new_fee: u32 },
    TokensBurned { burner: Address, amount: i128 },
}

pub struct EventDecoder;

impl EventDecoder {
    /// Decodes raw contract events into strongly typed contract events
    pub fn decode_burn_event(env: &Env, topics: Vec<Val>, data: Val) -> Option<ContractEvent> {
        let sig: Symbol = Symbol::new(env, "tokens_burned");
        if topics.first().map(|v| v == sig.to_val()).unwrap_or(false) {
            let burner = Address::from_val(env, &topics.get(1)?);
            let amount = i128::from_val(env, &data);
            Some(ContractEvent::TokensBurned { burner, amount })
        } else {
            None
        }
    }
}

@contract
pub struct LumenBazaarContract;

#[contractimpl]
impl LumenBazaarContract {
    /// Config validation helper
    fn validate_config(config: &Config) {
        if config.max_supply <= 0 {
            panic!("Config validation failed: max_supply must be positive");
        }
        if config.burn_fee_bps > 10000 {
            panic!("Config validation failed: burn_fee_bps cannot exceed 100% (10000 bps)");
        }
    }

    pub fn initialize(env: Env, admin: Address, config: Config) {
        admin.require_auth();
        Self::validate_config(&config);
        
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
    }

    /// Allowance query implementation
    pub fn allowance(env: Env, owner: Address, spender: Address) -> i128 {
        let key = DataKey::Allowance(AllowanceKey { owner, spender });
        env.storage().persistent().get(&key).unwrap_or(0)
    }

    pub fn update_config(env: Env, new_config: Config) {
        let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
            Some(a) => a,
            None => panic!("Not initialized"),
        };
        admin.require_auth();

        Self::validate_config(&new_config);
        env.storage().instance().set(&DataKey::Config, &new_config);

        // Emit config update event
        env.events().publish(
            (Symbol::new(&env, "config_updated"), admin.clone()),
            new_config.burn_fee_bps,
        );
    }
}
