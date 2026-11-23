//! Cross-repository encoding compatibility vectors.
//!
//! Rust is the source of truth. This module derives every vector from the
//! contract's own encoding code (`ids.rs`, `events.rs`, `validation.rs`) and
//! the Soroban XDR serializer, then checks the committed artifact:
//!
//! - `encoding_vectors_match_committed_artifact` regenerates the vectors in
//!   memory and fails when `artifacts/backend-handoff/encoding-vectors.v<N>.json`
//!   is stale. Regenerate intentionally with
//!   `LUMENBAZAAR_UPDATE_ENCODING_VECTORS=1 cargo test -p upto-session --all-features encoding_vectors`.
//! - `encoding_vectors_are_pinned_by_compatibility_lock` fails when the
//!   committed file no longer matches the checksum pinned for its version in
//!   `encoding-vectors.lock.json`, or when the changelog has no entry for it.
//!   Changing an existing vector therefore requires a new version, a new file,
//!   a lock entry and a changelog entry (see `artifacts/backend-handoff/README.md`).
//!
//! All inputs are synthetic: repeated-byte public keys and contract IDs with no
//! known secret material, plus the public testnet passphrase.

extern crate std;

use crate::{
    events,
    ids::{self, SessionIdInput, UsageCommitmentInput},
    validation, ContractError, Session, SessionStatus, UptoSessionContract, INTERFACE_VERSION,
    RECEIPT_SCHEMA_VERSION,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use soroban_sdk::{
    testutils::{Events as _, Ledger as _},
    xdr::{self, Limits, ScAddress, ScVal, ToXdr, WriteXdr},
    Address, Bytes, BytesN, Env, String as SorobanString,
};
use std::{
    borrow::ToOwned,
    format,
    path::PathBuf,
    str::FromStr,
    string::{String, ToString},
    vec,
    vec::Vec,
};

/// Version of the published vector set. Bump it (and add a new lock and
/// changelog entry) whenever any existing vector or encoding rule changes.
const ENCODING_VECTORS_VERSION: u32 = 1;
const UPDATE_ENV: &str = "LUMENBAZAAR_UPDATE_ENCODING_VECTORS";

const TESTNET_PASSPHRASE: &str = "Test SDF Network ; September 2015";
const CONTRACT_ID: &str = "CDA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4CFV6";
const BUYER: &str = "GAIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCF6M";
const SELLER: &str = "GARCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCFRVX";
const OTHER_ACCOUNT: &str = "GAZTGMZTGMZTGMZTGMZTGMZTGMZTGMZTGMZTGMZTGMZTGMZTGMZTHCM6";
const ASSET: &str = "CCS2LJNFUWS2LJNFUWS2LJNFUWS2LJNFUWS2LJNFUWS2LJNFUWS2LSSM";
const CONTRACT_SELLER: &str = "CBOFYXC4LROFYXC4LROFYXC4LROFYXC4LROFYXC4LROFYXC4LROFZ7GH";
const MUXED_ACCOUNT: &str = "MBCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIAAAAAAAAAAAA4WDM";

/// Backend policy for network passphrases supplied to the TypeScript helpers.
/// The contract itself only ever sees `sha256(passphrase_utf8)`.
const MAX_PASSPHRASE_UTF16_UNITS: usize = 256;
const MAX_RECEIPT_ID_CHARS: usize = 128;
const MAX_UNIT_CHARS: usize = 64;
/// Ledger used as "current" for create-session validation vectors.
const CURRENT_LEDGER: u32 = 1_000;

#[test]
fn encoding_vectors_match_committed_artifact() {
    let generated = render(&generate());
    let path = vectors_path(ENCODING_VECTORS_VERSION);
    if std::env::var(UPDATE_ENV).as_deref() == Ok("1") {
        std::fs::write(&path, &generated).expect("vector artifact must be writable");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing {}; run `{UPDATE_ENV}=1 cargo test -p upto-session --all-features encoding_vectors`",
            path.display()
        )
    });
    assert!(
        committed == generated,
        "{} is stale; the contract encoding or the generator changed. If the change is \
         intentional, follow the compatibility process in artifacts/backend-handoff/README.md",
        path.display()
    );
}

#[test]
fn encoding_vectors_are_pinned_by_compatibility_lock() {
    let dir = handoff_dir();
    let lock: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("encoding-vectors.lock.json"))
            .expect("compatibility lock must exist"),
    )
    .expect("compatibility lock must decode");
    assert_eq!(
        lock["current"],
        json!(ENCODING_VECTORS_VERSION),
        "lock `current` must equal the generator's ENCODING_VECTORS_VERSION"
    );
    let entry = &lock["versions"][ENCODING_VECTORS_VERSION.to_string()];
    let file = format!("encoding-vectors.v{ENCODING_VECTORS_VERSION}.json");
    assert_eq!(entry["file"], json!(file), "lock entry must name {file}");
    let bytes = std::fs::read(dir.join(&file)).expect("vector artifact must exist");
    assert_eq!(
        entry["sha256"],
        json!(hex::encode(Sha256::digest(&bytes))),
        "{file} changed without a compatibility update: existing versions are immutable. \
         Bump ENCODING_VECTORS_VERSION, write a new file, and add lock and changelog entries."
    );
    let changelog = std::fs::read_to_string(dir.join("encoding-vectors.CHANGELOG.md"))
        .expect("encoding vector changelog must exist");
    assert!(
        changelog
            .lines()
            .any(|line| line.trim() == format!("## v{ENCODING_VECTORS_VERSION}")),
        "encoding-vectors.CHANGELOG.md must contain a `## v{ENCODING_VECTORS_VERSION}` entry"
    );
}

