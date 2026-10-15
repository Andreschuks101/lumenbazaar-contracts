use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, String,
};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeBehavior {
    Reverting,
    Malformed,
}

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ProbeError {
    Reverted = 1,
}

#[contract]
pub struct ProbeTokenContract;

#[contractimpl]
impl ProbeTokenContract {
    pub fn initialize(env: Env, selected: ProbeBehavior) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("behavior"), &selected);
    }

    pub fn balance(env: Env, _id: Address) -> i128 {
        match behavior(&env) {
            ProbeBehavior::Reverting => panic_with_error!(&env, ProbeError::Reverted),
            ProbeBehavior::Malformed => -1,
        }
    }

    pub fn decimals(env: Env) -> u32 {
        match behavior(&env) {
            ProbeBehavior::Reverting => panic_with_error!(&env, ProbeError::Reverted),
            ProbeBehavior::Malformed => u32::MAX,
        }
    }

    pub fn name(env: Env) -> String {
        match behavior(&env) {
            ProbeBehavior::Reverting => panic_with_error!(&env, ProbeError::Reverted),
            ProbeBehavior::Malformed => String::from_str(&env, ""),
        }
    }

    pub fn symbol(env: Env) -> String {
        match behavior(&env) {
            ProbeBehavior::Reverting => panic_with_error!(&env, ProbeError::Reverted),
            ProbeBehavior::Malformed => String::from_str(&env, ""),
        }
    }
}

#[contract]
pub struct IncompatibleContract;

#[contractimpl]
impl IncompatibleContract {
    pub fn ping() -> bool {
        true
    }
}

fn behavior(env: &Env) -> ProbeBehavior {
    env.storage()
        .instance()
        .get(&soroban_sdk::symbol_short!("behavior"))
        .unwrap_or(ProbeBehavior::Reverting)
}
