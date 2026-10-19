import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  deriveSessionId,
  deriveSessionIdPreimage,
  validateSessionIdInput,
} from "../bindings/upto-session/dist/index.js";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const vector = JSON.parse(
  readFileSync(
    join(repoRoot, "artifacts", "backend-handoff", "domain-separation-v3.json"),
    "utf8",
  ),
);
const receiptFixtures = JSON.parse(
  readFileSync(
    join(repoRoot, "artifacts", "backend-handoff", "usage-receipt-v1.fixtures.json"),
    "utf8",
  ),
);

if (
  vector.protocol !== "lumenbazaar" ||
  vector.interfaceVersion !== 3 ||
  vector.sessionId.domain !== "lumenbazaar/session-id" ||
  vector.usageCommitment.domain !== "lumenbazaar/usage-commitment"
) {
  throw new Error("Domain vector labels or interface version changed");
}
if (!/^[0-9a-f]{64}$/.test(vector.networkIdHex)) {
  throw new Error("Domain vector network ID is not canonical lowercase hex");
}
assertDigest(vector.sessionId.preimageXdrHex, vector.sessionId.sha256, "session ID");
assertDigest(
  vector.usageCommitment.preimageXdrHex,
  vector.usageCommitment.sha256,
  "usage commitment",
);
if (vector.sessionId.sha256 === vector.usageCommitment.sha256) {
  throw new Error("Session and usage domains produced the same digest");
}
if (
  !receiptFixtures.vectors.some(
    (receipt) => receipt.sha256 === vector.usageCommitment.measurementHashHex,
  )
) {
  throw new Error("Usage commitment measurement hash has no canonical receipt vector");
}

// Check root sessionId fixture using TypeScript helper
const rootTsPreimage = deriveSessionIdPreimage({
  networkId: vector.networkIdHex,
  contractId: vector.contractId,
  buyer: vector.sessionId.buyer,
  seller: vector.sessionId.seller,
  asset: vector.sessionId.asset,
  maxAmount: vector.sessionId.maxAmount,
  expiresAtLedger: vector.sessionId.expiresAtLedger,
  resourceHash: vector.sessionId.resourceHashHex,
  sequence: vector.sessionId.sequence,
});
if (rootTsPreimage.toString("hex") !== vector.sessionId.preimageXdrHex) {
  throw new Error("TypeScript root preimage does not match JSON artifact");
}
const rootTsId = deriveSessionId({
  networkId: vector.networkIdHex,
  contractId: vector.contractId,
  buyer: vector.sessionId.buyer,
  seller: vector.sessionId.seller,
  asset: vector.sessionId.asset,
  maxAmount: vector.sessionId.maxAmount,
  expiresAtLedger: vector.sessionId.expiresAtLedger,
  resourceHash: vector.sessionId.resourceHashHex,
  sequence: vector.sessionId.sequence,
});
if (rootTsId !== vector.sessionId.sha256) {
  throw new Error("TypeScript root session ID does not match JSON artifact");
}

// Verify every fixture in vectors array
if (!Array.isArray(vector.vectors) || vector.vectors.length === 0) {
  throw new Error("domain-separation-v3.json must contain non-empty vectors array");
}

for (const fixture of vector.vectors) {
  if (!fixture.name || !fixture.description) {
    throw new Error("Every vector must have name and description");
  }
  if (!/^[0-9a-f]{64}$/.test(fixture.networkIdHex)) {
    throw new Error(`Vector ${fixture.name} networkIdHex is not canonical lowercase hex`);
  }
  if (fixture.networkPassphrase) {
    const derivedNetId = createHash("sha256")
      .update(Buffer.from(fixture.networkPassphrase, "utf8"))
      .digest("hex");
    if (derivedNetId !== fixture.networkIdHex) {
      throw new Error(`Vector ${fixture.name} networkPassphrase does not hash to networkIdHex`);
    }
  }

  assertDigest(fixture.preimageXdrHex, fixture.sha256, `vector ${fixture.name}`);

  // Test with networkPassphrase if available, otherwise networkId
  const input = {
    networkPassphrase: fixture.networkPassphrase,
    networkId: fixture.networkIdHex,
    contractId: fixture.contractId,
    buyer: fixture.buyer,
    seller: fixture.seller,
    asset: fixture.asset,
    maxAmount: fixture.maxAmount,
    expiresAtLedger: fixture.expiresAtLedger,
    resourceHash: fixture.resourceHashHex,
    sequence: fixture.sequence,
  };

  const tsPreimage = deriveSessionIdPreimage(input);
  if (tsPreimage.toString("hex") !== fixture.preimageXdrHex) {
    throw new Error(`Vector ${fixture.name} preimage mismatch between TypeScript and fixture`);
  }

  const tsId = deriveSessionId(input);
  if (tsId !== fixture.sha256) {
    throw new Error(`Vector ${fixture.name} session ID mismatch between TypeScript and fixture`);
  }
}

