use crate::{AssetStatus, ContractError, Session};
use soroban_sdk::{contracttype, Address, BytesN, Env};

pub const STORAGE_LAYOUT_VERSION: u32 = 3;
pub const DEFAULT_TTL_THRESHOLD: u32 = 100_000;
pub const TTL_EXTENSION_AMOUNT: u32 = 1_000_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    NextSessionSequence,
    Session(BytesN<32>),
    StorageLayoutVersion,
    SupportedAsset(Address),
    Liability(Address),
}

pub fn write_storage_layout_version(env: &Env) {
    env.storage()
        .instance()
        .set(&DataKey::StorageLayoutVersion, &STORAGE_LAYOUT_VERSION);
}

pub fn has_admin(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

pub fn write_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(DEFAULT_TTL_THRESHOLD, TTL_EXTENSION_AMOUNT);
}

pub fn write_supported_asset(env: &Env, asset: &Address) {
    env.storage().instance().set(
        &DataKey::SupportedAsset(asset.clone()),
        &AssetStatus::Active,
    );
}

pub fn is_supported_asset(env: &Env, asset: &Address) -> bool {
    read_asset_status(env, asset) == Some(AssetStatus::Active)
}

pub fn read_asset_status(env: &Env, asset: &Address) -> Option<AssetStatus> {
    env.storage()
        .instance()
        .get(&DataKey::SupportedAsset(asset.clone()))
}

pub fn write_asset_status(env: &Env, asset: &Address, status: &AssetStatus) {
    env.storage()
        .instance()
        .set(&DataKey::SupportedAsset(asset.clone()), status);
}

pub fn remove_asset_status(env: &Env, asset: &Address) {
    env.storage()
        .instance()
        .remove(&DataKey::SupportedAsset(asset.clone()));
}

pub fn read_liability(env: &Env, asset: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Liability(asset.clone()))
        .unwrap_or(0)
}

pub fn increase_liability(env: &Env, asset: &Address, amount: i128) -> Result<i128, ContractError> {
    let updated = read_liability(env, asset)
        .checked_add(amount)
        .ok_or(ContractError::LiabilityOverflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::Liability(asset.clone()), &updated);
    Ok(updated)
}

pub fn decrease_liability(env: &Env, asset: &Address, amount: i128) -> Result<i128, ContractError> {
    let updated = read_liability(env, asset)
        .checked_sub(amount)
        .filter(|value| *value >= 0)
        .ok_or(ContractError::LiabilityUnderflow)?;
    env.storage()
        .persistent()
        .set(&DataKey::Liability(asset.clone()), &updated);
    Ok(updated)
}

pub fn read_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Admin)
}

pub fn read_storage_layout_version(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::StorageLayoutVersion)
}

pub fn read_next_session_sequence(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::NextSessionSequence)
        .unwrap_or(0)
}

pub fn take_next_session_sequence(env: &Env) -> u64 {
    let sequence = read_next_session_sequence(env);
    env.storage()
        .instance()
        .set(&DataKey::NextSessionSequence, &(sequence + 1));
    sequence
}

pub fn write_session(env: &Env, session: &Session) {
    env.storage()
        .persistent()
        .set(&DataKey::Session(session.id.clone()), session);
    extend_session_and_liability_ttl(env, session);
}

pub fn extend_session_and_liability_ttl(env: &Env, session: &Session) {
    extend_session_ttl(env, &session.id);
    let liability_key = DataKey::Liability(session.asset.clone());
    if env.storage().persistent().has(&liability_key) {
        env.storage().persistent().extend_ttl(
            &liability_key,
            DEFAULT_TTL_THRESHOLD,
            TTL_EXTENSION_AMOUNT,
        );
    }
}

pub fn read_session(env: &Env, session_id: &BytesN<32>) -> Option<Session> {
    env.storage()
        .persistent()
        .get(&DataKey::Session(session_id.clone()))
}

pub fn has_session(env: &Env, session_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::Session(session_id.clone()))
}

pub fn extend_session_ttl(env: &Env, session_id: &BytesN<32>) {
    env.storage().persistent().extend_ttl(
        &DataKey::Session(session_id.clone()),
        DEFAULT_TTL_THRESHOLD,
        TTL_EXTENSION_AMOUNT,
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::SessionStatus;
    use soroban_sdk::{testutils::Address as _, Address, BytesN, Env};

    fn sample_session(env: &Env) -> Session {
        let id = BytesN::from_array(env, &[1; 32]);
        Session {
            id,
            buyer: Address::generate(env),
            seller: Address::generate(env),
            asset: Address::generate(env),
            max_amount: 100,
            escrowed_amount: 100,
            settled_amount: 0,
            expires_at_ledger: 50,
            resource_hash: BytesN::from_array(env, &[2; 32]),
            usage_hash: None,
            status: SessionStatus::Open,
        }
    }

    #[test]
    fn storage_layout_version_round_trips() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            assert_eq!(read_storage_layout_version(&env), None);
            write_storage_layout_version(&env);

            assert_eq!(
                read_storage_layout_version(&env),
                Some(STORAGE_LAYOUT_VERSION)
            );
        });
    }

    #[test]
    fn session_sequence_increments() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            assert_eq!(take_next_session_sequence(&env), 0);
            assert_eq!(take_next_session_sequence(&env), 1);
            assert_eq!(read_next_session_sequence(&env), 2);
        });
    }

    #[test]
    fn session_state_round_trips() {
        let env = Env::default();
        let contract_id = env.register(crate::UptoSessionContract, ());

        env.as_contract(&contract_id, || {
            let session = sample_session(&env);

            assert!(!has_session(&env, &session.id));
            write_session(&env, &session);

            assert!(has_session(&env, &session.id));
            assert_eq!(read_session(&env, &session.id), Some(session));
        });
    }
}
