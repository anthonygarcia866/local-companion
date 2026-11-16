#!/usr/bin/env node
// One Glim build at a time on this PC, across every Claude account (owner
// 2026-11-15): a glim-update or a release build (`npm run pack`) started
// while another runs waits, shown on the island ("… waits for … to
// finish"), and starts when the first one finishes. Two Claude Code
// sessions side by side are fine; only builds queue.
//
//   node scripts/build-lock.mjs run <what> -- <command …>
//        waits for the lock, runs the command holding it, releases it;
//        exits with the command's code.
//   node scripts/build-lock.mjs take <what> --pid <pid>
//        waits for the lock and leaves it held by process <pid> (an updater
//        script holds it for its whole run); prints the token to put in
//        GLIM_BUILD_LOCK so that what it starts doesn't wait for itself.
//   node scripts/build-lock.mjs release --pid <pid>
//
// The lock is the folder %LOCALAPPDATA%\Glim\build-lock\held (mkdir is
// atomic) with owner.json {what, pid, since, token}; a waiter leaves
// waiting-<pid>.json {what, pid, since} next to it, which Glim reads for
// the island (src-tauri/src/build_lock.rs). An owner whose process is gone
// is stale and cleared. Inside a holder (GLIM_BUILD_LOCK = its token) a
// nested `run` doesn't wait. Nothing here touches the network.

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const WHATS = ["glim-update", "release"];

export function lockDir(env = process.env) {
  const base = env.LOCALAPPDATA || path.join(env.USERPROFILE || env.HOME || ".", "AppData", "Local");
  return path.join(base, "Glim", "build-lock");
}

export function alive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === "EPERM";
  }
}

function readJson(p) {
  try {
    return JSON.parse(fs.readFileSync(p, "utf8"));
  } catch {
    return null;
  }
}

/** The current holder ({what, pid, since, token}) or null; a stale one is cleared. */
export function holder(dir, isAlive = alive) {
  const held = path.join(dir, "held");
  if (!fs.existsSync(held)) return null;
  const owner = readJson(path.join(held, "owner.json"));
  let age = Infinity;
  try {
    age = Date.now() - fs.statSync(held).mtimeMs;
  } catch {
    return null;
  }
  // Just made and not written yet: someone is taking it right now.
  if (!owner && age < 30_000) return { what: "?", pid: 0, since: 0, token: "" };
  if (owner && isAlive(owner.pid)) return owner;
  fs.rmSync(held, { recursive: true, force: true });
  return null;
}

/** One try: the lock for `pid`, or null when someone holds it. */
export function tryTake(dir, what, pid, isAlive = alive) {
  fs.mkdirSync(dir, { recursive: true });
  if (holder(dir, isAlive)) return null;
  try {
    fs.mkdirSync(path.join(dir, "held"));
  } catch {
    return null;
  }
  const owner = { what, pid, since: Date.now(), token: randomBytes(16).toString("hex") };
  fs.writeFileSync(path.join(dir, "held", "owner.json"), JSON.stringify(owner));
  return owner;
}

export function release(dir, pid) {
  const held = path.join(dir, "held");
  const owner = readJson(path.join(held, "owner.json"));
  if (owner && owner.pid === pid) fs.rmSync(held, { recursive: true, force: true });
}

/** Waits (announcing itself in waiting-<pid>.json) until the lock is ours. */
export async function take(dir, what, pid, { pollMs = 2000, isAlive = alive } = {}) {
  let owner = tryTake(dir, what, pid, isAlive);
  if (owner) return owner;
  const note = path.join(dir, `waiting-${pid}.json`);
  fs.writeFileSync(note, JSON.stringify({ what, pid, since: Date.now() }));
  try {
    for (;;) {
      await new Promise((r) => setTimeout(r, pollMs));
      owner = tryTake(dir, what, pid, isAlive);
      if (owner) return owner;
    }
  } finally {
    fs.rmSync(note, { force: true });
  }
}

/** Runs `cmd` holding the lock (no wait inside a holder). */
export async function run(what, cmd, { env = process.env, pollMs, out = console } = {}) {
  const dir = lockDir(env);
  const mine = holder(dir);
  const nested = mine && env.GLIM_BUILD_LOCK && mine.token === env.GLIM_BUILD_LOCK;
  let owner = null;
  if (!nested) {
    const before = holder(dir);
    if (before) out.log(`build-lock: ${what} waits for ${before.what} to finish`);
    owner = await take(dir, what, process.pid, { pollMs });
  }
  try {
    return await new Promise((resolve) => {
      const child = spawn(cmd.join(" "), { shell: true, stdio: "inherit", env: { ...env, GLIM_BUILD_LOCK: owner ? owner.token : env.GLIM_BUILD_LOCK } });
      child.on("exit", (code) => resolve(code ?? 1));
      child.on("error", () => resolve(1));
    });
  } finally {
    if (owner) release(dir, process.pid);
  }
}

async function main(argv) {
  const [verb, what, ...rest] = argv;
  const flag = (name) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const dir = lockDir();
  if (verb === "run" && WHATS.includes(what) && rest[0] === "--" && rest.length > 1) {
    return run(what, rest.slice(1));
  }
  if (verb === "take" && WHATS.includes(what) && flag("--pid")) {
    const pid = Number(flag("--pid"));
    if (holder(dir)) console.error(`build-lock: ${what} waits for ${holder(dir).what} to finish`);
    const owner = await take(dir, what, pid);
    console.log(owner.token);
    return 0;
  }
  if (verb === "release" && flag("--pid")) {
    release(dir, Number(flag("--pid")));
    return 0;
  }
  console.error("usage: build-lock.mjs run <glim-update|release> -- <command> | take <what> --pid <pid> | release --pid <pid>");
  return 4;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then((code) => { process.exitCode = code; });
}
