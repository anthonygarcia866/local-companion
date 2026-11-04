// Polished dictation (2026-11-03): the level picker and per-app levels in
// Settings → Writing → Dictation, and which results open a card.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { dictationGroup, dictationLevel, appLevel, setAppLevel } = await import("../src/settings/writing.ts");
const { needsCard, rawBadge } = await import("../src/core/voice.ts");
const { voiceText } = await import("../src/views/voice.ts");
const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");

const KNOWN = [
  { exe: "OUTLOOK.EXE", name: "Outlook", defaultOn: true, chat: false, fixes: true },
  { exe: "ms-teams.exe", name: "Teams", defaultOn: true, chat: true, fixes: false },
  { exe: "Code.exe", name: "VS Code", defaultOn: false, chat: false, fixes: false },
];

test("Polished is the default; a level per app is stored only where it differs", () => {
  assert.equal(DEFAULT_SETTINGS.dictationLevel, "polished");
  assert.equal(dictationLevel(undefined), "polished");
  assert.equal(dictationLevel("nonsense"), "polished");
  assert.equal(dictationLevel("verbatim"), "verbatim");
  let map = setAppLevel({}, "ms-teams.exe", "clean");
  assert.deepEqual(map, { "ms-teams.exe": "clean" });
  assert.equal(appLevel(map, "MS-TEAMS.EXE"), "clean");
  map = setAppLevel(map, "MS-Teams.exe", "");
  assert.deepEqual(map, {}, "back to the default: nothing stored");
});

test("the picker and the per-app rows save what is chosen", () => {
  const settings = { ...DEFAULT_SETTINGS, dictationLevels: {} };
  let saves = 0;
  const el = dictationGroup({ settings: () => settings, save: async () => { saves++; } }, KNOWN);
  const selects = el.find("select");
  const level = selects.find((s) => s.getAttribute("data-role") === "dictation-level");
  assert.equal(level.value, "polished");
  level.value = "clean";
  level.fire("change");
  assert.equal(settings.dictationLevel, "clean");
  const rows = selects.filter((s) => s.getAttribute("data-dictation-exe"));
  assert.deepEqual(rows.map((s) => s.getAttribute("data-dictation-exe")), ["OUTLOOK.EXE", "ms-teams.exe"]);
  rows[1].value = "verbatim";
  rows[1].fire("change");
  assert.deepEqual(settings.dictationLevels, { "ms-teams.exe": "verbatim" });
  assert.equal(saves, 2);
});

test("a polished fallback says so; verbatim needs no card or badge", () => {
  const typed = (cleaned, code) => ({ phase: "typed", cleaned, code, app: "Outlook", text: "", mode: "dictate", level: 0, question: "", steps: [], tip: false, history: [] });
  assert.equal(needsCard(typed(true, "")), false, "a polished text in the field says it");
  assert.equal(needsCard(typed(true, "polish-detail")), true);
  assert.match(voiceText(typed(true, "polish-detail")).note, /left out a detail you said/);
  assert.equal(voiceText(typed(true, "polish-detail")).title, "Typed in Outlook");
  assert.equal(needsCard(typed(true, "polish-borrowed")), true);
  assert.match(voiceText(typed(true, "polish-borrowed")).note, /added words you didn't say/);
  assert.equal(needsCard(typed(false, "verbatim")), false);
  assert.equal(rawBadge(typed(false, "verbatim")), false);
  assert.equal(needsCard(typed(false, "timeout")), true, "an unplanned raw result still shows");
});
