// Forbids `.unwrap()` and `.expect(...)` in production contract sources.
// Comments, string literals, `#[cfg(test)]` items and `#[test]` functions are ignored.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const CALL = /\.\s*(unwrap|expect)\s*\(/g;
const TEST_ATTR = /#\s*\[\s*(?:cfg\s*\(\s*test\s*\)|test)\s*\]/g;
const INNER_TEST_ATTR = /#\s*!\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/;

export function isTestPath(relativePath) {
  const parts = relativePath.replaceAll("\\", "/").split("/");
  const name = parts[parts.length - 1];
  if (name === "test.rs" || name.endsWith("_test.rs")) {
    return true;
  }
  return parts.slice(0, -1).some((part) => part === "tests" || part === "benches" || part === "fuzz");
}

// Replace comments and string/char literal contents with spaces, keeping newlines
// and offsets so positions in the masked text match the original.
function mask(src) {
  const out = src.split("");
  const blank = (from, to) => {
    for (let k = from; k < to; k += 1) {
      if (out[k] !== "\n") {
        out[k] = " ";
      }
    }
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    const next = src[i + 1];
    if (c === "/" && next === "/") {
      let end = src.indexOf("\n", i);
      end = end === -1 ? src.length : end;
      blank(i, end);
      i = end;
    } else if (c === "/" && next === "*") {
      let depth = 1;
      let k = i + 2;
      while (k < src.length && depth > 0) {
        if (src[k] === "/" && src[k + 1] === "*") {
          depth += 1;
          k += 2;
        } else if (src[k] === "*" && src[k + 1] === "/") {
          depth -= 1;
          k += 2;
        } else {
          k += 1;
        }
      }
      blank(i, k);
      i = k;
    } else if ((c === "r" || (c === "b" && next === "r")) && /^b?r#*"/.test(src.slice(i, i + 40)) && !/[A-Za-z0-9_]/.test(src[i - 1] ?? "")) {
      const m = /^b?r(#*)"/.exec(src.slice(i, i + 40));
      const close = `"${m[1]}`;
      let end = src.indexOf(close, i + m[0].length);
      end = end === -1 ? src.length : end + close.length;
      blank(i + m[0].length, end - close.length);
      i = end;
    } else if (c === '"') {
      let k = i + 1;
      while (k < src.length && src[k] !== '"') {
        k += src[k] === "\\" ? 2 : 1;
      }
      blank(i + 1, k);
      i = k + 1;
    } else if (c === "'") {
      const m = /^'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^\\'\n])'/.exec(src.slice(i, i + 14));
      if (m) {
        blank(i + 1, i + m[0].length - 1);
        i += m[0].length;
      } else {
        i += 1;
      }
    } else {
      i += 1;
    }
  }
  return out.join("");
}

// Blank out the item that follows a test attribute starting at `start`.
function itemEnd(masked, from) {
  let i = from;
  // skip further attributes
  for (;;) {
    while (/\s/.test(masked[i] ?? "")) {
      i += 1;
    }
    if (masked[i] === "#" && masked[i + 1] === "[") {
      let depth = 0;
      for (; i < masked.length; i += 1) {
        if (masked[i] === "[") depth += 1;
        if (masked[i] === "]" && --depth === 0) {
          i += 1;
          break;
        }
      }
    } else {
      break;
    }
  }
  let paren = 0;
  for (; i < masked.length; i += 1) {
    const c = masked[i];
    if (c === "(" || c === "[") paren += 1;
    else if (c === ")" || c === "]") paren -= 1;
    else if (c === ";" && paren === 0) return i + 1;
    else if (c === "{" && paren === 0) {
      let depth = 0;
      for (; i < masked.length; i += 1) {
        if (masked[i] === "{") depth += 1;
        if (masked[i] === "}" && --depth === 0) return i + 1;
      }
      return masked.length;
    }
  }
  return masked.length;
}

export function scanFile(content, filePath) {
  let masked = mask(content);
  if (INNER_TEST_ATTR.test(masked)) {
    return [];
  }
  const chars = masked.split("");
  for (const m of masked.matchAll(TEST_ATTR)) {
    const end = itemEnd(masked, m.index + m[0].length);
    for (let k = m.index; k < end; k += 1) {
      if (chars[k] !== "\n") chars[k] = " ";
    }
  }
  masked = chars.join("");

  const lineStarts = [0];
  for (let k = 0; k < masked.length; k += 1) {
    if (masked[k] === "\n") lineStarts.push(k + 1);
  }
  const lines = content.split("\n");
  const violations = [];
  for (const m of masked.matchAll(CALL)) {
    let lo = 0;
    let hi = lineStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (lineStarts[mid] <= m.index) lo = mid;
      else hi = mid - 1;
    }
    violations.push({
      file: filePath,
      line: lo + 1,
      column: m.index - lineStarts[lo] + 1,
      type: m[1],
      snippet: (lines[lo] ?? "").replace(/\r$/, "").trim(),
    });
  }
  return violations;
}

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      return entry.name === "target" || entry.name === "node_modules" ? [] : rustFiles(path);
    }
    return entry.name.endsWith(".rs") ? [path] : [];
  });
}

export function checkNoProductionUnwrap(repoRoot) {
  const results = [];
  for (const path of rustFiles(join(repoRoot, "contracts")).sort()) {
    const rel = relative(repoRoot, path).replaceAll("\\", "/");
    if (isTestPath(rel)) {
      continue;
    }
    for (const v of scanFile(readFileSync(path, "utf8"), rel)) {
      results.push(`${v.file}:${v.line}:${v.column}: forbidden ${v.type} call`);
    }
  }
  return results;
}
