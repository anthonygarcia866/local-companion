// "Allow screenshots for 10 minutes" (owner 2026-11-14): Settings → Privacy
// asks Rust (screen_capture.rs) to open the window and shows the time left
// with "Hide again now"; the island shows the same countdown; nothing about
// it is ever saved, so a restart comes back hidden.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { sent, replies, emit } = await import("./tauri.mjs");
const { countdown } = await import("../src/core/capture-allow.ts");
const { captureAllowBox } = await import("../src/settings/privacy.ts");

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("the countdown reads m:ss and ends on time", () => {
  assert.equal(countdown(600_000, 0), "10:00");
  assert.equal(countdown(600_000, 1), "10:00", "rounds up: never 9:59 at the start");
  assert.equal(countdown(61_000, 0), "1:01");
  assert.equal(countdown(9_000, 0), "0:09");
  assert.equal(countdown(1_000, 1_000), "");
  assert.equal(countdown(1_000, 50_000), "", "past the end (the PC slept)");
  assert.equal(countdown(0, 0), "", "never started");
});

test("Privacy: Allow → time left + Hide again now → back to the button", async () => {
  replies.set("capture_status", 0);
  const box = captureAllowBox(() => true, 60_000);
  await tick();
  assert.ok(role(box.el, "button", "capture-allow-button"));
  assert.equal(role(box.el, "button", "capture-hide-again"), undefined);
  replies.set("capture_allow", Date.now() + 600_000);
  role(box.el, "button", "capture-allow-button").fire("click");
  await tick();
  assert.equal(sent("capture_allow").length, 1);
  const left = box.el.find("span").find((s) => s.getAttribute("data-role") === "capture-allow-left");
  assert.match(left.textContent, /Screenshots allowed: (10:00|9:5\d) left/);
  replies.set("capture_hide_again", 0);
  role(box.el, "button", "capture-hide-again").fire("click");
  await tick();
  assert.equal(sent("capture_hide_again").length, 1);
  assert.ok(role(box.el, "button", "capture-allow-button"));
});

test("Privacy: follows Rust's event (the tray, or time's up) and shows nothing while hiding is off", async () => {
  replies.set("capture_status", 0);
  let hiding = true;
  const box = captureAllowBox(() => hiding, 60_000);
  await tick();
  emit("capture-allowed", Date.now() + 120_000);
  assert.ok(role(box.el, "button", "capture-hide-again"), "the tray opened it");
  emit("capture-allowed", 0);
  assert.ok(role(box.el, "button", "capture-allow-button"), "time's up");
  hiding = false;
  box.redraw();
  assert.equal(box.el.find("button").length, 0, "the switch is off: nothing to allow");
});

test("Rust: memory only, ten minutes, hidden again by itself; tray and island wired", () => {
  const rs = read("src-tauri/src/screen_capture.rs");
  assert.ok(rs.includes("pub const ALLOW_FOR: Duration = Duration::from_secs(10 * 60);"));
  assert.ok(rs.includes('end(&app, "time\'s up");'));
  const tray = read("src-tauri/src/tray.rs");
  assert.ok(tray.includes('("capture", CAPTURE)'));
  assert.ok(tray.includes("crate::screen_capture::allow_for_a_while(app);"));
  const main = read("src/main.ts");
  assert.ok(main.includes('onEvent<number>("capture-allowed"'));
  assert.ok(read("src/island/island.ts").includes("countdown(State.captureAllowedUntil, Date.now())"));
});