#[test]
fn encoding_vectors_generation_is_deterministic() {
    assert_eq!(render(&generate()), render(&generate()));
}

fn handoff_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/backend-handoff")
}

fn vectors_path(version: u32) -> PathBuf {
    handoff_dir().join(format!("encoding-vectors.v{version}.json"))
}

fn render(value: &Value) -> String {
    let mut out = serde_json::to_string_pretty(value).expect("vectors must serialize");
    out.push('\n');
    out
}

fn generate() -> Value {
    let env = Env::default();
    env.ledger().set_sequence_number(CURRENT_LEDGER);
    let contract = address(&env, CONTRACT_ID);
    env.register_at(&contract, UptoSessionContract, ());

    let session_ids = session_id_vectors(&env, &contract);
    let first_session_id = session_ids[0]["sessionIdHex"]
        .as_str()
        .expect("session id")
        .to_owned();

    json!({
        "schema": "lumenbazaar/encoding-vectors",
        "version": ENCODING_VECTORS_VERSION,
        "interfaceVersion": INTERFACE_VERSION,
        "generator": "contracts/upto-session/src/encoding_vectors_test.rs",
        "regenerate": format!(
            "{UPDATE_ENV}=1 cargo test -p upto-session --all-features encoding_vectors"
        ),
        "synthetic": "All addresses are repeated-byte synthetic strkeys with no known secret key. \
                      The only real network value is the public testnet passphrase.",
        "rules": rules(),
        "context": {
            "contractId": CONTRACT_ID,
            "buyer": BUYER,
            "seller": SELLER,
            "asset": ASSET,
            "currentLedger": CURRENT_LEDGER,
            "maxSessionDurationLedgers": validation::MAX_SESSION_DURATION_LEDGERS,
        },
        "networkPassphrases": passphrase_vectors(&env),
        "sessionIds": session_ids,
        "usageCommitments": usage_commitment_vectors(&env, &contract, &first_session_id),
        "usageReceipts": usage_receipt_vectors(&first_session_id),
        "addresses": address_vectors(&env),
        "amounts": amount_vectors(&env, &contract),
        "resourceHashes": resource_hash_vectors(&env, &contract),
        "sessionExpiry": expiry_vectors(&env, &contract),
        "events": event_vectors(&env, &contract),
    })
}

fn rules() -> Value {
    json!([
        "All XDR is the Soroban ScVal XDR produced by soroban-sdk `to_xdr`, lowercase hex.",
        "Session ID = sha256(ScVal Vec[Bytes(domain_xdr), Bytes(payload_xdr)]) where domain = \
         Vec[Bytes(\"lumenbazaar/session-id\"), U32(interface_version), Bytes(network_id), \
         Address(contract)] and payload = Vec[Address(buyer), Address(seller), Address(asset), \
         I128(max_amount), U32(expires_at_ledger), Bytes(resource_hash), U64(sequence), \
         Bytes(idempotency_key)].",
        "Usage commitment = sha256(ScVal Vec[Bytes(domain_xdr), Bytes(payload_xdr)]) where domain \
         uses the label \"lumenbazaar/usage-commitment\" and payload = Vec[U32(receipt_schema_version), \
         Bytes(session_id), I128(actual_amount), Bytes(resource_hash), Bytes(measurement_hash)].",
        "network_id = sha256(UTF-8 bytes of the network passphrase). No Unicode normalization is \
         applied when hashing: NFC and NFD forms of the same visible text produce different \
         network IDs, so producers must reject non-NFC input instead of normalizing it.",
        "Backend passphrase policy: non-empty, NFC, no leading or trailing whitespace, at most 256 \
         UTF-16 code units (JavaScript string length).",
        "measurement_hash = sha256(canonical usage receipt v1 UTF-8 bytes). Non-ASCII characters are \
         emitted as raw UTF-8, never as \\u escapes.",
        "Amounts are canonical base-10 strings matching ^(0|-?[1-9][0-9]*)$ that fit i128. \
         create_session additionally requires max_amount > 0 (ContractError::InvalidAmount).",
        "Resource hashes are exactly 32 bytes of lowercase hex. The all-zero hash is encodable but \
         rejected by create_session (ContractError::InvalidResourceHash).",
        "Contract-facing addresses are G (account) or C (contract) strkeys. Muxed M addresses are \
         not accepted as Address arguments.",
        "Events use #[contractevent] defaults: topics = [Symbol(snake_case event name), indexed \
         fields...], data = Map of the remaining fields keyed by Symbol in ascending key order.",
    ])
}

