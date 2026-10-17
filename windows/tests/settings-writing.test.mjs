// Settings → Writing (1d, docs/phase1-plan.md §14): the app switches draw the
// defaults plus the user's overrides, store only what differs, and keep an
// answered app's entry either way.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { writingSection, appOn, switchApp, answeredApps, setTone, categoryTone } = await import("../src/settings/writing.ts");
const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");

const KNOWN = [
  { exe: "WINWORD.EXE", name: "Word", defaultOn: true, chat: false, fixes: true },
  { exe: "slack.exe", name: "Slack", defaultOn: true, chat: true, fixes: false },
  { exe: "Code.exe", name: "VS Code", defaultOn: false, chat: false, fixes: false },
];

test("a switch reads the user's choice in any case, else the default", () => {
  const map = { "winword.exe": "off", "Code.exe": "on", "Notepad.exe": "maybe" };
  assert.equal(appOn(map, "WINWORD.EXE", true), false);
  assert.equal(appOn(map, "code.exe", false), true);
  assert.equal(appOn(map, "Notepad.exe", true), true, "an unknown value is the default");
  assert.equal(appOn({}, "OUTLOOK.EXE", true), true);
});

test("only what differs from the default is stored; answered apps keep their entry", () => {
  const word = KNOWN[0];
  let map = switchApp({}, "WINWORD.EXE", false, word);
  assert.deepEqual(map, { "WINWORD.EXE": "off" });
  map = switchApp(map, "winword.exe", true, word);
  assert.deepEqual(map, {}, "back at its default");
  map = switchApp({ "Acrobat.exe": "on" }, "acrobat.exe", false, null);
  assert.deepEqual(map, { "Acrobat.exe": "off" }, "Never-ed apps stay listed, or they'd be asked again");
});

test("the list is the built-in apps, then the answered ones", () => {
  assert.deepEqual(answeredApps({ "Acrobat.exe": "on", "winword.exe": "off", "Zed.exe": "off" }, KNOWN), ["Acrobat.exe", "Zed.exe"]);
  const settings = { ...DEFAULT_SETTINGS, writingApps: { "Acrobat.exe": "off", "WINWORD.EXE": "off" } };
  let saves = 0;
  const el = writingSection({ settings: () => settings, save: async () => { saves++; } }, KNOWN);
  const switches = el.find("button").filter((b) => b.getAttribute("data-exe"));
  assert.deepEqual(switches.map((b) => b.getAttribute("data-exe")), ["WINWORD.EXE", "slack.exe", "Code.exe", "Acrobat.exe"]);
  assert.deepEqual(switches.map((b) => b.classList.contains("on")), [false, true, false, false]);
  const text = el.textContent;
  assert.ok(text.includes("chat rules") && text.includes("suggestions only"), text);

  switches[2].fire("click");
  assert.deepEqual(settings.writingApps, { "Acrobat.exe": "off", "WINWORD.EXE": "off", "Code.exe": "on" });
  assert.equal(saves, 1);
});

const DEFAULTS = [["email", "formal"], ["documents", "formal"], ["chat", "casual"], ["other", "grammar"]];

test("a category's tone is stored only when it differs from the built-in one", () => {
  assert.deepEqual(setTone({}, "email", "formal", "formal"), {});
  assert.deepEqual(setTone({}, "email", "shorter", "formal"), { email: "shorter" });
  assert.deepEqual(setTone({ email: "shorter" }, "email", "formal", "formal"), {});
  assert.deepEqual(setTone({ "OUTLOOK.EXE": "casual" }, "outlook.exe", "", null), {}, "an app's own tone removed");
  assert.deepEqual(setTone({}, "OUTLOOK.EXE", "casual", null), { "OUTLOOK.EXE": "casual" });
  assert.equal(categoryTone({ chat: "pirate" }, "chat", "casual"), "casual", "an unknown tone reads as the built-in one");
  assert.equal(categoryTone({ chat: "friendlier" }, "chat", "casual"), "friendlier");
});

