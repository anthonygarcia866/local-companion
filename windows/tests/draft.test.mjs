// The "Draft email" card (voice 1.5d, decided 2026-10-16; core/draft.ts,
// views/draft.ts, settings/writing.ts draftGroup): Rust's card shown as it
// changes, chips to pick from, Change recipient's search, Create draft only
// when every name has an address, Cancel, and the island sending ids,
// indexes and a search's text only. Nothing here can send an email.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State, DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const { nextDraft, changeTarget, chipText, draftPlacement, meansMe } = await import("../src/core/draft.ts");
const { nextVoice } = await import("../src/core/voice.ts");
const { buildDraft, closeDraft, draftClock, draftCodeText } = await import("../src/views/draft.ts");
const { draftGroup, looksLikeAddress } = await import("../src/settings/writing.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const maria = { name: "Maria Lopez", address: "maria.lopez@example.com" };
const maria2 = { name: "Maria Okafor", address: "mokafor@example.com" };
const sam = { name: "Sam Nguyen", address: "sam@example.com" };
const cardOf = (o = {}) => ({
  id: 4,
  recipients: [
    { said: "Maria", line: "to", found: { kind: "pick", choices: [maria, maria2] } },
    { said: "Sam", line: "cc", found: { kind: "one", contact: sam } },
  ],
  subject: "Plumber Tuesday",
  body: "Hi Maria,\n\nThe plumber comes Tuesday at 10am.",
  changing: null,
  results: [],
  ...o,
});
const ev = (o) => ({ phase: "card", card: cardOf(), code: "", dryRun: false, ready: false, ...o });

test("Rust's events become the card; created and failures are short notes; closed is gone", () => {
  let d = nextDraft(null, ev({}));
  assert.equal(d.phase, "open");
  assert.equal(d.card.id, 4);
  d = nextDraft(d, ev({ ready: true, code: "no-sender" }));
  assert.deepEqual([d.ready, d.code, d.busy], [true, "no-sender", false]);
  assert.equal(nextDraft(d, { phase: "created", card: null, code: "", dryRun: false, ready: false }).phase, "created");
  assert.equal(nextDraft(d, { phase: "closed", card: null, code: "", dryRun: false, ready: false }), null);
  assert.equal(nextDraft(null, { phase: "closed", card: null, code: "no-outlook", dryRun: false, ready: false }).phase, "failed");
});

test("Change recipient goes to the name still open, else the only one, else asks which", () => {
  assert.equal(changeTarget(cardOf()), 0);
  const resolved = cardOf({ recipients: [{ said: "Maria", line: "to", found: { kind: "one", contact: maria } }, { said: "Sam", line: "cc", found: { kind: "one", contact: sam } }] });
  assert.equal(changeTarget(resolved), null);
  assert.equal(changeTarget(cardOf({ recipients: [resolved.recipients[1]] })), 0);
  assert.equal(chipText(maria), "Maria Lopez · maria.lopez@example.com");
  assert.equal(chipText({ name: "a@b.co", address: "a@b.co" }), "a@b.co");
});

test("the card shows wherever the voice card does, behind an approval, never over fullscreen", () => {
  assert.equal(draftPlacement({ presence: "pill", approval: false }), "show");
  assert.equal(draftPlacement({ presence: "hidden", approval: false }), "show");
  assert.equal(draftPlacement({ presence: "pill", approval: true }), "queue");
  assert.equal(draftPlacement({ presence: "indicator", approval: false }), "drop");
  State.draft = nextDraft(null, ev({}));
  State.pendingApproval = { questions: null };
  assert.equal(State.lanternView(), "approval");
  State.pendingApproval = null;
  assert.equal(State.lanternView(), "draft");
  State.draft = null;
});

function view() {
  const calls = [];
  const v = buildDraft({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  return { v, calls };
}
const roles = (v, role) => v.el.find("button").filter((b) => b.getAttribute("data-role") === role);

test("chips pick by index; Create draft waits until every name has an address, then sends the id only", () => {
  State.mode = "expanded";
  State.view = "draft";
  State.draft = nextDraft(null, ev({}));
  const { v } = view();
  v.sync();
  assert.match(v.el.textContent, /Which Maria\?/);
  assert.match(v.el.textContent, /Subject: Plumber Tuesday/);
  assert.match(v.el.textContent, /The plumber comes Tuesday at 10am\./);
  assert.match(v.el.textContent, /Pick who it goes to first, or hold a voice shortcut and say it\./);
  const create = roles(v, "create")[0];
  assert.equal(create.disabled, true);
  const n = sent("draft_pick").length;
  roles(v, "draft-choice")[1].fire("click");
  assert.deepEqual(sent("draft_pick").slice(n), [{ id: 4, index: 0, choice: 1 }]);
  // Rust answers with the resolved card.
  State.draft = nextDraft(State.draft, ev({ ready: true, card: cardOf({ recipients: [{ said: "Maria", line: "to", found: { kind: "one", contact: maria2 } }, cardOf().recipients[1]] }) }));
  v.sync();
  assert.equal(roles(v, "create")[0].disabled, false);
  assert.match(v.el.textContent, /Create draft saves it in Outlook's Drafts/);
  const m = sent("draft_create").length;
  roles(v, "create")[0].fire("click");
  roles(v, "create")[0].fire("click");
  assert.deepEqual(sent("draft_create").slice(m), [{ id: 4 }], "once, by id");
  assert.equal(roles(v, "create")[0].disabled, true, "busy");
  assert.equal(draftClock.running(), false, "the card itself never closes by itself");
});

test("Change recipient lends the keyboard for the search, sends its text, and a result's pick gives it back", async () => {
  State.mode = "expanded";
  State.draft = nextDraft(null, ev({ ready: true, card: cardOf({ recipients: [{ said: "Sam", line: "to", found: { kind: "one", contact: sam } }] }) }));
  const { v } = view();
  v.sync();
  const f = sent("lend_keyboard").length;
  roles(v, "change")[0].fire("click");
  assert.deepEqual(sent("draft_change").at(-1), { id: 4, index: 0 });
  assert.deepEqual(sent("lend_keyboard").slice(f), [{ on: true }]);
  State.draft = nextDraft(State.draft, ev({ ready: true, card: cardOf({ recipients: [{ said: "Sam", line: "to", found: { kind: "one", contact: sam } }], changing: 0, results: [maria] }) }));
  v.sync();
  const input = v.el.find("input")[0];
  // Clicked back into after typing elsewhere: it borrows the keyboard again.
  const g = sent("lend_keyboard").length;
  input.fire("pointerdown");
  assert.deepEqual(sent("lend_keyboard").slice(g), [{ on: true }]);
  input.value = "mar";
  input.fire("input");
  await new Promise((r) => setTimeout(r, 350));
  assert.deepEqual(sent("draft_search").at(-1), { id: 4, query: "mar" });
  roles(v, "draft-result")[0].fire("click");
  assert.deepEqual(sent("draft_pick").at(-1), { id: 4, index: 0, choice: 0 });
  assert.deepEqual(sent("lend_keyboard").at(-1), { on: false }, "back to the user's app");
});

test("Cancel tells Rust and closes; a note closes by itself; every code has words", () => {
  State.mode = "expanded";
  State.view = "draft";
  State.draft = nextDraft(null, ev({}));
  const { v, calls } = view();
  v.sync();
  const n = sent("draft_cancel").length;
  roles(v, "cancel")[0].fire("click");
  assert.deepEqual(sent("draft_cancel").slice(n), [{ id: 4 }]);
  assert.equal(State.draft, null);
  assert.deepEqual(calls, [["collapse"]]);
  State.draft = nextDraft(null, { phase: "created", card: null, code: "", dryRun: true, ready: false });
  v.sync();
  assert.match(v.el.textContent, /Dry run: nothing was saved or opened\./);
  assert.equal(draftClock.running(), true);
  closeDraft(null);
  assert.equal(sent("draft_cancel").length, n + 1, "a note's close isn't a cancel");
  for (const code of ["no-outlook", "no-account", "no-sender", "outlook-failed", "shared-account", "no-match"]) assert.ok(draftCodeText(code).length > 10, code);
  const rust = read("src-tauri/src/voice/draft.rs") + read("src-tauri/src/voice/draft_worker.rs");
  for (const code of ["shared-account", "no-match"]) assert.ok(rust.includes(`"${code}"`), code);
  State.mode = "hidden";
});

test("a change to the draft just made: the new text on the card, Update draft by id, then a note (fix b, 2026-10-17)", () => {
  State.mode = "expanded";
  State.view = "draft";
  const edit = cardOf({ id: 7, edit: true, recipients: [], subject: "Shift", body: "The shift moves to Wednesday." });
  State.draft = nextDraft(null, ev({ card: edit, ready: true }));
  const { v } = view();
  v.sync();
  assert.match(v.el.textContent, /Change the draft/);
  assert.match(v.el.textContent, /Subject: Shift/);
  assert.match(v.el.textContent, /The shift moves to Wednesday\./);
  assert.match(v.el.textContent, /Update draft puts this into the draft in Outlook and opens it there\. You send it yourself\./);
  assert.deepEqual(v.el.find("button").map((b) => b.textContent), ["Update draft", "Cancel"], "no recipients to change, nothing that sends");
  const n = sent("draft_create").length;
  roles(v, "create")[0].fire("click");
  roles(v, "create")[0].fire("click");
  assert.deepEqual(sent("draft_create").slice(n), [{ id: 7 }], "once, by id: Rust holds the draft's Outlook id");
  State.draft = nextDraft(State.draft, { phase: "updated", card: null, code: "", dryRun: false, ready: false });
  v.sync();
  assert.match(v.el.textContent, /Changed in Outlook's Drafts and opened there\. You send it from Outlook\./);
  assert.equal(draftClock.running(), true, "the note closes by itself");
  closeDraft(null);
  // Nothing to change: a note says so.
  State.draft = nextDraft(null, { phase: "closed", card: null, code: "draft-gone", dryRun: false, ready: false });
  v.sync();
  assert.match(v.el.textContent, /No draft from Glim in the last 10 minutes to change/);
  closeDraft(null);
  for (const code of ["draft-gone", "edit-failed"]) assert.ok(draftCodeText(code).length > 10, code);
  const rust = read("src-tauri/src/voice/draft.rs") + read("src-tauri/src/voice/draft_worker.rs") + read("src-tauri/src/voice/run.rs");
  for (const code of ["draft-gone", "edit-failed", "\"updated\""]) assert.ok(rust.includes(code), code);
  State.mode = "hidden";
});

test("Settings → Email drafts: the sender's address is checked and stored; remembered names are forgotten through Rust", () => {
  assert.equal(DEFAULT_SETTINGS.draftFrom, "");
  for (const ok of ["a@b.co", "anthony@example.com"]) assert.ok(looksLikeAddress(ok), ok);
  for (const bad of ["", "anthony", "a@b", "a b@c.com", "a@b.com."]) assert.ok(!looksLikeAddress(bad), bad);
  const settings = { ...DEFAULT_SETTINGS, draftRecipients: { maria: "mokafor@example.com" } };
  let saves = 0;
  let redraw = null;
  const group = draftGroup({ settings: () => settings, save: async () => { saves++; }, watch: (r) => { redraw = r; } });
  const role = (r) => [...group.walk()].find((e) => e.getAttribute("data-role") === r);
  const from = role("draft-from");
  from.value = "not an address";
  from.fire("change");
  assert.equal(settings.draftFrom, "");
  assert.match(role("draft-from-why").textContent, /doesn't look like an email address/);
  from.value = " anthony@example.com ";
  from.fire("change");
  assert.equal(settings.draftFrom, "anthony@example.com");
  assert.equal(saves, 1);
  assert.match(group.textContent, /maria → mokafor@example\.com/);
  const n = sent("draft_forget").length;
  [...group.walk()].find((e) => e.getAttribute("data-role") === "draft-forget").fire("click");
  assert.deepEqual(sent("draft_forget").slice(n), [{ name: "maria" }]);
  assert.equal(saves, 1, "the page doesn't write the names itself");
  settings.draftRecipients = {};
  redraw();
  assert.match(group.textContent, /No names remembered yet/);
});

test("the island sends ids, indexes and a search's text only, never logs, and has nothing that sends mail", () => {
  for (const file of ["src/core/draft.ts", "src/views/draft.ts"]) {
    const src = read(file);
    for (const banned of ["console.", "localStorage", "sessionStorage"]) assert.ok(!src.includes(banned), `${file}: ${banned}`);
  }
  const calls = read("src/views/draft.ts").match(/Bridge\.\w+\(/g);
  assert.deepEqual([...new Set(calls)].sort(), ["Bridge.draftCancel(", "Bridge.draftChange(", "Bridge.draftCreate(", "Bridge.draftPick(", "Bridge.draftSearch("]);
  const bridge = read("src/core/bridge.ts");
  assert.ok(!/draft\w*Send|sendMail|send_mail/i.test(bridge));
  const main = read("src/main.ts");
  const handler = main.slice(main.indexOf('"draft-email"'));
  const body = handler.slice(0, handler.indexOf("\n  });") + 5);
  for (const banned of ["Bridge.", "console.", "localStorage"]) assert.ok(!body.includes(banned), banned);
  const lib = read("src-tauri/src/lib.rs");
  for (const cmd of ["draft_pick", "draft_change", "draft_search", "draft_create", "draft_cancel", "draft_forget"]) assert.ok(new RegExp(`\\n\\s+${cmd},\\n`).test(lib), cmd);
  assert.ok(lib.includes("settings.draft_recipients = current.draft_recipients.clone();"), "the page can't overwrite Rust's picks");
});

// ── 4c (2026-10-16 retest): "me", answers by voice, never the shared mailbox ──

test("\"me\" with no address in Settings says where to put it, not \"not in your address book\"", () => {
  for (const said of ["me", "Me.", "myself", " my email ", "MY ADDRESS"]) assert.equal(meansMe(said), true, said);
  for (const said of ["Maria", "meet", "home", ""]) assert.equal(meansMe(said), false, said);
  State.mode = "expanded";
  State.view = "draft";
  State.draft = nextDraft(null, ev({ card: cardOf({ recipients: [{ said: "me", line: "to", found: { kind: "none" } }] }) }));
  const { v } = view();
  v.sync();
  assert.match(v.el.textContent, /Add your email address in Settings → Email drafts first\./);
  assert.doesNotMatch(v.el.textContent, /No “me” in your address book/);
  // Rust's list of words is the same.
  const rust = read("src-tauri/src/voice/draft.rs");
  assert.ok(rust.includes('const ME: [&str; 6] = ["me", "myself", "my self", "my email", "my address", "my own address"];'));
  State.mode = "hidden";
});

test("while the card asks who, a voice answer from either key: Listening for your answer…", () => {
  State.mode = "expanded";
  State.view = "draft";
  State.draft = nextDraft(null, ev({}));
  State.voice = nextVoice(null, { phase: "listening", level: 0, cleaned: false, text: "", code: "", app: "", mode: "answer" });
  const { v } = view();
  v.sync();
  assert.match(v.el.textContent, /Listening for your answer…/);
  State.voice = nextVoice(State.voice, { phase: "transcribing", level: 0, cleaned: false, text: "", code: "", app: "", mode: "answer" });
  v.sync();
  assert.match(v.el.textContent, /Working on your answer…/);
  // Rust answers with the card again; the note goes back to the card's own.
  State.voice = null;
  State.draft = nextDraft(State.draft, ev({ code: "no-match" }));
  v.sync();
  assert.match(v.el.textContent, /No match for that\. Say a name from the list, or an email address\./);
  // Rust routes it: the worker says when it waits, and voice::run sends it there, never to Claude.
  const worker = read("src-tauri/src/voice/draft_worker.rs");
  assert.ok(worker.includes("WAITING.store(phase == \"card\" && card.is_some_and(|c| c.asking().is_some()), Ordering::SeqCst);"));
  const run = read("src-tauri/src/voice/run.rs");
  assert.ok(run.includes("let draft = asked.is_none() && super::draft_worker::waiting();"));
  assert.ok(run.includes("super::draft_worker::answer(app, transcript);"));
  State.mode = "hidden";
});
