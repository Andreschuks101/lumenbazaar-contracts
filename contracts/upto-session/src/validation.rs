use crate::{ContractError, Session};
use soroban_sdk::{Address, BytesN, Env};

pub const MAX_SESSION_DURATION_LEDGERS: u32 = 518_400;
pub const MAX_ADMIN_ROTATION_DURATION_LEDGERS: u32 = 17_280;

pub fn validate_admin_rotation(
    env: &Env,
    current_admin: &Address,
    pending_admin: &Address,
    expires_at_ledger: u32,
) -> Result<(), ContractError> {
    if current_admin == pending_admin {
        return Err(ContractError::InvalidAdmin);
    }
    let current_ledger = env.ledger().sequence();
    if expires_at_ledger <= current_ledger
        || expires_at_ledger - current_ledger > MAX_ADMIN_ROTATION_DURATION_LEDGERS
    {
        return Err(ContractError::InvalidAdminRotationExpiry);
    }
    Ok(())
}

pub fn validate_create_session(
    env: &Env,
    buyer: &Address,
    seller: &Address,
    asset: &Address,
    max_amount: i128,
    expires_at_ledger: u32,
    resource_hash: &BytesN<32>,
) -> Result<(), ContractError> {
    if buyer == seller {
        return Err(ContractError::InvalidSeller);
    }

    if asset == buyer || asset == seller {
        return Err(ContractError::InvalidAsset);
    }

    if max_amount <= 0 {
        return Err(ContractError::InvalidAmount);
    }

    if expires_at_ledger <= env.ledger().sequence() {
        return Err(ContractError::ExpiredSession);
    }

    if expires_at_ledger - env.ledger().sequence() > MAX_SESSION_DURATION_LEDGERS {
        return Err(ContractError::SessionDurationTooLong);
    }

    if resource_hash == &BytesN::from_array(env, &[0; 32]) {
        return Err(ContractError::InvalidResourceHash);
    }

    Ok(())
}

pub fn validate_settlement_amount(
    env: &Env,
    session: &Session,
    actual_amount: i128,
) -> Result<(), ContractError> {
    if actual_amount <= 0 {
        return Err(ContractError::InvalidAmount);
    }

    if actual_amount > session.max_amount {
        return Err(ContractError::AmountExceedsCap);
    }

    if env.ledger().sequence() >= session.expires_at_ledger {
        return Err(ContractError::ExpiredSession);
    }

    Ok(())
}

