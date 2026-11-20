#!/usr/bin/env node
import { defaultContracts, formatPreflightReport, runPreflight } from "../lib/deployment-preflight.mjs";

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const json = args.includes("--json");
let contracts = defaultContracts;
const index = args.indexOf("--contracts");
if (index !== -1) {
  const names = (args[index + 1] ?? "").split(",").filter(Boolean);
  const unknown = names.filter((n) => !defaultContracts.some((c) => c.name === n));
  if (names.length === 0 || unknown.length > 0) {
    console.error(`--contracts accepts a comma-separated subset of: ${defaultContracts.map((c) => c.name).join(", ")}`);
    process.exit(1);
  }
  contracts = defaultContracts.filter((c) => names.includes(c.name));
}

const report = await runPreflight({ dryRun, contracts });
if (json) {
  const { redactText } = await import("../lib/deployment-logging.mjs");
  console.log(redactText(JSON.stringify(report, null, 2)));
} else {
  console.log(formatPreflightReport(report));
}
process.exit(report.success ? 0 : 1);
