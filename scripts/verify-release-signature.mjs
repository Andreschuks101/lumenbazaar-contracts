import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { verifyReleaseSignature } from "./lib/release-signature.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const artifact = argument("--artifact");
const signature = argument("--signature");
if (!artifact || !signature) throw new Error("Usage: --artifact <file> --signature <file>");
const result = verifyReleaseSignature({
  artifactPath: resolve(repoRoot, artifact),
  signaturePath: resolve(repoRoot, signature),
  signersPath: join(repoRoot, "release-signers.json"),
  at: argument("--at") || new Date(),
});
console.log(`verified ${result.artifactSha256} signed by ${result.identity} (${result.keyId})`);

function argument(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? null : process.argv[index + 1];
}