pub fn validate_usage_hash(env: &Env, usage_hash: &BytesN<32>) -> Result<(), ContractError> {
    if usage_hash == &BytesN::from_array(env, &[0; 32]) {
        return Err(ContractError::InvalidUsageHash);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address,
    };

    fn valid_inputs(env: &Env) -> (Address, Address, Address, BytesN<32>) {
        (
            Address::generate(env),
            Address::generate(env),
            Address::generate(env),
            BytesN::from_array(env, &[3; 32]),
        )
    }

    #[test]
    fn accepts_valid_inputs() {
        let env = Env::default();
        let (buyer, seller, asset, resource_hash) = valid_inputs(&env);

        assert_eq!(
            validate_create_session(&env, &buyer, &seller, &asset, 100, 10, &resource_hash),
            Ok(())
        );
    }

    #[test]
    fn validates_admin_rotation_parties_and_expiry() {
        let env = Env::default();
        env.ledger().set_sequence_number(10);
        let current_admin = Address::generate(&env);
        let pending_admin = Address::generate(&env);

        assert_eq!(
            validate_admin_rotation(&env, &current_admin, &pending_admin, 11),
            Ok(())
        );
        assert_eq!(
            validate_admin_rotation(&env, &current_admin, &current_admin, 11),
            Err(ContractError::InvalidAdmin)
        );
        assert_eq!(
            validate_admin_rotation(&env, &current_admin, &pending_admin, 10),
            Err(ContractError::InvalidAdminRotationExpiry)
        );
        assert_eq!(
            validate_admin_rotation(
                &env,
                &current_admin,
                &pending_admin,
                11 + MAX_ADMIN_ROTATION_DURATION_LEDGERS
            ),
            Err(ContractError::InvalidAdminRotationExpiry)
        );
    }

    #[test]
    fn rejects_invalid_amounts() {
        let env = Env::default();
        let (buyer, seller, asset, resource_hash) = valid_inputs(&env);

        assert_eq!(
            validate_create_session(&env, &buyer, &seller, &asset, 0, 10, &resource_hash),
            Err(ContractError::InvalidAmount)
        );
        assert_eq!(
            validate_create_session(&env, &buyer, &seller, &asset, -1, 10, &resource_hash),
            Err(ContractError::InvalidAmount)
        );
    }

    #[test]
    fn rejects_expired_sessions() {
        let env = Env::default();
        env.ledger().set_sequence_number(10);
        let (buyer, seller, asset, resource_hash) = valid_inputs(&env);

        assert_eq!(
            validate_create_session(&env, &buyer, &seller, &asset, 100, 10, &resource_hash),
            Err(ContractError::ExpiredSession)
        );
    }

    #[test]
    fn rejects_empty_resource_hash() {
        let env = Env::default();
        let (buyer, seller, asset, _) = valid_inputs(&env);
        let empty_hash = BytesN::from_array(&env, &[0; 32]);

        assert_eq!(
            validate_create_session(&env, &buyer, &seller, &asset, 100, 10, &empty_hash),
            Err(ContractError::InvalidResourceHash)
        );
    }

    #[test]
    fn rejects_invalid_seller_or_asset_bindings() {
        let env = Env::default();
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let resource_hash = BytesN::from_array(&env, &[3; 32]);

        assert_eq!(
            validate_create_session(&env, &buyer, &buyer, &asset, 100, 10, &resource_hash),
            Err(ContractError::InvalidSeller)
        );
        assert_eq!(
            validate_create_session(&env, &buyer, &seller, &buyer, 100, 10, &resource_hash),
            Err(ContractError::InvalidAsset)
        );
    }

    #[test]
    fn rejects_invalid_settlement_amounts() {
        let env = Env::default();
        let (buyer, seller, asset, resource_hash) = valid_inputs(&env);
        let session = Session {
            id: BytesN::from_array(&env, &[1; 32]),
            buyer,
            seller,
            asset,
            max_amount: 100,
            escrowed_amount: 100,
            settled_amount: 0,
            expires_at_ledger: 50,
            resource_hash,
            usage_hash: None,
            status: crate::SessionStatus::Open,
        };

        assert_eq!(
            validate_settlement_amount(&env, &session, 0),
            Err(ContractError::InvalidAmount)
        );
        assert_eq!(
            validate_settlement_amount(&env, &session, -1),
            Err(ContractError::InvalidAmount)
        );
        assert_eq!(
            validate_settlement_amount(&env, &session, 101),
            Err(ContractError::AmountExceedsCap)
        );
    }

    #[test]
    fn rejects_settlement_at_or_after_expiry() {
        let env = Env::default();
        env.ledger().set_sequence_number(50);
        let (buyer, seller, asset, resource_hash) = valid_inputs(&env);
        let session = Session {
            id: BytesN::from_array(&env, &[1; 32]),
            buyer,
            seller,
            asset,
            max_amount: 100,
            escrowed_amount: 100,
            settled_amount: 0,
            expires_at_ledger: 50,
            resource_hash,
            usage_hash: None,
            status: crate::SessionStatus::Open,
        };

        assert_eq!(
            validate_settlement_amount(&env, &session, 50),
            Err(ContractError::ExpiredSession)
        );
    }

    #[test]
    fn rejects_empty_usage_hash() {
        let env = Env::default();
        let empty_hash = BytesN::from_array(&env, &[0; 32]);

        assert_eq!(
            validate_usage_hash(&env, &empty_hash),
            Err(ContractError::InvalidUsageHash)
        );
    }
}
