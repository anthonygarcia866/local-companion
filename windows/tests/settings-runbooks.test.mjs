// Settings → Runbooks (local agency v1, part 2): Run on an unapproved or
// changed runbook shows every step first and approves only the version
// shown; ask steps ask on every run; an approved runbook runs at once; a
// voice request runs through the same cards; progress and the last runs show.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { runbooksSection, stateText, askSteps, progressLine } = await import("../src/settings/runbooks.ts");
const { sent, replies, emit } = await import("./tauri.mjs");

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const all = (el) => el.find("div").map((d) => d.textContent).join(" | ");

const steps = [
  { kind: "env_secret", text: "Write secret EXAMPLE_TOKEN as EXAMPLE_KEY=… into C:\\example\\project\\.env.local", ask: false },
  { kind: "check", text: "Check that C:\\example\\project\\.env.local exists", ask: false },
];
const book = (over = {}) => ({ name: "example", description: "", state: "new", error: "", hash: "h1", steps, folders: ["C:\\example\\project"], apps: [], commands: [], ...over });
const list = (books, runs = []) => ({ folder: "C:\\example\\Glim\\runbooks", books, runs });

test("labels: state, ask steps, progress", () => {
  assert.equal(stateText(book()), "Needs approval");
  assert.equal(stateText(book({ state: "approved" })), "Approved");
  assert.match(stateText(book({ state: "changed" })), /approve again/);
  assert.match(stateText(book({ state: "invalid", error: "step 1: bad" })), /step 1: bad/);
  assert.deepEqual(askSteps(book({ steps: [steps[0], { ...steps[1], ask: true }] })), [2]);
  assert.equal(progressLine({ step: 1, total: 2, kind: "check", ok: true, detail: "exists" }), "Step 1 of 2: OK (exists)");
  assert.equal(progressLine({ step: 2, total: 2, kind: "check", ok: false, detail: "not there" }), "Step 2 of 2 failed: not there");
});

test("Run on a new runbook shows every step and approves only that version; then it runs", async () => {
  replies.set("runbooks_list", list([book()]));
  replies.set("runbook_take_request", null);
  const el = runbooksSection();
  await tick();
  const runs = sent("runbook_run").length;
  role(el, "button", "runbook-run").fire("click");
  const card = role(el, "div", "runbook-approval");
  assert.ok(card, "the approval card");
  assert.match(card.textContent, /EXAMPLE_TOKEN/);
  assert.match(card.textContent, /Check that/);
  assert.equal(sent("runbook_run").length, runs, "nothing runs before approval");
  // Cancel: nothing approved.
  const approvals = sent("runbook_approve").length;
  role(el, "button", "runbook-cancel").fire("click");
  assert.equal(sent("runbook_approve").length, approvals);
  // Approve: the hash shown, then it runs.
  role(el, "button", "runbook-run").fire("click");
  replies.set("runbooks_list", list([book({ state: "approved" })]));
  role(el, "button", "runbook-approve").fire("click");
  await tick();
  assert.deepEqual(sent("runbook_approve").slice(approvals), [{ name: "example", hash: "h1" }]);
  assert.deepEqual(sent("runbook_run").slice(runs), [{ name: "example", hash: "h1", confirmed: [], voice: false }]);
  // Progress and the end.
  emit("runbook-progress", { name: "example", step: 1, total: 2, kind: "env_secret", ok: true, detail: "written" });
  emit("runbook-progress", { name: "example", step: 2, total: 2, kind: "check", ok: true, detail: "exists" });
  replies.set("runbooks_list", list([book({ state: "approved" })], [{ time: "2026-11-12 10:00:02", name: "example", trigger: "settings", outcome: "OK", ok: true, refused: false, done: 2, total: 2 }]));
  emit("runbook-done", { name: "example", ok: true, done: 2, total: 2, refused: "" });
  await tick();
  const text = all(el);
  assert.match(text, /Step 1 of 2: OK \(written\)/);
  assert.match(text, /Done: all 2 steps OK/);
  assert.match(text, /2026-11-12 10:00:02/);
});

test("a changed runbook says so on its card", async () => {
  replies.set("runbooks_list", list([book({ state: "changed", hash: "h2" })]));
  const el = runbooksSection();
  await tick();
  role(el, "button", "runbook-run").fire("click");
  assert.match(role(el, "div", "runbook-approval").textContent, /changed since you approved it/);
});

test("ask steps ask on every run, and only Run now runs them", async () => {
  const b = book({ state: "approved", steps: [steps[0], { kind: "run", text: "Run `npm test` in C:\\example\\project (stops after 120 s)", ask: true }] });
  replies.set("runbooks_list", list([b]));
  const el = runbooksSection();
  await tick();
  const runs = sent("runbook_run").length;
  role(el, "button", "runbook-run").fire("click");
  const card = role(el, "div", "runbook-ask");
  assert.ok(card);
  assert.match(card.textContent, /npm test/);
  assert.doesNotMatch(card.textContent, /EXAMPLE_TOKEN/, "only the asking steps");
  assert.equal(sent("runbook_run").length, runs);
  role(el, "button", "runbook-ask-run").fire("click");
  await tick();
  assert.deepEqual(sent("runbook_run").slice(runs), [{ name: "example", hash: "h1", confirmed: [2], voice: false }]);
});

test("a voice request goes through the same cards and runs as voice", async () => {
  replies.set("runbooks_list", list([book({ state: "approved" })]));
  replies.set("runbook_take_request", null);
  const el = runbooksSection();
  await tick();
  const runs = sent("runbook_run").length;
  emit("runbook-request", "example");
  await tick();
  assert.deepEqual(sent("runbook_run").slice(runs), [{ name: "example", hash: "h1", confirmed: [], voice: true }]);
  assert.ok(el);
});

test("an invalid runbook can't run and says why", async () => {
  replies.set("runbooks_list", list([book({ state: "invalid", error: "step 1: unknown step type \"delete\"", steps: [] })]));
  const el = runbooksSection();
  await tick();
  assert.notEqual(role(el, "button", "runbook-run").getAttribute("disabled"), null);
  assert.match(all(el), /unknown step type/);
});
