// Phone notifications (phone companion step 1, docs/phone-plan.md): when the
// island asks Rust to push (core/phone.ts), the hooks and the wait tick that
// ask, and Settings → Phone. Nothing here reaches a phone: the bridge is the
// test stand-in, and Rust's sending is tested against a fake server.

import { afterEach, beforeEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { calls, emit, replies, sent } = await import("./tauri.mjs");
const {
  APPROVAL_MAX_SEC, APPROVAL_MIN_SEC, DEFAULT_PHONE, IDLE_MAX_MIN, IDLE_MIN_MIN, PhoneWatcher, approvalDelaySec, away, idleAfterMin, inQuietHours,
  minutes, phoneSettings, projectOf,
} = await import("../src/core/phone.ts");
const { DEFAULT_SETTINGS, State } = await import("../src/core/state.ts");
const { registerHookHandlers } = await import("../src/island/hooks.ts");
const { phoneSection } = await import("../src/settings/phone.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const on = (more = {}) => ({ ...DEFAULT_PHONE, enabled: true, ...more });
const at = (h, m = 0) => new Date(2026, 9, 18, h, m);
const NOON = at(12);
/** Away from the PC for 5 minutes, or at it (10 s since the last input). */
const AWAY = 5 * 60_000;
const AT_PC = 10_000;
const wait = (requestId, seconds, pillId = "integration_claude") => ({ requestId, pillId, project: "local-companion", waitedMs: seconds * 1000 });

test("off by default, and nothing goes while it's off", () => {
  assert.equal(DEFAULT_PHONE.enabled, false);
  assert.equal(DEFAULT_SETTINGS.phone.enabled, false);
  assert.deepEqual(phoneSettings(undefined), DEFAULT_PHONE, "an older settings.json");
  const w = new PhoneWatcher();
  assert.deepEqual(w.approvals([wait("r1", 600)], DEFAULT_PHONE, NOON, AWAY), []);
  assert.equal(w.finished("integration_claude", "x", false, DEFAULT_PHONE, NOON, AWAY), null);
  assert.equal(w.limit("integration_claude", "x", "usage", "2 PM", DEFAULT_PHONE, NOON, AWAY), null);
});

test("an approval pings once, after the delay (60 s by default)", () => {
  assert.equal(DEFAULT_PHONE.approvalAfterSec, 60);
  const w = new PhoneWatcher();
  assert.deepEqual(w.approvals([wait("r1", 59)], on(), NOON, AWAY), []);
  assert.deepEqual(w.approvals([wait("r1", 60)], on(), NOON, AWAY), [{ kind: "approval", pillId: "integration_claude", project: "local-companion" }]);
  assert.deepEqual(w.approvals([wait("r1", 61)], on(), NOON, AWAY), [], "once");
  // Each request on its own clock; one answered before its delay never pings.
  assert.deepEqual(w.approvals([wait("r1", 70), wait("r2", 30)], on(), NOON, AWAY), []);
  assert.deepEqual(w.approvals([wait("r1", 71)], on(), NOON, AWAY), [], "r2 was answered");
  assert.deepEqual(w.approvals([wait("r2", 90)], on(), NOON, AWAY).map((p) => p.kind), ["approval"], "r2 is a new request that waited");
  // The switch for approvals alone.
  assert.deepEqual(new PhoneWatcher().approvals([wait("r3", 90)], on({ approval: false }), NOON, AWAY), []);
});

test("the delay stays between 15 and 100 s (the relay gives up at ~108 s)", () => {
  assert.deepEqual([APPROVAL_MIN_SEC, APPROVAL_MAX_SEC], [15, 100]);
  assert.equal(approvalDelaySec(on({ approvalAfterSec: 5 })), 15);
  assert.equal(approvalDelaySec(on({ approvalAfterSec: 500 })), 100);
  assert.equal(approvalDelaySec(on({ approvalAfterSec: 45.4 })), 45);
  assert.equal(approvalDelaySec(on({ approvalAfterSec: Number.NaN })), 60);
  const w = new PhoneWatcher();
  assert.equal(w.approvals([wait("r1", 20)], on({ approvalAfterSec: 20 }), NOON, AWAY).length, 1);
});

test("quiet hours: nothing is sent inside them, nothing is sent late", () => {
  const q = on({ quietHours: true, quietFrom: "22:00", quietTo: "07:00" });
  assert.equal(inQuietHours(q, at(23)), true);
  assert.equal(inQuietHours(q, at(3)), true);
  assert.equal(inQuietHours(q, at(7)), false, "the end is outside");
  assert.equal(inQuietHours(q, at(21, 59)), false);
  assert.equal(inQuietHours(q, at(22)), true);
  const day = on({ quietHours: true, quietFrom: "12:30", quietTo: "13:30" });
  assert.equal(inQuietHours(day, at(13)), true);
  assert.equal(inQuietHours(day, at(14)), false);
  assert.equal(inQuietHours(on({ quietFrom: "22:00", quietTo: "07:00" }), at(23)), false, "switched off");
  assert.equal(inQuietHours(on({ quietHours: true, quietFrom: "9:00", quietTo: "9:00" }), at(9)), false, "from = to: never");
  assert.equal(inQuietHours(on({ quietHours: true, quietFrom: "25:00", quietTo: "07:00" }), at(3)), false, "a bad time: never");
  assert.deepEqual([minutes("07:30"), minutes("7:05"), minutes("24:00"), minutes("x")], [450, 425, null, null]);
  // An approval due at 23:00 isn't sent, nor at 07:00 when quiet hours end.
  const w = new PhoneWatcher();
  assert.deepEqual(w.approvals([wait("r1", 60)], q, at(23), AWAY), []);
  assert.deepEqual(w.approvals([wait("r1", 600)], q, at(7), AWAY), []);
  assert.equal(w.finished("integration_claude", "x", false, q, at(23), AWAY), null);
});

test("a finished session pings, unless one of its requests still waits", () => {
  const w = new PhoneWatcher();
  assert.deepEqual(w.finished("integration_claude", "local-companion", false, on(), NOON, AWAY), { kind: "finished", pillId: "integration_claude", project: "local-companion" });
  assert.equal(w.finished("integration_claude", "local-companion", true, on(), NOON, AWAY), null);
  assert.equal(w.finished("integration_claude", "local-companion", false, on({ finished: false }), NOON, AWAY), null);
});

test("a limit pings once per pill and reset, usage or spend", () => {
  const w = new PhoneWatcher();
  assert.equal(w.limit("integration_claude", "p", "usage", "2 PM", on(), NOON, AWAY)?.kind, "usageLimit");
  assert.equal(w.limit("integration_claude", "p", "usage", "2 PM", on(), NOON, AWAY), null, "the same limit again");
  assert.equal(w.limit("integration_claude", "p", "spend", null, on(), NOON, AWAY)?.kind, "spendLimit");
  assert.equal(w.limit("agent_codex", "p", "usage", "2 PM", on(), NOON, AWAY)?.kind, "usageLimit", "another pill");
  assert.equal(w.limit("integration_claude", "p", "usage", "Oct 19, 9 AM", on({ limit: false }), NOON, AWAY), null);
});

test("only when away: no keyboard or mouse input for 2 minutes by default", () => {
  assert.deepEqual([DEFAULT_PHONE.idleOnly, DEFAULT_PHONE.idleAfterMin], [true, 2]);
  assert.equal(away(on(), AT_PC), false);
  assert.equal(away(on(), 119_999), false);
  assert.equal(away(on(), 120_000), true);
  assert.equal(away(on(), null), true, "Windows couldn't say: never silence the phone");
  assert.equal(away(on({ idleOnly: false }), 0), true, "the check switched off");
  assert.equal(away(on({ idleAfterMin: 10 }), AWAY), false);
  assert.deepEqual([IDLE_MIN_MIN, IDLE_MAX_MIN], [1, 60]);
  assert.deepEqual([idleAfterMin(on({ idleAfterMin: 0 })), idleAfterMin(on({ idleAfterMin: 99 })), idleAfterMin(on({ idleAfterMin: Number.NaN }))], [1, 60, 2]);

  // At the PC an approval waits on its card; it pings once you've been away long enough.
  const w = new PhoneWatcher();
  assert.equal(w.due([wait("r1", 60)], on()), true);
  assert.deepEqual(w.approvals([wait("r1", 60)], on(), NOON, AT_PC), []);
  assert.equal(w.due([wait("r1", 70)], on()), true, "still due: not counted while you're at the PC");
  assert.deepEqual(w.approvals([wait("r1", 90)], on(), NOON, 120_000).map((p) => p.kind), ["approval"]);
  assert.equal(w.due([wait("r1", 91)], on()), false, "once");
  assert.equal(w.due([wait("r2", 30)], on()), false, "not past the delay: no need to ask for the idle time");
  assert.equal(w.due([wait("r2", 90)], on({ enabled: false })), false);

  // Finished and limits: dropped at the PC (you saw them), a limit only once.
  assert.equal(w.finished("integration_claude", "p", false, on(), NOON, AT_PC), null);
  assert.equal(w.finished("integration_claude", "p", false, on({ idleOnly: false }), NOON, AT_PC)?.kind, "finished");
  assert.equal(w.limit("integration_claude", "p", "usage", "2 PM", on(), NOON, AT_PC), null);
  assert.equal(w.limit("integration_claude", "p", "usage", "2 PM", on(), NOON, AWAY), null, "the same limit later: already seen");
});

test("the project is the pill's name, nothing else about the session", () => {
  const tasks = [{ id: "integration_claude", name: "local-companion", sessionCwd: "C:\\Users\\antho\\local-companion" }];
  assert.equal(projectOf(tasks, "integration_claude"), "local-companion");
  assert.equal(projectOf(tasks, "agent_codex"), "");
});

// ── The island: hooks and the wait tick ───────────────────────────────────────

const island = { alert() {}, setView() {}, reveal() {}, dropPin() {} };
registerHookHandlers(island);
const hook = (payload) => emit("hook", payload);

beforeEach(() => {
  mock.timers.enable({ apis: ["setTimeout"] });
  calls.length = 0;
  State.tasks = [];
  State.pendingApproval = null;
  State.approvalQueue = [];
  State.settings = { ...DEFAULT_SETTINGS, phone: on() };
  State.loadIntegrationTasks();
  replies.clear();
});

// setTimeout is mocked for the hook tests: the idle answer needs a real turn.
const settle = () => new Promise((r) => setImmediate(r));

afterEach(() => {
  mock.timers.runAll();
  mock.timers.reset();
});

test("a Stop asks Rust for a \"finished\" push: kind, pill and project only", async () => {
  replies.set("phone_idle_ms", AWAY);
  hook({ hook_event_name: "UserPromptSubmit", session_id: "s1", cwd: "C:\\Users\\antho\\local-companion", prompt: "secret prompt text" });
  hook({ hook_event_name: "Stop", session_id: "s1", cwd: "C:\\Users\\antho\\local-companion", last_assistant_message: "secret answer" });
  await settle();
  const pushes = sent("phone_notify");
  assert.equal(pushes.length, 1);
  assert.deepEqual(Object.keys(pushes[0]).sort(), ["kind", "pillId", "project"]);
  assert.equal(pushes[0].kind, "finished");
  assert.ok(!JSON.stringify(pushes).includes("secret"), "never the prompt or the answer");
});

test("a usage or spend limit asks for its push; off, nothing is asked", async () => {
  replies.set("phone_idle_ms", AWAY);
  hook({ hook_event_name: "StopFailure", session_id: "s1", error: "billing_error", last_assistant_message: "Spend limit reached" });
  await settle();
  assert.deepEqual(sent("phone_notify").map((p) => p.kind), ["spendLimit"]);
  calls.length = 0;
  State.settings = { ...DEFAULT_SETTINGS };
  hook({ hook_event_name: "Stop", session_id: "s2" });
  hook({ hook_event_name: "StopFailure", session_id: "s2", error: "rate_limit" });
  await settle();
  assert.deepEqual(sent("phone_notify"), [], "the default: off");
  assert.deepEqual(sent("phone_idle_ms"), [], "off: not even the idle time is asked for");
});

test("at the PC, a Stop asks for the idle time and sends nothing", async () => {
  replies.set("phone_idle_ms", AT_PC);
  hook({ hook_event_name: "UserPromptSubmit", session_id: "s3", cwd: "C:\\Users\\antho\\local-companion", prompt: "x" });
  hook({ hook_event_name: "Stop", session_id: "s3", cwd: "C:\\Users\\antho\\local-companion" });
  await settle();
  assert.equal(sent("phone_idle_ms").length, 1);
  assert.deepEqual(sent("phone_notify"), []);
});

test("the wait tick hands waiting requests to the watcher", () => {
  const src = read("src/island/island.ts");
  const tick = src.slice(src.indexOf("  private tickWait() {"), src.indexOf("  /** The request has its answer"));
  assert.ok(tick.includes("phone.due(waiting, s) && !this.askingIdle"), "the idle time is asked for only when one is due");
  assert.ok(tick.includes("phone.approvals(waiting, s, new Date(), idleMs)"));
  assert.ok(tick.includes("void Bridge.phoneNotify(p.kind, p.pillId, p.project)"));
  assert.ok(tick.includes("project: projectOf(State.tasks, r.pillId)"));
  // Nothing but the kind, the pill and the project ever goes to Rust.
  for (const file of ["src/island/island.ts", "src/island/hooks.ts"]) {
    for (const call of read(file).match(/Bridge\.phoneNotify\([^)]*\)/g) ?? []) {
      assert.equal(call, "Bridge.phoneNotify(p.kind, p.pillId, p.project)", `${file}: ${call}`);
    }
  }
});

// ── Settings → Phone ──────────────────────────────────────────────────────────

const toggle = (onNow, onChange) => {
  const el = document.createElement("button");
  el.className = onNow ? "switch on" : "switch";
  el.addEventListener("click", () => onChange(!el.className.includes("on")));
  return el;
};

test("Settings → Phone: off by default, every setting, and the test button", async () => {
  const settings = { ...DEFAULT_SETTINGS };
  let saves = 0;
  const el = await phoneSection({ settings: () => settings, save: async () => void saves++, toggle });
  const role = (r) => el.find("div").find((d) => d.getAttribute("data-role") === r);
  for (const r of ["phone-enabled", "phone-keys", "phone-approval", "phone-delay", "phone-finished", "phone-limit", "phone-meeting", "phone-project", "phone-idle", "phone-quiet", "phone-test"]) {
    assert.ok(role(r), r);
  }
  const test = role("phone-test").find("button")[0];
  assert.equal(test.getAttribute("disabled"), "", "off and no keys: the test button waits");
  assert.equal(role("phone-meeting").find("button")[0].getAttribute("disabled"), "", "the meeting needs #34");
  // Only when away: on, 2 minutes; off greys the minutes out.
  assert.equal(role("phone-idle").find("input")[0].getAttribute("value"), "2");
  role("phone-idle").find("button")[0].fire("click");
  assert.equal(settings.phone.idleOnly, false);
  assert.equal(role("phone-idle").find("input")[0].getAttribute("disabled"), "");
  role("phone-enabled").find("button")[0].fire("click");
  assert.equal(settings.phone.enabled, true);
  assert.ok(saves > 0);
  // The keys go to Rust once, on Save, and leave the page at once.
  const [user, token] = el.find("input").filter((i) => i.getAttribute("type") === "password");
  user.value = "uQiRzpo4DXghDmr9QzzfQu27cmVRsG";
  token.value = "azGDORePK8gMaC0QOYAMyEEuzJnyUi";
  el.find("button").find((b) => b.textContent === "Save keys").fire("click");
  // setTimeout is mocked for the hook tests: wait a real turn.
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(sent("phone_keys_save").at(-1), { user: "uQiRzpo4DXghDmr9QzzfQu27cmVRsG", token: "azGDORePK8gMaC0QOYAMyEEuzJnyUi" });
  assert.deepEqual([user.value, token.value], ["", ""]);
});

test("the page never keeps the keys, and settings.json has no place for them", () => {
  const src = read("src/settings/phone.ts");
  for (const banned of ["console.", "localStorage", "sessionStorage"]) assert.ok(!src.includes(banned), banned);
  assert.deepEqual(Object.keys(DEFAULT_SETTINGS.phone).filter((k) => /key|token/i.test(k)), []);
  // The settings round trip as Rust has them (settings.rs PhoneSettings).
  assert.deepEqual(Object.keys(DEFAULT_PHONE).sort(), ["approval", "approvalAfterSec", "enabled", "finished", "idleAfterMin", "idleOnly", "includeProject", "limit", "meeting", "quietFrom", "quietHours", "quietTo"]);
  const rs = read("src-tauri/src/settings.rs");
  for (const field of ["enabled: false", "approval_after_sec: 60", "include_project: true", "idle_only: true", "idle_after_min: 2", "quiet_hours: false"]) assert.ok(rs.includes(field), field);
});
