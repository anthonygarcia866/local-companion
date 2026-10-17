// The Allow key (fix e, owner 2026-10-17; core/allow.ts, island.ts
// allowApproval, the approval card in views.ts): Ctrl+Alt+Shift+Y allows a
// waiting permission only with its card open, showing the whole command, for
// at least 1.5 s, and never for a command the owner's Claude Code "ask" rules
// name; those stay click-only and the card says so.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { ALLOW_DELAY_MS, BUILT_IN_RISKY, allowCheck, cardOffScreen, cardOnScreen, globMatches, isRisky, onScreen, setLiveRules, liveRules } = await import("../src/core/allow.ts");
const { buildApproval } = await import("../src/views/views.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const req = (command, extra = {}) => ({ requestId: "r1", sessionId: "s1", pillId: "integration_claude", tool: "Bash", command, ...extra });
const check = (o) => allowCheck({ request: req("Bash · npm test"), open: true, fullyShown: true, shownMs: 2_000, live: [], ...o });

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

test("only an open permission card showing the whole command", () => {
  assert.equal(check({ request: null }), "no-card");
  assert.equal(check({ request: req("x", { questions: [{ question: "Which?" }] }) }), "question");
  assert.equal(check({ open: false }), "not-open", "folded, queued, or another view");
  assert.equal(check({ fullyShown: false }), "cut-off");
  assert.equal(check({ request: req("Bash · npm run something long…") }), "cut-off", "shortened by the relay");
});

test("never for a command the ask rules name, however long the card waits", () => {
  const risky = [
    "Bash · rm -rf node_modules",
    "Bash · cd x && rm -rf build",
    "PowerShell · Remove-Item -Path C:\\tmp -Recurse -Force",
    "PowerShell · Remove-Item C:\\tmp -Force -Recurse",
    "Bash · git push --force origin main",
    "Bash · git push -f origin main",
    "Bash · gh pr merge 27 --merge",
    "Bash · git merge origin/main",
    "Bash · psql -c 'DROP TABLE tenants'",
    "Bash · sqlite3 db.sqlite \"drop table x\"",
    "Bash · echo 'TRUNCATE TABLE leases' | psql",
    "Bash · npx supabase db reset",
    "Bash · git status\ngit merge feature",
  ];
  for (const c of risky) {
    assert.ok(isRisky(c), c);
    assert.equal(check({ request: req(c), shownMs: 60_000 }), "risky", c);
  }
  for (const c of ["Bash · npm test", "Bash · git status", "Bash · git log --oneline", "Edit · src/a.ts", "Bash · git push origin main", "Bash · rm notes.txt", "Bash · grep merge src"]) {
    assert.ok(!isRisky(c), c);
  }
  // The live rules from settings.json count too (a rule added later).
  assert.ok(!isRisky("Bash · terraform destroy"));
  assert.ok(isRisky("Bash · terraform destroy", ["terraform destroy*"]));
  assert.equal(check({ request: req("Bash · terraform destroy"), live: ["terraform destroy*"] }), "risky");
  setLiveRules(["a*", 7, "", null]);
  assert.deepEqual(liveRules(), ["a*"]);
  setLiveRules([]);
  // Globs: * is anything, the rest literal, any case.
  assert.ok(globMatches("*Remove-Item* -Recurse*-Force*", "remove-item x -recurse -force"));
  assert.ok(!globMatches("git merge*", "echo git merge"));
  assert.ok(globMatches("a.b(c)*", "A.B(C) d"));
  assert.ok(!globMatches("a.b", "axb"), "regex characters are literal");
  // The owner's own rules (2026-10-17) are all in the built-in list.
  for (const rule of ["supabase *", "psql *", "*drop table*", "*truncate table*", "rm -rf *", "*Remove-Item* -Recurse*-Force*", "git push --force*", "gh pr merge*", "git merge*"]) {
    assert.ok(BUILT_IN_RISKY.includes(rule), rule);
  }
});

test("the card shows the keys on the buttons, and says when only a click allows", async () => {
  State.mode = "expanded";
  State.view = "approval";
  State.tasks = [];
  State.approvalQueue = [];
  State.pendingApproval = req("Bash · npm test", { requestId: "k1" });
  const card = buildApproval(new Proxy({}, { get: () => () => {} }));
  const n = sent("approval_ask_rules").length;
  card.sync();
  assert.equal(sent("approval_ask_rules").length, n + 1, "the live rules are read for each new request");
  card.sync();
  assert.equal(sent("approval_ask_rules").length, n + 1, "once per request");
  const keyText = (role) => card.el.find("span").find((s) => s.getAttribute("data-role") === role);
  assert.equal(keyText("deny-key").textContent, "Ctrl+Alt+Shift+N");
  assert.equal(keyText("allow-key").textContent, "Ctrl+Alt+Shift+Y");
  const risk = card.el.find("div").find((d) => d.getAttribute("data-role") === "allow-risk");
  assert.equal(risk.style.display, "none");
  // A risky one: no key on Allow, and a line saying why.
  State.pendingApproval = req("Bash · git merge origin/main", { requestId: "k2" });
  card.sync();
  assert.equal(keyText("allow-key").textContent, "");
  assert.equal(keyText("allow-key").style.display, "none");
  assert.equal(keyText("deny-key").textContent, "Ctrl+Alt+Shift+N", "Deny keeps its key");
  assert.equal(risk.style.display, "");
  assert.match(risk.textContent, /Click Allow to allow this one: it matches your Claude Code “ask” rules, so the Allow key won't\./);
  // A rebound key shows as bound.
  State.settings = { ...State.settings, shortcuts: { allowApproval: { keys: "Ctrl+Alt+Shift+K", enabled: true } } };
  State.pendingApproval = req("Bash · ls", { requestId: "k3" });
  card.sync();
  assert.equal(keyText("allow-key").textContent, "Ctrl+Alt+Shift+K");
  State.settings = { ...State.settings, shortcuts: {} };
  State.pendingApproval = null;
  State.mode = "hidden";
  cardOffScreen();
});

test("the island: the key goes through allowCheck, logs why it refused, and the clock stops off screen", () => {
  const island = read("src/island/island.ts");
  const fn = island.slice(island.indexOf("  allowApproval() {"), island.indexOf("  takeKeyboard() {"));
  assert.ok(fn.includes('open: State.mode === "expanded" && State.view === "approval"'));
  assert.ok(fn.includes("onScreen(req.requestId, performance.now())") && fn.includes("live: liveRules()"));
  assert.ok(fn.includes('if (!req || check !== "ok")') && fn.includes('this.emote("annoyed")'));
  assert.ok(fn.indexOf('Bridge.approvalDecision(req.requestId, "allow")') > fn.indexOf('check !== "ok"'), "allowed only after the check");
  assert.ok(island.includes('if (!(expanded && State.view === "approval")) cardOffScreen();'));
  // Rust: the rules come from every config folder's settings.json, read only.
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(lib.includes("fn approval_ask_rules() -> Vec<String> {\n    hooks::ask_patterns()\n}"));
  assert.ok(/\n\s+approval_ask_rules,\n/.test(lib), "registered");
});
