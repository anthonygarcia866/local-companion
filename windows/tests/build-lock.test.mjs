// scripts/build-lock.mjs: one glim-update / release build at a time across
// both Claude accounts (owner 2026-11-15).
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { lockDir, holder, tryTake, take, release, run } from "../scripts/build-lock.mjs";

function scratch() {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), "glim-lock-"));
  return { base, env: { ...process.env, LOCALAPPDATA: base, GLIM_BUILD_LOCK: "" }, dir: lockDir({ LOCALAPPDATA: base }) };
}

const quiet = { log() {} };

test("a second build waits for the first, shows as waiting, then runs", async () => {
  const { base, env, dir } = scratch();
  const order = [];
  const node = JSON.stringify(process.execPath);
  const step = (name, ms) => [node, "-e", JSON.stringify(`setTimeout(() => {}, ${ms})`)];
  const first = run("release", step("first", 600), { env, pollMs: 50, out: quiet }).then((c) => order.push(["first", c, Date.now()]));
  await new Promise((r) => setTimeout(r, 150));
  assert.equal(holder(dir).what, "release");
  // A second one (as from another account's session) waits...
  const fakePid = 999_999;
  const second = take(dir, "glim-update", fakePid, { pollMs: 50, isAlive: (p) => p === process.pid || p === fakePid });
  await new Promise((r) => setTimeout(r, 150));
  assert.ok(fs.existsSync(path.join(dir, `waiting-${fakePid}.json`)), "the island can see it waiting");
  assert.equal(JSON.parse(fs.readFileSync(path.join(dir, `waiting-${fakePid}.json`), "utf8")).what, "glim-update");
  // ...and gets the lock only after the first finished.
  const owner = await second;
  order.push(["second", 0, Date.now()]);
  await first;
  assert.deepEqual(order.map((o) => o[0]), ["first", "second"]);
  assert.equal(owner.what, "glim-update");
  assert.ok(!fs.existsSync(path.join(dir, `waiting-${fakePid}.json`)), "no longer waiting");
  release(dir, fakePid);
  assert.equal(holder(dir), null);
  fs.rmSync(base, { recursive: true, force: true });
});

test("inside the holder (its token), a nested build doesn't wait for itself", async () => {
  const { base, env, dir } = scratch();
  const owner = tryTake(dir, "glim-update", process.pid);
  const code = await run("release", [JSON.stringify(process.execPath), "-e", "0"], { env: { ...env, GLIM_BUILD_LOCK: owner.token }, pollMs: 50, out: quiet });
  assert.equal(code, 0);
  assert.equal(holder(dir).what, "glim-update", "still the updater's");
  release(dir, process.pid);
  fs.rmSync(base, { recursive: true, force: true });
});

test("an owner whose process is gone is cleared; the exit code comes through", async () => {
  const { base, env, dir } = scratch();
  tryTake(dir, "release", 999_998, () => true);
  assert.equal(holder(dir, () => false), null, "stale: cleared");
  const code = await run("release", [JSON.stringify(process.execPath), "-e", "process.exit(3)"], { env, pollMs: 50, out: quiet });
  assert.equal(code, 3);
  assert.equal(holder(dir), null, "released after the command");
  fs.rmSync(base, { recursive: true, force: true });
});

test("npm run pack goes through the lock", () => {
  const pkg = JSON.parse(fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.equal(pkg.scripts.pack, "node scripts/build-lock.mjs run release -- npm run pack:build");
  assert.match(pkg.scripts["pack:build"], /tauri build && node scripts\/pack\.mjs/);
});
