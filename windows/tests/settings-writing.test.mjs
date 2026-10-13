// Settings → Writing (1d, docs/phase1-plan.md §14): the app switches draw the
// defaults plus the user's overrides, store only what differs, and keep an
// answered app's entry either way.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { writingSection, appOn, switchApp, answeredApps } = await import("../src/settings/writing.ts");
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
