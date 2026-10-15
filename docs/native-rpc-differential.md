# Native And RPC Differential Suite

Native Soroban host tests are the pull-request correctness gate. Public testnet
RPC comparison is additional evidence and never makes pull-request correctness
depend on external network availability.

The deterministic fixture definitions are in
`artifacts/resource-usage/native-rpc-differential-fixtures.json`. They cover
session creation, a settled-session read, and replay rejection for settlement,
cancellation, and expiry recovery. Run the native gate with:

```bash
node scripts/run-native-rpc-differential.mjs --native-only
```

Run the opt-in public comparison with Stellar CLI 28.0.0:

```bash
node scripts/run-native-rpc-differential.mjs \
  --output target/native-rpc-differential.json
```

The runner builds unsigned transactions from public testnet accounts and calls
`simulateTransaction` directly. It never submits a transaction and requires no
secret or signing key. Result semantics, authorization entry counts, and
successful contract-event counts must match exactly.

Instruction and fee differences are preserved with absolute and percentage
deltas. They are classified as `expected_environmental` because native fixtures
use fresh in-memory state while RPC uses a specific public ledger, including
archival restoration, footprint, and rent costs. Terminal failures can reject
before RPC produces a resource envelope; those are classified as
`expected_early_rejection`. Any result, authorization, or contract-event mismatch
is a `regression` and fails the run.

The scheduled workflow uploads the complete JSON report. A transport failure,
timeout, rate limit, or missing RPC response is recorded as `outage` and exits
with the dedicated code `2`; the workflow reports a warning instead of treating
external availability as a contract regression.
