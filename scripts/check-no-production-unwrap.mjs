// Fails when production contract code calls `.unwrap()` or `.expect(...)`.
//
//   node scripts/check-no-production-unwrap.mjs
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkNoProductionUnwrap } from "./lib/unwrap-gate.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const violations = checkNoProductionUnwrap(repoRoot);

if (violations.length > 0) {
  for (const violation of violations) {
    console.error(violation);
  }
  process.exit(1);
}
console.log("Production unwrap/expect check passed (0 violations)");
