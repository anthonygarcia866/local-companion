// Settings → Privacy → Activity watch (owner 2026-11-24): a switch that asks
// Rust (activity_watch.rs) to log, on this PC only, new terminal / Word /
// Outlook processes and the window in front, for 7 days, then off by itself.
// The page never saves it in settings.json.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { sent, replies } = await import("./tauri.mjs");
const { activityWatchBox } = await import("../src/settings/privacy.ts");

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("Activity watch: off → on for 7 days (until shown, log button) → off", async () => {
  replies.set("activity_watch_status", 0);
  const box = activityWatchBox();
  await tick();
  const sw = () => role(box, "button", "activity-watch-switch");
  assert.equal(sw().getAttribute("aria-pressed"), "false");
  assert.equal(role(box, "button", "activity-watch-show"), undefined, "no log button while off");
  const until = Math.floor(Date.now() / 1000) + 7 * 86400;
  replies.set("activity_watch_set", until);
  sw().fire("click");
  await tick();
  assert.deepEqual(sent("activity_watch_set").at(-1), { on: true });
  assert.equal(sw().getAttribute("aria-pressed"), "true");
  assert.match(role(box, "span", "activity-watch-until").textContent, /^On until .+\.$/);
  role(box, "button", "activity-watch-show").fire("click");
  await tick();
  assert.equal(sent("activity_watch_reveal").length, 1);
  replies.set("activity_watch_set", 0);
  sw().fire("click");
  await tick();
  assert.deepEqual(sent("activity_watch_set").at(-1), { on: false });
  assert.equal(sw().getAttribute("aria-pressed"), "false");
});

test("Activity watch: a stored end in the past reads as off", async () => {
  replies.set("activity_watch_status", Math.floor(Date.now() / 1000) - 10);
  const box = activityWatchBox();
  await tick();
  assert.equal(role(box, "button", "activity-watch-switch").getAttribute("aria-pressed"), "false");
});

test("Rust: the commands are registered, the thread starts, and it's in CLAUDE.md and PRIVACY.md", () => {
  const lib = read("src-tauri/src/lib.rs");
  for (const c of ["activity_watch_status", "activity_watch_set", "activity_watch_reveal"]) {
    assert.ok(lib.includes(`            ${c},\n`), `${c} registered`);
  }
  assert.ok(lib.includes("activity_watch::start();"));
  assert.match(read("../CLAUDE.md"), /Activity watch/);
  assert.match(read("../PRIVACY.md"), /Activity watch/);
  assert.ok(!read("src/core/state.ts").includes("activityWatch"), "not a settings.json field");
});
