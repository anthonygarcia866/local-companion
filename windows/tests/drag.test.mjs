// Dragging Glim from the open island (src/island/drag.ts, island.ts): the
// lantern or the top bar starts a drag in every view, a control never does,
// and the click that ends a drag never presses a button.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { isControl, startsBarDrag, TOP_BAR_HEIGHT } = await import("../src/island/drag.ts");
const { h } = await import("../src/views/dom.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("buttons, links, fields and anything inside them are controls", () => {
  const label = h("span", { text: "Allow" });
  const button = h("button", {}, label);
  const field = h("div");
  field.setAttribute("role", "textbox");
  for (const el of [button, label, h("a"), h("input"), h("textarea"), field]) assert.ok(isControl(el));
  for (const el of [h("div"), h("span", { text: "Claude Code" }), null]) assert.ok(!isControl(el));
});

test("a left press on the top bar starts a drag, below it or on a button it doesn't", () => {
  const title = h("div", { class: "title" });
  const card = h("div", { class: "card" }, title, h("button", {}, h("span", { text: "Deny" })));
  assert.ok(startsBarDrag(0, 10, title));
  assert.ok(startsBarDrag(0, TOP_BAR_HEIGHT - 1, card));
  assert.ok(!startsBarDrag(0, TOP_BAR_HEIGHT, title), "below the bar: the card's own area");
  assert.ok(!startsBarDrag(2, 10, title), "a right press is the menu's");
  assert.ok(!startsBarDrag(0, 10, card.children[1]), "a button on the bar stays a button");
});

test("every drag start swallows the click that ends it, in the capture phase", () => {
  const src = read("src/island/island.ts");
  const starts = src.split("void Bridge.dockDragStart();").length - 1;
  const swallowed = src.match(/this\.swallowClick = true;\n\s*(?:this\.cancelBotHover\(\);\n\s*)?void Bridge\.dockDragStart\(\);/g) ?? [];
  assert.equal(starts, 3, "the closed pill, the lantern and the top bar");
  assert.equal(swallowed.length, starts);
  assert.match(src, /window\.addEventListener\("click", \(e\) => \{\n\s*if \(!this\.swallowClick\) return;[\s\S]*?e\.stopPropagation\(\);[\s\S]*?\}, true\);/);
  // A new press always starts clean, so a drag that fired no click can't eat the next real one.
  assert.match(src, /addEventListener\("mousedown", \(e\) => \{\n\s*State\.lastActivity = performance\.now\(\);\n\s*this\.swallowClick = false;/);
});
