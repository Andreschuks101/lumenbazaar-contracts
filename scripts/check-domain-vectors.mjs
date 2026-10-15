import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const vector = JSON.parse(
  readFileSync(
    join(repoRoot, "artifacts", "backend-handoff", "domain-separation-v3.json"),
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