fn address(env: &Env, strkey: &str) -> Address {
    Address::from_string(&SorobanString::from_str(env, strkey))
}

fn hex_bytes(value: &Bytes) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn xdr_hex(value: &ScVal) -> String {
    hex::encode(WriteXdr::to_xdr(value, Limits::none()).expect("ScVal must encode"))
}

fn network_id(env: &Env, passphrase: &str) -> [u8; 32] {
    env.crypto()
        .sha256(&Bytes::from_slice(env, passphrase.as_bytes()))
        .to_array()
}

fn contract_error(error: ContractError) -> Value {
    json!({ "name": format!("{error:?}"), "code": error as u32 })
}

/// Backend-side passphrase checks that Rust can evaluate exactly. NFC
/// membership is declared per vector because the contract crate carries no
/// Unicode normalization tables; the TypeScript verifier checks it.
fn passphrase_error(passphrase: &str, is_nfc: bool) -> Option<&'static str> {
    if passphrase.is_empty() {
        Some("empty")
    } else if passphrase.trim() != passphrase {
        Some("surrounding_whitespace")
    } else if passphrase.encode_utf16().count() > MAX_PASSPHRASE_UTF16_UNITS {
        Some("too_long")
    } else if !is_nfc {
        Some("not_nfc")
    } else {
        None
    }
}

fn passphrase_cases() -> Vec<(&'static str, String, bool)> {
    vec![
        ("public-testnet", TESTNET_PASSPHRASE.to_owned(), true),
        (
            "latin-precomposed-nfc",
            "R\u{e9}seau synth\u{e9}tique ; 2026".to_owned(),
            true,
        ),
        (
            "latin-decomposed-nfd",
            "Re\u{301}seau synthe\u{301}tique ; 2026".to_owned(),
            false,
        ),
        (
            "cjk-multibyte",
            "\u{5408}\u{6210}\u{30c6}\u{30b9}\u{30c8}\u{30cd}\u{30c3}\u{30c8} ; 2026".to_owned(),
            true,
        ),
        (
            "emoji-astral-with-variation-selector",
            "Synthetic \u{1f6f0}\u{fe0f} network ; 2026".to_owned(),
            true,
        ),
        (
            "combining-marks-canonical-order",
            "q\u{323}\u{307} synthetic ; 2026".to_owned(),
            true,
        ),
        (
            "combining-marks-noncanonical-order",
            "q\u{307}\u{323} synthetic ; 2026".to_owned(),
            false,
        ),
        ("empty", String::new(), true),
        ("leading-whitespace", format!(" {TESTNET_PASSPHRASE}"), true),
        ("max-length-256-ascii", "a".repeat(256), true),
        ("below-max-255-ascii", "a".repeat(255), true),
        ("over-max-257-ascii", "a".repeat(257), true),
        (
            "max-length-256-utf16-units-of-2-byte-utf8",
            "\u{e9}".repeat(256),
            true,
        ),
        ("max-length-128-astral-emoji", "\u{1f600}".repeat(128), true),
        (
            "over-max-255-ascii-plus-astral-emoji",
            format!("{}\u{1f600}", "a".repeat(255)),
            true,
        ),
    ]
}

fn passphrase_vectors(env: &Env) -> Value {
    let vectors: Vec<Value> = passphrase_cases()
        .into_iter()
        .map(|(name, passphrase, is_nfc)| {
            let error = passphrase_error(&passphrase, is_nfc);
            let mut vector = Map::new();
            vector.insert("name".into(), json!(name));
            vector.insert("passphrase".into(), json!(passphrase));
            vector.insert("utf8Hex".into(), json!(hex::encode(passphrase.as_bytes())));
            vector.insert("utf8Bytes".into(), json!(passphrase.len()));
            vector.insert(
                "utf16Units".into(),
                json!(passphrase.encode_utf16().count()),
            );
            vector.insert("nfc".into(), json!(is_nfc));
            vector.insert("accepted".into(), json!(error.is_none()));
            if let Some(error) = error {
                vector.insert("error".into(), json!(error));
            }
            if !passphrase.is_empty() {
                vector.insert(
                    "networkIdHex".into(),
                    json!(hex::encode(network_id(env, &passphrase))),
                );
            }
            Value::Object(vector)
        })
        .collect();
    Value::Array(vectors)
}

struct SessionCase {
    name: &'static str,
    passphrase: Option<String>,
    network_id: [u8; 32],
    buyer: &'static str,
    seller: &'static str,
    max_amount: i128,
    expires_at_ledger: u32,
    resource_hash: [u8; 32],
    sequence: u64,
    idempotency_key: [u8; 32],
}

