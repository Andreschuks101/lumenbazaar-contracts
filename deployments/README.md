# Deployments

Deployment manifests are written here by the repo scripts.

- `local.json` is ignored and belongs to a developer machine.
- `testnet.json` is ignored until the project chooses to publish a specific deployment.
- Example manifests may be committed when they document the expected shape.
- `testnet-2026-09-06.json` is the sanitized public evidence for the first testnet deployment. It
  contains no signing material and does not claim a live payment flow.
- `testnet-2026-10-07.json` is the immutable corrected v2 deployment evidence. It records the
  reviewed source commit, dedicated public admin, exact toolchain, WASM and interface hashes, and
  upload/deploy/initialize transactions without replacing the September record.
- `testnet-v2-supported-assets.json` is the public initialization input for the corrected escrow
  deployment.

For testnet:

1. Copy `.env.testnet.example` to `.env.testnet` or export the same variables in your shell.
2. Run `node scripts/testnet/deploy-testnet.mjs`.
3. Run `node scripts/testnet/initialize-testnet.mjs`.
4. Run `node scripts/testnet/verify-testnet.mjs`.
5. Share the resulting `deployments/testnet.json` with backend operators through the agreed secure handoff channel.

Validate committed public evidence with `node scripts/check-deployment-evidence.mjs`.

`replacement-rehearsal.example.json` documents the client routing state used by
the executable local replacement rehearsal. Live rehearsal evidence is written
under ignored `target/rehearsals/`; it is local evidence, not a public deployment
claim.

The exact build, deployment, and independent verification commands for the corrected deployment
are in `docs/testnet-deployment-2026-10-07.md`.
