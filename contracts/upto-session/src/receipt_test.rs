extern crate std;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::string::String;
use std::vec::Vec;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    vectors: Vec<Vector>,
    invalid_encodings: Vec<InvalidEncoding>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vector {
    name: String,
    receipt: UsageReceipt,
    canonical_utf8: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct InvalidEncoding {
    name: String,
    json: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UsageReceipt {
    schema_version: u32,
    protocol: String,
    interface_version: u32,
    network_passphrase: String,
    contract_id: String,
    session_id: String,
    amount: String,
    resource_hash: String,
    measurement: Measurement,
    receipt_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Measurement {
    unit: String,
    quantity: String,
    started_at_ledger: u32,
    ended_at_ledger: u32,
}

#[test]
fn rust_usage_receipt_vectors_match_canonical_hashes() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../artifacts/backend-handoff/usage-receipt-v1.fixtures.json"
    ))
    .expect("fixture must decode");

    for vector in fixture.vectors {
        let canonical = serde_json::to_string(&vector.receipt).expect("receipt must encode");
        assert_eq!(canonical, vector.canonical_utf8, "{}", vector.name);
        assert_eq!(
            hex::encode(Sha256::digest(canonical.as_bytes())),
            vector.sha256,
            "{}",
            vector.name
        );
    }
}

#[test]
fn rust_rejects_ambiguous_or_noncanonical_encodings() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../artifacts/backend-handoff/usage-receipt-v1.fixtures.json"
    ))
    .expect("fixture must decode");

    for invalid in fixture.invalid_encodings {
        let accepted = serde_json::from_str::<UsageReceipt>(&invalid.json)
            .ok()
            .and_then(|receipt| serde_json::to_string(&receipt).ok())
            .is_some_and(|canonical| canonical == invalid.json);
        assert!(!accepted, "{} unexpectedly decoded", invalid.name);
    }
}