fn session_cases(env: &Env) -> Vec<SessionCase> {
    let testnet = network_id(env, TESTNET_PASSPHRASE);
    let base = || SessionCase {
        name: "",
        passphrase: Some(TESTNET_PASSPHRASE.to_owned()),
        network_id: testnet,
        buyer: BUYER,
        seller: SELLER,
        max_amount: 420_000_000,
        expires_at_ledger: 5_067_600,
        resource_hash: [0x09; 32],
        sequence: 7,
        idempotency_key: [0x08; 32],
    };
    let nfc = "R\u{e9}seau synth\u{e9}tique ; 2026".to_owned();
    let nfd = "Re\u{301}seau synthe\u{301}tique ; 2026".to_owned();
    vec![
        SessionCase {
            name: "baseline-testnet",
            ..base()
        },
        SessionCase {
            name: "sequence-zero",
            sequence: 0,
            ..base()
        },
        SessionCase {
            name: "sequence-u64-max",
            sequence: u64::MAX,
            ..base()
        },
        SessionCase {
            name: "max-amount-one",
            max_amount: 1,
            ..base()
        },
        SessionCase {
            name: "max-amount-above-js-safe-integer",
            max_amount: 9_007_199_254_740_993,
            ..base()
        },
        SessionCase {
            name: "max-amount-i128-max",
            max_amount: i128::MAX,
            ..base()
        },
        SessionCase {
            name: "expires-at-u32-max",
            expires_at_ledger: u32::MAX,
            ..base()
        },
        SessionCase {
            name: "resource-hash-all-ff",
            resource_hash: [0xff; 32],
            ..base()
        },
        SessionCase {
            name: "idempotency-key-all-zero",
            idempotency_key: [0x00; 32],
            ..base()
        },
        SessionCase {
            name: "contract-seller-address",
            seller: CONTRACT_SELLER,
            ..base()
        },
        SessionCase {
            name: "network-passphrase-nfc",
            network_id: network_id(env, &nfc),
            passphrase: Some(nfc),
            ..base()
        },
        // Same visible passphrase in NFD: hashing the raw bytes yields a
        // different network ID and therefore a different session ID. The
        // passphrase itself is rejected by the backend policy, so the vector
        // carries only the network ID.
        SessionCase {
            name: "network-id-from-nfd-bytes",
            network_id: network_id(env, &nfd),
            passphrase: None,
            ..base()
        },
    ]
}

fn session_id_vectors(env: &Env, contract: &Address) -> Vec<Value> {
    session_cases(env)
        .into_iter()
        .map(|case| {
            env.ledger().set_network_id(case.network_id);
            let buyer = address(env, case.buyer);
            let seller = address(env, case.seller);
            let asset = address(env, ASSET);
            let resource_hash = BytesN::from_array(env, &case.resource_hash);
            let idempotency_key = BytesN::from_array(env, &case.idempotency_key);
            let input = SessionIdInput {
                buyer: &buyer,
                seller: &seller,
                asset: &asset,
                max_amount: case.max_amount,
                expires_at_ledger: case.expires_at_ledger,
                resource_hash: &resource_hash,
                sequence: case.sequence,
                idempotency_key: &idempotency_key,
            };
            let (preimage, session_id) = env.as_contract(contract, || {
                (
                    ids::session_id_preimage(env, &input),
                    ids::derive_session_id(env, &input),
                )
            });
            let mut vector = Map::new();
            vector.insert("name".into(), json!(case.name));
            if let Some(passphrase) = case.passphrase {
                vector.insert("networkPassphrase".into(), json!(passphrase));
            }
            vector.insert("networkIdHex".into(), json!(hex::encode(case.network_id)));
            vector.insert("contractId".into(), json!(CONTRACT_ID));
            vector.insert("buyer".into(), json!(case.buyer));
            vector.insert("seller".into(), json!(case.seller));
            vector.insert("asset".into(), json!(ASSET));
            vector.insert("maxAmount".into(), json!(case.max_amount.to_string()));
            vector.insert("expiresAtLedger".into(), json!(case.expires_at_ledger));
            vector.insert(
                "resourceHashHex".into(),
                json!(hex::encode(case.resource_hash)),
            );
            vector.insert("sequence".into(), json!(case.sequence.to_string()));
            vector.insert(
                "idempotencyKeyHex".into(),
                json!(hex::encode(case.idempotency_key)),
            );
            vector.insert("preimageXdrHex".into(), json!(hex_bytes(&preimage)));
            vector.insert(
                "sessionIdHex".into(),
                json!(hex::encode(session_id.to_array())),
            );
            Value::Object(vector)
        })
        .collect()
}

