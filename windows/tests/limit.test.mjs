// Usage and spend limits (src/core/limit.ts): the reset time read from
// Claude Code's text, and the card that shows it.

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { limitKind, isLimitNotice, parseLimit, limitHasReset } = await import("../src/core/limit.ts");
const { State } = await import("../src/core/state.ts");
const { buildError, limitTitle } = await import("../src/views/views.ts");

const LA = "America/Los_Angeles";
// 2026-10-11 10:00 local time, whatever this machine's zone.
const MORNING = new Date(2026, 9, 11, 10, 0).getTime();

test("only rate_limit and billing_error are limits", () => {
  assert.equal(limitKind("rate_limit"), "usage");
  assert.equal(limitKind("billing_error"), "spend");
  for (const e of ["server_error", "overloaded", "authentication_failed", undefined, 3]) assert.equal(limitKind(e), null);
});

test("limit notices are told apart from other notifications", () => {
  assert.ok(isLimitNotice("Usage Rate Limit reached"));
  assert.ok(isLimitNotice("You've hit your limit · resets 2pm"));
  assert.ok(!isLimitNotice("Claude needs your permission to use Bash"));
});

test("the reset time is read and placed today, in this computer's zone", () => {
  const limit = parseLimit("usage", [undefined, `You've hit your limit · resets 2pm (${LA})`], MORNING, LA);
  assert.equal(limit.resets, "2 PM");
  assert.equal(limit.resetsAt, new Date(2026, 9, 11, 14, 0).getTime());
  assert.equal(limitHasReset(limit, MORNING), false);
  assert.equal(limitHasReset(limit, new Date(2026, 9, 11, 14, 0).getTime()), true);
});

test("a time already past today resets tomorrow; minutes are kept", () => {
  const limit = parseLimit("usage", ["resets 9:30am"], MORNING, LA);
  assert.equal(limit.resets, "9:30 AM");
  assert.equal(limit.resetsAt, new Date(2026, 9, 12, 9, 30).getTime());
});

test("another zone is shown as written and never placed (no Retry from a guess)", () => {
  const limit = parseLimit("usage", ["You've hit your limit · resets 5pm (Europe/Paris)"], MORNING, LA);
  assert.equal(limit.resets, "5 PM (Europe/Paris)");
  assert.equal(limit.resetsAt, null);
  assert.equal(limitHasReset(limit, MORNING + 48 * 3600e3), false);
});

test("a date is shown as written; no reset text gives no time", () => {
  assert.deepEqual(parseLimit("spend", ["Monthly limit · resets Nov 1, 12am"], MORNING, LA).resets, "Nov 1, 12 AM");
  assert.deepEqual(parseLimit("spend", ["You've hit your monthly spend limit."], MORNING, LA), { kind: "spend", resets: null, resetsAt: null });
});

test("the card's title says the limit and when it resets", () => {
  assert.equal(limitTitle({ kind: "usage", resets: "2 PM", resetsAt: null }, MORNING), "Usage limit, resets 2 PM");
  assert.equal(limitTitle({ kind: "spend", resets: null, resetsAt: null }, MORNING), "Spend limit");
  assert.equal(limitTitle({ kind: "usage", resets: "2 PM", resetsAt: MORNING - 1 }, MORNING), "The limit has reset. Continue in the terminal.");
});

test("a limit card has no Retry: Open terminal and OK only", () => {
  State.tasks = [{
    id: "integration_claude", name: "repo", color: "#fff", state: "ratelimit", stepIndex: 0, steps: [], source: "claudeCode",
    isIntegration: true, sessionId: "s", limit: { kind: "usage", resets: "2 PM", resetsAt: null },
  }];
  State.setFocus("integration_claude");
  const view = buildError(new Proxy({}, { get: () => () => {} }));
  view.sync();
  const shown = view.el.find("button").filter((b) => b.style.display !== "none").map((b) => b.children[0].textContent);
  assert.deepEqual(shown, ["Open terminal", "OK"]);
  assert.equal(view.el.find(".title")[0].textContent, "Usage limit, resets 2 PM");
  // The same card for an error keeps Retry.
  State.tasks[0].state = "error";
  view.sync();
  const again = view.el.find("button").filter((b) => b.style.display !== "none").map((b) => b.children[0].textContent);
  assert.deepEqual(again, ["Retry", "Open terminal"]);
});
