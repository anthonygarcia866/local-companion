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
const { afterDragLeave } = await import("../src/core/drop-zone.ts");
const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("leaving the zone folds the bucket away, back to what was waiting", () => {
  assert.equal(afterDragLeave("upload", true, null), "collapse");
  assert.equal(afterDragLeave("upload", true, "approval"), "approval");
  assert.equal(afterDragLeave("upload", true, "suggestions"), "suggestions");
  assert.equal(afterDragLeave("uploading", true, null), "stay", "a file already taken carries on");
  assert.equal(afterDragLeave("upload", false, null), "stay");
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
