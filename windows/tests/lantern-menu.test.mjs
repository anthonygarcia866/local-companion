// The lantern's right-click menu (src/island/menu.ts): Settings…, Hide, Ember,
// Dock position. Drawn in the island page (no native popup: that would need
// the foreground and take focus).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { sent } from "./tauri.mjs";
import { DOCK_LABELS, MENU_W, menuPlacement, runMenuAction } from "../src/island/menu.ts";
import { DOCKS } from "../src/core/layout.ts";

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("the menu opens at the cursor and stays inside the window", () => {
  assert.deepEqual(menuPlacement(100, 40, MENU_W, 250, 720, 320), { x: 100, y: 40 });
  // Near the right edge: pulled back in.
  assert.deepEqual(menuPlacement(700, 40, MENU_W, 250, 720, 320), { x: 720 - MENU_W - 6, y: 40 });
  // Too low to open downwards: opens upwards from the cursor.
  assert.deepEqual(menuPlacement(100, 300, MENU_W, 250, 720, 320), { x: 100, y: 50 });
  // Taller than the room either way: as high as it goes.
  assert.deepEqual(menuPlacement(100, 100, MENU_W, 400, 720, 320).y, 6);
});

test("each item does what Settings and the tray do", () => {
  const n = (cmd) => sent(cmd).length;
  const [s, v, d] = [n("open_settings_window"), n("set_visibility"), n("set_dock")];
  runMenuAction({ kind: "settings" });
  runMenuAction({ kind: "visibility", visibility: "hidden" });
  runMenuAction({ kind: "visibility", visibility: "ember" });
  runMenuAction({ kind: "dock", dock: "left" });
  assert.equal(n("open_settings_window"), s + 1);
  assert.deepEqual(sent("set_visibility").slice(v), [{ visibility: "hidden" }, { visibility: "ember" }]);
  assert.deepEqual(sent("set_dock").slice(d), [{ dock: "left" }]);
});

test("every dock is in the menu, with Settings' labels", () => {
  assert.deepEqual(Object.keys(DOCK_LABELS).sort(), [...DOCKS].sort());
  const settings = read("src/settings/main.ts");
  for (const label of Object.values(DOCK_LABELS)) assert.ok(settings.includes(`t("${label}")`), label);
});

test("right-click on the lantern opens the menu; a right press never opens the island", () => {
  const island = read("src/island/island.ts");
  assert.match(island, /else if \(e\.button !== 2\) this\.fsm\.click\(\);/);
  assert.match(island, /if \(State\.mode === "expanded" && !this\.isBotHit\(e\.clientX, e\.clientY\)\) return;\n\s+e\.preventDefault\(\);\n\s+this\.dockPress = null;\n\s+this\.menu\.show\(e\.clientX, e\.clientY, State\.dock\);/);
});

test("the open menu takes the mouse: its rect joins the island's hit rect", () => {
  const island = read("src/island/island.ts");
  const geo = island.slice(island.indexOf("  private applyGeometry() {"));
  assert.match(geo, /const m = this\.menuRect;\n\s+if \(m\) \{/);
  assert.match(geo, /void Bridge\.setIslandRect\(rect\.x, rect\.y, rect\.w, rect\.h\);/);
});

test("the menu closes when Glim goes to the ember or hides", () => {
  const main = read("src/main.ts");
  assert.match(main, /if \(kind !== "pill"\) island\.closeMenu\(\);/);
});

test("it is never a native popup menu (that would take focus)", () => {
  for (const p of ["src/island/menu.ts", "src/island/island.ts"]) {
    assert.ok(!/popup_menu|popupMenu|Menu\.new|\.popup\(/.test(read(p)), p);
  }
});
