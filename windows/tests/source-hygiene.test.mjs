// A string literal broken across lines where an escape was meant ("\r\n"
// written as a real line break) still compiles, and even passes here on LF
// files, then fails on CI's CRLF checkout (seen twice on 2026-10-10).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WINDOWS = join(dirname(fileURLToPath(import.meta.url)), "..");

function files(dir, out = []) {
  for (const e of readdirSync(join(WINDOWS, dir), { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) files(p, out);
    else if (/\.(rs|ts|mjs)$/.test(e.name)) out.push(p);
  }
  return out;
}

test("no string literal opens on a call and breaks the line where an escape belongs", () => {
  const broken = /\.(?:replace|replaceAll|find|contains|includes|split|starts_with|ends_with)\("\r?$/m;
  const hits = [];
  for (const f of [...files("src-tauri/src"), ...files("src"), ...files("tests"), ...files("scripts")]) {
    readFileSync(join(WINDOWS, f), "utf8").split("\n").forEach((line, i) => {
      if (broken.test(line)) hits.push(`${f}:${i + 1}: ${line.trim()}`);
    });
  }
  assert.deepEqual(hits, []);
});
