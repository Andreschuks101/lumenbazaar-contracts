import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  canonicalizeUsageReceipt,
  hashUsageReceipt,
  parseCanonicalUsageReceipt,
} from "../bindings/upto-session/dist/usageReceipt.js";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const fixture = JSON.parse(
  readFileSync(
    join(repoRoot, "artifacts", "backend-handoff", "usage-receipt-v1.fixtures.json"),
    "utf8",
  ),
);

for (const vector of fixture.vectors) {
  assert.equal(canonicalizeUsageReceipt(vector.receipt), vector.canonicalUtf8, vector.name);
  assert.equal(hashUsageReceipt(vector.receipt), vector.sha256, vector.name);
  assert.deepEqual(parseCanonicalUsageReceipt(vector.canonicalUtf8), vector.receipt, vector.name);
}
for (const invalid of fixture.invalidEncodings) {
  assert.throws(() => parseCanonicalUsageReceipt(invalid.json), undefined, invalid.name);
}

console.log("TypeScript usage receipt fixtures passed");