fn usage_commitment_vectors(env: &Env, contract: &Address, session_id_hex: &str) -> Value {
    env.ledger()
        .set_network_id(network_id(env, TESTNET_PASSPHRASE));
    let mut session_id = [0u8; 32];
    hex::decode_to_slice(session_id_hex, &mut session_id).expect("session id hex");
    let measurement =
        Sha256::digest(canonical_receipt(&baseline_receipt(session_id_hex)).as_bytes());
    let cases: [(&str, i128, [u8; 32]); 3] = [
        ("actual-amount-one", 1, measurement.into()),
        ("actual-amount-typical", 210_000_000, measurement.into()),
        ("actual-amount-i128-max", i128::MAX, [0x5a; 32]),
    ];
    let vectors: Vec<Value> = cases
        .iter()
        .map(|(name, actual_amount, measurement_hash)| {
            let session_id = BytesN::from_array(env, &session_id);
            let resource_hash = BytesN::from_array(env, &[0x09; 32]);
            let measurement_hash = BytesN::from_array(env, measurement_hash);
            let input = UsageCommitmentInput {
                session_id: &session_id,
                actual_amount: *actual_amount,
                resource_hash: &resource_hash,
                measurement_hash: &measurement_hash,
                receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            };
            let (preimage, commitment) = env.as_contract(contract, || {
                (
                    ids::usage_commitment_preimage(env, &input),
                    ids::derive_usage_commitment(env, &input),
                )
            });
            json!({
                "name": name,
                "networkPassphrase": TESTNET_PASSPHRASE,
                "networkIdHex": hex::encode(network_id(env, TESTNET_PASSPHRASE)),
                "contractId": CONTRACT_ID,
                "receiptSchemaVersion": RECEIPT_SCHEMA_VERSION,
                "sessionIdHex": hex::encode(session_id.to_array()),
                "actualAmount": actual_amount.to_string(),
                "resourceHashHex": hex::encode([0x09u8; 32]),
                "measurementHashHex": hex::encode(measurement_hash.to_array()),
                "preimageXdrHex": hex_bytes(&preimage),
                "usageHashHex": hex::encode(commitment.to_array()),
            })
        })
        .collect();
    Value::Array(vectors)
}

/// Field order is the canonical usage receipt v1 order.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Measurement {
    unit: String,
    quantity: String,
    started_at_ledger: u32,
    ended_at_ledger: u32,
}

fn baseline_receipt(session_id_hex: &str) -> UsageReceipt {
    UsageReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        protocol: "lumenbazaar".into(),
        interface_version: INTERFACE_VERSION,
        network_passphrase: TESTNET_PASSPHRASE.into(),
        contract_id: CONTRACT_ID.into(),
        session_id: session_id_hex.into(),
        amount: "210000000".into(),
        resource_hash: hex::encode([0x09u8; 32]),
        measurement: Measurement {
            unit: "request".into(),
            quantity: "42".into(),
            started_at_ledger: 5_067_500,
            ended_at_ledger: 5_067_542,
        },
        receipt_id: "usage_synthetic_0001".into(),
    }
}

fn canonical_receipt(receipt: &UsageReceipt) -> String {
    serde_json::to_string(receipt).expect("receipt must encode")
}

fn usage_receipt_vectors(session_id_hex: &str) -> Value {
    let base = || baseline_receipt(session_id_hex);
    let with = |f: &dyn Fn(&mut UsageReceipt)| {
        let mut receipt = base();
        f(&mut receipt);
        receipt
    };
    let valid: Vec<(&str, UsageReceipt)> = vec![
        ("baseline-ascii", base()),
        (
            "passphrase-nfc-latin",
            with(&|r| r.network_passphrase = "R\u{e9}seau synth\u{e9}tique ; 2026".into()),
        ),
        (
            "passphrase-emoji",
            with(&|r| r.network_passphrase = "Synthetic \u{1f6f0}\u{fe0f} network ; 2026".into()),
        ),
        (
            "receipt-id-max-128",
            with(&|r| r.receipt_id = format!("r{}", "x".repeat(MAX_RECEIPT_ID_CHARS - 1))),
        ),
        (
            "receipt-id-below-max-127",
            with(&|r| r.receipt_id = format!("r{}", "x".repeat(MAX_RECEIPT_ID_CHARS - 2))),
        ),
        (
            "unit-max-64",
            with(&|r| r.measurement.unit = format!("u{}", "x".repeat(MAX_UNIT_CHARS - 1))),
        ),
        ("amount-one", with(&|r| r.amount = "1".into())),
        (
            "amount-i128-max",
            with(&|r| r.amount = i128::MAX.to_string()),
        ),
    ];
    let invalid: Vec<(&str, &str, UsageReceipt)> = vec![
        (
            "passphrase-nfd-latin",
            "not_nfc",
            with(&|r| r.network_passphrase = "Re\u{301}seau synthe\u{301}tique ; 2026".into()),
        ),
        (
            "passphrase-leading-whitespace",
            "surrounding_whitespace",
            with(&|r| r.network_passphrase = format!(" {TESTNET_PASSPHRASE}")),
        ),
        (
            "passphrase-empty",
            "empty",
            with(&|r| r.network_passphrase = String::new()),
        ),
        (
            "receipt-id-empty",
            "empty",
            with(&|r| r.receipt_id = String::new()),
        ),
        (
            "receipt-id-over-max-129",
            "too_long",
            with(&|r| r.receipt_id = format!("r{}", "x".repeat(MAX_RECEIPT_ID_CHARS))),
        ),
        (
            "receipt-id-non-ascii",
            "invalid_characters",
            with(&|r| r.receipt_id = "usage_caf\u{e9}".into()),
        ),
        (
            "unit-over-max-65",
            "too_long",
            with(&|r| r.measurement.unit = format!("u{}", "x".repeat(MAX_UNIT_CHARS))),
        ),
        (
            "amount-zero",
            "non_positive",
            with(&|r| r.amount = "0".into()),
        ),
        (
            "amount-leading-zero",
            "non_canonical_decimal",
            with(&|r| r.amount = "0210000000".into()),
        ),
    ];

    let valid: Vec<Value> = valid
        .into_iter()
        .map(|(name, receipt)| {
            let canonical = canonical_receipt(&receipt);
            json!({
                "name": name,
                "receipt": serde_json::to_value(&receipt).expect("receipt value"),
                "canonicalUtf8": canonical,
                "canonicalUtf8Bytes": canonical.len(),
                "sha256": hex::encode(Sha256::digest(canonical.as_bytes())),
            })
        })
        .collect();
    let invalid: Vec<Value> = invalid
        .into_iter()
        .map(|(name, error, receipt)| {
            json!({
                "name": name,
                "error": error,
                "receipt": serde_json::to_value(&receipt).expect("receipt value"),
            })
        })
        .collect();
    json!({ "valid": valid, "invalid": invalid })
}

