import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  classifyAbi,
  validateBreakingRelease,
} from "./lib/abi-compatibility.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const baseline = JSON.parse(
  readFileSync(
    join(repoRoot, "artifacts", "interface", "upto-session-release-baseline.json"),
    "utf8",
  ),
);

const identical = classifyAbi(baseline, structuredClone(baseline));
assert.equal(identical.breaking.length, 0);
assert.match(identical.compatible.join("\n"), /remains compatible/);

const additiveAbi = structuredClone(baseline);
additiveAbi.methods.push({ name: "query_new_state", inputs: [], outputs: ["u32"] });
additiveAbi.errors.push({ name: "NewFailure", value: 23 });
const additive = classifyAbi(baseline, additiveAbi);
assert.equal(additive.breaking.length, 0);
assert.match(additive.additive.join("\n"), /method added: query_new_state/);
assert.match(additive.additive.join("\n"), /error added: NewFailure=23/);

const changedMethod = structuredClone(baseline);
changedMethod.methods.find((value) => value.name === "settle").inputs[1].type = "u64";
assert.match(
  classifyAbi(baseline, changedMethod).breaking.join("\n"),
  /method changed: settle/,
);

const changedEvent = structuredClone(baseline);
changedEvent.events.find((value) => value.name === "SessionSettled").fields.pop();
assert.match(
  classifyAbi(baseline, changedEvent).breaking.join("\n"),
  /event changed: SessionSettled/,
);

const changedError = structuredClone(baseline);
changedError.errors.find((value) => value.name === "InvalidAmount").value = 99;
assert.match(
  classifyAbi(baseline, changedError).breaking.join("\n"),
  /error renumbered: InvalidAmount 3 -> 99/,
);

const breakingV2 = { ...baseline, interfaceVersion: baseline.interfaceVersion };
assert.match(
  validateBreakingRelease(baseline, breakingV2, null),
  /increased interface version/,
);
const breakingV3 = { ...baseline, interfaceVersion: baseline.interfaceVersion + 1 };
assert.match(validateBreakingRelease(baseline, breakingV3, null), /migration evidence/);
assert.equal(
  validateBreakingRelease(baseline, breakingV3, {
    fromInterfaceVersion: baseline.interfaceVersion,
    toInterfaceVersion: baseline.interfaceVersion + 1,
    baselineSpecSha256: baseline.sourceSpecSha256,
    compatibilityNotes: ["Deploy a replacement contract and route v2 sessions by contract ID."],
  }),
  null,
);

console.log("ABI compatibility checker tests passed");
