// Settings → Remote (local agency v1, part 3): off by default; the first
// "on" shows the consent card and only "Turn on" turns it on; folders show
// their fixed command and status; Remove and the desktop app switch go to
// Rust; the page never sends a command of its own.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { remoteSection, remoteTasks, entryState, clock, activityLine, nextClientName, addedText } = await import("../src/settings/remote.ts");
const { sent, replies } = await import("./tauri.mjs");

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const roleText = (el, r) => el.find("div").concat(el.find("span")).filter((d) => d.getAttribute("data-role") === r).map((d) => d.textContent);
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const FOLDER = "C:\\path\\to\\example-project";
const CMD = "claude remote-control --spawn worktree --capacity 1 --name \"example-project (PC)\"";

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

const rstatus = (over = {}) => ({ on: false, consented: false, killed: false, clients: [], activity: [], ...over });
const FP = "3F2A 91C0 0000 1111 2222 3333 4444 5555";
const client = (name, added = 1763200000) => ({ name, fingerprint: FP, added });

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

test("cloud clients: Add sends the name and key to Rust; only the fingerprint comes back; the token shows once", async () => {
  replies.set("remote_status", rstatus({ clients: [client("account-1", 0)] }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  assert.match(roleText(el, "remote-client").join(" | "), /account-1.*Added with the first setup/);
  // The next free name is filled in.
  assert.equal(role(el, "input", "remote-client-name").value, "account-2");
  const key = "ab".repeat(32);
  const token = "c".repeat(64);
  role(el, "input", "remote-key").value = key;
  replies.set("remote_client_add", { status: rstatus({ clients: [client("account-1", 0), client("account-2")] }), cloudToken: token });
  role(el, "button", "remote-client-add").fire("click");
  await tick();
  assert.deepEqual(sent("remote_client_add").at(-1), { name: "account-2", key });
  assert.equal(roleText(el, "remote-client").length, 2);
  const text = el.find("div").map((d) => d.textContent).join(" | ");
  assert.match(text, /Fingerprint: 3F2A 91C0/);
  assert.ok(!text.includes(key), "the key itself is never shown");
  assert.equal(role(el, "input", "remote-cloud-token").value, token);
  role(el, "button", "remote-cloud-done").fire("click");
  assert.equal(role(el, "input", "remote-cloud-token"), undefined, "gone after Done");
});

test("cloud clients: Remove names the one client and reminds about the runbook", async () => {
  replies.set("remote_status", rstatus({ clients: [client("account-1"), client("account-2")] }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  replies.set("remote_client_remove", rstatus({ clients: [client("account-1")] }));
  el.find("button").filter((b) => b.getAttribute("data-role") === "remote-client-remove")[1].fire("click");
  await tick();
  assert.deepEqual(sent("remote_client_remove").at(-1), { name: "account-2" });
  assert.equal(roleText(el, "remote-client").length, 1);
  assert.match(roleText(el, "remote-client-removed")[0], /relay-tokens runbook/);
});

test("cloud clients: the next free name and the added date", () => {
  assert.equal(nextClientName([]), "account-1");
  assert.equal(nextClientName([client("account-1"), client("account-3")]), "account-2");
  assert.equal(addedText(client("account-1", 0)), "Added with the first setup");
  assert.match(addedText(client("account-2")), /^Added /);
});

test("require sealed tasks: off by default, shown with the encryption key, one click goes to Rust", async () => {
  const keyed = { on: true, consented: true, pcEncKey: "45".repeat(32), pcEncFingerprint: FP };
  replies.set("remote_status", rstatus(keyed));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  const row = role(el, "div", "remote-require-sealed");
  assert.ok(row, "shown beside the encryption key");
  const sw = row.find("button").find((b) => b.getAttribute("data-role") === "switch");
  assert.equal(sw.getAttribute("aria-checked"), "false", "off by default");
  replies.set("remote_require_sealed", rstatus({ ...keyed, requireSealed: true }));
  sw.fire("click");
  await tick();
  assert.deepEqual(sent("remote_require_sealed").at(-1), { on: true });
  const on = role(el, "div", "remote-require-sealed").find("button").find((b) => b.getAttribute("data-role") === "switch");
  assert.equal(on.getAttribute("aria-checked"), "true");
  // No encryption key yet: no switch (nothing could be sealed).
  replies.set("remote_status", rstatus({ on: true, consented: true }));
  const bare = remoteTasks(toggle, 60_000);
  await tick();
  assert.equal(role(bare, "div", "remote-require-sealed"), undefined);
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
  assert.equal(activityLine({ at, ok: true, kind: "session-status", reason: "", id: "x", client: "account-2" }), "09:07 account-2 · Accepted: session-status");
  assert.equal(activityLine({ at, ok: false, kind: "", reason: "wrong-client", id: "", client: "account-1" }), "09:07 account-1 · Refused: wrong-client");
  assert.equal(activityLine({ at, ok: false, kind: "", reason: "expired", id: "" }), "09:07 Refused: expired");
  assert.equal(activityLine({ at, ok: false, kind: "kill-switch", reason: "", id: "" }), "09:07 Stop remote tasks");
  replies.set("remote_status", rstatus({ activity: [{ at, ok: false, kind: "", reason: "bad-signature", id: "" }, { at, ok: true, kind: "runbook", reason: "", id: "y" }] }));
  const el = remoteTasks(toggle);
  await tick();
  const lines = el.find("div").filter((d) => d.getAttribute("data-role") === "remote-activity-line").map((d) => d.textContent);
  assert.deepEqual(lines, ["09:07 Refused: bad-signature", "09:07 Accepted: runbook"]);
});

// ── Settings → Remote fixes (owner 2026-11-14) ───────────────────────────────

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

test("the Relay: line follows the connection live, without redrawing what's typed", async () => {
  replies.set("remote_status", rstatus({ on: true, consented: true, relay: "https://a.b.workers.dev", connection: "off" }));
  const el = remoteTasks(toggle, 5);
  await tick();
  assert.deepEqual(roleText(el, "remote-connection"), ["Relay: Off"]);
  const typed = role(el, "input", "remote-key");
  typed.value = "half a ke";
  replies.set("remote_status", rstatus({ on: true, consented: true, relay: "https://a.b.workers.dev", connection: "connected" }));
  await wait(30);
  assert.deepEqual(roleText(el, "remote-connection"), ["Relay: Connected"]);
  assert.equal(role(el, "input", "remote-key"), typed, "the same field: typing isn't interrupted");
  assert.equal(typed.value, "half a ke");
  replies.set("remote_status", rstatus({ on: true, consented: true, relay: "https://a.b.workers.dev", connection: "reconnecting" }));
  await wait(30);
  assert.deepEqual(roleText(el, "remote-connection"), ["Relay: Reconnecting…"]);
});

test("Save key and Save address each show Saved, and only when it worked", async () => {
  replies.set("remote_status", rstatus());
  const el = remoteTasks(toggle, 60_000);
  await tick();
  assert.deepEqual(roleText(el, "remote-key-saved"), []);
  role(el, "input", "remote-key").value = "ab".repeat(32);
  replies.set("remote_client_add", { status: rstatus({ clients: [client("account-1")] }), cloudToken: "c".repeat(64) });
  role(el, "button", "remote-client-add").fire("click");
  await tick();
  assert.deepEqual(roleText(el, "remote-key-saved"), ["Saved"]);
  role(el, "input", "remote-relay").value = "https://a.b.workers.dev";
  replies.set("remote_relay", rstatus({ relay: "https://a.b.workers.dev" }));
  role(el, "button", "remote-relay-save").fire("click");
  await tick();
  assert.deepEqual(roleText(el, "remote-relay-saved"), ["Saved"]);
  assert.deepEqual(roleText(el, "remote-key-saved"), [], "one at a time");
  // A thenable that rejects: invoke() adopts it, as when Rust answers Err.
  replies.set("remote_relay", { then: (_ok, fail) => fail(new Error("That isn't a relay address")) });
  role(el, "button", "remote-relay-save").fire("click");
  await tick();
  assert.deepEqual(roleText(el, "remote-relay-saved"), [], "a refusal is not Saved");
});

test("no client yet: no fingerprint line at all", async () => {
  replies.set("remote_status", rstatus({ clients: [], pcPublicKey: "", pcFingerprint: "" }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  assert.deepEqual(roleText(el, "remote-fingerprint"), []);
  assert.deepEqual(roleText(el, "remote-pc-fingerprint"), []);
});

test("remote runbooks: limits are set here (one call to Rust), the list is read only, activity names the runbook", async () => {
  const on = { on: true, consented: true };
  replies.set("remote_status", rstatus({ ...on, runbooksPerHour: 5, runbooksPerHourPc: 10, remoteRunbooks: [] }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  assert.match(roleText(el, "remote-runbooks")[0], /No runbook may run remotely/);
  const perClient = role(el, "input", "remote-runbooks-client");
  assert.equal(perClient.value ?? perClient.getAttribute("value"), "5");
  const perPc = role(el, "input", "remote-runbooks-pc");
  perPc.value = perPc.value ?? perPc.getAttribute("value");
  assert.equal(perPc.value, "10");
  replies.set("remote_runbook_limits", rstatus({ ...on, runbooksPerHour: 7, runbooksPerHourPc: 10, remoteRunbooks: [] }));
  perClient.value = "7";
  perClient.fire("change");
  await tick();
  assert.deepEqual(sent("remote_runbook_limits").at(-1), { perClient: 7, perPc: 10 });
  replies.set("remote_status", rstatus({ ...on, remoteRunbooks: ["glim-check (cloud)"] }));
  const listed = remoteTasks(toggle, 60_000);
  await tick();
  assert.match(roleText(listed, "remote-runbooks")[0], /glim-check \(cloud\)/);
  const line = activityLine({ at: 1_700_000_000, ok: true, kind: "runbook", reason: "", id: "x", client: "account-1", runbook: "glim-check" });
  assert.match(line, /runbook glim-check/);
});

test("cloud clients: a checkbox per command (saved at once) and the Claude account for its sessions", async () => {
  const ALL = ["session-status", "run-status", "transcript-tail", "start-session", "send-to-session", "runbook", "keepalive-restart"];
  const accounts = [{ name: "Claude", client: "account-1" }, { name: "Claude2", client: "account-2" }];
  const muse = { ...client("muse"), commands: ["session-status", "run-status"], account: "" };
  const full = { ...client("account-1"), commands: ALL, account: "Claude" };
  replies.set("remote_status", rstatus({ clients: [full, muse], commands: ALL, accounts }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  const rows = el.find("div").filter((d) => d.getAttribute("data-role") === "remote-client");
  const boxes = (row) => row.find("input").filter((i) => i.getAttribute("data-role") === "remote-client-command");
  assert.deepEqual(boxes(rows[0]).map((b) => b.checked), ALL.map(() => true), "account-1 keeps every command");
  assert.deepEqual(boxes(rows[1]).filter((b) => b.checked).map((b) => b.getAttribute("data-command")), ["session-status", "run-status"]);
  // No account: session-status answers not-your-session, and the row says so.
  assert.match(roleText(rows[1], "remote-client-no-account")[0], /session-status and keepalive-restart answer not-your-session/);
  assert.equal(roleText(rows[0], "remote-client-no-account").length, 0, "account-1 has its account");
  // Ticking runbook sends muse's whole new list, in order.
  replies.set("remote_client_commands", rstatus({ clients: [full, { ...muse, commands: ["session-status", "run-status", "runbook"] }], commands: ALL, accounts }));
  const rb = boxes(rows[1]).find((b) => b.getAttribute("data-command") === "runbook");
  rb.checked = true;
  rb.fire("change");
  await tick();
  assert.deepEqual(sent("remote_client_commands").at(-1), { name: "muse", commands: ["session-status", "run-status", "runbook"] });
  // A session command without an account: the page says it will be refused.
  replies.set("remote_status", rstatus({ clients: [full, { ...muse, commands: ["session-status", "start-session"] }], commands: ALL, accounts }));
  const el2 = remoteTasks(toggle, 60_000);
  await tick();
  assert.match(roleText(el2, "remote-client-no-account")[0], /refused/);
  // The account choice goes to Rust as the account's name ("" = none).
  const pick = el2.find("select").filter((s) => s.getAttribute("data-role") === "remote-client-account")[1];
  pick.value = "Claude2";
  pick.fire("change");
  await tick();
  assert.deepEqual(sent("remote_client_account").at(-1), { name: "muse", account: "Claude2" });
});

test("cloud clients: two quick unticks both stick (the list comes from the boxes, not the last redraw)", async () => {
  const ALL = ["session-status", "run-status", "transcript-tail", "start-session", "send-to-session", "runbook", "keepalive-restart"];
  replies.set("remote_status", rstatus({ clients: [{ ...client("muse"), commands: ALL, account: "Claude" }], commands: ALL, accounts: [{ name: "Claude", client: "muse" }] }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  const box = (k) => el.find("input").find((i) => i.getAttribute("data-command") === k);
  const [a, b] = [box("start-session"), box("send-to-session")];
  a.checked = false;
  a.fire("change");
  b.checked = false;
  b.fire("change");
  assert.deepEqual(sent("remote_client_commands").at(-1).commands, ["session-status", "run-status", "transcript-tail", "runbook", "keepalive-restart"]);
});

test("cloud clients: Add agent client sends the name and key only, shows no token, and is marked on its row", async () => {
  replies.set("remote_status", rstatus({ clients: [] }));
  const el = remoteTasks(toggle, 60_000);
  await tick();
  role(el, "input", "remote-client-name").value = "muse";
  role(el, "input", "remote-key").value = "ab".repeat(32);
  replies.set("remote_agent_client_add", rstatus({ clients: [{ ...client("muse"), agent: true, commands: ["session-status", "run-status"], account: "" }] }));
  role(el, "button", "remote-agent-client-add").fire("click");
  await tick();
  assert.deepEqual(sent("remote_agent_client_add").at(-1), { name: "muse", key: "ab".repeat(32) });
  assert.equal(role(el, "input", "remote-cloud-token"), undefined, "no token to show");
  assert.match(roleText(el, "remote-agent-added")[0], /relay-tokens runbook/);
  assert.match(roleText(el, "remote-client-agent")[0], /public key only/);
});
