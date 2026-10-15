# Threat Model

This document covers the `upto-session` contract and the example-only policy wallet. The custom
`test-token` exists only for local and testnet validation and is not a production asset.

The accepted funding design is
[ADR 0001: Escrow-Funded Upto Sessions](adr/0001-escrow-funded-upto-sessions.md).
The current source implements the selected escrow model. The corrected v2
testnet deployment and lifecycle evidence are published in
`deployments/testnet-2026-10-07.json` and
`artifacts/testnet/lifecycle-2026-10-07.json`.

## Assets and trust boundaries

- Buyers authorize full-cap escrow at session creation and cancellation for
  their own funds.
- Sellers authorize settlement and cannot settle more than the buyer's cap.
- The configured token contract is trusted to implement the expected transfer interface correctly.
- The `resource_hash` and `usage_hash` values are commitments only. The contract does not validate
  off-chain resource contents or metering calculations.
- RPC nodes, Horizon, indexers, backend services, and user interfaces are outside the on-chain trust
  boundary. Clients must verify finalized transaction results before presenting settlement as final.

## Authorization and replay

- `create_session` requires buyer authorization and binds buyer, seller, asset, cap, expiry, resource
  hash, and an incrementing sequence into the session ID. Its nested token call
  transfers the cap into contract custody.
- `settle` requires the stored seller's authorization and finalizes a session once.
- Before expiry, `cancel` requires the stored buyer's authorization, refunds the escrow, and prevents later settlement.
- At or after expiry, `recover_expired` requires no caller authorization and can refund only the stored buyer.
- Repeated settlement and cancellation attempts fail against finalized state.
- Soroban rejects a callback that tries to invoke the active escrow contract
  again while its token transfer is on the call stack. Test-only callback-token
  fixtures cover create, settle, cancel, and recovery and assert that no second
  success event, authorization, balance movement, or liability transition is
  produced. This regression evidence does not make arbitrary token behavior
  trustworthy.

## Amount, expiry, and storage risks

- Caps and settlement amounts must be positive, and settlement cannot exceed the stored cap.
- Settlement at or after the expiry ledger is rejected.
- Contract storage TTL can outlive or expire independently of off-chain records. Operators must not
  treat a missing off-chain record as proof that an on-chain session is absent.
- TTL extension is limited to open sessions. Tests cover missing, settled, and cancelled session
  rejection. The public lifecycle evidence also covers settlement rejection at the live expiry
  boundary and permissionless recovery seven ledgers after expiry.

## Administration and upgrades

- `upto-session` stores an administrator during one-time initialization but currently exposes no
  contract upgrade entry point.
- Deployed instances are therefore operationally immutable through this interface. A code change
  requires a new deployment and explicit migration or reconfiguration by clients.
- The policy wallet owner can update its own spending policy. That authority does not grant access to
  another buyer's session or permit settlement by a different seller.

## Known limitations

- `upto` is a LumenBazaar extension and is not presented as an upstream-standard Stellar x402 scheme.
- The live evidence uses LBT, a custom test-only token, not Circle USDC or another production asset.
- Rejected operations fail during simulation and therefore have no submitted transaction hash; the
  evidence records the observation ledger, exact command, and deterministic rejection instead.
- Testnet lifecycle evidence does not prove mainnet readiness, production monitoring, or backend
  idempotency.
- Production use requires an independently reviewed token, a live transaction submission client,
  durable backend idempotency, and monitoring for failed or delayed finality.
