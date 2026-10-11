// Visibility modes, the page's half (src/island/presence.ts). The rule itself —
// recording is never hidden — is enforced in Rust (presence.rs tests); these
// check that the page draws the indicator only through the reserved s-record
// path, and draws something for every kind Rust can send while recording.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const { presenceLook, isPresenceKind } = await import("../src/island/presence.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");

test("each kind draws what it says", () => {
  assert.deepEqual(presenceLook("pill"), { island: true, dot: false, record: false });
  assert.deepEqual(presenceLook("ember"), { island: false, dot: true, record: false });
  assert.deepEqual(presenceLook("hidden"), { island: false, dot: false, record: false });
  assert.deepEqual(presenceLook("indicator"), { island: false, dot: false, record: true });
});

test("the recording indicator is the reserved s-record lantern, and only it is", () => {
  const src = read("src/island/presence.ts");
  const calls = [...src.matchAll(/showReserved\(([^)]*)\)/g)].map(([, a]) => a.replace(/\s+/g, " "));
  assert.deepEqual(calls, ['RECORDING_STATE, "screen-recording"']);
  // Shown only when the look says record.
  assert.match(src, /if \(look\.record\) this\.indicator\.showReserved\(RECORDING_STATE, "screen-recording"\)/);
  for (const kind of ["pill", "ember", "hidden"]) assert.equal(presenceLook(kind).record, false, kind);
});

test("the page knows every kind Rust sends", () => {
  const rust = read("src-tauri/src/presence.rs");
  const kinds = [...rust.matchAll(/Presence::\w+ => "(\w+)"/g)].map(([, k]) => k);
  assert.deepEqual(kinds.sort(), ["ember", "hidden", "indicator", "pill"]);
  for (const k of kinds) assert.ok(isPresenceKind(k), k);
  assert.equal(isPresenceKind("nonsense"), false);
});

test("RECORDING_STATE is s-record", async () => {
  const { RECORDING_STATE, RESERVED_LANTERN_STATES } = await import("../src/mascot/lantern.ts");
  assert.equal(RESERVED_LANTERN_STATES[RECORDING_STATE], "screen-recording");
});
