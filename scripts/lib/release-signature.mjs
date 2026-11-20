import { createHash, verify } from "node:crypto";
import { readFileSync } from "node:fs";

export function verifyReleaseSignature({ artifactPath, signaturePath, signersPath, at = new Date() }) {
  const bytes = readFileSync(artifactPath);
  const attestation = JSON.parse(readFileSync(signaturePath, "utf8"));
  const registry = JSON.parse(readFileSync(signersPath, "utf8"));
  const signer = registry.signers?.find((entry) => entry.keyId === attestation.keyId);
  if (!signer) throw new Error(`Unknown release signer: ${attestation.keyId}`);
  if (signer.revoked) throw new Error(`Release signer is revoked: ${signer.keyId}`);
  const instant = new Date(at).getTime();
  if (instant < Date.parse(signer.validFrom) || instant >= Date.parse(signer.expiresAt)) {
    throw new Error(`Release signer is not valid at verification time: ${signer.keyId}`);
  }
  if (signer.algorithm !== "ed25519" || attestation.algorithm !== "ed25519") {
    throw new Error("Only pinned Ed25519 release signatures are accepted");
  }
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== attestation.artifactSha256) throw new Error("Signed artifact checksum does not match artifact bytes");
  if (!verify(null, bytes, signer.publicKeyPem, Buffer.from(attestation.signature, "base64"))) {
    throw new Error(`Invalid release signature from ${signer.keyId}`);
  }
  return { keyId: signer.keyId, identity: signer.identity, artifactSha256: digest };
}
