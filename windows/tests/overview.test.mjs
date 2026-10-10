// The overview's right card is the switcher for the other pills; with none it
// was an empty card (live test 2026-10-10). It is hidden then.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");

test("the switcher card shows only when there is another pill", async () => {
  const { showsSwitcher } = await import("../src/core/layout.ts");
  assert.equal(showsSwitcher(0), false);
  assert.equal(showsSwitcher(1), true);
  assert.equal(showsSwitcher(4), true);
});

test("an overview without the switcher hides the card and widens the session card", () => {
  const views = read("src/views/views.ts");
  assert.match(views, /el\.classList\.toggle\("solo", !showsSwitcher\(others\.length\)\)/);
  const css = read("src/style.css").replace(/\r\n/g, "\n");
  assert.match(css, /\.overview\.solo > \.right \{\n  display: none;/);
  assert.match(css, /\.overview\.solo > \.left \{\n  flex: 1 1 auto;\n  width: auto;/);
});