fn address_vectors(env: &Env) -> Value {
    let cases: [(&str, &str); 9] = [
        ("account-g", BUYER),
        ("account-g-other", OTHER_ACCOUNT),
        ("contract-c", CONTRACT_ID),
        ("contract-c-asset", ASSET),
        ("muxed-m", MUXED_ACCOUNT),
        (
            "bad-checksum",
            "GAIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCF6A",
        ),
        (
            "lowercase",
            "gairceirceirceirceirceirceirceirceirceirceirceirceircf6m",
        ),
        (
            "truncated-55-chars",
            "GAIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCF6",
        ),
        ("empty", ""),
    ];
    let vectors: Vec<Value> = cases
        .iter()
        .map(|(name, strkey)| {
            let mut vector = Map::new();
            vector.insert("name".into(), json!(name));
            vector.insert("strkey".into(), json!(strkey));
            match ScAddress::from_str(strkey) {
                Ok(sc_address @ (ScAddress::Account(_) | ScAddress::Contract(_))) => {
                    let kind = if matches!(sc_address, ScAddress::Account(_)) {
                        "account"
                    } else {
                        "contract"
                    };
                    let expected = xdr_hex(&ScVal::Address(sc_address));
                    // The contract's own conversion must agree with the XDR type.
                    assert_eq!(hex_bytes(&address(env, strkey).to_xdr(env)), expected);
                    vector.insert("accepted".into(), json!(true));
                    vector.insert("kind".into(), json!(kind));
                    vector.insert("scValXdrHex".into(), json!(expected));
                }
                Ok(_) => {
                    vector.insert("accepted".into(), json!(false));
                    vector.insert("error".into(), json!("unsupported_muxed_address"));
                }
                Err(_) => {
                    vector.insert("accepted".into(), json!(false));
                    vector.insert("error".into(), json!("invalid_strkey"));
                }
            }
            Value::Object(vector)
        })
        .collect();
    Value::Array(vectors)
}

fn classify_amount(text: &str) -> Result<i128, &'static str> {
    if text.is_empty() {
        return Err("empty");
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    let canonical = text == "0"
        || (!digits.is_empty()
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && !digits.starts_with('0'));
    if !canonical {
        return Err("non_canonical_decimal");
    }
    text.parse::<i128>().map_err(|_| "out_of_range_i128")
}

fn create_session_result(
    env: &Env,
    contract: &Address,
    max_amount: i128,
    expires_at_ledger: u32,
    resource_hash: [u8; 32],
) -> Result<(), ContractError> {
    let buyer = address(env, BUYER);
    let seller = address(env, SELLER);
    let asset = address(env, ASSET);
    let resource_hash = BytesN::from_array(env, &resource_hash);
    env.as_contract(contract, || {
        validation::validate_create_session(
            env,
            &buyer,
            &seller,
            &asset,
            max_amount,
            expires_at_ledger,
            &resource_hash,
        )
    })
}

fn insert_create_session(vector: &mut Map<String, Value>, result: Result<(), ContractError>) {
    match result {
        Ok(()) => {
            vector.insert("createSessionAccepted".into(), json!(true));
        }
        Err(error) => {
            vector.insert("createSessionAccepted".into(), json!(false));
            vector.insert("contractError".into(), contract_error(error));
        }
    }
}

