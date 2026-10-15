export function createMigrationState(oldDeployment, replacementDeployment) {
  validateDeployment(oldDeployment);
  validateDeployment(replacementDeployment);
  if (oldDeployment.contractId === replacementDeployment.contractId) {
    throw new Error("Old and replacement contract IDs must be different.");
  }
  return {
    schemaVersion: 1,
    activeRole: "old",
    deployments: {
      old: { ...oldDeployment, acceptsNewSessions: true },
      replacement: { ...replacementDeployment, acceptsNewSessions: true },
    },
    sessionRoutes: {},
    transitions: [{ action: "initialize", activeRole: "old" }],
  };
}

export function switchActiveDeployment(state, role) {
  const deployment = deploymentForRole(state, role);
  if (!deployment.acceptsNewSessions) {
    throw new Error(`Deployment ${role} is deprecated for new sessions.`);
  }
  state.activeRole = role;
  state.transitions.push({ action: "switch", activeRole: role });
  return deployment.contractId;
}

export function deprecateDeployment(state, role) {
  if (state.activeRole === role) {
    throw new Error("The active deployment cannot be deprecated.");
  }
  deploymentForRole(state, role).acceptsNewSessions = false;
  state.transitions.push({ action: "deprecate", role });
}

export function routeNewSession(state) {
  const deployment = deploymentForRole(state, state.activeRole);
  if (!deployment.acceptsNewSessions) {
    throw new Error(`Deployment ${state.activeRole} is deprecated for new sessions.`);
  }
  return deployment.contractId;
}

export function recordSessionRoute(state, sessionId, contractId = routeNewSession(state)) {
  if (!sessionId || typeof sessionId !== "string") {
    throw new Error("Session ID must be a non-empty string.");
  }
  const knownContractIds = Object.values(state.deployments).map((entry) => entry.contractId);
  if (!knownContractIds.includes(contractId)) {
    throw new Error(`Unknown deployment contract ID: ${contractId}`);
  }
  const existing = state.sessionRoutes[sessionId];
  if (existing && existing !== contractId) {
    throw new Error(`Session ${sessionId} is already pinned to ${existing}.`);
  }
  state.sessionRoutes[sessionId] = contractId;
  return contractId;
}

export function routeExistingSession(state, sessionId) {
  const contractId = state.sessionRoutes[sessionId];
  if (!contractId) {
    throw new Error(`No deployment route is recorded for session ${sessionId}.`);
  }
  return contractId;
}

export function validateMigrationState(state) {
  if (state.schemaVersion !== 1 || !state.deployments || !state.sessionRoutes) {
    throw new Error("Invalid replacement migration state.");
  }
  const oldId = deploymentForRole(state, "old").contractId;
  const replacementId = deploymentForRole(state, "replacement").contractId;
  if (oldId === replacementId) {
    throw new Error("Replacement migration contract IDs are ambiguous.");
  }
  deploymentForRole(state, state.activeRole);
  for (const [sessionId, contractId] of Object.entries(state.sessionRoutes)) {
    if (!sessionId || (contractId !== oldId && contractId !== replacementId)) {
      throw new Error(`Invalid route for session ${sessionId}.`);
    }
  }
  return state;
}

function deploymentForRole(state, role) {
  const deployment = state.deployments?.[role];
  if (!deployment) {
    throw new Error(`Unknown deployment role: ${role}`);
  }
  return deployment;
}

function validateDeployment(deployment) {
  if (
    !deployment?.contractId ||
    !deployment?.release ||
    !Number.isInteger(deployment?.interfaceVersion)
  ) {
    throw new Error("A deployment requires contractId, release, and interfaceVersion.");
  }
}