// Test that changing ANY bound field changes the derived ID
const baseInput = {
  networkPassphrase: "Test SDF Network ; September 2015",
  contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
  buyer: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFCT4",
  seller: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M",
  asset: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAITA4",
  maxAmount: "420000000",
  expiresAtLedger: 5067600,
  resourceHash: "0909090909090909090909090909090909090909090909090909090909090909",
  sequence: "7",
  interfaceVersion: 3,
};
const baseId = deriveSessionId(baseInput);

const perturbations = [
  { field: "networkPassphrase", mod: { networkPassphrase: "Public Global Stellar Network ; September 2015" } },
  { field: "networkId", mod: { networkPassphrase: undefined, networkId: "0808080808080808080808080808080808080808080808080808080808080808" } },
  { field: "contractId", mod: { contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M" } },
  { field: "buyer", mod: { buyer: "GCYEX7MPJL64ZJ7ABZSPRC7YEBSI7OMC62FFEVFHCZFREBOYJPQDUCYJ" } },
  { field: "seller", mod: { seller: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFCT4" } },
  { field: "asset", mod: { asset: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM" } },
  { field: "maxAmount", mod: { maxAmount: "420000001" } },
  { field: "expiresAtLedger", mod: { expiresAtLedger: 5067601 } },
  { field: "resourceHash", mod: { resourceHash: "0a09090909090909090909090909090909090909090909090909090909090909" } },
  { field: "sequence", mod: { sequence: "8" } },
];

for (const { field, mod } of perturbations) {
  const perturbedId = deriveSessionId({ ...baseInput, ...mod });
  if (perturbedId === baseId) {
    throw new Error(`Perturbing field ${field} failed to change derived session ID`);
  }
}

// Test validation: invalid inputs must fail before hashing
const invalidCases = [
  { name: "missing network", input: { ...baseInput, networkPassphrase: "", networkId: undefined }, pattern: /networkPassphrase or networkId/ },
  { name: "mismatched networkPassphrase and networkId", input: { ...baseInput, networkId: "00".repeat(32) }, pattern: /does not match/ },
  { name: "invalid networkId length", input: { ...baseInput, networkPassphrase: undefined, networkId: "0102" }, pattern: /64-character lowercase hex/ },
  { name: "invalid contract address prefix", input: { ...baseInput, contractId: "G" + baseInput.contractId.slice(1) }, pattern: /valid Stellar contract address/ },
  { name: "malformed contract address checksum", input: { ...baseInput, contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KX" }, pattern: /valid Stellar/ },
  { name: "invalid buyer address", input: { ...baseInput, buyer: "B" + baseInput.buyer.slice(1) }, pattern: /valid Stellar address/ },
  { name: "invalid seller address", input: { ...baseInput, seller: "INVALID_SELLER" }, pattern: /valid Stellar address/ },
  { name: "invalid asset address", input: { ...baseInput, asset: "G" + baseInput.asset.slice(1) }, pattern: /valid Stellar contract address/ },
  { name: "zero maxAmount", input: { ...baseInput, maxAmount: "0" }, pattern: /positive/ },
  { name: "negative maxAmount", input: { ...baseInput, maxAmount: "-100" }, pattern: /positive/ },
  { name: "non-integer maxAmount", input: { ...baseInput, maxAmount: "100.5" }, pattern: /positive canonical decimal string/ },
  { name: "zero expiresAtLedger", input: { ...baseInput, expiresAtLedger: 0 }, pattern: /positive uint32/ },
  { name: "negative expiresAtLedger", input: { ...baseInput, expiresAtLedger: -1 }, pattern: /positive uint32/ },
  { name: "invalid resourceHash chars", input: { ...baseInput, resourceHash: "09".repeat(31) + "ZZ" }, pattern: /64-character lowercase hex/ },
  { name: "negative sequence", input: { ...baseInput, sequence: "-1" }, pattern: /non-negative/ },
  { name: "v2 interfaceVersion rejected explicitly", input: { ...baseInput, interfaceVersion: 2 }, pattern: /v2 and v3 domain rules remain explicitly separated/ },
  { name: "unknown interfaceVersion rejected", input: { ...baseInput, interfaceVersion: 4 }, pattern: /expected 3/ },
];

for (const { name, input, pattern } of invalidCases) {
  let threw = false;
  try {
    deriveSessionId(input);
  } catch (err) {
    threw = true;
    if (!pattern.test(err.message)) {
      throw new Error(`Case "${name}" threw unexpected error message: "${err.message}" (expected ${pattern})`);
    }
  }
  if (!threw) {
    throw new Error(`Case "${name}" unexpectedly succeeded instead of failing before hashing`);
  }
}

console.log("domain separation vectors passed");

function assertDigest(preimageHex, expected, label) {
  if (!/^(?:[0-9a-f]{2})+$/.test(preimageHex) || !/^[0-9a-f]{64}$/.test(expected)) {
    throw new Error(`${label} vector is not canonical lowercase hex`);
  }
  const actual = createHash("sha256").update(Buffer.from(preimageHex, "hex")).digest("hex");
  if (actual !== expected) {
    throw new Error(`${label} vector digest mismatch`);
  }
}
