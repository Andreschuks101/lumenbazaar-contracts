# Bounded Fuzz Targets

Fuzz targets for public `upto-session` and `policy-wallet-example` inputs and the core validation
helpers. All inputs are synthetic and no target ever contacts a network.

| Target | Covers |
| --- | --- |
| `create` | `create_session`: parties (including equal, unfunded, unknown-contract, and the escrow's own address), amounts, expiries around the duration limit, resource hashes (all-zero, all-0xFF, single-bit, raw, and wrong-length via raw invocation). |
| `settle` | `settle` on open, settled, cancelled, recovered, and lapsed sessions, unknown and off-by-one session IDs, amounts around the cap, ledgers around expiry, usage hashes, and wrong-length session IDs and usage hashes. |
| `policy` | `set_policy` with arbitrary caps, authorization limits, windows, allowlists, and limits, followed by up to eight `check_payment` / `authorize_payment` steps across a day boundary. |
| `token_amount` | `validate_asset_cap`, `validate_asset_policy`, `validate_settlement_amount`, `validate_cap_increase_proposal`, `validate_create_session`, `validate_expiry_extension`, `validate_admin_rotation` (including ledger rollover boundaries), and checked liability arithmetic. |

Every target checks that:

1. each call returns `Ok` or a defined contract error (a host failure, trap, or undecodable result
   fails the target);
2. the call succeeds **exactly** when an independent statement of the documented rule says it
   should (success on invalid input is a **validation bypass**);
3. a rejected call leaves balances, liabilities, sequence counters, sessions, and wallet storage
   unchanged.

The harness is `fuzz/src` (crate `lumenbazaar-fuzz`, a workspace member). Both runners call the
same functions, so the quick CI run and an extended libFuzzer run test the same code path.

## Quick seeded corpus (CI)

`fuzz/seeds/manifest.json` records every committed seed exactly once with its
SHA-256, target, input schema, purpose, generator, and expected fixed outcome.
`node scripts/verify-fuzz-artifacts.mjs` fails on missing, duplicate, stale, or
oversized entries. Reviewed minimized crashes live under `fuzz/crashes`; CI
replays them with a 60-second wall-clock and 2 GiB virtual-memory bound.

Runs on the pinned stable toolchain as part of `cargo test --workspace --all-features`:

```bash
cargo test -p lumenbazaar-fuzz
```

For every target it replays every file in `fuzz/seeds/<target>/`, four mutations of each seed, and
64 generated inputs from a fixed SplitMix64 seed. It is deterministic: the same command always runs
the same inputs. Increase the generated count locally with `LUMENBAZAAR_FUZZ_ITERATIONS=10000`
(PowerShell: `$env:LUMENBAZAAR_FUZZ_ITERATIONS = "10000"`).

## Extended runs (optional, never required for a pull request)

Extended runs use [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) and need a
nightly toolchain, so they are manual or scheduled, not part of PR CI. The package lives in
`fuzz/libfuzzer`, has its own workspace, and is excluded from the root workspace.

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --locked
cargo test -p lumenbazaar-fuzz  # run the quick corpus on stable first
cargo +nightly fuzz run --fuzz-dir fuzz/libfuzzer create -- -max_total_time=600 -max_len=256
cargo +nightly fuzz run --fuzz-dir fuzz/libfuzzer settle -- -max_total_time=600 -max_len=256
cargo +nightly fuzz run --fuzz-dir fuzz/libfuzzer policy -- -max_total_time=600 -max_len=256
cargo +nightly fuzz run --fuzz-dir fuzz/libfuzzer token_amount -- -max_total_time=600 -max_len=256
```

libFuzzer does not read the hex seeds directly. Convert one with
`xxd -r -p <(grep -v '^#' fuzz/seeds/create/00-valid-default.hex) > fuzz/libfuzzer/corpus/create/00`
or let it start from an empty corpus; every all-zero prefix is already a valid call. Crashes are
written to `fuzz/libfuzzer/artifacts/<target>/`, which is ignored by git.

cargo-fuzz is not available on Windows with the MSVC or GNU host toolchains used by some
contributors. Run extended campaigns on Linux or macOS (or WSL).

## Input layouts

All integers are big-endian. Missing bytes read as zero, and a zero selector always means a valid
default, so short inputs are valid calls and each nonzero byte introduces one boundary. Amount
selectors (`% 10`): 0 default, 1 `0`, 2 `1`, 3 `-1`, 4 `i128::MIN`, 5 `i128::MAX`,
6 `i128::MAX - 1`, 7 `i128::MIN + 1`, 8 raw, 9 raw mapped to `1..=1e9`. Hash selectors (`% 5`):
0 default, 1 all-zero, 2 all-0xFF, 3 raw, 4 single bit.

| Target | Offset: field |
| --- | --- |
| `create` | 0 ledger selector, 1–4 raw ledger; 5 buyer; 6 seller; 7 asset; 8 amount selector, 9–24 raw; 25 expiry selector, 26–29 raw; 30 resource hash selector, 31–62 raw (also used for synthetic addresses and wrong-length size); 63 malformed (odd: raw invoke with a wrong-length resource hash). |
| `settle` | 0 cap selector, 1–16 raw; 17 session state; 18 session ID selector, 19–50 raw; 51 ledger selector, 52–55 raw; 56 amount selector, 57–72 raw; 73 usage hash selector, 74–105 raw; 106 malformed (1 wrong-length session ID, 2 wrong-length usage hash). |
| `policy` | 0 payment cap, 17 daily cap (amount selectors); 34 authorization limit, 35–38 raw; 39 `valid_from`, 40–43 raw; 44 `valid_until`, 45–48 raw; 49 seller, 50 asset, 51 resource allowlist masks; 52 seller limit, 53 resource limit; from 54, eight-byte steps: ledger move, seller, asset, resource, amount selector, two raw amount bytes, check-only flag. |
| `token_amount` | 0 helper (`% 8`); 1 `a` and 18 `b` (amount selectors); 35–38 `x`; 39–42 `y`; 43 ledger selector (`% 6`, including rollover-adjacent values), 44–47 raw. |

The source of truth is the target code in `fuzz/src`.

## Adding corpus entries

Seeds are text files of hex bytes, one input per file, with `#` comments:

```text
# settle: usage selector 1 => all-zero usage hash (InvalidUsageHash)
# Synthetic. Layout: fuzz/README.md.
00000000 ...
```

Add a file under `fuzz/seeds/<target>/` with a numbered, descriptive name and a comment saying
which rule it exercises. Use synthetic values only: never real account IDs, contract IDs from a
live deployment, keys, signatures, or production data. Run `cargo test -p lumenbazaar-fuzz`.

## Triage process

1. **Reproduce.** A CI failure prints `fuzz target `<name>` failed on input <hex>`. A libFuzzer
   failure writes `fuzz/libfuzzer/artifacts/<target>/crash-*`. Save the input as a seed file
   (`xxd -p crash-... > fuzz/seeds/<target>/NN-name.hex`) and run `cargo test -p lumenbazaar-fuzz`
   to reproduce on stable, or `cargo +nightly fuzz run --fuzz-dir fuzz/libfuzzer <target> <file>`.
2. **Minimize.** `cargo +nightly fuzz tmin --fuzz-dir fuzz/libfuzzer <target> <file>`, or reduce by
   hand: zero bytes until the failure disappears, keeping only the selectors that matter.
3. **Classify.**
   - **Panic or host failure**: the contract trapped or returned an error the client could not
     decode. Severity depends on whether funds can be locked.
   - **Validation bypass**: a call succeeded on input the rules reject, or a rejected call changed
     state. Treat as security-relevant until shown otherwise.
   - **Spurious rejection**: a valid call was rejected. Usually a harness oracle or documentation
     mismatch; fix whichever is wrong and say which.
   - **Resource exhaustion**: a run that is slow or hits host budget limits. Note the input size
     and whether it is reachable within transaction limits.
4. **Report privately when security-relevant.** Any bypass, over-cap, double-settlement, expiry,
   binding, or replay finding follows [SECURITY.md](../SECURITY.md): use the repository security
   advisory flow or contact maintainers directly, and do not open a public issue or commit the
   reproducing seed until a fix or mitigation is available.
5. **Fix with a permanent regression.** The fix PR contains the minimized seed in
   `fuzz/seeds/<target>/` and a named unit test in the affected contract crate. It states the
   classification, the rule that was violated, the root cause, the behavior change, and the
   validation commands run. If the harness oracle was wrong rather than the contract, the PR says
   so and updates the oracle and this layout table.

## Limits

- The targets run in the Soroban unit-test host with mocked authorization. They do not exercise
  signature verification, archival, fees, or real network limits.
- Fuzzed ledgers that create storage are bounded to 100,000,000, and storage entries are given a
  long minimum lifetime so TTL lapse is not reported as a validation failure. TTL behavior is
  covered by `docs/storage-lifecycle.md`.
- The quick CI run is a bounded smoke test, not a fuzzing campaign. Absence of failures is not
  proof of absence of bugs.

## Findings recorded while writing the targets

- `create_session` accepts the escrow contract's own address as `seller` (seed
  `create/17-seller-is-escrow-contract.hex`). The oracle treats it as valid because no documented
  rule rejects it. Whether it should be rejected is a maintainer decision.
