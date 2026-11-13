// Settings → Secrets (local agency v1, part 1): the value goes to Rust once,
// on Save, and leaves the page at once; the page only ever shows names and
// dates; Delete asks first; Replace keeps the name.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { secretsSection, savedText } = await import("../src/settings/secrets.ts");
const { sent, replies } = await import("./tauri.mjs");

const tick = () => new Promise((r) => setTimeout(r, 0));
const VALUE = "placeholder-SENTINEL-value";
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const roles = (el, tag, r) => el.find(tag).filter((x) => x.getAttribute("data-role") === r);
const text = (el) => el.find("span").map((s) => s.textContent).join(" | ");

test("saved dates read as local time", () => {
  const ms = new Date(2026, 10, 12, 14, 5).getTime();
  assert.equal(savedText(ms), "saved 2026-11-12 14:05");
  assert.equal(savedText(0), "saved ?");
});

test("the value goes to Rust once, on Save, and leaves the page at once", async () => {
  replies.set("secrets_list", []);
  const el = secretsSection();
  await tick();
  const value = role(el, "input", "secrets-value");
  assert.equal(value.getAttribute("type"), "password", "masked");
  role(el, "input", "secrets-name").value = "EXAMPLE_TOKEN";
  value.value = VALUE;
  const n = sent("secrets_save").length;
  replies.set("secrets_save", { name: "EXAMPLE_TOKEN", savedMs: 1_700_000_000_000 });
  replies.set("secrets_list", [{ name: "EXAMPLE_TOKEN", savedMs: 1_700_000_000_000 }]);
  role(el, "button", "secrets-save").fire("click");
  assert.equal(value.value, "", "emptied as soon as it is sent");
  await tick();
  await tick();
  assert.deepEqual(sent("secrets_save").slice(n), [{ name: "EXAMPLE_TOKEN", value: VALUE }]);
  // The list shows the name and its date, and nothing on the page holds the value.
  assert.match(text(el), /EXAMPLE_TOKEN/);
  assert.match(text(el), /saved \d{4}-\d\d-\d\d/);
  for (const input of el.find("input")) assert.ok(!String(input.value).includes("SENTINEL"));
  assert.ok(!text(el).includes("SENTINEL"));
});

test("Delete asks first; Cancel keeps it, Delete deletes it", async () => {
  replies.set("secrets_list", [{ name: "EXAMPLE_TOKEN", savedMs: 1_700_000_000_000 }]);
  const el = secretsSection();
  await tick();
  const n = sent("secrets_delete").length;
  role(el, "button", "secrets-delete").fire("click");
  assert.equal(sent("secrets_delete").length, n, "nothing deleted before the answer");
  assert.ok(role(el, "div", "secrets-confirm"), "asks");
  role(el, "button", "secrets-delete-no").fire("click");
  assert.equal(sent("secrets_delete").length, n);
  assert.equal(roles(el, "div", "secrets-confirm").length, 0);

  role(el, "button", "secrets-delete").fire("click");
  replies.set("secrets_list", []);
  role(el, "button", "secrets-delete-yes").fire("click");
  await tick();
  await tick();
  assert.deepEqual(sent("secrets_delete").slice(n), [{ name: "EXAMPLE_TOKEN" }]);
  assert.match(text(el) + el.find("div").map((d) => d.textContent).join(" "), /No secrets yet/);
});

test("Replace keeps the name and sends a new value", async () => {
  replies.set("secrets_list", [{ name: "EXAMPLE_TOKEN", savedMs: 1_700_000_000_000 }]);
  const el = secretsSection();
  await tick();
  role(el, "button", "secrets-replace").fire("click");
  const name = role(el, "input", "secrets-name");
  assert.equal(name.value, "EXAMPLE_TOKEN");
  assert.notEqual(name.getAttribute("disabled"), null, "the name is locked");
  role(el, "input", "secrets-value").value = "placeholder-new";
  const n = sent("secrets_save").length;
  replies.set("secrets_save", { name: "EXAMPLE_TOKEN", savedMs: 1_700_000_000_001 });
  role(el, "button", "secrets-save").fire("click");
  await tick();
  assert.deepEqual(sent("secrets_save").slice(n), [{ name: "EXAMPLE_TOKEN", value: "placeholder-new" }]);
});

test("Secrets is in the sidebar, after Memory", async () => {
  const { NAV } = await import("../src/settings/nav.ts");
  const ids = NAV.map((e) => e.id);
  assert.equal(ids.indexOf("settings-secrets"), ids.indexOf("settings-memory") + 1);
});
