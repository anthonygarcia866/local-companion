// The Rewrite card (1c, docs/phase1-plan.md §13): the hotkey or the button
// opens it without sending anything; a tone click sends a tone id only;
// Rust's events move it along; "Use this" and Copy send the rewrite's id
// only; the lantern shows the "data leaving" state while text is on its way.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { TONES, chooseTone, sending, nextRewrite, sendingToApi, costText } = await import("../src/core/rewrite.ts");
const { buildRewrite, failureText, rewriteButtons, rewriteNote } = await import("../src/views/rewrite.ts");
const { runGlobalShortcut } = await import("../src/island/shortcuts.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

/** A Rust event with defaults. */
const ev = (phase, extra = {}) => ({
  phase, id: 7, tone: "", text: "", mode: "view", app: "", offerable: false,
  model: "", costUsd: 0, code: "", retryAfter: null, outcome: "", ...extra,
});

const done = (extra = {}) => nextRewrite(nextRewrite(sending("formal"), ev("sending", { model: "Claude Haiku 5.5" })), ev("done", {
  text: "Thank you for the update.", mode: "apply", app: "Word", offerable: true, model: "Claude Haiku 5.5", costUsd: 0.000115, ...extra,
}));

function view() {
  let setTo = null;
  const v = buildRewrite({ setView: (name) => { setTo = name; } });
  v.sync();
  return { v, setTo: () => setTo };
}

test("Rust's events move the card along, and only the card's own rewrite counts", () => {
  let r = sending("formal");
  assert.equal(sendingToApi(r), false, "not until Rust takes the request");
  r = nextRewrite(r, ev("sending", { model: "Claude Haiku 5.5" }));
  assert.deepEqual([r.phase, r.id, r.model], ["sending", 7, "Claude Haiku 5.5"]);
  assert.equal(sendingToApi(r), true);
  // An answer for another rewrite is ignored.
  assert.equal(nextRewrite(r, ev("done", { id: 6, text: "old" })), r);
  const d = nextRewrite(r, ev("done", { text: "Thank you.", mode: "apply", app: "Word", offerable: true, costUsd: 0.0001 }));
  assert.deepEqual([d.phase, d.text, d.tone, d.mode, d.offerable], ["done", "Thank you.", "formal", "apply", true]);
  assert.equal(sendingToApi(d), false);
  // What "Use this" did.
  const applied = nextRewrite({ ...d, busy: true }, ev("result", { outcome: "applied", app: "Word" }));
  assert.deepEqual([applied.outcome, applied.busy], ["applied", false]);
  // A closed card stays closed.
  assert.equal(nextRewrite(null, ev("done", { text: "x" })), null);
  // A refusal before anything was sent (id 0) still lands.
  const f = nextRewrite(sending("shorter"), ev("failed", { id: 0, code: "no-key" }));
  assert.deepEqual([f.phase, f.code], ["failed", "no-key"]);
  // The answer came for a rewrite Rust dropped: back to the tones.
  assert.equal(nextRewrite(r, ev("stale")).phase, "choose");
});

test("the hotkey opens the card and sends nothing", () => {
  let alerted = null;
  const n = sent("writing_rewrite").length;
  const host = { alert: (v) => { alerted = v; }, setView() {}, collapse() {}, emote() {}, setPinned() {}, takeKeyboard() { throw new Error("the island must not take the keyboard"); }, denyApproval() {} };
  runGlobalShortcut(host, "rewrite", () => {});
  assert.equal(alerted, "rewrite");
  assert.equal(State.rewrite.phase, "choose");
  assert.equal(sent("writing_rewrite").length, n);
});

test("a tone click sends the tone id only, once", () => {
  State.rewrite = chooseTone();
  const { v } = view();
  const tones = v.el.find(".rewrite-tone");
  assert.deepEqual(tones.map((b) => b.textContent), ["Friendlier", "More formal", "Shorter", "Fix grammar"]);
  assert.deepEqual([...TONES], ["friendlier", "formal", "shorter", "grammar"]);
  assert.match(v.el.find(".suggestion-more")[0].textContent, /Nothing is typed until you click Use this/);
  const n = sent("writing_rewrite").length;
  tones[1].fire("click");
  assert.deepEqual(sent("writing_rewrite").slice(n), [{ tone: "formal" }]);
  assert.equal(State.rewrite.phase, "sending");
  assert.equal(v.el.find(".title")[0].textContent, "Rewriting with Claude…");
  // While it is on its way, nothing else goes.
  State.rewrite = sending("formal");
  v.sync();
  assert.equal(v.el.find(".rewrite-tone").length, 0);
});

test("the rewrite shows as a suggestion; Use this and Copy send its id only", () => {
  State.rewrite = done();
  const { v } = view();
  assert.equal(v.el.find(".rewrite-text")[0].textContent, "Thank you for the update.");
  assert.equal(v.el.find(".title")[0].textContent, "Rewrite: More formal");
  assert.equal(v.el.find(".suggestion-more")[0].textContent, "Claude Haiku 5.5 · about $0.0001");
  const use = v.el.find(".apply-all");
  assert.deepEqual(use.map((b) => b.textContent), ["Use this"]);
  const n = sent("writing_rewrite_apply").length;
  use[0].fire("click");
  assert.deepEqual(sent("writing_rewrite_apply").slice(n), [{ id: 7 }]);
  assert.equal(State.rewrite.busy, true);
  use[0].fire("click");
  assert.equal(sent("writing_rewrite_apply").length, n + 1, "one click, one apply");
  // Typed: the note says how to undo, and the buttons go.
  State.rewrite = { ...State.rewrite, busy: false, outcome: "applied" };
  v.sync();
  assert.equal(v.el.find(".suggestion-more")[0].textContent, "Rewritten. Ctrl+Z in Word undoes it.");
  assert.equal(v.el.find(".apply-all").length, 0);
});

test("Use this only where Glim types, and only for an answer it would type", () => {
  assert.deepEqual(rewriteButtons(done()), ["use"]);
  assert.deepEqual(rewriteButtons(done({ offerable: false })), ["copy"]);
  assert.deepEqual(rewriteButtons(done({ mode: "copy" })), ["copy"]);
  assert.deepEqual(rewriteButtons(done({ mode: "view" })), [], "chat apps: shown only, like suggestions");
  assert.match(rewriteNote(done({ mode: "view", app: "Slack" })), /Suggestions only in Slack for now\.$/);
  State.rewrite = done({ mode: "copy" });
  const { v } = view();
  const n = sent("writing_rewrite_copy").length;
  v.el.find(".apply-all")[0].fire("click");
  assert.deepEqual(sent("writing_rewrite_copy").slice(n), [{ id: 7 }]);
});

test("closing the card tells Rust to drop the rewrite", () => {
  State.rewrite = done();
  const { v, setTo } = view();
  const n = sent("writing_rewrite_dismiss").length;
  v.el.find(".suggestion-dismiss")[0].fire("click");
  assert.equal(sent("writing_rewrite_dismiss").length, n + 1);
  assert.equal(State.rewrite, null);
  assert.ok(setTo());
});

test("every code Rust can send has its own words", () => {
  const rust = read("src-tauri/src/ai/rewrite.rs") + read("src-tauri/src/ai/claude.rs") + read("src-tauri/src/capture/uia.rs");
  const codes = ["paused", "local-brain", "no-key", "key-error", "cap", "no-paragraph", "too-long", "sensitive", "truncated", "refused",
    "bad-answer", "network", "api-key", "api-billing", "api-permission", "api-rate", "api-busy", "api-server", "api-other"];
  for (const c of codes) assert.ok(rust.includes(`"${c}"`), `Rust sends ${c}`);
  const words = codes.map((c) => failureText(c, null));
  assert.equal(new Set(words).size, codes.length);
  assert.equal(failureText("api-rate", 30), "Too many requests right now. Try again in 30 s.");
  State.rewrite = nextRewrite(sending("shorter"), ev("failed", { id: 0, code: "cap" }));
  const { v } = view();
  assert.equal(v.el.find(".title")[0].textContent, "Not rewritten");
  assert.match(v.el.find(".rewrite-why")[0].textContent, /cap is reached/);
});

test("the cost reads as dollars", () => {
  assert.equal(costText(0), "$0");
  assert.equal(costText(0.00002), "<$0.0001");
  assert.equal(costText(0.000115), "$0.0001");
  assert.equal(costText(0.0123), "$0.01");
  assert.equal(costText(1.5), "$1.50");
});

test("the card sends ids only and never logs; the event handler only stores", () => {
  const src = read("src/views/rewrite.ts");
  for (const banned of ["addEventListener", "console.", "localStorage", "keydown"]) assert.ok(!src.includes(banned), banned);
  assert.deepEqual(src.match(/Bridge\.\w+\([^)]*\)/g), [
    "Bridge.writingRewriteDismiss()",
    "Bridge.writingRewrite(tone)",
    "Bridge.writingRewriteDismiss()",
    "Bridge.writingRewriteApply(r.id)",
    "Bridge.writingRewriteCopy(r.id)",
  ]);
  const main = read("src/main.ts");
  const handler = main.slice(main.indexOf('"writing-rewrite"'));
  const body = handler.slice(0, handler.indexOf("\n  });") + 5);
  for (const banned of ["Bridge.", "console.", "localStorage"]) assert.ok(!body.includes(banned), banned);
  // The data-leaving state is shown only while Rust is sending.
  assert.match(read("src/island/island.ts"), /else if \(sendingToApi\(State\.rewrite\)\) this\.lantern\.showReserved\(DELEGATE_STATE, "api-request"\);/);
});