test("Rewrite tones: a select per kind of app, and a tone of its own for one app", () => {
  const settings = { ...DEFAULT_SETTINGS, writingApps: {}, rewriteTones: { "OUTLOOK.EXE": "shorter" } };
  let saves = 0;
  const known = [...KNOWN, { exe: "OUTLOOK.EXE", name: "Outlook", defaultOn: true, chat: false, fixes: true }];
  const el = writingSection({ settings: () => settings, save: async () => { saves++; } }, known, DEFAULTS);
  const selects = el.find("select");
  const byCat = (id) => selects.find((s) => s.getAttribute("data-category") === id);
  assert.deepEqual(DEFAULTS.map(([id]) => byCat(id).value), ["formal", "formal", "casual", "grammar"]);
  assert.ok(el.textContent.includes("Chat apps and chat sites"));
  // Casual is offered.
  assert.ok(byCat("chat").find("option").some((o) => o.getAttribute("value") === "casual"));

  const chat = byCat("chat");
  chat.value = "friendlier";
  chat.fire("change");
  assert.deepEqual(settings.rewriteTones, { "OUTLOOK.EXE": "shorter", chat: "friendlier" });

  // Outlook has its own; the add row offers the other apps the checker is on in.
  const own = el.find("select").find((s) => s.getAttribute("data-exe") === "OUTLOOK.EXE");
  assert.equal(own.value, "shorter");
  const app = el.find("select").find((s) => s.getAttribute("data-role") === "tone-app");
  const options = app.find("option").map((o) => o.getAttribute("value"));
  assert.deepEqual(options, ["WINWORD.EXE", "slack.exe"], "on apps only, not Outlook again, not VS Code (off)");
  app.value = "slack.exe";
  const pick = el.find("select").find((s) => s.getAttribute("data-role") === "tone-pick");
  pick.value = "formal";
  el.find("button").find((b) => b.textContent === "Add").fire("click");
  assert.deepEqual(settings.rewriteTones, { "OUTLOOK.EXE": "shorter", chat: "friendlier", "slack.exe": "formal" });
  assert.equal(saves, 2);
});

test("Strictness: one global level, full by default, stored as its id", () => {
  const settings = { ...DEFAULT_SETTINGS, writingApps: {}, rewriteTones: {} };
  assert.equal(settings.checkerStrictness, "full");
  let saves = 0;
  const el = writingSection({ settings: () => settings, save: async () => { saves++; } }, KNOWN, DEFAULTS);
  const select = el.find("select").find((s) => s.getAttribute("data-role") === "strictness");
  assert.deepEqual(select.find("option").map((o) => o.getAttribute("value")), ["spelling", "standard", "full"]);
  assert.equal(select.value, "full");
  select.value = "spelling";
  select.fire("change");
  assert.equal(settings.checkerStrictness, "spelling");
  select.value = "loud";
  select.fire("change");
  assert.equal(settings.checkerStrictness, "full", "anything else is full");
  assert.equal(saves, 2);
});

test("Strictness: a line under each level says what it covers, the current one marked, and the AI note", () => {
  const settings = { ...DEFAULT_SETTINGS, writingApps: {}, rewriteTones: {}, checkerStrictness: "standard" };
  const el = writingSection({ settings: () => settings, save: async () => {} }, KNOWN, DEFAULTS);
  const covers = el.find("ul").find((u) => u.getAttribute("data-role") === "strictness-covers");
  const lines = () => covers.find("li").map((li) => [li.getAttribute("data-level"), li.className, li.textContent]);
  assert.deepEqual(lines().map(([l, c]) => [l, c]), [["spelling", ""], ["standard", "current"], ["full", ""]]);
  assert.match(lines()[0][2], /^Spelling only Misspelled words/);
  assert.match(lines()[2][2], /8am in the morning/);
  const select = el.find("select").find((s) => s.getAttribute("data-role") === "strictness");
  select.value = "full";
  select.fire("change");
  assert.deepEqual(lines().map(([, c]) => c), ["", "", "current"]);
  const note = el.find("div").find((d) => d.getAttribute("data-role") === "strictness-ai-note");
  assert.match(note.textContent, /Alex and I was.*need the AI/);
});
