// "Check with AI" (docs/phase1-plan.md §15.1; src/core/ai-check.ts): the
// button at the foot of the suggestions list sends nothing but the request,
// the lantern delegates while it runs, the AI's suggestions are marked, the
// notes say what happened, a failed automatic check stays quiet, and the
// "check automatically" setting is off by default.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State, DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const { aiSending, nextAiCheck, aiCheckNote, AI_KIND } = await import("../src/core/ai-check.ts");
const { buildSuggestions } = await import("../src/views/views.ts");
const { aiAutoRow } = await import("../src/settings/writing.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const ev = (o) => ({ phase: "done", id: 7, auto: false, added: 0, code: "", retryAfter: null, ...o });
const item = (problem, extra = {}) => ({
  start: 0, end: problem.length, problem, before: "", after: " at the unit.", kind: "Spelling",
  message: "m", replacements: ["x"], clear: true, ...extra,
});

test("the lantern delegates only while a check is on its way", () => {
  assert.equal(aiSending(ev({ phase: "sending" })), true);
  assert.equal(aiSending(ev({ phase: "done" })), false);
  assert.equal(aiSending(null), false);
  assert.match(read("src/island/island.ts"), /sendingToApi\(State\.rewrite\) \|\| aiSending\(State\.aiCheck\)/);
});

test("a failed automatic check says nothing; a failed click says why", () => {
  assert.equal(nextAiCheck(ev({ phase: "failed", auto: true, code: "no-key" })), null);
  const click = ev({ phase: "failed", code: "no-key" });
  assert.equal(nextAiCheck(click), click);
  assert.equal(aiCheckNote(click), "Add your Anthropic API key in Settings → AI first.");
  assert.equal(aiCheckNote(ev({ phase: "failed", code: "sensitive" })), "Not sent: this has a card, ID or bank number in it, or it's a payment field. Glim never reads or sends those.");
  assert.equal(aiCheckNote(ev({ phase: "failed", code: "cap" })), "This month's spending cap is reached, so nothing more is sent this month.");
  assert.equal(aiCheckNote(ev({ phase: "failed", code: "no-paragraph" })), "Click in the paragraph you want checked first.");
  assert.equal(aiCheckNote(ev({ phase: "failed", code: "changed" })), "The paragraph changed while Claude was checking. Check again.");
});

test("the note says what the check found", () => {
  assert.equal(aiCheckNote(ev({ phase: "sending" })), "Checking with Claude…");
  assert.equal(aiCheckNote(ev({ added: 1 })), "Claude found 1 more suggestion.");
  assert.equal(aiCheckNote(ev({ added: 2 })), "Claude found 2 more suggestions.");
  assert.equal(aiCheckNote(ev({ added: 0 })), "Claude found nothing more to fix.");
  assert.equal(aiCheckNote(null), "");
});

test("Check with AI sits at the foot of the list, sends no words, and waits while one runs", () => {
  State.writing = [item("recieved")];
  State.writingFix = { checkId: 3, mode: "apply", app: "Notepad" };
  State.aiCheck = null;
  const view = buildSuggestions();
  view.sync();
  const button = view.el.find("button").find((b) => b.textContent === "Check with AI");
  assert.ok(button, "the button is there");
  assert.ok(view.el.querySelector(".suggestion-pager").find("button").includes(button), "at the foot of the list");
  const n = sent("writing_ai_check").length;
  button.fire("click");
  assert.equal(sent("writing_ai_check").length, n + 1);
  assert.deepEqual(sent("writing_ai_check").at(-1) ?? {}, {}, "no arguments: Rust has the paragraph");
  State.aiCheck = ev({ phase: "sending" });
  view.sync();
  assert.equal(button.disabled, true);
  assert.ok(view.el.textContent.includes("Checking with Claude…"));
  button.fire("click");
  assert.equal(sent("writing_ai_check").length, n + 1, "not twice");
  State.aiCheck = null;
  State.writing = [];
});

test("the AI's suggestions are marked AI; Harper's aren't", () => {
  State.writing = [item("Alex and I was", { kind: AI_KIND, replacements: ["Alex and I were"], clear: false }), item("recieved")];
  State.writingFix = { checkId: 4, mode: "apply", app: "Notepad" };
  const view = buildSuggestions();
  view.sync();
  const rows = view.el.find(".suggestion");
  assert.equal(rows.length, 2);
  assert.equal(rows[0].find(".ai-badge").length, 1);
  assert.equal(rows[0].find(".ai-badge")[0].textContent, "AI");
  assert.equal(rows[1].find(".ai-badge").length, 0);
  State.writing = [];
});

test("check automatically is off by default and the switch stores it", () => {
  assert.equal(DEFAULT_SETTINGS.aiAutoCheck, false);
  const settings = { ...DEFAULT_SETTINGS };
  let saves = 0;
  const row = aiAutoRow({ settings: () => settings, save: async () => { saves++; } });
  const sw = row.find("button").find((b) => b.getAttribute("data-role") === "ai-auto-check");
  assert.equal(sw.getAttribute("aria-pressed"), "false");
  sw.fire("click");
  assert.equal(settings.aiAutoCheck, true);
  sw.fire("click");
  assert.equal(settings.aiAutoCheck, false);
  assert.equal(saves, 2);
  assert.match(row.textContent, /only when you click Check with AI/);
});

test("Rust side: Haiku only, forced tool, sensitive scan before sending, setting off by default", () => {
  const check = read("src-tauri/src/ai/check.rs");
  assert.ok(check.includes("pub const MODEL: &str = claude::DEFAULT_MODEL;"));
  assert.ok(check.includes('"tool_choice": { "type": "tool", "name": TOOL }'));
  const claude = read("src-tauri/src/ai/claude.rs");
  const send = claude.slice(claude.indexOf("pub(super) async fn send"));
  assert.ok(send.indexOf("contains_sensitive_number") < send.indexOf("net::request"), "scanned before anything leaves");
  assert.ok(read("src-tauri/src/settings.rs").includes("ai_auto_check: false,"));
});
