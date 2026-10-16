# Replacement Deployment Rehearsal

`upto-session` is immutable through its public interface. Replacing it requires
a second deployment and explicit client routing; session IDs are not portable
between contract IDs.

Run the deterministic routing checks and command rehearsal without a network:

```bash
node --test scripts/local/replacement-routing.test.mjs
node scripts/local/rehearse-replacement.mjs --dry-run
```

For the live local rehearsal, start Stellar Quickstart, configure the exact
release toolchain, and provide separate buyer and seller identities:

```bash
export LUMENBAZAAR_REHEARSAL_BUYER_ACCOUNT=local-buyer
export LUMENBAZAAR_REHEARSAL_BUYER_ADDRESS=G...
export LUMENBAZAAR_REHEARSAL_SELLER_ADDRESS=G...
node scripts/local/rehearse-replacement.mjs
```

`LUMENBAZAAR_REHEARSAL_OLD_WASM` and
`LUMENBAZAAR_REHEARSAL_NEW_WASM` may select different reviewed artifacts. By
default the script deploys the current artifact twice, which isolates routing
and rollback behavior from bytecode changes.

The script deploys unique old and replacement contract IDs, creates an open old
session, switches the active integration configuration, creates a replacement
session, and verifies each session only against its originating contract. It
then rolls the active configuration back to the old deployment without changing
either recorded session route, returns to the replacement, and marks the old
deployment as unavailable for new sessions. Live evidence is written under
`target/rehearsals/`, which is intentionally untracked.

Backend storage must persist both `session_id` and `contract_id`. Never derive
an existing session's destination from the current active contract setting.
Deprecation is an integration policy: old sessions remain readable and retain
their original interface behavior, while new sessions use the replacement.
