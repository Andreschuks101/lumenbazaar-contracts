import { Address, hash, xdr } from "@stellar/stellar-sdk";
import { Buffer } from "buffer";

export const SESSION_ID_DOMAIN = "lumenbazaar/session-id";
export const INTERFACE_VERSION = 3;

const MAX_I128 = 0x7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffffn;
const MAX_U64 = 0xffff_ffff_ffff_ffffn;
const MAX_U32 = 0xffff_ffff;

export interface SessionIdInputV3 {
  networkPassphrase?: string;
  networkId?: string;
  contractId: string;
  buyer: string;
  seller: string;
  asset: string;
  maxAmount: string | bigint | number;
  expiresAtLedger: number;
  resourceHash: string;
  sequence: string | bigint | number;
  interfaceVersion?: number;
}

export function validateSessionIdInput(
  value: unknown,
): asserts value is SessionIdInputV3 {
  if (!isRecord(value)) {
    throw new Error("Session ID input must be an object");
  }

  if (value.interfaceVersion !== undefined) {
    if (value.interfaceVersion === 2) {
      throw new Error(
        "unsupported interfaceVersion 2: v2 and v3 domain rules remain explicitly separated",
      );
    }
    if (value.interfaceVersion !== INTERFACE_VERSION) {
      throw new Error(
        `unsupported interfaceVersion ${value.interfaceVersion}: expected ${INTERFACE_VERSION}`,
      );
    }
  }

  const hasPassphrase = typeof value.networkPassphrase === "string" && value.networkPassphrase.length > 0;
  const hasNetworkId = typeof value.networkId === "string" && value.networkId.length > 0;

  if (!hasPassphrase && !hasNetworkId) {
    throw new Error("Either networkPassphrase or networkId must be provided");
  }

  if (hasPassphrase) {
    if (!isBoundedCanonicalString(value.networkPassphrase, 256)) {
      throw new Error("networkPassphrase is invalid");
    }
  }

  if (hasNetworkId) {
    if (typeof value.networkId !== "string" || !/^[0-9a-f]{64}$/.test(value.networkId)) {
      throw new Error("networkId must be a 64-character lowercase hex string");
    }
  }

  if (hasPassphrase && hasNetworkId) {
    const derivedNetworkId = Buffer.from(hash(Buffer.from(value.networkPassphrase as string, "utf8"))).toString("hex");
    if (derivedNetworkId !== value.networkId) {
      throw new Error("networkPassphrase does not match networkId");
    }
  }

  if (typeof value.contractId !== "string" || !/^C[A-Z2-7]{55}$/.test(value.contractId)) {
    throw new Error("contractId is invalid: must be a valid Stellar contract address");
  }
  try {
    Address.fromString(value.contractId);
  } catch {
    throw new Error("contractId is not a valid Stellar address");
  }

  if (typeof value.buyer !== "string" || !/^[CG][A-Z2-7]{55}$/.test(value.buyer)) {
    throw new Error("buyer is invalid: must be a valid Stellar address");
  }
  try {
    Address.fromString(value.buyer);
  } catch {
    throw new Error("buyer is not a valid Stellar address");
  }

  if (typeof value.seller !== "string" || !/^[CG][A-Z2-7]{55}$/.test(value.seller)) {
    throw new Error("seller is invalid: must be a valid Stellar address");
  }
  try {
    Address.fromString(value.seller);
  } catch {
    throw new Error("seller is not a valid Stellar address");
  }

  if (typeof value.asset !== "string" || !/^C[A-Z2-7]{55}$/.test(value.asset)) {
    throw new Error("asset is invalid: must be a valid Stellar contract address");
  }
  try {
    Address.fromString(value.asset);
  } catch {
    throw new Error("asset is not a valid Stellar address");
  }

  const parsedMaxAmount = parseI128(value.maxAmount, "maxAmount");
  if (parsedMaxAmount <= 0n) {
    throw new Error("maxAmount must be positive");
  }

  if (!isUint32(value.expiresAtLedger) || value.expiresAtLedger === 0) {
    throw new Error("expiresAtLedger must be a positive uint32");
  }

  if (typeof value.resourceHash !== "string" || !/^[0-9a-f]{64}$/.test(value.resourceHash)) {
    throw new Error("resourceHash must be a 64-character lowercase hex string");
  }

  const parsedSequence = parseU64(value.sequence, "sequence");
  if (parsedSequence < 0n) {
    throw new Error("sequence cannot be negative");
  }
}

