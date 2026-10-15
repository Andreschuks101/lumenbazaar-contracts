import { readFile } from "node:fs/promises";

const bindings = await import("../bindings/upto-session/dist/index.js");
const distSource = await readFile(
  new URL("../bindings/upto-session/dist/index.js", import.meta.url),
  "utf8",
);
const packageJson = JSON.parse(
  await readFile(new URL("../bindings/upto-session/package.json", import.meta.url), "utf8"),
);

const requiredMethods = [
  "interface_version",
  "initialize",
  "create_session",
  "deprecate_asset",
  "remove_asset",
  "get_session",
  "settle",
  "cancel",
  "recover_expired",
  "extend_ttl",
];

if (typeof bindings.Client !== "function") {
  throw new Error("Expected generated Client export");
}

if (bindings.ContractError?.[5]?.message !== "SessionNotFound") {
  throw new Error("Expected stable SessionNotFound error mapping");
}
if (bindings.ContractError?.[22]?.message !== "SessionNotExpired") {
  throw new Error("Expected stable SessionNotExpired error mapping");
}
if (bindings.ContractError?.[25]?.message !== "AssetLiabilityOutstanding") {
  throw new Error("Expected complete asset lifecycle error mapping");
}
if (packageJson.version !== "3.0.0") {
  throw new Error("Expected generated binding package version 3.0.0");
}

for (const method of requiredMethods) {
  if (!distSource.includes(method)) {
    throw new Error(`Expected generated binding to include ${method}`);
  }
}

console.log("upto-session TypeScript binding smoke test passed");
