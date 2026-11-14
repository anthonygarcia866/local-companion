// Settings → Remote (local agency v1, part 3): off by default; the first
// "on" shows the consent card and only "Turn on" turns it on; folders show
// their fixed command and status; Remove and the desktop app switch go to
// Rust; the page never sends a command of its own.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { remoteSection, remoteTasks, entryState, clock, activityLine } = await import("../src/settings/remote.ts");
const { sent, replies } = await import("./tauri.mjs");

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const FOLDER = "C:\\path\\to\\example-project";
const CMD = "claude remote-control --spawn same-dir --capacity 1 --name \"example-project (PC)\"";

/** A stand-in for Settings' switch: a button whose click flips it. */
function toggle(on, onChange) {
  const b = globalThis.document.createElement("button");
  b.setAttribute("data-role", "switch");
  b.setAttribute("aria-checked", String(on));
  b.addEventListener("click", () => onChange(!on));
  return b;
}

const status = (over = {}, entries = []) => ({ settings: { on: false, consented: false, folders: [], desktopApp: false, ...over }, entries });
const entry = (over = {}) => ({ id: FOLDER.toLowerCase(), folder: FOLDER, command: CMD, state: "running", at: 1_700_000_000, lastExit: null, why: "", failedLastHour: 0, ...over });

test("status lines", () => {
  const at = new Date(2026, 10, 12, 14, 5).getTime() / 1000;
  assert.equal(clock(at), "14:05");
  assert.equal(entryState(entry({ at }), true), "Running since 14:05");
  assert.equal(entryState(entry({ state: "waiting", lastExit: 1, at }), true), "Stopped (exit code 1); starting again at 14:05");
  assert.match(entryState(entry({ state: "start-failed", why: "no claude", at }), true), /Couldn't start \(no claude\)/);
  assert.equal(entryState(entry(), false), "Off");
});

test("the first on shows the consent card; only Turn on turns it on", async () => {
  replies.set("keepalive_status", status());
  const el = remoteSection(toggle);
  await tick();
  const n = sent("keepalive_set").length;
  role(el, "button", "switch").fire("click");
  assert.ok(role(el, "div", "remote-consent"), "the card");
  assert.equal(sent("keepalive_set").length, n, "nothing turned on before the answer");
  role(el, "button", "remote-consent-no").fire("click");
  assert.equal(sent("keepalive_set").length, n, "Not now keeps it off");
  role(el, "button", "switch").fire("click");
  replies.set("keepalive_set", status({ on: true, consented: true }));
  role(el, "button", "remote-consent-yes").fire("click");
  await tick();
  assert.deepEqual(sent("keepalive_set").slice(n), [{ on: true, consent: true }]);
});

test("folders show the fixed command and status; add and remove go to Rust", async () => {
  replies.set("keepalive_status", status({ on: true, consented: true, folders: [FOLDER] }, [entry()]));
  const el = remoteSection(toggle);
  await tick();
  const text = el.find("div").map((d) => d.textContent).join(" | ");
  assert.ok(text.includes(CMD), "the fixed command is shown");
  assert.match(text, /Running since/);
  const adds = sent("keepalive_add").length;
  role(el, "input", "remote-folder").value = "C:\\path\\to\\other";
  role(el, "button", "remote-add").fire("click");
  await tick();
  assert.deepEqual(sent("keepalive_add").slice(adds), [{ folder: "C:\\path\\to\\other" }], "only a folder: never a command");
  const removes = sent("keepalive_remove").length;
  role(el, "button", "remote-remove").fire("click");
  await tick();
  assert.deepEqual(sent("keepalive_remove").slice(removes), [{ folder: FOLDER }]);
});

test("three stops in an hour are shown", async () => {
  replies.set("keepalive_status", status({ on: true, consented: true, folders: [FOLDER] }, [entry({ state: "waiting", lastExit: 1, failedLastHour: 3 })]));
  const el = remoteSection(toggle);
  await tick();
  assert.match(el.find("div").map((d) => d.textContent).join(" | "), /Stopped 3 times in the last hour/);
});

// ── Remote tasks (remote courier core, 2026-11-14): no transport yet ─────────

const rstatus = (over = {}) => ({ on: false, consented: false, killed: false, fingerprint: "", activity: [], ...over });

test("remote tasks: off by default, the card first, only Turn on turns it on", async () => {
  replies.set("remote_status", rstatus());
  const el = remoteTasks(toggle);
  await tick();
  const n = sent("remote_set").length;
  role(el, "button", "switch").fire("click");
  assert.ok(role(el, "div", "remote-tasks-consent"), "the card");
  assert.equal(sent("remote_set").length, n, "nothing before the answer");
  role(el, "button", "remote-tasks-no").fire("click");
  assert.equal(sent("remote_set").length, n, "Not now keeps it off");
  role(el, "button", "switch").fire("click");
  replies.set("remote_set", rstatus({ on: true, consented: true }));
  role(el, "button", "remote-tasks-yes").fire("click");
  await tick();
  assert.deepEqual(sent("remote_set").slice(n), [{ on: true, consent: true }]);
});

test("remote tasks: the key goes to Rust, only its fingerprint comes back", async () => {
  replies.set("remote_status", rstatus());
  const el = remoteTasks(toggle);
  await tick();
  const key = "ab".repeat(32);
  role(el, "input", "remote-key").value = key;
  replies.set("remote_key", rstatus({ fingerprint: "3F2A 91C0 0000 1111 2222 3333 4444 5555" }));
  role(el, "button", "remote-key-save").fire("click");
  await tick();
  assert.deepEqual(sent("remote_key").at(-1), { key });
  const text = el.find("div").map((d) => d.textContent).join(" | ");
  assert.match(text, /Fingerprint: 3F2A 91C0/);
  assert.ok(!text.includes(key), "the key itself is never shown");
});

test("remote tasks: the kill switch is one click, and Allow again undoes it", async () => {
  replies.set("remote_status", rstatus({ on: true, consented: true }));
  const el = remoteTasks(toggle);
  await tick();
  replies.set("remote_kill", rstatus({ on: true, consented: true, killed: true }));
  role(el, "button", "remote-kill").fire("click");
  await tick();
  assert.deepEqual(sent("remote_kill").at(-1), { killed: true });
  assert.ok(role(el, "div", "remote-killed"));
  replies.set("remote_kill", rstatus({ on: true, consented: true }));
  role(el, "button", "remote-allow").fire("click");
  await tick();
  assert.deepEqual(sent("remote_kill").at(-1), { killed: false });
});

test("remote activity: times, outcomes and kinds or reasons, newest first", async () => {
  const at = new Date(2026, 10, 14, 9, 7).getTime() / 1000;
  assert.equal(activityLine({ at, ok: true, kind: "session-status", reason: "", id: "x" }), "09:07 Accepted: session-status");
  assert.equal(activityLine({ at, ok: false, kind: "", reason: "expired", id: "" }), "09:07 Refused: expired");
  assert.equal(activityLine({ at, ok: false, kind: "kill-switch", reason: "", id: "" }), "09:07 Stop remote tasks");
  replies.set("remote_status", rstatus({ activity: [{ at, ok: false, kind: "", reason: "bad-signature", id: "" }, { at, ok: true, kind: "runbook", reason: "", id: "y" }] }));
  const el = remoteTasks(toggle);
  await tick();
  const lines = el.find("div").filter((d) => d.getAttribute("data-role") === "remote-activity-line").map((d) => d.textContent);
  assert.deepEqual(lines, ["09:07 Refused: bad-signature", "09:07 Accepted: runbook"]);
});