export function deriveSessionIdPreimage(input: SessionIdInputV3): Buffer {
  validateSessionIdInput(input);

  const networkIdBuffer = input.networkPassphrase
    ? Buffer.from(hash(Buffer.from(input.networkPassphrase, "utf8")))
    : Buffer.from(input.networkId as string, "hex");

  const domainScVal = xdr.ScVal.scvVec([
    xdr.ScVal.scvBytes(Buffer.from(SESSION_ID_DOMAIN, "utf8")),
    xdr.ScVal.scvU32(INTERFACE_VERSION),
    xdr.ScVal.scvBytes(networkIdBuffer),
    Address.fromString(input.contractId).toScVal(),
  ]);

  const maxAmountBig = parseI128(input.maxAmount, "maxAmount");
  const hi = xdr.Int64.fromString((maxAmountBig >> 64n).toString());
  const lo = xdr.Uint64.fromString((maxAmountBig & 0xffff_ffff_ffff_ffffn).toString());

  const sequenceBig = parseU64(input.sequence, "sequence");

  const payloadScVal = xdr.ScVal.scvVec([
    Address.fromString(input.buyer).toScVal(),
    Address.fromString(input.seller).toScVal(),
    Address.fromString(input.asset).toScVal(),
    xdr.ScVal.scvI128(new xdr.Int128Parts({ hi, lo })),
    xdr.ScVal.scvU32(input.expiresAtLedger),
    xdr.ScVal.scvBytes(Buffer.from(input.resourceHash, "hex")),
    xdr.ScVal.scvU64(xdr.Uint64.fromString(sequenceBig.toString())),
  ]);

  const topScVal = xdr.ScVal.scvVec([
    xdr.ScVal.scvBytes(domainScVal.toXDR()),
    xdr.ScVal.scvBytes(payloadScVal.toXDR()),
  ]);

  return Buffer.from(topScVal.toXDR());
}

export function deriveSessionId(input: SessionIdInputV3): string {
  const preimage = deriveSessionIdPreimage(input);
  return Buffer.from(hash(preimage)).toString("hex");
}

function parseI128(value: unknown, field: string): bigint {
  if (typeof value === "bigint") {
    if (value <= 0n || value > MAX_I128) {
      throw new Error(`${field} must be between 1 and ${MAX_I128}`);
    }
    return value;
  }
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value) || value <= 0) {
      throw new Error(`${field} must be a positive safe integer`);
    }
    return BigInt(value);
  }
  if (typeof value === "string") {
    if (!/^[1-9][0-9]*$/.test(value)) {
      throw new Error(`${field} must be a positive canonical decimal string`);
    }
    const parsed = BigInt(value);
    if (parsed > MAX_I128) {
      throw new Error(`${field} exceeds maximum i128 value`);
    }
    return parsed;
  }
  throw new Error(`${field} must be a string, number, or bigint`);
}

function parseU64(value: unknown, field: string): bigint {
  if (typeof value === "bigint") {
    if (value < 0n || value > MAX_U64) {
      throw new Error(`${field} must be between 0 and ${MAX_U64}`);
    }
    return value;
  }
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new Error(`${field} must be a non-negative safe integer`);
    }
    return BigInt(value);
  }
  if (typeof value === "string") {
    if (!/^(0|[1-9][0-9]*)$/.test(value)) {
      throw new Error(`${field} must be a non-negative canonical decimal string`);
    }
    const parsed = BigInt(value);
    if (parsed > MAX_U64) {
      throw new Error(`${field} exceeds maximum u64 value`);
    }
    return parsed;
  }
  throw new Error(`${field} must be a string, number, or bigint`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isUint32(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0 && Number(value) <= MAX_U32;
}

function isBoundedCanonicalString(value: unknown, maxLength: number): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= maxLength &&
    value === value.normalize("NFC") &&
    value === value.trim()
  );
}
