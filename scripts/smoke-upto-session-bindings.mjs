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
  "get_asset_max_session_amount",
  "set_asset_max_session_amount",
  "get_admin",
  "get_admin_rotation",
  "propose_admin",
  "accept_admin",
  "cancel_admin_rotation",
  "get_configuration",
  "create_session",
  "deprecate_asset",
  "remove_asset",
  "get_session",
  "get_escrow_liability",
  "settle",
  "cancel",
  "recover_expired",
  "recover_expired_batch",
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
if (bindings.ContractError?.[24]?.message !== "AssetCapNotConfigured") {
  throw new Error("Expected stable asset cap error mapping");
}
if (bindings.ContractError?.[29]?.message !== "AdminRotationExpired") {
  throw new Error("Expected complete governance error mapping");
}
if (bindings.ContractError?.[31]?.message !== "InvalidBatchSize") {
  throw new Error("Expected stable InvalidBatchSize error mapping");
}
if (bindings.ContractError?.[32]?.message !== "InvalidAssetInterface") {
  throw new Error("Expected complete v3 ContractError mapping");
}
if (bindings.ContractError?.[35]?.message !== "AssetLiabilityOutstanding") {
  throw new Error("Expected complete asset lifecycle error mapping");
}
if (packageJson.version !== "3.0.0") {
  throw new Error("Expected generated binding package version 3.0.0");
}
if (typeof bindings.deriveSessionId !== "function") {
  throw new Error("Expected deriveSessionId export");
}
if (typeof bindings.deriveSessionIdPreimage !== "function") {
  throw new Error("Expected deriveSessionIdPreimage export");
}

for (const method of requiredMethods) {
  if (!distSource.includes(method)) {
    throw new Error(`Expected generated binding to include ${method}`);
  }
}

console.log("upto-session TypeScript binding smoke test passed");
