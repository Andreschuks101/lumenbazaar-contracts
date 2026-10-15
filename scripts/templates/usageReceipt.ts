import { hash } from "@stellar/stellar-sdk";
import { Buffer } from "buffer";

export interface UsageMeasurementV1 {
  unit: string;
  quantity: string;
  startedAtLedger: number;
  endedAtLedger: number;
}

export interface UsageReceiptV1 {
  schemaVersion: 1;
  protocol: "lumenbazaar";
  interfaceVersion: number;
  networkPassphrase: string;
  contractId: string;
  sessionId: string;
  amount: string;
  resourceHash: string;
  measurement: UsageMeasurementV1;
  receiptId: string;
}

export function canonicalizeUsageReceipt(receipt: UsageReceiptV1): string {
  validateReceipt(receipt);
  return JSON.stringify({
    schemaVersion: receipt.schemaVersion,
    protocol: receipt.protocol,
    interfaceVersion: receipt.interfaceVersion,
    networkPassphrase: receipt.networkPassphrase,
    contractId: receipt.contractId,
    sessionId: receipt.sessionId,
    amount: receipt.amount,
    resourceHash: receipt.resourceHash,
    measurement: {
      unit: receipt.measurement.unit,
      quantity: receipt.measurement.quantity,
      startedAtLedger: receipt.measurement.startedAtLedger,
      endedAtLedger: receipt.measurement.endedAtLedger,
    },
    receiptId: receipt.receiptId,
  });
}

export function hashUsageReceipt(receipt: UsageReceiptV1): string {
  return Buffer.from(hash(Buffer.from(canonicalizeUsageReceipt(receipt), "utf8"))).toString("hex");
}

export function parseCanonicalUsageReceipt(value: string): UsageReceiptV1 {
  const parsed: unknown = JSON.parse(value);
  validateReceipt(parsed);
  const canonical = canonicalizeUsageReceipt(parsed);
  if (canonical !== value) {
    throw new Error("usage receipt is not the canonical byte encoding");
  }
  return parsed;
}

function validateReceipt(value: unknown): asserts value is UsageReceiptV1 {
  if (!isRecord(value) || !hasExactKeys(value, [
    "schemaVersion", "protocol", "interfaceVersion", "networkPassphrase",
    "contractId", "sessionId", "amount", "resourceHash", "measurement", "receiptId",
  ])) {
    throw new Error("usage receipt fields do not match schema v1");
  }
  if (value.schemaVersion !== 1 || value.protocol !== "lumenbazaar") {
    throw new Error("unsupported usage receipt version or protocol");
  }
  if (!isUint32(value.interfaceVersion) || value.interfaceVersion === 0) {
    throw new Error("interfaceVersion must be a positive u32");
  }
  if (!isBoundedCanonicalString(value.networkPassphrase, 256)) {
    throw new Error("networkPassphrase is invalid");
  }
  if (typeof value.contractId !== "string" || !/^C[A-Z2-7]{55}$/.test(value.contractId)) {
    throw new Error("contractId is invalid");
  }
  if (typeof value.sessionId !== "string" || !/^[0-9a-f]{64}$/.test(value.sessionId)) {
    throw new Error("sessionId is invalid");
  }
  if (typeof value.amount !== "string" || !/^[1-9][0-9]*$/.test(value.amount)) {
    throw new Error("amount must be a positive canonical decimal string");
  }
  if (typeof value.resourceHash !== "string" || !/^[0-9a-f]{64}$/.test(value.resourceHash)) {
    throw new Error("resourceHash is invalid");
  }
  if (!isRecord(value.measurement) || !hasExactKeys(value.measurement, [
    "unit", "quantity", "startedAtLedger", "endedAtLedger",
  ])) {
    throw new Error("measurement fields do not match schema v1");
  }
  if (typeof value.measurement.unit !== "string" || !/^[a-z][a-z0-9._-]{0,63}$/.test(value.measurement.unit)) {
    throw new Error("measurement unit is invalid");
  }
  if (typeof value.measurement.quantity !== "string" || !/^[1-9][0-9]*$/.test(value.measurement.quantity)) {
    throw new Error("measurement quantity must be a positive canonical decimal string");
  }
  if (!isUint32(value.measurement.startedAtLedger) || !isUint32(value.measurement.endedAtLedger) || value.measurement.endedAtLedger < value.measurement.startedAtLedger) {
    throw new Error("measurement ledger range is invalid");
  }
  if (typeof value.receiptId !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(value.receiptId)) {
    throw new Error("receiptId is invalid");
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === expected.length && expected.every((key) => actual.includes(key));
}

function isUint32(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0 && Number(value) <= 0xffff_ffff;
}

function isBoundedCanonicalString(value: unknown, maxLength: number): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= maxLength &&
    value === value.normalize("NFC") && value === value.trim();
}
