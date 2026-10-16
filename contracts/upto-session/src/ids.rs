use soroban_sdk::{xdr::ToXdr, Address, Bytes, BytesN, Env};

pub const SESSION_ID_DOMAIN: &[u8] = b"lumenbazaar/session-id";
pub const USAGE_COMMITMENT_DOMAIN: &[u8] = b"lumenbazaar/usage-commitment";

pub struct SessionIdInput<'a> {
    pub buyer: &'a Address,
    pub seller: &'a Address,
    pub asset: &'a Address,
    pub max_amount: i128,
    pub expires_at_ledger: u32,
    pub resource_hash: &'a BytesN<32>,
    pub sequence: u64,
}

pub struct UsageCommitmentInput<'a> {
    pub session_id: &'a BytesN<32>,
    pub actual_amount: i128,
    pub resource_hash: &'a BytesN<32>,
    pub measurement_hash: &'a BytesN<32>,
    pub receipt_schema_version: u32,
}

pub fn derive_session_id(env: &Env, input: &SessionIdInput<'_>) -> BytesN<32> {
    derive_session_id_for_version(env, input, crate::INTERFACE_VERSION)
}

pub fn derive_usage_commitment(env: &Env, input: &UsageCommitmentInput<'_>) -> BytesN<32> {
    derive_usage_commitment_for_version(env, input, crate::INTERFACE_VERSION)
}

pub fn session_id_preimage(env: &Env, input: &SessionIdInput<'_>) -> Bytes {
    session_id_preimage_for_version(env, input, crate::INTERFACE_VERSION)
}

pub fn usage_commitment_preimage(env: &Env, input: &UsageCommitmentInput<'_>) -> Bytes {
    usage_commitment_preimage_for_version(env, input, crate::INTERFACE_VERSION)
}

fn derive_session_id_for_version(
    env: &Env,
    input: &SessionIdInput<'_>,
    interface_version: u32,
) -> BytesN<32> {
    env.crypto()
        .sha256(&session_id_preimage_for_version(
            env,
            input,
            interface_version,
        ))
        .to_bytes()
}

fn derive_usage_commitment_for_version(
    env: &Env,
    input: &UsageCommitmentInput<'_>,
    interface_version: u32,
) -> BytesN<32> {
    env.crypto()
        .sha256(&usage_commitment_preimage_for_version(
            env,
            input,
            interface_version,
        ))
        .to_bytes()
}

fn session_id_preimage_for_version(
    env: &Env,
    input: &SessionIdInput<'_>,
    interface_version: u32,
) -> Bytes {
    let domain = domain_preimage(env, SESSION_ID_DOMAIN, interface_version);
    let payload = (
        input.buyer.clone(),
        input.seller.clone(),
        input.asset.clone(),
        input.max_amount,
        input.expires_at_ledger,
        input.resource_hash.clone(),
        input.sequence,
    )
        .to_xdr(env);

    (domain, payload).to_xdr(env)
}

fn usage_commitment_preimage_for_version(
    env: &Env,
    input: &UsageCommitmentInput<'_>,
    interface_version: u32,
) -> Bytes {
    let domain = domain_preimage(env, USAGE_COMMITMENT_DOMAIN, interface_version);
    let payload = (
        input.receipt_schema_version,
        input.session_id.clone(),
        input.actual_amount,
        input.resource_hash.clone(),
        input.measurement_hash.clone(),
    )
        .to_xdr(env);

    (domain, payload).to_xdr(env)
}

