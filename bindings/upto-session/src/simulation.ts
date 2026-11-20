import { decodeContractError, type DecodedContractError } from "./contractErrors.js";

export type SimulationOutcome<T> =
  | { status: "ready"; transaction: T; resourceFee: string; footprint: unknown; authorization: readonly unknown[] }
  | { status: "restore_required"; transaction: T; restorePreamble: unknown }
  | { status: "contract_error"; error: DecodedContractError }
  | { status: "network_error"; error: unknown };

export interface SimulatedTransactionLike {
  simulation?: {
    minResourceFee?: string | number | bigint;
    transactionData?: unknown;
    result?: { auth?: unknown[] };
    restorePreamble?: unknown;
  };
}

/** Builds and simulates a call. This helper has no signing or submission API. */
export async function simulateContractCall<T extends SimulatedTransactionLike>(
  assemble: () => Promise<T>,
): Promise<SimulationOutcome<T>> {
  try {
    const transaction = await assemble();
    const simulation = transaction.simulation;
    if (!simulation) {
      return { status: "network_error", error: new Error("RPC returned no simulation result") };
    }
    if (simulation.restorePreamble) {
      return { status: "restore_required", transaction, restorePreamble: simulation.restorePreamble };
    }
    return {
      status: "ready",
      transaction,
      resourceFee: String(simulation.minResourceFee ?? "0"),
      footprint: simulation.transactionData,
      authorization: Object.freeze([...(simulation.result?.auth ?? [])]),
    };
  } catch (error) {
    const decoded = decodeContractError(error);
    return decoded.kind === "contract" || decoded.kind === "unknown_contract"
      ? { status: "contract_error", error: decoded }
      : { status: "network_error", error };
  }
}
