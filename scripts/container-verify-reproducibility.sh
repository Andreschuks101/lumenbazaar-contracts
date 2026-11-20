#!/usr/bin/env bash
set -euo pipefail
umask 077
first="$(mktemp -d)"
second="$(mktemp -d)"
source_epoch="${SOURCE_DATE_EPOCH:-0}"
trap 'rm -rf "$first" "$second"' EXIT

for output in "$first" "$second"; do
  tar --exclude=target --exclude=.git -cf - . | tar -xf - -C "$output"
  (cd "$output" && SOURCE_DATE_EPOCH="$source_epoch" stellar contract build --locked)
  find "$output/target/wasm32v1-none/release" -name '*.wasm' -type f -print0 | sort -z | xargs -0 sha256sum > "$output/wasm.sha256"
  while IFS= read -r wasm; do
    stellar contract info interface --wasm "$wasm" --output json
  done < <(find "$output/target/wasm32v1-none/release" -name '*.wasm' -type f | sort) > "$output/specs.json"
done

sed "s#${first}#BUILD#g" "$first/wasm.sha256" > "$first/normalized.sha256"
sed "s#${second}#BUILD#g" "$second/wasm.sha256" > "$second/normalized.sha256"
cmp "$first/normalized.sha256" "$second/normalized.sha256"
cmp "$first/specs.json" "$second/specs.json"
echo "two clean contract builds produced identical WASM hashes and specs"
