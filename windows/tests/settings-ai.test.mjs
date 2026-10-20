// Settings → AI (1c): the key goes to Rust once, on Save, and the field is
// emptied at once; only a hint ever comes back. The cap, the prices and the
// spend line read right.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { aiSection, parseCap, priceText, spendCost, spendText } = await import("../src/settings/ai.ts");
const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("a typed cap is dollars, or no cap", () => {
  assert.equal(parseCap("5"), 5);
  assert.equal(parseCap(" $12.345 "), 12.35);
  for (const none of ["", "0", "-3", "abc", "Infinity"]) assert.equal(parseCap(none), 0, none);
});

test("prices and spend read as dollars", () => {
  const haiku = { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", inputUsdPerMtok: 0.1, outputUsdPerMtok: 0.5, pricesChecked: "2026-10-12" };
  assert.equal(priceText(haiku), "$0.10 / $0.50 per million tokens");
  assert.equal(spendText({ month: "2026-10", usd: 0.0123, requests: 3, capUsd: 0, atCap: false }), "This month: $0.0123 (3 requests)");
  assert.equal(spendText({ month: "2026-10", usd: 0, requests: 1, capUsd: 0, atCap: false }), "This month: $0.00 (1 request)");
  // Tiny amounts keep their digits (a rewrite is ~$0.00004; the owner saw "$0.00", 2026-10-13).
  assert.equal(spendCost(0.00013), "$0.0001");
  assert.equal(spendCost(0.0042), "$0.0042");
  assert.equal(spendCost(0.0999), "$0.0999");
  assert.equal(spendCost(0.00004), "under $0.0001");
  assert.equal(spendCost(0.47), "$0.47");
  assert.equal(spendCost(12.3456), "$12.35");
  assert.equal(spendCost(0), "$0.00");
  assert.equal(spendCost(Number.NaN), "$0.00");
});

test("the key goes to Rust once, on Save, and leaves the page at once", async () => {
  const settings = { ...DEFAULT_SETTINGS };
  const el = await aiSection({ settings: () => settings, save: async () => {} });
  const input = el.find("input").find((i) => i.getAttribute("type") === "password");
  assert.ok(input, "a password field");
  input.value = "sk-ant-api03-SENTINEL-abcd";
  const n = sent("ai_key_save").length;
  el.find("button").find((b) => b.textContent === "Save key").fire("click");
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent("ai_key_save").slice(n), [{ key: "sk-ant-api03-SENTINEL-abcd" }]);
  assert.equal(input.value, "", "emptied as soon as it is sent");
});

test("the page never keeps, logs or stores the key", () => {
  const src = read("src/settings/ai.ts");
  for (const banned of ["console.log", "console.error", "console.warn", "console.info", "localStorage", "sessionStorage", "settings().aiKey"]) assert.ok(!src.includes(banned), banned);
  // The key reaches Rust through one call only.
  assert.deepEqual(src.match(/Bridge\.aiKey\w+\([^)]*\)/g), ["Bridge.aiKeyStatus()", "Bridge.aiKeyRemove()", "Bridge.aiKeySave(typed)"]);
  // And settings.json has no place for it.
  assert.ok(!Object.keys(DEFAULT_SETTINGS).some((k) => /key/i.test(k) && k !== "shortcuts"));
});
