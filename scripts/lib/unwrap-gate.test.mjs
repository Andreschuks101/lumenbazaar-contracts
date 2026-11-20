import assert from "node:assert/strict";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { checkNoProductionUnwrap, isTestPath, scanFile } from "./unwrap-gate.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("detects unwrap and expect with line numbers", () => {
  const src = `fn a() {\n    let x = foo().unwrap();\n}\nfn b() {\n    bar().expect("boom");\n}\n`;
  const found = scanFile(src, "contracts/x/src/lib.rs");
  assert.deepEqual(
    found.map((v) => [v.file, v.line, v.type]),
    [
      ["contracts/x/src/lib.rs", 2, "unwrap"],
      ["contracts/x/src/lib.rs", 5, "expect"],
    ],
  );
  assert.equal(found[0].column, 18);
  assert.equal(found[0].snippet, "let x = foo().unwrap();");
});

test("test paths are exempt", () => {
  for (const p of [
    "contracts/a/src/foo_test.rs",
    "contracts/a/src/test.rs",
    "contracts/a/tests/session.rs",
    "contracts/a/benches/b.rs",
    "contracts/a/fuzz/f.rs",
  ]) {
    assert.equal(isTestPath(p), true, p);
  }
  assert.equal(isTestPath("contracts/a/src/lib.rs"), false);
  assert.equal(isTestPath("contracts/a/src/contest.rs"), false);
});

test("cfg(test) modules and test functions are exempt", () => {
  const src = `fn prod() -> u32 { 1 }\n#[cfg(test)]\nmod tests {\n    fn t() { x().unwrap(); y().expect("z"); }\n}\n#[test]\nfn solo() { x().unwrap(); }\nfn bad() { x().unwrap(); }\n`;
  const found = scanFile(src, "a.rs");
  assert.deepEqual(found.map((v) => v.line), [8]);
});

test("comments and string literals are ignored", () => {
  const src = `// a.unwrap()\n/* b.expect("x") /* nested.unwrap() */ */\nfn f() { let s = ".unwrap()"; let r = r#"x.expect("y")"#; let c = '\\''; }\n`;
  assert.deepEqual(scanFile(src, "a.rs"), []);
});

test("safe alternatives are permitted", () => {
  const src = `fn f(o: Option<u32>) -> u32 {\n    let _ = o.unwrap_or(1);\n    let _ = o.unwrap_or_default();\n    let _ = o.unwrap_or_else(|| 2);\n    match o { Some(v) => v, None => 0 }\n}\n`;
  assert.deepEqual(scanFile(src, "a.rs"), []);
});

test("current workspace contracts have no production unwrap or expect", () => {
  assert.deepEqual(checkNoProductionUnwrap(repoRoot), []);
});
