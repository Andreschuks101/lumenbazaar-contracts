import { Buffer } from "buffer";
import { Address } from "@stellar/stellar-sdk";
import {
  AssembledTransaction,
  Client as ContractClient,
  ClientOptions as ContractClientOptions,
  MethodOptions,
  Result,
  Spec as ContractSpec,
} from "@stellar/stellar-sdk/contract";
import type {
  u32,
  i32,
  u64,
  i64,
  u128,
  i128,
  u256,
  i256,
  Option,
  Timepoint,
  Duration,
} from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}





export interface Session {
  asset: string;
  buyer: string;
  escrowed_amount: i128;
  expires_at_ledger: u32;
  id: Buffer;
  max_amount: i128;
  resource_hash: Buffer;
  seller: string;
  settled_amount: i128;
  status: SessionStatus;
  usage_hash: Option<Buffer>;
}


export interface AssetPolicy {
  asset: string;
  max_session_amount: i128;
}


export interface AdminRotation {
  expires_at_ledger: u32;
  pending_admin: string;
}

export type SessionStatus = {tag: "Open", values: void} | {tag: "Settled", values: void} | {tag: "Cancelled", values: void} | {tag: "Expired", values: void};

export const ContractError = {
  1: {message:"AlreadyInitialized"},
  2: {message:"Unauthorized"},
  3: {message:"InvalidAmount"},
  4: {message:"ExpiredSession"},
  5: {message:"SessionNotFound"},
  6: {message:"SessionAlreadySettled"},
  7: {message:"SessionCancelled"},
  8: {message:"AmountExceedsCap"},
  9: {message:"InvalidAsset"},
  10: {message:"InvalidSeller"},
  11: {message:"TtlExtensionFailed"},
  12: {message:"InvalidResourceHash"},
  13: {message:"InvalidUsageHash"},
  14: {message:"UnsupportedAsset"},
  15: {message:"InvalidSupportedAssets"},
  16: {message:"SessionDurationTooLong"},
  17: {message:"LiabilityOverflow"},
  18: {message:"LiabilityUnderflow"},
  19: {message:"EscrowUnderfunded"},
  20: {message:"NotInitialized"},
  21: {message:"SessionExpired"},
  22: {message:"SessionNotExpired"},
  23: {message:"InvalidAssetCap"},
  24: {message:"AssetCapNotConfigured"},
  25: {message:"InvalidAdmin"},
  26: {message:"InvalidAdminRotationExpiry"},
  27: {message:"AdminRotationPending"},
  28: {message:"AdminRotationNotFound"},
  29: {message:"AdminRotationExpired"},
  30: {message:"TokenInvariantViolation"}
}









