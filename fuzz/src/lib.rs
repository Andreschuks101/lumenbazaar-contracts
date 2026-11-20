//! Bounded fuzz harness for public LumenBazaar contract inputs.
//!
//! Each target takes arbitrary bytes, decodes them with a fixed layout (`input`), calls the real
//! contracts in the Soroban test host, and checks three things:
//!
//! 1. The call returns `Ok` or a defined contract error. A host failure or an undecodable result
//!    is a crash.
//! 2. The call succeeds exactly when an independent statement of the documented rules says it
//!    should. Success on invalid input is a validation bypass.
//! 3. A rejected call leaves balances, liabilities, counters, and stored records unchanged.
//!
//! The same functions run in two places: `tests/seeded_corpus.rs` replays the checked-in seeds and
//! a fixed-seed generator inside `cargo test --workspace`, and `fuzz/libfuzzer` runs them under
//! libFuzzer for optional extended campaigns. See `fuzz/README.md`.
//!
//! Test-only code. Nothing here is deployed.

pub mod input;

mod amount;
mod policy;
mod session;

pub use amount::token_amount;
pub use policy::policy;
pub use session::{create, settle};

use core::fmt::Debug;

/// Default ledger sequence for targets that do not fuzz it.
pub const DEFAULT_LEDGER: u32 = 1_000;
/// Upper bound for fuzzed ledgers that create storage, far below `u32::MAX` so the host's TTL
/// arithmetic stays defined. Pure validation targets use the full `u32` range.
pub const MAX_LEDGER: u32 = 100_000_000;
/// Minimum entry lifetime in the fuzz host, longer than any ledger range a target walks.
pub const ENTRY_TTL: u32 = 4_000_000;

/// A fuzz target: decodes and checks one input, panicking on any violation.
pub type Target = fn(&[u8]);

/// Every target, by the name used for its seed directory and libFuzzer binary.
pub const TARGETS: &[(&str, Target)] = &[
    ("create", create),
    ("settle", settle),
    ("policy", policy),
    ("token_amount", token_amount),
];

/// Maps a typed `try_` client result to `Ok` or a defined contract error, and fails on anything
/// else.
pub(crate) fn defined<T, E, C: Debug, I: Debug>(
    result: Result<Result<T, C>, Result<E, I>>,
) -> Result<T, E> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Err(Ok(error)) => Err(error),
        Ok(Err(conversion)) => panic!("undecodable contract result: {conversion:?}"),
        Err(Err(invoke)) => panic!("host failure instead of a contract error: {invoke:?}"),
    }
}

/// Parses a seed file: hex digits, with whitespace ignored and `#` starting a comment.
pub fn parse_hex_seed(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or(""))
        .flat_map(str::bytes)
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if !digits.len().is_multiple_of(2) {
        return Err("odd number of hex digits".into());
    }
    digits
        .chunks(2)
        .map(|pair| {
            let pair = core::str::from_utf8(pair).map_err(|e| e.to_string())?;
            u8::from_str_radix(pair, 16).map_err(|_| format!("invalid hex byte {pair:?}"))
        })
        .collect()
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Deterministic SplitMix64 generator for the quick CI campaign. No external randomness.
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Random input of up to `max_len` bytes. Bytes are biased toward small selector values so
    /// generated inputs exercise every boundary branch, not only the raw-value branches.
    pub fn input(&mut self, max_len: usize) -> Vec<u8> {
        let len = (self.next_u64() % (max_len as u64 + 1)) as usize;
        (0..len)
            .map(|_| {
                let value = self.next_u64();
                if value.is_multiple_of(4) {
                    (value >> 8) as u8
                } else {
                    ((value >> 8) % 12) as u8
                }
            })
            .collect()
    }

    /// Copy of `seed`, padded to `max_len`, with one to four bytes replaced.
    pub fn mutate(&mut self, seed: &[u8], max_len: usize) -> Vec<u8> {
        let mut out = seed.to_vec();
        out.resize(max_len.max(seed.len()), 0);
        for _ in 0..1 + self.next_u64() % 4 {
            let at = (self.next_u64() % out.len().max(1) as u64) as usize;
            if let Some(byte) = out.get_mut(at) {
                *byte = (self.next_u64() >> 16) as u8;
            }
        }
        out
    }
}

/// Runs `target` on `data`. On failure, reports the target and the input as hex (ready to save
/// as a seed file) before re-raising the panic.
pub fn run_reporting(name: &str, target: Target, data: &[u8]) {
    if let Err(panic) = std::panic::catch_unwind(|| target(data)) {
        eprintln!("fuzz target `{name}` failed on input {}", to_hex(data));
        std::panic::resume_unwind(panic);
    }
}
