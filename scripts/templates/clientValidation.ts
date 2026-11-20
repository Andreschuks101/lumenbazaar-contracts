import { Networks, StrKey } from "@stellar/stellar-sdk";
import type { ClientOptions } from "@stellar/stellar-sdk/contract";
import { Client } from "./index.js";

const SHA256 = /^[0-9a-f]{64}$/;

export interface DeploymentIdentity {
  contractId: string;
  networkPassphrase: string;
  network: "testnet" | "custom";
  manifestSha256: string;
  /** Required for custom networks so equal passphrases cannot alias deployments. */
  networkId?: string;
}

export function validateClientIdentity(options: ClientOptions, deployment: DeploymentIdentity): void {
  if (!StrKey.isValidContract(options.contractId) || options.contractId !== deployment.contractId) {
    throw new Error("Contract ID is invalid or does not match the deployment manifest");
  }
  if (options.networkPassphrase !== deployment.networkPassphrase || !SHA256.test(deployment.manifestSha256)) {
    throw new Error("Client network or deployment manifest identity does not match");
  }
  if (deployment.network === "testnet") {
    if (options.networkPassphrase !== Networks.TESTNET) throw new Error("Testnet deployment requires the Stellar testnet passphrase");
  } else if (!deployment.networkId || !/^[a-z0-9][a-z0-9._-]{2,63}$/i.test(deployment.networkId)) {
    throw new Error("Custom networks require an explicit stable networkId");
  }
}

/** Validates manifest association before a generated client can invoke RPC. */
export function createValidatedClient(options: ClientOptions, deployment: DeploymentIdentity): Client {
  validateClientIdentity(options, deployment);
  return new Client(options);
}