export interface Client {
  /**
   * Construct and simulate a cancel transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  cancel: ({session_id}: {session_id: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a settle transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  settle: ({session_id, actual_amount, usage_hash}: {session_id: Buffer, actual_amount: i128, usage_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<string>>>

  /**
   * Construct and simulate a extend_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  extend_ttl: ({session_id}: {session_id: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  initialize: ({admin, asset_policies}: {admin: string, asset_policies: Array<AssetPolicy>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_session transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_session: ({session_id}: {session_id: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Session>>>

  /**
   * Construct and simulate a accept_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  accept_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a propose_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  propose_admin: ({pending_admin, expires_at_ledger}: {pending_admin: string, expires_at_ledger: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a create_session transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  create_session: ({buyer, seller, asset, max_amount, expires_at_ledger, resource_hash}: {buyer: string, seller: string, asset: string, max_amount: i128, expires_at_ledger: u32, resource_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Buffer>>>

  /**
   * Construct and simulate a recover_expired transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  recover_expired: ({session_id}: {session_id: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a interface_version transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  interface_version: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_admin_rotation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_admin_rotation: (options?: MethodOptions) => Promise<AssembledTransaction<Option<AdminRotation>>>

  /**
   * Construct and simulate a get_escrow_liability transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_escrow_liability: ({asset}: {asset: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a cancel_admin_rotation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  cancel_admin_rotation: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_asset_max_session_amount transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_asset_max_session_amount: ({asset}: {asset: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a set_asset_max_session_amount transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_asset_max_session_amount: ({asset, max_session_amount}: {asset: string, max_session_amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

}
export class Client extends ContractClient {
  static async deploy<T = Client>(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions &
      Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
      }
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy(null, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([
        "AAAAAAAAAAAAAAAGY2FuY2VsAAAAAAABAAAAAAAAAApzZXNzaW9uX2lkAAAAAAPuAAAAIAAAAAEAAAPpAAAAAgAAB9AAAAANQ29udHJhY3RFcnJvcgAAAA==",
        "AAAAAAAAAAAAAAAGc2V0dGxlAAAAAAADAAAAAAAAAApzZXNzaW9uX2lkAAAAAAPuAAAAIAAAAAAAAAANYWN0dWFsX2Ftb3VudAAAAAAAAAsAAAAAAAAACnVzYWdlX2hhc2gAAAAAA+4AAAAgAAAAAQAAA+kAAAACAAAH0AAAAA1Db250cmFjdEVycm9yAAAA",
        "AAAAAAAAAAAAAAAJZ2V0X2FkbWluAAAAAAAAAAAAAAEAAAPpAAAAEwAAB9AAAAANQ29udHJhY3RFcnJvcgAAAA==",
        "AAAAAAAAAAAAAAAKZXh0ZW5kX3R0bAAAAAAAAQAAAAAAAAAKc2Vzc2lvbl9pZAAAAAAD7gAAACAAAAABAAAD6QAAAAIAAAfQAAAADUNvbnRyYWN0RXJyb3IAAAA=",
        "AAAAAAAAAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAgAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAA5hc3NldF9wb2xpY2llcwAAAAAD6gAAB9AAAAALQXNzZXRQb2xpY3kAAAAAAQAAA+kAAAACAAAH0AAAAA1Db250cmFjdEVycm9yAAAA",
        "AAAAAAAAAAAAAAALZ2V0X3Nlc3Npb24AAAAAAQAAAAAAAAAKc2Vzc2lvbl9pZAAAAAAD7gAAACAAAAABAAAD6QAAB9AAAAAHU2Vzc2lvbgAAAAfQAAAADUNvbnRyYWN0RXJyb3IAAAA=",
        "AAAAAAAAAAAAAAAMYWNjZXB0X2FkbWluAAAAAAAAAAEAAAPpAAAAAgAAB9AAAAANQ29udHJhY3RFcnJvcgAAAA==",
        "AAAAAAAAAAAAAAANcHJvcG9zZV9hZG1pbgAAAAAAAAIAAAAAAAAADXBlbmRpbmdfYWRtaW4AAAAAAAATAAAAAAAAABFleHBpcmVzX2F0X2xlZGdlcgAAAAAAAAQAAAABAAAD6QAAAAIAAAfQAAAADUNvbnRyYWN0RXJyb3IAAAA=",
        "AAAAAAAAAAAAAAAOY3JlYXRlX3Nlc3Npb24AAAAAAAYAAAAAAAAABWJ1eWVyAAAAAAAAEwAAAAAAAAAGc2VsbGVyAAAAAAATAAAAAAAAAAVhc3NldAAAAAAAABMAAAAAAAAACm1heF9hbW91bnQAAAAAAAsAAAAAAAAAEWV4cGlyZXNfYXRfbGVkZ2VyAAAAAAAABAAAAAAAAAANcmVzb3VyY2VfaGFzaAAAAAAAA+4AAAAgAAAAAQAAA+kAAAPuAAAAIAAAB9AAAAANQ29udHJhY3RFcnJvcgAAAA==",
        "AAAAAAAAAAAAAAAPcmVjb3Zlcl9leHBpcmVkAAAAAAEAAAAAAAAACnNlc3Npb25faWQAAAAAA+4AAAAgAAAAAQAAA+kAAAACAAAH0AAAAA1Db250cmFjdEVycm9yAAAA",
        "AAAAAAAAAAAAAAARaW50ZXJmYWNlX3ZlcnNpb24AAAAAAAAAAAAAAQAAAAQ=",
        "AAAAAAAAAAAAAAASZ2V0X2FkbWluX3JvdGF0aW9uAAAAAAAAAAAAAQAAA+gAAAfQAAAADUFkbWluUm90YXRpb24AAAA=",
        "AAAAAAAAAAAAAAAUZ2V0X2VzY3Jvd19saWFiaWxpdHkAAAABAAAAAAAAAAVhc3NldAAAAAAAABMAAAABAAAD6QAAAAsAAAfQAAAADUNvbnRyYWN0RXJyb3IAAAA=",
        "AAAAAAAAAAAAAAAVY2FuY2VsX2FkbWluX3JvdGF0aW9uAAAAAAAAAAAAAAEAAAPpAAAAAgAAB9AAAAANQ29udHJhY3RFcnJvcgAAAA==",
        "AAAAAAAAAAAAAAAcZ2V0X2Fzc2V0X21heF9zZXNzaW9uX2Ftb3VudAAAAAEAAAAAAAAABWFzc2V0AAAAAAAAEwAAAAEAAAPpAAAACwAAB9AAAAANQ29udHJhY3RFcnJvcgAAAA==",
        "AAAAAAAAAAAAAAAcc2V0X2Fzc2V0X21heF9zZXNzaW9uX2Ftb3VudAAAAAIAAAAAAAAABWFzc2V0AAAAAAAAEwAAAAAAAAASbWF4X3Nlc3Npb25fYW1vdW50AAAAAAALAAAAAQAAA+kAAAACAAAH0AAAAA1Db250cmFjdEVycm9yAAAA",
        "AAAAAQAAAAAAAAAAAAAAB1Nlc3Npb24AAAAACwAAAAAAAAAFYXNzZXQAAAAAAAATAAAAAAAAAAVidXllcgAAAAAAABMAAAAAAAAAD2VzY3Jvd2VkX2Ftb3VudAAAAAALAAAAAAAAABFleHBpcmVzX2F0X2xlZGdlcgAAAAAAAAQAAAAAAAAAAmlkAAAAAAPuAAAAIAAAAAAAAAAKbWF4X2Ftb3VudAAAAAAACwAAAAAAAAANcmVzb3VyY2VfaGFzaAAAAAAAA+4AAAAgAAAAAAAAAAZzZWxsZXIAAAAAABMAAAAAAAAADnNldHRsZWRfYW1vdW50AAAAAAALAAAAAAAAAAZzdGF0dXMAAAAAB9AAAAANU2Vzc2lvblN0YXR1cwAAAAAAAAAAAAAKdXNhZ2VfaGFzaAAAAAAD6AAAA+4AAAAg",
        "AAAAAQAAAAAAAAAAAAAAC0Fzc2V0UG9saWN5AAAAAAIAAAAAAAAABWFzc2V0AAAAAAAAEwAAAAAAAAASbWF4X3Nlc3Npb25fYW1vdW50AAAAAAAL",
        "AAAAAQAAAAAAAAAAAAAADUFkbWluUm90YXRpb24AAAAAAAACAAAAAAAAABFleHBpcmVzX2F0X2xlZGdlcgAAAAAAAAQAAAAAAAAADXBlbmRpbmdfYWRtaW4AAAAAAAAT",
        "AAAAAgAAAAAAAAAAAAAADVNlc3Npb25TdGF0dXMAAAAAAAAEAAAAAAAAAAAAAAAET3BlbgAAAAAAAAAAAAAAB1NldHRsZWQAAAAAAAAAAAAAAAAJQ2FuY2VsbGVkAAAAAAAAAAAAAAAAAAAHRXhwaXJlZAA=",
        "AAAABAAAAAAAAAAAAAAADUNvbnRyYWN0RXJyb3IAAAAAAAAeAAAAAAAAABJBbHJlYWR5SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAADFVuYXV0aG9yaXplZAAAAAIAAAAAAAAADUludmFsaWRBbW91bnQAAAAAAAADAAAAAAAAAA5FeHBpcmVkU2Vzc2lvbgAAAAAABAAAAAAAAAAPU2Vzc2lvbk5vdEZvdW5kAAAAAAUAAAAAAAAAFVNlc3Npb25BbHJlYWR5U2V0dGxlZAAAAAAAAAYAAAAAAAAAEFNlc3Npb25DYW5jZWxsZWQAAAAHAAAAAAAAABBBbW91bnRFeGNlZWRzQ2FwAAAACAAAAAAAAAAMSW52YWxpZEFzc2V0AAAACQAAAAAAAAANSW52YWxpZFNlbGxlcgAAAAAAAAoAAAAAAAAAElR0bEV4dGVuc2lvbkZhaWxlZAAAAAAACwAAAAAAAAATSW52YWxpZFJlc291cmNlSGFzaAAAAAAMAAAAAAAAABBJbnZhbGlkVXNhZ2VIYXNoAAAADQAAAAAAAAAQVW5zdXBwb3J0ZWRBc3NldAAAAA4AAAAAAAAAFkludmFsaWRTdXBwb3J0ZWRBc3NldHMAAAAAAA8AAAAAAAAAFlNlc3Npb25EdXJhdGlvblRvb0xvbmcAAAAAABAAAAAAAAAAEUxpYWJpbGl0eU92ZXJmbG93AAAAAAAAEQAAAAAAAAASTGlhYmlsaXR5VW5kZXJmbG93AAAAAAASAAAAAAAAABFFc2Nyb3dVbmRlcmZ1bmRlZAAAAAAAABMAAAAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAUAAAAAAAAAA5TZXNzaW9uRXhwaXJlZAAAAAAAFQAAAAAAAAARU2Vzc2lvbk5vdEV4cGlyZWQAAAAAAAAWAAAAAAAAAA9JbnZhbGlkQXNzZXRDYXAAAAAAFwAAAAAAAAAVQXNzZXRDYXBOb3RDb25maWd1cmVkAAAAAAAAGAAAAAAAAAAMSW52YWxpZEFkbWluAAAAGQAAAAAAAAAaSW52YWxpZEFkbWluUm90YXRpb25FeHBpcnkAAAAAABoAAAAAAAAAFEFkbWluUm90YXRpb25QZW5kaW5nAAAAGwAAAAAAAAAVQWRtaW5Sb3RhdGlvbk5vdEZvdW5kAAAAAAAAHAAAAAAAAAAU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"
      ]),
      options
    )
  }
  public readonly fromJSON = {
    cancel: this.txFromJSON<Result<void>>,
        settle: this.txFromJSON<Result<void>>,
        get_admin: this.txFromJSON<Result<string>>,
        extend_ttl: this.txFromJSON<Result<void>>,
        initialize: this.txFromJSON<Result<void>>,
        get_session: this.txFromJSON<Result<Session>>,
        accept_admin: this.txFromJSON<Result<void>>,
        propose_admin: this.txFromJSON<Result<void>>,
        create_session: this.txFromJSON<Result<Buffer>>,
        recover_expired: this.txFromJSON<Result<void>>,
        interface_version: this.txFromJSON<u32>,
        get_admin_rotation: this.txFromJSON<Option<AdminRotation>>,
        get_escrow_liability: this.txFromJSON<Result<i128>>,
        cancel_admin_rotation: this.txFromJSON<Result<void>>,
        get_asset_max_session_amount: this.txFromJSON<Result<i128>>,
        set_asset_max_session_amount: this.txFromJSON<Result<void>>
  }
}