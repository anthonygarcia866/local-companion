// The last update's result (core/update-notice.ts; update_result.rs): one
// line for the island, the timings for Settings, and where it may show.

import { test } from "node:test";
import assert from "node:assert/strict";

const { updateLine, updateTimings, updateNoticeShows } = await import("../src/core/update-notice.ts");

const base = { runId: "r", commit: "1a2b3c4d", ok: true, selftest: "6/6", failedStep: "", error: "", finishedAt: "", totalMs: 65_000, steps: [] };

test("ok: the commit and the self-test", () => {
  assert.equal(updateLine(base), "Updated to 1a2b3c4d: self-test 6/6");
});

test("failed: the step and why", () => {
  assert.equal(updateLine({ ...base, ok: false, failedStep: "preflight", error: "CI still running on 1a2b3c4d" }), "Update to 1a2b3c4d failed at preflight: CI still running on 1a2b3c4d");
  assert.equal(updateLine({ ...base, ok: false, commit: "" }), "Update to ? failed at ?: ?");
});

test("timings: each step, a failed one marked, then the total", () => {
  const u = { ...base, steps: [{ name: "test", ok: true, ms: 34_400, detail: "" }, { name: "pack", ok: false, ms: 1_000, detail: "" }] };
  assert.equal(updateTimings(u), "test 34 s · pack 1 s ✗ · total 1 min 5 s");
});

test("the island shows it only on the pill with no approval waiting", () => {
  assert.equal(updateNoticeShows({ presence: "pill", paused: false, approval: false }), true);
  assert.equal(updateNoticeShows({ presence: "pill", paused: true, approval: false }), true);
  assert.equal(updateNoticeShows({ presence: "pill", paused: false, approval: true }), false);
  assert.equal(updateNoticeShows({ presence: "hidden", paused: false, approval: false }), false);
});
