// The file drop zone (1d retest, 2026-10-14): a file dragged anywhere else
// on screen passes Glim by; only a drag onto the pill or the ember (within
// 40 px) arms the bucket, and leaving disarms it at once. The arming itself
// is Rust's (island.rs drop_zone_armed, with its own tests); here: the
// bucket folds away on leave, the switch, and the wiring.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { DropZoneSession } = await import("../src/core/drop-zone.ts");
const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

// Retest defect (2026-10-16): a drag that passed near the pill opened the
// bucket, and leaving went to "what was waiting", which is nearly always the
// writing suggestions, so the island stayed open. Now it goes back to exactly
// how it looked before the drag.
test("pass near, then drop elsewhere: the pill folds back even with suggestions waiting", () => {
  const s = new DropZoneSession();
  s.enter(false, "suggestions");
  // Rust's disarm (the drop landed elsewhere) reaches the island as a leave.
  assert.equal(s.leave("upload", true), "collapse");
});

test("pass near, then Esc: the pill folds back", () => {
  const s = new DropZoneSession();
  s.enter(false, "empty");
  assert.equal(s.leave("upload", true), "collapse");
});

test("an island that was open before the drag goes back to the same view", () => {
  const s = new DropZoneSession();
  s.enter(true, "overview");
  assert.equal(s.leave("upload", true), "overview");
});

test("only the first enter counts: repeated overs don't overwrite what was there", () => {
  const s = new DropZoneSession();
  s.enter(false, "suggestions");
  s.enter(true, "upload");
  assert.equal(s.leave("upload", true), "collapse");
});

test("drop on the bucket still works: the file carries on, a later leave changes nothing", () => {
  const s = new DropZoneSession();
  s.enter(false, "suggestions");
  s.drop();
  assert.equal(s.leave("uploading", true), "stay", "a file already taken carries on");
  assert.equal(s.leave("upload", false), "stay");
});

test("the island wires the session into enter, leave and drop", () => {
  const island = read("src/island/island.ts");
  assert.ok(island.includes('this.dropZone.enter(State.mode === "expanded", State.view);'));
  assert.ok(island.includes('this.dropZone.leave(State.view, State.mode === "expanded")'));
  assert.ok(island.includes("this.dropZone.drop();"));
  assert.ok(!island.includes("afterDragLeave"));
});

test("the File drop zone switch is on by default and turns the zone off entirely", () => {
  assert.equal(DEFAULT_SETTINGS.fileDropZone, true);
  const bridge = read("src/core/bridge.ts");
  assert.ok(bridge.includes("if (!hasFiles(e) || !enabled()) return;"), "off: the page refuses the drag");
  const island = read("src/island/island.ts");
  assert.ok(island.includes("void onDragDrop((e) => this.onDragDrop(e), () => State.settings.fileDropZone !== false);"));
  assert.ok(island.includes("if (State.fileDragOver || !State.settings.fileDropZone) return;"));
  const lib = read("src-tauri/src/lib.rs");
  assert.equal(lib.split("drop_zone.store(").length - 1, 2, "at launch and on every save");
});

test("Rust's disarm reaches the island as a leave, even without a dragleave", () => {
  const bridge = read("src/core/bridge.ts");
  const at = bridge.indexOf('listen("file-drag-disarm"');
  assert.ok(at > 0);
  assert.match(bridge.slice(at, at + 200), /depth = 0;\s+handler\(\{ type: "leave" \}\);/);
});
