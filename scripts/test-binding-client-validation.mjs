import assert from "node:assert/strict";
import { validateClientIdentity } from "../bindings/upto-session/dist/clientValidation.js";

const TESTNET = "Test SDF Network ; September 2015";
const PUBLIC = "Public Global Stellar Network ; September 2015";
const contractId = "CDI5ZVB4VUQQJ5DJ25M4QBXYI273LN4PHN7TW6BT6VHERELK26SH7YY3";
const options = { contractId, networkPassphrase: TESTNET, rpcUrl: "https://example.test/rpc" };
const deployment = { contractId, networkPassphrase: TESTNET, network: "testnet", manifestSha256: "a".repeat(64) };

for (const environment of ["node", "browser"]) {
  if (environment === "browser") globalThis.window = {};
  assert.doesNotThrow(() => validateClientIdentity(options, deployment));
  assert.throws(() => validateClientIdentity({ ...options, networkPassphrase: PUBLIC }, deployment), /network/);
  assert.throws(() => validateClientIdentity(options, { ...deployment, network: "custom", networkId: undefined }), /networkId/);
  delete globalThis.window;
}