fn amount_vectors(env: &Env, contract: &Address) -> Value {
    let cases: Vec<(&str, String)> = vec![
        ("one", "1".into()),
        ("js-max-safe-integer", "9007199254740991".into()),
        ("above-js-max-safe-integer", "9007199254740993".into()),
        ("i128-max", i128::MAX.to_string()),
        ("zero", "0".into()),
        ("negative-one", "-1".into()),
        ("i128-min", i128::MIN.to_string()),
        (
            "i128-max-plus-one",
            "170141183460469231731687303715884105728".into(),
        ),
        (
            "i128-min-minus-one",
            "-170141183460469231731687303715884105729".into(),
        ),
        ("empty", String::new()),
        ("leading-zero", "01".into()),
        ("negative-zero", "-0".into()),
        ("explicit-plus", "+1".into()),
        ("decimal-point", "1.0".into()),
        ("exponent", "1e3".into()),
        ("leading-whitespace", " 1".into()),
    ];
    let vectors: Vec<Value> = cases
        .into_iter()
        .map(|(name, text)| {
            let mut vector = Map::new();
            vector.insert("name".into(), json!(name));
            vector.insert("decimal".into(), json!(text));
            match classify_amount(&text) {
                Ok(amount) => {
                    vector.insert("encodable".into(), json!(true));
                    vector.insert("scValXdrHex".into(), json!(hex_bytes(&amount.to_xdr(env))));
                    insert_create_session(
                        &mut vector,
                        create_session_result(
                            env,
                            contract,
                            amount,
                            CURRENT_LEDGER + 100,
                            [0x09; 32],
                        ),
                    );
                }
                Err(error) => {
                    vector.insert("encodable".into(), json!(false));
                    vector.insert("error".into(), json!(error));
                }
            }
            Value::Object(vector)
        })
        .collect();
    Value::Array(vectors)
}

fn classify_resource_hash(text: &str) -> Result<[u8; 32], &'static str> {
    if text.is_empty() {
        return Err("empty");
    }
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(if text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            "non_canonical_hex"
        } else {
            "invalid_hex"
        });
    }
    if !text.len().is_multiple_of(2) {
        return Err("invalid_hex");
    }
    if text.len() != 64 {
        return Err("invalid_length");
    }
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(text, &mut bytes).map_err(|_| "invalid_hex")?;
    Ok(bytes)
}

fn resource_hash_vectors(env: &Env, contract: &Address) -> Value {
    let cases: Vec<(&str, String)> = vec![
        ("repeated-09", "09".repeat(32)),
        ("all-ff", "ff".repeat(32)),
        ("lowest-nonzero", format!("{}01", "00".repeat(31))),
        ("all-zero", "00".repeat(32)),
        ("empty", String::new()),
        ("short-31-bytes", "09".repeat(31)),
        ("long-33-bytes", "09".repeat(33)),
        ("uppercase", "AB".repeat(32)),
        ("non-hex", format!("{}zz", "09".repeat(31))),
        ("odd-length", format!("{}0", "09".repeat(31))),
    ];
    let vectors: Vec<Value> = cases
        .into_iter()
        .map(|(name, text)| {
            let mut vector = Map::new();
            vector.insert("name".into(), json!(name));
            vector.insert("hex".into(), json!(text));
            match classify_resource_hash(&text) {
                Ok(bytes) => {
                    vector.insert("encodable".into(), json!(true));
                    vector.insert(
                        "scValXdrHex".into(),
                        json!(hex_bytes(&BytesN::from_array(env, &bytes).to_xdr(env))),
                    );
                    insert_create_session(
                        &mut vector,
                        create_session_result(
                            env,
                            contract,
                            420_000_000,
                            CURRENT_LEDGER + 100,
                            bytes,
                        ),
                    );
                }
                Err(error) => {
                    vector.insert("encodable".into(), json!(false));
                    vector.insert("error".into(), json!(error));
                }
            }
            Value::Object(vector)
        })
        .collect();
    Value::Array(vectors)
}

fn expiry_vectors(env: &Env, contract: &Address) -> Value {
    let max = validation::MAX_SESSION_DURATION_LEDGERS;
    let cases: [(&str, u32); 6] = [
        ("current-ledger", CURRENT_LEDGER),
        ("current-ledger-plus-one", CURRENT_LEDGER + 1),
        ("max-duration-minus-one", CURRENT_LEDGER + max - 1),
        ("max-duration", CURRENT_LEDGER + max),
        ("max-duration-plus-one", CURRENT_LEDGER + max + 1),
        ("u32-max", u32::MAX),
    ];
    let vectors: Vec<Value> = cases
        .iter()
        .map(|(name, expires_at_ledger)| {
            let mut vector = Map::new();
            vector.insert("name".into(), json!(name));
            vector.insert("currentLedger".into(), json!(CURRENT_LEDGER));
            vector.insert("expiresAtLedger".into(), json!(expires_at_ledger));
            insert_create_session(
                &mut vector,
                create_session_result(env, contract, 420_000_000, *expires_at_ledger, [0x09; 32]),
            );
            Value::Object(vector)
        })
        .collect();
    Value::Array(vectors)
}

fn typed(kind: &str, value: Value) -> Value {
    json!({ "type": kind, "value": value })
}

fn last_event(env: &Env, contract: &Address) -> (Vec<ScVal>, ScVal) {
    let events = env.events().all().filter_by_contract(contract);
    let event = events
        .events()
        .last()
        .expect("contract event must be emitted")
        .clone();
    match event.body {
        xdr::ContractEventBody::V0(body) => (body.topics.to_vec(), body.data),
    }
}

