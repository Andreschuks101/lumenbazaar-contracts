import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const checklist = readFileSync(join(repoRoot, "docs", "security-checklist.md"), "utf8");
const tests = readFileSync(
  join(repoRoot, "contracts", "upto-session", "src", "test.rs"),
  "utf8",
);
const validation = readFileSync(
  join(repoRoot, "contracts", "upto-session", "src", "validation.rs"),
  "utf8",
);
const propertyTests = readFileSync(
  join(repoRoot, "contracts", "upto-session", "src", "property_test.rs"),
  "utf8",
);

const controls = [
  {
    name: "Authorization boundaries",
    tests: [
      "initialize_stores_admin_once",
      "create_session_requires_buyer_auth",
      "settle_requires_seller_auth",
      "settle_rejects_wrong_seller_auth",
      "cancel_requires_buyer_auth",
    ],
  },
  {
    name: "Cap enforcement",
    tests: ["settle_rejects_invalid_amounts_and_expiry", "rejects_invalid_settlement_amounts"],
  },
  {
    name: "Double-settlement prevention",
    tests: [
      "settle_prevents_double_settlement",
      "settle_rejects_finalized_sessions",
      "cancel_rejects_already_settled_session",
    ],
  },
  {
    name: "Expiry handling",
    tests: [
      "rejects_expired_sessions",
      "expiry_boundary_selects_exactly_one_terminal_path",
      "expiry_recovery_is_permissionless_fixed_recipient_and_single_use",
      "expiry_recovery_emits_versioned_receipt",
    ],
  },
  {
    name: "Asset and seller binding",
    tests: ["rejects_invalid_seller_or_asset_bindings", "invalid_create_inputs_do_not_consume_sequence"],
  },
  {
    name: "Event correctness",
    tests: [
      "create_session_emits_stable_event",
      "settle_emits_stable_event",
      "cancel_emits_stable_event",
    ],
  },
  {
    name: "Generated state-machine invariants",
    tests: ["generated_sequences_preserve_value_caps_and_isolation"],
  },
  {
    name: "Adversarial token behavior",
    tests: [
      "adversarial_create_failures_roll_back_funds_liability_and_sequence",
      "adversarial_terminal_failures_leave_the_session_open_and_funded",
      "nested_token_callback_cannot_reenter_a_terminal_transition",
    ],
  },
];

for (const control of controls) {
  if (!checklist.includes(control.name)) {
    throw new Error(`Checklist is missing ${control.name}`);
  }
  for (const test of control.tests) {
    if (!checklist.includes(test)) {
      throw new Error(`Checklist does not cite ${test}`);
    }
    if (!tests.includes(test) && !validation.includes(test) && !propertyTests.includes(test)) {
      throw new Error(`Cited test does not exist: ${test}`);
    }
  }
}

console.log("security checklist check passed");