fn domain_preimage(env: &Env, label: &[u8], interface_version: u32) -> Bytes {
    (
        Bytes::from_slice(env, label),
        interface_version,
        env.ledger().network_id(),
        env.current_contract_address(),
    )
        .to_xdr(env)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address,
    };
    use std::string::ToString as _;

    #[test]
    fn sequence_changes_session_id_for_identical_inputs() {
        let env = Env::default();
        let contract = Address::generate(&env);
        env.register_at(&contract, crate::UptoSessionContract, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let resource_hash = BytesN::from_array(&env, &[9; 32]);

        let (first, second) = env.as_contract(&contract, || {
            (
                derive_session_id(
                    &env,
                    &SessionIdInput {
                        buyer: &buyer,
                        seller: &seller,
                        asset: &asset,
                        max_amount: 100,
                        expires_at_ledger: 50,
                        resource_hash: &resource_hash,
                        sequence: 0,
                    },
                ),
                derive_session_id(
                    &env,
                    &SessionIdInput {
                        buyer: &buyer,
                        seller: &seller,
                        asset: &asset,
                        max_amount: 100,
                        expires_at_ledger: 50,
                        resource_hash: &resource_hash,
                        sequence: 1,
                    },
                ),
            )
        });

        assert_ne!(first, second);
    }

    #[test]
    fn changing_bound_fields_changes_session_id() {
        let env = Env::default();
        let contract = Address::generate(&env);
        env.register_at(&contract, crate::UptoSessionContract, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let other_seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let resource_hash = BytesN::from_array(&env, &[9; 32]);

        let (first, second) = env.as_contract(&contract, || {
            (
                derive_session_id(
                    &env,
                    &SessionIdInput {
                        buyer: &buyer,
                        seller: &seller,
                        asset: &asset,
                        max_amount: 100,
                        expires_at_ledger: 50,
                        resource_hash: &resource_hash,
                        sequence: 0,
                    },
                ),
                derive_session_id(
                    &env,
                    &SessionIdInput {
                        buyer: &buyer,
                        seller: &other_seller,
                        asset: &asset,
                        max_amount: 100,
                        expires_at_ledger: 50,
                        resource_hash: &resource_hash,
                        sequence: 0,
                    },
                ),
            )
        });

        assert_ne!(first, second);
    }

    #[test]
    fn domains_prevent_network_contract_and_version_reuse() {
        let env = Env::default();
        env.ledger().set_network_id([7; 32]);
        let first_contract = Address::generate(&env);
        let second_contract = Address::generate(&env);
        env.register_at(&first_contract, crate::UptoSessionContract, ());
        env.register_at(&second_contract, crate::UptoSessionContract, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let resource_hash = BytesN::from_array(&env, &[9; 32]);
        let input = SessionIdInput {
            buyer: &buyer,
            seller: &seller,
            asset: &asset,
            max_amount: 100,
            expires_at_ledger: 50,
            resource_hash: &resource_hash,
            sequence: 0,
        };

        let first = env.as_contract(&first_contract, || derive_session_id(&env, &input));
        let other_contract = env.as_contract(&second_contract, || derive_session_id(&env, &input));
        assert_ne!(first, other_contract);

        let old_version = env.as_contract(&first_contract, || {
            derive_session_id_for_version(&env, &input, crate::INTERFACE_VERSION - 1)
        });
        assert_ne!(first, old_version);

        env.ledger().set_network_id([8; 32]);
        let other_network = env.as_contract(&first_contract, || derive_session_id(&env, &input));
        assert_ne!(first, other_network);
    }

    #[test]
    fn field_boundaries_and_usage_domains_are_unambiguous() {
        let env = Env::default();
        env.ledger().set_network_id([7; 32]);
        let contract = Address::generate(&env);
        env.register_at(&contract, crate::UptoSessionContract, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let resource_hash = BytesN::from_array(&env, &[9; 32]);
        let measurement_hash = BytesN::from_array(&env, &[10; 32]);
        let first_input = SessionIdInput {
            buyer: &buyer,
            seller: &seller,
            asset: &asset,
            max_amount: 1,
            expires_at_ledger: 23,
            resource_hash: &resource_hash,
            sequence: 4,
        };
        let second_input = SessionIdInput {
            max_amount: 12,
            expires_at_ledger: 3,
            ..first_input
        };
        let (first, second, usage, old_usage) = env.as_contract(&contract, || {
            let first = derive_session_id(&env, &first_input);
            let second = derive_session_id(&env, &second_input);
            let usage_input = UsageCommitmentInput {
                session_id: &first,
                actual_amount: 1,
                resource_hash: &resource_hash,
                measurement_hash: &measurement_hash,
                receipt_schema_version: 1,
            };
            (
                first.clone(),
                second,
                derive_usage_commitment(&env, &usage_input),
                derive_usage_commitment_for_version(
                    &env,
                    &usage_input,
                    crate::INTERFACE_VERSION - 1,
                ),
            )
        });
        assert_ne!(first, second);
        assert_ne!(usage, old_usage);
        assert_ne!(first, usage);
    }

    #[test]
    fn domain_compatibility_vectors_match_artifact() {
        let env = Env::default();
        env.ledger().set_network_id([7; 32]);
        let contract = Address::generate(&env);
        env.register_at(&contract, crate::UptoSessionContract, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let asset = Address::generate(&env);
        let resource_hash = BytesN::from_array(&env, &[9; 32]);
        let measurement_hash = BytesN::from_array(
            &env,
            &[
                0x67, 0x46, 0x38, 0x31, 0x87, 0x29, 0xbd, 0x03, 0x42, 0x4e, 0xaf, 0x36, 0x4a, 0xcb,
                0xeb, 0x12, 0x74, 0xd2, 0x05, 0xce, 0x27, 0x11, 0x25, 0x73, 0x7e, 0x47, 0x4c, 0x1e,
                0x50, 0x7c, 0x63, 0xc2,
            ],
        );
        env.as_contract(&contract, || {
            let session_input = SessionIdInput {
                buyer: &buyer,
                seller: &seller,
                asset: &asset,
                max_amount: 420_000_000,
                expires_at_ledger: 5_067_600,
                resource_hash: &resource_hash,
                sequence: 7,
            };
            let session_id = derive_session_id(&env, &session_input);
            let usage_input = UsageCommitmentInput {
                session_id: &session_id,
                actual_amount: 210_000_000,
                resource_hash: &resource_hash,
                measurement_hash: &measurement_hash,
                receipt_schema_version: 1,
            };
            let fixture: serde_json::Value = serde_json::from_str(include_str!(
                "../../../artifacts/backend-handoff/domain-separation-v3.json"
            ))
            .expect("domain fixture must decode");
            assert_eq!(fixture["contractId"], contract.to_string().to_string());
            assert_eq!(fixture["sessionId"]["buyer"], buyer.to_string().to_string());
            assert_eq!(
                fixture["sessionId"]["seller"],
                seller.to_string().to_string()
            );
            assert_eq!(fixture["sessionId"]["asset"], asset.to_string().to_string());
            assert_eq!(
                fixture["sessionId"]["preimageXdrHex"],
                hex_bytes(&session_id_preimage(&env, &session_input))
            );
            assert_eq!(
                fixture["sessionId"]["sha256"],
                hex_array(session_id.to_array())
            );
            assert_eq!(
                fixture["usageCommitment"]["preimageXdrHex"],
                hex_bytes(&usage_commitment_preimage(&env, &usage_input))
            );
            assert_eq!(
                fixture["usageCommitment"]["sha256"],
                hex_array(derive_usage_commitment(&env, &usage_input).to_array())
            );
        });
    }

    fn hex_bytes(value: &Bytes) -> std::string::String {
        value
            .iter()
            .map(|byte| std::format!("{byte:02x}"))
            .collect()
    }

    fn hex_array(value: impl AsRef<[u8]>) -> std::string::String {
        value
            .as_ref()
            .iter()
            .map(|byte| std::format!("{byte:02x}"))
            .collect()
    }
}
