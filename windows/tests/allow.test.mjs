// The Allow key (fix e, redesigned by the owner 2026-10-17; core/allow.ts,
// island.ts allowApproval, the approval card in views.ts): Ctrl+Alt+Shift+Y
// allows any waiting permission once its card has been open, showing the whole
// command, for 1.5 s. Destructive commands take a second press within 3 s.
// Plus the waiting-approval escalation (core/approval-wait.ts).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State, DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const {
  ALLOW_DELAY_MS, CONFIRM_WINDOW_MS, DESTRUCTIVE, REASON_MS, allowCheck, armedFor, cardOffScreen, cardOnScreen, expandWindow, expandedFor, globMatches, isDestructive, onScreen, press, refusalFor,
} = await import("../src/core/allow.ts");
const { ApprovalWait, ESCALATE_AFTER_MS, PULSE_EVERY_MS } = await import("../src/core/approval-wait.ts");
const { buildApproval } = await import("../src/views/views.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const req = (command, extra = {}) => ({ requestId: "r1", sessionId: "s1", pillId: "integration_claude", tool: "Bash", command, ...extra });
const check = (o) => allowCheck({ request: req("Bash · npm test"), open: true, fullyShown: true, shownMs: 2_000, ...o });

test("the key allows only after the card has been on screen 1.5 s", () => {
  assert.equal(ALLOW_DELAY_MS, 1_500);
  assert.equal(check({ shownMs: 0 }), "too-soon");
  assert.equal(check({ shownMs: 1_499 }), "too-soon");
  assert.equal(check({ shownMs: 1_500 }), "ok");
  // The clock is per request, and starts again after the card was away.
  cardOffScreen();
  cardOnScreen("r1", 10_000, true);
  assert.deepEqual(onScreen("r1", 11_000), { shownMs: 1_000, fullyShown: true });
  cardOnScreen("r1", 11_600, true);
  assert.equal(onScreen("r1", 11_600).shownMs, 1_600, "still the same showing");
  assert.equal(onScreen("r2", 11_600).shownMs, 0, "another request: not shown yet");
  cardOnScreen("r2", 11_700, true);
  assert.equal(onScreen("r2", 11_800).shownMs, 100, "a new request starts its own clock");
  cardOffScreen();
  assert.deepEqual(onScreen("r2", 20_000), { shownMs: 0, fullyShown: false }, "folded or closed: the clock stops");
  cardOnScreen("r2", 20_000, true);
  assert.equal(onScreen("r2", 21_000).shownMs, 1_000, "opened again: it starts over");
  cardOffScreen();
});

test("every permission card takes the key, once open showing the whole command", () => {
  assert.equal(check({ request: null }), "no-card");
  assert.equal(check({ request: req("x", { questions: [{ question: "Which?" }] }) }), "question");
  assert.equal(check({ open: false }), "not-open", "folded, queued, or another view");
  assert.equal(check({ fullyShown: false }), "expand", "cut off: the press shows it all first");
  assert.equal(check({ request: req("Bash · npm run something long…") }), "too-long", "shortened by the relay: click only");
  assert.equal(check({ request: req("Bash · npm run something long…"), fullyShown: false }), "too-long", "expanding can't show what the relay cut");
  // The old design kept "ask"-rule commands click-only: no longer (owner, 2026-10-17).
  for (const c of ["Bash · git merge origin/main", "Bash · gh pr merge 27 --merge", "Bash · git push origin main", "Edit · src/a.ts", "Bash · npm test"]) {
    assert.equal(check({ request: req(c) }), "ok", c);
  }
});

test("destructive commands are recognised, the rest are not", () => {
  const destructive = [
    "Bash · rm -rf node_modules",
    "Bash · cd x && rm -rf build",
    "Bash · rm -fr build",
    "PowerShell · Remove-Item -Path C:\\tmp -Recurse -Force",
    "PowerShell · Remove-Item C:\\tmp -Force -Recurse",
    "Bash · git push --force origin main",
    "Bash · git push -f origin main",
    "Bash · git push origin main --force-with-lease",
    "Bash · psql -c 'DROP TABLE tenants'",
    "Bash · sqlite3 db.sqlite \"drop table x\"",
    "Bash · echo 'TRUNCATE TABLE leases' | psql",
    "Bash · npx supabase db reset",
    "Bash · supabase migration up",
    "Bash · mysql -e 'select 1'",
    "Bash · git status\nrm -rf dist",
  ];
  for (const c of destructive) assert.ok(isDestructive(c), c);
  for (const c of ["Bash · npm test", "Bash · git status", "Bash · git merge origin/main", "Bash · gh pr merge 27 --merge", "Bash · git push origin main", "Bash · rm notes.txt", "Bash · grep merge src", "Edit · src/a.ts", "Bash · ls supabase/migrations"]) {
    assert.ok(!isDestructive(c), c);
  }
  // Globs: * is anything, the rest literal, any case.
  assert.ok(globMatches("*Remove-Item*-Recurse*-Force*", "remove-item x -recurse -force"));
  assert.ok(globMatches("a.b(c)*", "A.B(C) d"));
  assert.ok(!globMatches("a.b", "axb"), "regex characters are literal");
  // The owner's list (2026-10-17) is all there.
  for (const rule of ["*rm -rf*", "*Remove-Item*-Recurse*-Force*", "*git push --force*", "*drop table*", "*truncate *", "psql*", "*supabase *"]) {
    assert.ok(DESTRUCTIVE.includes(rule), rule);
  }
});

test("a destructive command: the first press arms, a second within 3 s allows", () => {
  assert.equal(CONFIRM_WINDOW_MS, 3_000);
  const r = req("Bash · rm -rf build", { requestId: "d1" });
  cardOffScreen();
  cardOnScreen("d1", 0, true);
  assert.equal(press(r, true, 1_000), "too-soon", "the 1.5 s delay comes first");
  assert.equal(armedFor(1_000), null, "a press too soon doesn't arm");
  assert.equal(press(r, true, 2_000), "armed");
  assert.deepEqual(armedFor(2_000), { requestId: "d1", at: 2_000 });
  assert.equal(press(r, true, 4_900), "ok", "the second press, 2.9 s later");
  assert.equal(armedFor(4_900), null, "allowing clears it");
  // Exactly at the window's end still counts; a moment later it doesn't.
  assert.equal(press(r, true, 10_000), "armed");
  assert.equal(press(r, true, 13_000), "ok");
  cardOffScreen();
});

test("the two-press window resets: too late, another request, or the card leaves", () => {
  const r = req("Bash · git push --force", { requestId: "d2" });
  cardOffScreen();
  cardOnScreen("d2", 0, true);
  assert.equal(press(r, true, 2_000), "armed");
  assert.equal(armedFor(5_001), null, "the card drops \"Press again\" after 3 s");
  assert.equal(press(r, true, 5_001), "armed", "too late: this press is a first press again");
  assert.equal(press(r, true, 6_000), "ok");
  // Armed for one request, then another comes up: the new one starts over.
  cardOnScreen("d2", 7_000, true);
  assert.equal(press(r, true, 9_000), "armed");
  cardOnScreen("d3", 9_500, true);
  assert.equal(armedFor(9_500), null, "a new card forgets the first press");
  const r3 = req("Bash · rm -rf x", { requestId: "d3" });
  assert.equal(press(r3, true, 11_000), "armed", "and needs its own two presses");
  // Folded between the presses: forgotten, and the delay starts over.
  cardOffScreen();
  assert.equal(armedFor(11_100), null);
  assert.equal(press(r3, false, 11_200), "not-open");
  cardOnScreen("d3", 12_000, true);
  assert.equal(press(r3, true, 12_500), "too-soon");
  // A plain command never arms.
  cardOnScreen("p1", 20_000, true);
  assert.equal(press(req("Bash · npm test", { requestId: "p1" }), true, 21_600), "ok");
  assert.equal(armedFor(21_600), null);
  cardOffScreen();
});

test("the escalation: after 30 s the lantern pulses every 4 s and the sound comes once", () => {
  assert.equal(ESCALATE_AFTER_MS, 30_000);
  assert.equal(PULSE_EVERY_MS, 4_000);
  const w = new ApprovalWait();
  let t = w.tick(["a"], 0);
  assert.deepEqual(t, { pulse: false, chime: false, ended: [] });
  t = w.tick(["a"], 29_999);
  assert.equal(t.pulse, false, "not before 30 s");
  t = w.tick(["a"], 30_000);
  assert.equal(t.pulse, true);
  assert.equal(t.chime, true, "the sound, once");
  t = w.tick(["a"], 31_000);
  assert.deepEqual([t.pulse, t.chime], [false, false], "the next pulse waits 4 s");
  t = w.tick(["a"], 34_000);
  assert.deepEqual([t.pulse, t.chime], [true, false], "pulses again, no second sound");
  // A second request, queued at 35 s, escalates on its own clock: one more sound.
  w.tick(["a", "b"], 35_000);
  t = w.tick(["a", "b"], 64_999);
  assert.equal(t.chime, false, "b has waited 29.999 s");
  t = w.tick(["a", "b"], 65_000);
  assert.equal(t.chime, true, "b has waited 30 s now");
  // Answered: the wait is logged with how, and the pulsing stops.
  w.decided("a", "allow (key)");
  t = w.tick(["b"], 66_000);
  assert.deepEqual(t.ended, [{ requestId: "a", waitedMs: 66_000, how: "allow (key)", escalated: true }]);
  t = w.tick([], 67_000);
  assert.deepEqual(t.ended, [{ requestId: "b", waitedMs: 32_000, how: "answered elsewhere", escalated: true }]);
  assert.equal(t.pulse, false, "nothing waiting: no pulse");
  // A quick answer: logged, never escalated.
  w.tick(["c"], 60_000);
  w.decided("c", "deny (click)");
  t = w.tick([], 62_500);
  assert.deepEqual(t.ended, [{ requestId: "c", waitedMs: 2_500, how: "deny (click)", escalated: false }]);
  assert.equal(w.waited("c", 70_000), 0);
  // A new card right after an escalated one starts from nothing.
  w.tick(["d"], 100_000);
  t = w.tick(["d"], 101_000);
  assert.equal(t.pulse, false);
});

test("the card shows both keys and, after a first press, \"Press again to allow · 3\"", () => {
  State.mode = "expanded";
  State.view = "approval";
  State.tasks = [];
  State.approvalQueue = [];
  State.pendingApproval = req("Bash · rm -rf build", { requestId: "k1" });
  const card = buildApproval(new Proxy({}, { get: () => () => {} }));
  const now = performance.now();
  card.sync();
  const keyText = (role) => card.el.find("span").find((s) => s.getAttribute("data-role") === role);
  const confirm = card.el.find("div").find((d) => d.getAttribute("data-role") === "allow-confirm");
  assert.equal(keyText("deny-key").textContent, "Ctrl+Alt+Shift+N");
  assert.equal(keyText("allow-key").textContent, "Ctrl+Alt+Shift+Y", "every card shows the Allow key, destructive too");
  assert.equal(confirm.style.display, "none");
  // A first press (fake clock well past the delay): the card names the command.
  cardOffScreen();
  cardOnScreen("k1", now - 5_000, true);
  assert.equal(press(State.pendingApproval, true, now), "armed");
  card.sync();
  assert.equal(confirm.style.display, "");
  assert.equal(confirm.textContent, "Press again to allow · 3");
  // A rebound key shows as bound.
  State.settings = { ...State.settings, shortcuts: { allowApproval: { keys: "Ctrl+Alt+Shift+K", enabled: true } } };
  State.pendingApproval = req("Bash · ls", { requestId: "k3" });
  card.sync();
  assert.equal(keyText("allow-key").textContent, "Ctrl+Alt+Shift+K");
  assert.equal(confirm.style.display, "none", "another request: no prompt");
  State.settings = { ...State.settings, shortcuts: {} };
  State.pendingApproval = null;
  State.mode = "hidden";
  cardOffScreen();
});

// Bug 2 (owner's retest 2026-10-18): press, wait, press, wait, press, and the
// key stopped working ("allow key refused (cut-off)" from then on). The
// "Press again" line takes room from the command's box, and the redraw that
// takes it away measured the box before it did: "cut-off" stuck until
// something else redrew the card. The fit is now measured at the press.
function squeezedCard(requestId, command) {
  State.mode = "expanded";
  State.view = "approval";
  State.tasks = [];
  State.approvalQueue = [];
  State.pendingApproval = req(command, { requestId });
  const card = buildApproval(new Proxy({}, { get: () => () => {} }));
  const code = card.el.find("div").find((d) => d.className === "code");
  const confirm = card.el.find("div").find((d) => d.getAttribute("data-role") === "allow-confirm");
  // The real layout: 30 px of command; 40 px of box, 20 px while the line shows.
  code.scrollHeight = 30;
  code.scrollWidth = code.clientWidth = 300;
  Object.defineProperty(code, "clientHeight", { get: () => (confirm.style.display === "none" ? 40 : 20) });
  return { card, confirm };
}

function withClock(fn) {
  const real = performance.now;
  let clock = 0;
  performance.now = () => clock;
  try {
    fn((t) => (clock = t));
  } finally {
    performance.now = real;
    State.pendingApproval = null;
    State.mode = "hidden";
    cardOffScreen();
  }
}

test("the two-press window works after any number of resets (bug 2)", () => {
  withClock((at) => {
    cardOffScreen();
    const { card } = squeezedCard("c1", "PowerShell · Remove-Item C:/tmp/x -Recurse -Force");
    at(0);
    card.sync();
    // Five times: a first press, then nothing until the window has closed.
    // The island redraws the card on the press and when the window ends.
    let t = 2_000;
    for (let round = 1; round <= 5; round++) {
      at(t);
      assert.equal(press(State.pendingApproval, true, t), "armed", `round ${round}: a first press`);
      card.sync();
      at(t + CONFIRM_WINDOW_MS + 50);
      card.sync();
      t += CONFIRM_WINDOW_MS + 1_000;
    }
    // Then the two presses allow.
    at(t);
    assert.equal(press(State.pendingApproval, true, t), "armed");
    card.sync();
    at(t + 800);
    assert.equal(press(State.pendingApproval, true, t + 800), "ok", "the second press allows");
  });
});

test("a plain command isn't refused as cut off by a stale measurement", () => {
  withClock((at) => {
    cardOffScreen();
    const { card, confirm } = squeezedCard("c2", "Bash · npm test");
    at(0);
    // Drawn mid-expand (or with a line still showing): it measured cut off.
    confirm.style.display = "";
    card.sync();
    // Nothing redraws it; the box is whole by the time of the press.
    confirm.style.display = "none";
    at(2_000);
    assert.equal(press(State.pendingApproval, true, 2_000), "ok");
  });
});

test("after a first press the card counts down: \"Press again to allow · 3…2…1\"", () => {
  withClock((at) => {
    cardOffScreen();
    const { card, confirm } = squeezedCard("c3", "Bash · rm -rf build");
    at(0);
    card.sync();
    at(2_000);
    assert.equal(press(State.pendingApproval, true, 2_000), "armed");
    const shown = (t) => {
      at(t);
      card.sync();
      return confirm.style.display === "none" ? "" : confirm.textContent;
    };
    assert.equal(shown(2_000), "Press again to allow · 3");
    assert.equal(shown(3_000), "Press again to allow · 2");
    assert.equal(shown(4_000), "Press again to allow · 1");
    assert.equal(shown(4_999), "Press again to allow · 1");
    assert.equal(shown(5_050), "", "the window has closed");
  });
});

test("the island: the key goes through press(), the wait is ticked, logged and escalated", () => {
  const island = read("src/island/island.ts");
  const fn = island.slice(island.indexOf("  allowApproval() {"), island.indexOf("  takeKeyboard() {"));
  assert.ok(fn.includes('press(req, State.mode === "expanded" && State.view === "approval", now)'));
  assert.ok(fn.includes('if ((check === "armed" || check === "expand") && req)') && fn.includes("CONFIRM_WINDOW_MS + 50"), "armed or expanded: the card asks, then drops it");
  assert.ok(fn.includes('if (check === "expand") this.animateGeometry(false);'), "expanding grows the card");
  assert.ok(fn.includes("window.setTimeout(() => State.notify(), REASON_MS + 50);"), "a refusal's reason shows, then goes");
  assert.ok(island.includes('if (State.mode === "expanded" && State.view === "approval" && expandedFor(State.pendingApproval?.requestId)) {\n      h = APPROVAL_EXPANDED_H;'));
  assert.ok(fn.includes("for (let s = 1_000; s < CONFIRM_WINDOW_MS; s += 1_000) window.setTimeout(() => State.notify(), s);"), "and counts down each second");
  assert.ok(fn.includes('if (!req || check !== "ok")') && fn.includes('this.emote("annoyed")'));
  assert.ok(fn.indexOf('Bridge.approvalDecision(req.requestId, "allow")') > fn.indexOf('check !== "ok"'), "allowed only after the check");
  assert.ok(island.includes('if (!(expanded && State.view === "approval")) cardOffScreen();'));
  assert.ok(island.includes("window.setInterval(() => this.tickWait(), 1_000);"));
  const tick = island.slice(island.indexOf("  private tickWait() {"), island.indexOf("  /** The request has its answer"));
  assert.ok(tick.includes("if (tick.pulse) this.lantern.attention();"));
  assert.ok(tick.includes("if (tick.chime && State.settings.approvalSound === true) playSoftChime();"), "the sound only when it's on");
  assert.ok(tick.includes("approval waited ${(e.waitedMs / 1000).toFixed(1)} s"), "the wait is logged, never the command");
  assert.ok(!tick.includes("command"));
  for (const how of ["`${d} (click)`", '"answered (click)"', '"sent to the terminal"', '"deny (key)"', '"allow (key)"']) {
    assert.ok(island.includes(`this.wait.decided(req.requestId, ${how})`), how);
  }
  // The ask-rule reading is gone (no Rust command, no bridge call).
  assert.ok(!read("src-tauri/src/lib.rs").includes("approval_ask_rules"));
  assert.ok(!read("src/core/bridge.ts").includes("approvalAskRules"));
});

// The Allow key on long commands (fix 2026-10-25; glim.log 2026-10-24: 7
// presses on 2 bash approvals, every one "refused (cut-off)"). A cut-off
// command's first press expands the card; the next press, within the
// window and at least 1.5 s after expanding, allows.
const LONG = "Bash · cd ~/local-companion && git fetch origin && git merge --no-edit origin/main && npm --prefix windows test && npx tsc --noEmit -p windows";
const LONG_RM = "Bash · cd ~/local-companion/windows && rm -rf dist target/release/bundle node_modules/.vite && npm ci && npm run build && npm run pack";

// What the card measures: cut off until expanded, then whole (a command that
// fits the expanded box), unless `scrolls`.
function longCard(id, { scrolls = false } = {}) {
  cardOnScreen(id, 0, () => expandedFor(id) !== null && !scrolls);
}

test("a long command: the first press expands the card, the second allows", () => {
  cardOffScreen();
  const r = req(LONG, { requestId: "L1" });
  longCard("L1");
  assert.equal(press(r, true, 2_000), "expand", "cut off: shown in full first, never allowed blind");
  assert.deepEqual(expandedFor("L1"), { requestId: "L1", at: 2_000, armedAt: 2_000 });
  assert.equal(press(r, true, 3_600), "ok", "the second press, 1.6 s after expanding");
  assert.equal(expandedFor("L1"), null, "allowed: done");
  cardOffScreen();
});

test("the 1.5 s delay still applies after expanding, even on a card open for long", () => {
  cardOffScreen();
  const r = req(LONG, { requestId: "L2" });
  longCard("L2");
  assert.equal(press(r, true, 60_000), "expand");
  assert.equal(press(r, true, 60_900), "too-soon", "0.9 s after the whole command appeared");
  assert.equal(refusalFor("L2", 60_900), "too-soon", "the card says why");
  assert.equal(refusalFor("L2", 60_900 + REASON_MS + 1), null, "for a moment");
  assert.equal(press(r, true, 61_600), "ok");
  cardOffScreen();
});

test("expanded, the window closes like the two-press one: the next press opens it again", () => {
  cardOffScreen();
  const r = req(LONG, { requestId: "L3" });
  longCard("L3", { scrolls: true });
  assert.equal(press(r, true, 1_000), "expand");
  assert.ok(expandWindow("L3", 1_000 + CONFIRM_WINDOW_MS));
  assert.equal(expandWindow("L3", 1_001 + CONFIRM_WINDOW_MS), null);
  assert.equal(press(r, true, 10_000), "expand", "too late: this press only opens the window again");
  assert.equal(expandedFor("L3").at, 1_000, "the card stayed expanded: the delay counts from the first expansion");
  // Taller than the expanded box: not until its end has been on screen.
  assert.equal(press(r, true, 10_200), "scroll");
  assert.equal(refusalFor("L3", 10_200), "scroll", "the card says to scroll");
  cardOnScreen("L3", 10_300, true); // scrolled to the end
  assert.equal(press(r, true, 10_400), "ok", "then the next press allows straight away");
  // Folded: it starts small again and must be expanded again.
  longCard("L4");
  assert.equal(press(req(LONG, { requestId: "L4" }), true, 22_000), "expand");
  cardOffScreen();
  assert.equal(expandedFor("L4"), null);
  longCard("L4");
  assert.equal(press(req(LONG, { requestId: "L4" }), true, 25_000), "expand");
  cardOffScreen();
});

test("a long destructive command: expand, then its own two presses", () => {
  cardOffScreen();
  const r = req(LONG_RM, { requestId: "LD" });
  assert.ok(isDestructive(LONG_RM));
  longCard("LD");
  assert.equal(press(r, true, 2_000), "expand", "expanding is not one of the two");
  assert.equal(press(r, true, 2_500), "too-soon");
  assert.equal(press(r, true, 3_600), "armed", "the first of two");
  assert.equal(press(r, true, 4_500), "ok", "the second");
  cardOffScreen();
});

test("a short command: one press, unchanged", () => {
  cardOffScreen();
  cardOnScreen("S1", 0, true);
  assert.equal(press(req("Bash · npm test", { requestId: "S1" }), true, 1_600), "ok");
  assert.equal(expandedFor("S1"), null, "nothing to expand");
  cardOffScreen();
});

test("a command the relay cut short: refused, and the card says to click", () => {
  withClock((at) => {
    cardOffScreen();
    const cut = `Bash · ${"echo x && ".repeat(40)}…`;
    const { card } = squeezedCard("T1", cut);
    at(0);
    card.sync();
    const why = card.el.find("div").find((d) => d.getAttribute("data-role") === "allow-why");
    assert.equal(why.textContent, "Too long to show in full: click Allow.", "said before any press");
    assert.equal(why.style.display, "");
    at(2_000);
    assert.equal(press(State.pendingApproval, true, 2_000), "too-long");
    assert.equal(expandedFor("T1"), null, "never expanded into an allow");
    at(5_000);
    assert.equal(press(State.pendingApproval, true, 5_000), "too-long");
  });
});

test("the card: expanded to the whole command, \"Press again to allow\", and a refusal's reason", () => {
  withClock((at) => {
    cardOffScreen();
    const { card, confirm } = squeezedCard("V1", LONG);
    const code = card.el.find("div").find((d) => d.className.split(" ")[0] === "code");
    const why = card.el.find("div").find((d) => d.getAttribute("data-role") === "allow-why");
    // One line of a long command: it doesn't fit.
    code.scrollWidth = 900;
    at(0);
    card.sync();
    assert.ok(!code.classList.contains("expanded"));
    at(2_000);
    assert.equal(press(State.pendingApproval, true, 2_000), "expand");
    card.sync();
    assert.ok(code.classList.contains("expanded"), "the box wraps and grows");
    // Wrapped, it fits the expanded box (with the "Press again" line showing).
    code.scrollWidth = 300;
    code.scrollHeight = 15;
    assert.equal(confirm.textContent, "Press again to allow · 3");
    at(2_500);
    assert.equal(press(State.pendingApproval, true, 2_500), "too-soon");
    card.sync();
    assert.equal(why.textContent, "Too soon: read it, then press again.");
    at(2_500 + REASON_MS + 100);
    card.sync();
    assert.equal(why.style.display, "none", "the reason goes after a moment");
    // A destructive one asks for two.
    cardOffScreen();
    State.pendingApproval = req(LONG_RM, { requestId: "V2" });
    code.scrollWidth = 900;
    at(10_000);
    card.sync();
    at(12_000);
    assert.equal(press(State.pendingApproval, true, 12_000), "expand");
    card.sync();
    assert.equal(confirm.textContent, "Read it, then press twice to allow");
    // At the boundary (security review of #59): it would fit without the
    // "Press again" line, but with it showing its last line is out of view.
    cardOffScreen();
    State.pendingApproval = req(LONG, { requestId: "V4" });
    code.scrollWidth = 900;
    code.scrollHeight = 30;
    code.scrollTop = 0;
    at(30_000);
    card.sync();
    at(32_000);
    assert.equal(press(State.pendingApproval, true, 32_000), "expand");
    card.sync();
    code.scrollWidth = 300;
    assert.equal(confirm.style.display, "", "the line is on screen");
    assert.equal(code.clientHeight, 20, "and takes room from the box");
    at(34_000);
    assert.equal(press(State.pendingApproval, true, 34_000), "scroll", "measured as displayed: not allowed");
    code.scrollTop = 10;
    assert.equal(press(State.pendingApproval, true, 34_100), "ok", "scrolled to the end: allowed");
    // Taller than the expanded box (it scrolls): not until scrolled to its end.
    cardOffScreen();
    State.pendingApproval = req(LONG, { requestId: "V3" });
    code.scrollWidth = 900;
    at(20_000);
    card.sync();
    at(22_000);
    assert.equal(press(State.pendingApproval, true, 22_000), "expand");
    card.sync();
    code.scrollWidth = 300;
    code.scrollHeight = 400;
    code.scrollTop = 0;
    at(24_000);
    assert.equal(press(State.pendingApproval, true, 24_000), "scroll");
    card.sync();
    assert.equal(why.textContent, "Scroll to the end of the command, then press again.");
    code.scrollTop = 400 - code.clientHeight;
    at(24_500);
    assert.equal(press(State.pendingApproval, true, 24_500), "ok", "its end has been on screen");
  });
  // The CSS and the height exist.
  assert.match(read("src/style.css"), /\.code\.expanded \{[^}]*white-space: pre-wrap;[^}]*overflow-y: auto;/);
  assert.match(read("src/core/layout.ts"), /export const APPROVAL_EXPANDED_H = 300;/);
});

test("the relay keeps the card's field up to 32 KB on a permission request", () => {
  const hook = read("hook/src/main.rs");
  assert.match(hook, /const MAX_APPROVAL_LEN: usize = 32 \* 1024;/);
  assert.match(hook, /matches!\(event, "PermissionRequest" \| "PreToolUse"\)/);
  // The same fields the island shows (hooks.ts APPROVAL_FIELDS).
  const island = read("src/island/hooks.ts");
  const shown = [...island.slice(island.indexOf("const APPROVAL_FIELDS = ["), island.indexOf("] as const;")).matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
  const kept = [...hook.slice(hook.indexOf("const APPROVAL_FIELDS: &[&str] = &["), hook.indexOf("const MAX_APPROVAL_LEN")).matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
  assert.deepEqual(kept, shown);
});

test("the soft sound is off by default and has its Settings switch", () => {
  assert.notEqual(DEFAULT_SETTINGS.approvalSound, true);
  const rs = read("src-tauri/src/settings.rs");
  assert.ok(rs.includes("approval_sound: false,"));
  const main = read("src/settings/main.ts");
  assert.ok(main.includes('toggle(settings.approvalSound === true, (v) => { settings.approvalSound = v; void save(); })'));
});
