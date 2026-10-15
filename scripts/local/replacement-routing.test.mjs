import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  createMigrationState,
  deprecateDeployment,
  recordSessionRoute,
  routeExistingSession,
  routeNewSession,
  switchActiveDeployment,
  validateMigrationState,
} from "./replacement-routing.mjs";

const oldDeployment = {
  contractId: "C-OLD-ESCROW",
  release: "old-v2",
  interfaceVersion: 2,
};
const replacementDeployment = {
  contractId: "C-REPLACEMENT-ESCROW",
  release: "replacement-v2",
  interfaceVersion: 2,
};
const repoRoot = join(fileURLToPath(new URL(".", import.meta.url)), "..", "..");

test("existing sessions stay pinned through switch and rollback", () => {
  const state = createMigrationState(oldDeployment, replacementDeployment);
  recordSessionRoute(state, "old-session");

  switchActiveDeployment(state, "replacement");
  recordSessionRoute(state, "new-session");
  assert.equal(routeExistingSession(state, "old-session"), oldDeployment.contractId);
  assert.equal(
    routeExistingSession(state, "new-session"),
    replacementDeployment.contractId,
  );

  switchActiveDeployment(state, "old");
  assert.equal(routeNewSession(state), oldDeployment.contractId);
  assert.equal(routeExistingSession(state, "old-session"), oldDeployment.contractId);
  assert.equal(
    routeExistingSession(state, "new-session"),
    replacementDeployment.contractId,
  );
  validateMigrationState(state);
});

test("deprecation preserves old routes and blocks new old sessions", () => {
  const state = createMigrationState(oldDeployment, replacementDeployment);
  recordSessionRoute(state, "old-session");
  switchActiveDeployment(state, "replacement");
  deprecateDeployment(state, "old");

  assert.equal(routeExistingSession(state, "old-session"), oldDeployment.contractId);
  assert.equal(routeNewSession(state), replacementDeployment.contractId);
  assert.throws(() => switchActiveDeployment(state, "old"), /deprecated/);
});

test("ambiguous deployments and cross-contract route rewrites fail closed", () => {
  assert.throws(
    () => createMigrationState(oldDeployment, { ...replacementDeployment, contractId: oldDeployment.contractId }),
    /must be different/,
  );
  const state = createMigrationState(oldDeployment, replacementDeployment);
  recordSessionRoute(state, "old-session");
  assert.throws(
    () => recordSessionRoute(state, "old-session", replacementDeployment.contractId),
    /already pinned/,
  );
  assert.throws(() => routeExistingSession(state, "unknown-session"), /No deployment route/);
});

test("committed replacement manifest remains a valid routing example", () => {
  const example = JSON.parse(
    readFileSync(join(repoRoot, "deployments", "replacement-rehearsal.example.json"), "utf8"),
  );
  validateMigrationState(example);
  assert.equal(routeExistingSession(example, "old-session-id"), "C-OLD-ESCROW");
  assert.equal(
    routeExistingSession(example, "replacement-session-id"),
    "C-REPLACEMENT-ESCROW",
  );
  assert.equal(routeNewSession(example), "C-REPLACEMENT-ESCROW");
});