fn event_vector(
    env: &Env,
    contract: &Address,
    name: &str,
    flow: &str,
    expected_topics: Vec<Value>,
    expected_data: Value,
) -> Value {
    let (topics, data) = last_event(env, contract);
    assert_eq!(
        topics.first(),
        Some(&ScVal::Symbol(
            xdr::ScSymbol::try_from(name).expect("event name must be a symbol")
        )),
        "unexpected event name for {flow}"
    );
    json!({
        "name": name,
        "flow": flow,
        "topicsXdrHex": topics.iter().map(xdr_hex).collect::<Vec<_>>(),
        "dataXdrHex": xdr_hex(&data),
        "expected": { "topics": expected_topics, "data": expected_data },
    })
}

fn event_vectors(env: &Env, contract: &Address) -> Value {
    env.ledger()
        .set_network_id(network_id(env, TESTNET_PASSPHRASE));
    let session_id_bytes = [0x5e; 32];
    let session_id = BytesN::from_array(env, &session_id_bytes);
    let session_hex = hex::encode(session_id_bytes);
    let buyer = address(env, BUYER);
    let seller = address(env, SELLER);
    let asset = address(env, ASSET);
    let usage_hash = [0x0e; 32];
    let max_amount = i128::MAX;
    let session = Session {
        id: session_id.clone(),
        buyer: buyer.clone(),
        seller: seller.clone(),
        asset: asset.clone(),
        max_amount,
        escrowed_amount: max_amount,
        settled_amount: 0,
        expires_at_ledger: CURRENT_LEDGER + 100,
        resource_hash: BytesN::from_array(env, &[0x09; 32]),
        usage_hash: None,
        status: SessionStatus::Open,
        creation_ledger: CURRENT_LEDGER,
        finalized_ledger: None,
        resource_scheme_version: validation::SUPPORTED_RESOURCE_SCHEME_VERSION,
        usage_scheme_version: None,
    };
    let with_name = |name: &str| {
        vec![
            typed("symbol", json!(name)),
            typed("bytes", json!(session_hex)),
        ]
    };

    let mut vectors = Vec::new();

    env.as_contract(contract, || events::publish_session_created(env, &session));
    vectors.push(event_vector(
        env,
        contract,
        "session_created",
        "create",
        with_name("session_created"),
        json!({
            "asset": typed("address", json!(ASSET)),
            "buyer": typed("address", json!(BUYER)),
            "escrowed_amount": typed("i128", json!(max_amount.to_string())),
            "event_version": typed("u32", json!(2)),
            "expires_at_ledger": typed("u32", json!(CURRENT_LEDGER + 100)),
            "max_amount": typed("i128", json!(max_amount.to_string())),
            "resource_hash": typed("bytes", json!(hex::encode([0x09u8; 32]))),
            "seller": typed("address", json!(SELLER)),
        }),
    ));

    let actual_amount: i128 = 9_007_199_254_740_993;
    let refunded_amount = max_amount - actual_amount;
    env.as_contract(contract, || {
        events::publish_session_settled(
            env,
            &session_id,
            &seller,
            &asset,
            actual_amount,
            refunded_amount,
            &BytesN::from_array(env, &usage_hash),
        )
    });
    vectors.push(event_vector(
        env,
        contract,
        "session_settled",
        "settle",
        with_name("session_settled"),
        json!({
            "actual_amount": typed("i128", json!(actual_amount.to_string())),
            "asset": typed("address", json!(ASSET)),
            "event_version": typed("u32", json!(2)),
            "refunded_amount": typed("i128", json!(refunded_amount.to_string())),
            "seller": typed("address", json!(SELLER)),
            "usage_hash": typed("bytes", json!(hex::encode(usage_hash))),
        }),
    ));

    env.as_contract(contract, || {
        events::publish_session_cancelled(env, &session_id, &buyer, &asset, 0)
    });
    vectors.push(event_vector(
        env,
        contract,
        "session_cancelled",
        "cancel",
        with_name("session_cancelled"),
        json!({
            "asset": typed("address", json!(ASSET)),
            "buyer": typed("address", json!(BUYER)),
            "event_version": typed("u32", json!(2)),
            "refunded_amount": typed("i128", json!("0")),
        }),
    ));

    env.as_contract(contract, || {
        events::publish_session_expiry_extended(
            env,
            &session_id,
            &buyer,
            &seller,
            CURRENT_LEDGER + 100,
            u32::MAX,
        )
    });
    vectors.push(event_vector(
        env,
        contract,
        "session_expiry_extended",
        "extend",
        with_name("session_expiry_extended"),
        json!({
            "buyer": typed("address", json!(BUYER)),
            "event_version": typed("u32", json!(2)),
            "new_expires_at_ledger": typed("u32", json!(u32::MAX)),
            "previous_expires_at_ledger": typed("u32", json!(CURRENT_LEDGER + 100)),
            "seller": typed("address", json!(SELLER)),
        }),
    ));

    Value::Array(vectors)
}
