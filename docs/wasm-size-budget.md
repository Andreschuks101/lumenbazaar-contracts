# WASM Size Budget

Optimized contract size is a release gate because unexpected growth increases
deployment cost and can reveal accidental dependency or generated-code
expansion. Security checks must not be removed solely to reduce size.

The machine-readable baseline is
`artifacts/resource-usage/wasm-size-budget.json`. It records the exact Rust,
target, Soroban SDK, Stellar CLI, and optimizer versions used to produce the
baseline. Run the gate only after the release build:

```bash
stellar contract build --locked
node scripts/check-wasm-size-budget.mjs --report wasm-size-report.json
```

The report includes each contract's absolute byte delta and percentage change.
Both `maxBytes` and `maxIncreasePercent` must pass. A maintainer may adjust the
committed baseline and thresholds when a reviewed feature intentionally grows a
contract.

## Reviewed overrides

An over-budget build fails by default. A temporary override requires a committed
record under `docs/wasm-size-reviews/` and an explicit invocation such as:

```bash
node scripts/check-wasm-size-budget.mjs \
  --review docs/wasm-size-reviews/2026-10-example.md
```

The record must name every affected contract in backticks and include `Reason`,
`Security impact`, and `Approval` sections. CI does not enable overrides by
default, so a pull request changing the gate to use one exposes that decision
for review. After approval, update the baseline and remove the temporary
override rather than carrying it indefinitely.
