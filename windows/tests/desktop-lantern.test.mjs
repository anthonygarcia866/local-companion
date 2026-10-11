// The desktop character's window is parked off screen but stays shown on
// Windows, and Windows moves it back into view; a lantern drawn while he is
// home showed up as a stray lantern on the desktop (live test 2026-10-10).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../src/desktop/main.ts", import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("the desktop lantern starts undrawn", () => {
  const ctor = src.slice(src.indexOf("constructor(canvas"), src.indexOf("canvas.append(el);"));
  assert.match(ctor, /el\.style\.display = "none";/);
});

test("it is drawn exactly while he is on the desktop", () => {
  const body = src.slice(src.indexOf("private setVisible(on: boolean) {"));
  const first = body.split("\n").slice(1, 3).join("\n");
  assert.match(first, /this\.visible = on;\n\s+this\.lantern\.el\.style\.display = on \? "" : "none";/);
});
