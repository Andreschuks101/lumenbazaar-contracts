import { join } from "node:path";
import {
  hasFlag,
  invokeContract,
  readManifest,
  repoRoot,
  sourceAccount,
} from "../lib/stellar-cli.mjs";

const dryRun = hasFlag("--dry-run");
const source = sourceAccount();
const manifest = readManifest(join(repoRoot, "deployments", "local.json"));
const contracts = manifest.contracts || {};

const admin = process.env.LUMENBAZAAR_ADMIN || source;
const tokenAdmin = process.env.LUMENBAZAAR_TEST_TOKEN_ADMIN || admin;
const policyOwner = process.env.LUMENBAZAAR_POLICY_OWNER || admin;
const maxSessionAmount = process.env.LUMENBAZAAR_MAX_SESSION_AMOUNT || "1000000000";
if (!/^[1-9][0-9]*$/.test(maxSessionAmount)) {
  throw new Error("LUMENBAZAAR_MAX_SESSION_AMOUNT must be a positive integer");
}
const uptoSessionContract =
  process.env.LUMENBAZAAR_UPTO_SESSION_CONTRACT_ID ||
  contracts["upto-session"]?.contractId ||
  "dry-run-upto-session";
const testTokenContract =
  process.env.LUMENBAZAAR_TEST_TOKEN_CONTRACT_ID ||
  contracts["test-token"]?.contractId ||
  "dry-run-test-token";
const policyWalletContract =
  process.env.LUMENBAZAAR_POLICY_WALLET_CONTRACT_ID ||
  contracts["policy-wallet-example"]?.contractId ||
  "dry-run-policy-wallet";

invokeContract({
  contractId: testTokenContract,
  source: tokenAdmin,
  network: "local",
  args: ["initialize", "--admin", tokenAdmin],
  dryRun,
});

invokeContract({
  contractId: uptoSessionContract,
  source: admin,
  network: "local",
  args: [
    "initialize",
    "--admin",
    admin,
    "--asset_policies",
    JSON.stringify([
      { asset: testTokenContract, max_session_amount: maxSessionAmount },
    ]),
  ],
  dryRun,
});

invokeContract({
  contractId: policyWalletContract,
  source: policyOwner,
  network: "local",
  args: [
    "initialize",
    "--owner",
    policyOwner,
    "--upto_session_contract",
    uptoSessionContract,
  ],
  dryRun,
});
