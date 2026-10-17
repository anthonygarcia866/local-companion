// The Google Docs tip (1d retest, 2026-10-14; core/docs-tip.ts and
// src-tauri/src/writing/docs_tip.rs): where it shows, "Got it" dismisses it
// for good (Rust stores it), "Not now" and the 30 s clock only close it, and
// it waits behind an approval like the "Check writing in…" question.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { docsTipPlacement, DOCS_TIP_TIMEOUT_MS } = await import("../src/core/docs-tip.ts");
const { buildDocsTip, closeDocsTip, docsTipClock } = await import("../src/views/docs-tip.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const place = (o) => docsTipPlacement({ presence: "pill", paused: false, approval: false, ask: false, showing: false, ...o });

test("the tip shows with the pill, waits behind an approval, and is dropped otherwise", () => {
  assert.equal(place({}), "show");
  assert.equal(place({ approval: true }), "queue");
  for (const o of [{ presence: "ember" }, { presence: "hidden" }, { presence: "indicator" }, { paused: true }, { ask: true }, { showing: true }]) {
    assert.equal(place(o), "drop", JSON.stringify(o));
  }
  State.writingDocsTip = true;
  State.pendingApproval = { questions: null };
  assert.equal(State.lanternView(), "approval");
  State.pendingApproval = null;
  assert.equal(State.lanternView(), "writingDocsTip");
  State.writingAsk = { exe: "Acrobat.exe", name: "Acrobat" };
  assert.equal(State.lanternView(), "writingAsk", "the question comes first");
  State.writingAsk = null;
  State.writingDocsTip = false;
});

function card() {
  const calls = [];
  const v = buildDocsTip({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  return { v, calls };
}

test("Got it dismisses it for good; Not now only closes it", () => {
  for (const [label, dismissed] of [["Got it", 1], ["Not now", 0]]) {
    State.writingDocsTip = true;
    State.view = "writingDocsTip";
    State.mode = "expanded";
    const { v, calls } = card();
    v.sync();
    assert.match(v.el.textContent, /Glim can't read Google Docs yet/);
    assert.doesNotMatch(v.el.textContent, /Ctrl\+Alt\+Z/, "no more advice that doesn't work (Decision D)");
    const n = sent("writing_docs_tip_dismiss").length;
    v.el.find("button").find((b) => b.textContent === label).fire("click");
    assert.equal(sent("writing_docs_tip_dismiss").length - n, dismissed, label);
    assert.equal(State.writingDocsTip, false);
    assert.deepEqual(calls, [["collapse"]]);
    assert.equal(docsTipClock.running(), false);
  }
  State.mode = "hidden";
});

test("the card's clock runs while it is in sight and closes it without dismissing", () => {
  assert.equal(DOCS_TIP_TIMEOUT_MS, 30_000);
  State.writingDocsTip = true;
  State.view = "writingDocsTip";
  State.mode = "expanded";
  const { v } = card();
  v.sync();
  assert.equal(docsTipClock.running(), true);
  const n = sent("writing_docs_tip_dismiss").length;
  closeDocsTip(false, null);
  assert.equal(sent("writing_docs_tip_dismiss").length, n);
  assert.equal(docsTipClock.running(), false);
  State.mode = "hidden";
});

test("dismissal is Rust's: a save from Settings can't bring the tip back", () => {
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(lib.includes("settings.writing_docs_tip_dismissed = current.writing_docs_tip_dismissed;"));
  assert.ok(lib.includes("fn writing_docs_tip_dismiss(app: AppHandle, shared: State<Shared>)"));
});

// Decision D (2026-10-17): Docs is unsupported, so Rust reads nothing there and
// the notice follows the document's focus, not whether a field looked readable.
test("Google Docs is never read, and the notice doesn't depend on a readable field", () => {
  const uia = read("src-tauri/src/capture/uia.rs");
  const docs = uia.slice(uia.indexOf("    if docs_site {"), uia.indexOf("    if !is_editable(&edit_signals(element, control)) {"));
  assert.match(docs, /meta\.pattern = crate::writing::docs_tip::SKIPPED_DOCS\.into\(\);/);
  assert.match(docs, /return skipped;/);
  assert.ok(!docs.includes("read_text("), "nothing is read in Docs");
  assert.ok(uia.includes("let in_docs = reading.docs_site && crate::writing::apps::enabled(&reading.capture.meta.app);"));
  assert.ok(!uia.includes("docs_tip::unreadable("), "the old readability test is gone");
});
