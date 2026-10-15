import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const specPath = join(repoRoot, "artifacts", "spec", "upto-session.json");
const outputPath = join(
  repoRoot,
  "artifacts",
  "interface",
  "upto-session-v3.json",
);
const check = process.argv.includes("--check");
const specText = readFileSync(specPath, "utf8").replace(/\r\n/g, "\n");
const spec = JSON.parse(specText);

const events = spec
  .filter((entry) => entry.event_v0 !== undefined)
  .map(({ event_v0: event }) => ({
    name: event.name,
    prefixTopics: event.prefix_topics,
    fields: event.params.map((field) => ({
      name: field.name,
      location: field.location,
      type: field.type,
    })),
  }));
const errorEntry = spec.find(
  (entry) => entry.udt_error_enum_v0?.name === "ContractError",
);
const methods = spec
  .filter((entry) => entry.function_v0 !== undefined)
  .map((entry) => entry.function_v0.name)
  .sort(compareAscii);

if (events.length !== 6 || errorEntry === undefined) {
  throw new Error("Expected six public events and ContractError in the canonical spec.");
}
const expectedEventNames = [
  "AssetDeprecated",
  "AssetRemoved",
  "SessionCancelled",
  "SessionCreated",
  "SessionRecovered",
  "SessionSettled",
];
if (events.map((event) => event.name).join(",") !== expectedEventNames.join(",")) {
  throw new Error("Canonical spec public event set changed.");
}
if (
  errorEntry.udt_error_enum_v0.cases.length !== 25 ||
  errorEntry.udt_error_enum_v0.cases.some((entry, index) => entry.value !== index + 1)
) {
  throw new Error("ContractError golden mapping must contain contiguous codes 1 through 25.");
}
for (const event of events) {
  const version = event.fields.find((field) => field.name === "event_version");
  if (version?.type !== "u32") {
    throw new Error(`${event.name} is missing its u32 event_version field.`);
  }
}
if (!methods.includes("interface_version")) {
  throw new Error("Canonical spec is missing interface_version.");
}

const fixture = {
  contractVersion: "0.3.0",
  interfaceVersion: 3,
  sourceSpecSha256: createHash("sha256").update(specText).digest("hex"),
  methods,
  events,
  errors: errorEntry.udt_error_enum_v0.cases.map(({ name, value }) => ({
    name,
    value,
  })),
};
const output = `${JSON.stringify(fixture, null, 2)}\n`;

if (check) {
  const committed = readFileSync(outputPath, "utf8").replace(/\r\n/g, "\n");
  if (committed !== output) {
    throw new Error("Interface fixture drift detected; regenerate the fixture.");
  }
  console.log("interface fixtures are current");
} else {
  mkdirSync(dirname(outputPath), { recursive: true });
  writeFileSync(outputPath, output, "ascii");
}

function compareAscii(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}
