// Automated testing, stage 3 (owner, 2026-10-28): the REAL release glim.exe on
// CI's Windows runner. Never run on the owner's desktop: it presses real keys
// (SendInput) and captures the real screen.
//
//   node e2e/app/run.mjs            (from windows/, after `npx tauri build --no-bundle`)
//
// What it does, in this order:
//   1. Test-safe folders: APPDATA, LOCALAPPDATA and USERPROFILE point at a
//      scratch folder, with a settings.json that turns autostart and the
//      calendar off. No real Claude config, no real Glim data.
//   2. `glim.exe --selftest` (stage 1) must pass.
//   3. Glim (built with --features e2e) starts with GLIM_E2E_CDP_PORT, which
//      adds a DevTools port to its webviews (lib.rs browser_args; only that
//      build can), so the island's page is read over CDP (connectOverCDP).
//   4. The Allow key end to end: permission requests go in through the real
//      relay (target/release/glim-hook.exe PermissionRequest, as Claude Code
//      runs it) and the real Ctrl+Alt+Shift+Y goes in through SendInput.
//      The answer is the relay's own stdout (what Claude Code would get).
//      short: 1 press; too soon then 1; long: expand then allow; destructive:
//      expand + 2.
//   5. Capture exclusion: with the setting on, a desktop capture (BitBlt, what
//      a screenshot tool or a screen share sees) of the island's rectangle has
//      none of Glim's amber; with it off (Glim restarted), it has.
//   6. All along: no window of Glim's that wasn't meant to show ever becomes
//      visible (a watcher samples every window of the process every 150 ms).
//
// Screenshots go to e2e-results/app/ (a CI artifact). The summary line goes to
// the job summary. A part the runner can't do (no input desktop: SendInput
// sends 0 events) is reported as SKIPPED with the reason, and the rest runs.

import { spawn } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync, readFileSync, existsSync, appendFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { chromium } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));
const windowsDir = join(here, "..", "..");
const exe = join(windowsDir, "target", "release", "glim.exe");
const hookExe = join(windowsDir, "target", "release", "glim-hook.exe");
const out = join(windowsDir, "e2e-results", "app");
const root = join(process.env.RUNNER_TEMP ?? process.env.TEMP, "glim-app-e2e");
const PORT = 9339;
const ALLOW_DELAY = 1_500 + 300; // core/allow.ts, plus room
const LONG = `npm run build -- --filter ${Array.from({ length: 12 }, (_, i) => `packages/module-${i}`).join(" ")} --verbose`;
const LONG_DESTRUCTIVE = `rm -rf build && ${LONG}`;

const results = [];
const started = Date.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function result(name, status, detail = "") {
  results.push({ name, status, detail });
  console.log(`${status.padEnd(7)} ${name}${detail ? `: ${detail}` : ""}`);
}

// ── The scratch profile ──────────────────────────────────────────────────────

rmSync(root, { recursive: true, force: true });
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
const folders = { APPDATA: join(root, "Roaming"), LOCALAPPDATA: join(root, "Local"), USERPROFILE: join(root, "home") };
for (const f of Object.values(folders)) mkdirSync(f, { recursive: true });
const settingsFile = join(folders.APPDATA, "Glim", "settings.json");
function writeSettings(hideFromCapture) {
  mkdirSync(dirname(settingsFile), { recursive: true });
  writeFileSync(settingsFile, JSON.stringify({ autostart: false, calendarOn: false, hideFromCapture }, null, 1));
}
const env = { ...process.env, ...folders, HOME: folders.USERPROFILE, GLIM_E2E_CDP_PORT: String(PORT) };
delete env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
delete env.GLIM_DEV;
delete env.GLIM_CAPTURABLE;

// ── The Win32 helper (win.ps1), one process ──────────────────────────────────

function helper() {
  const p = spawn("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", join(here, "win.ps1")], { stdio: ["pipe", "pipe", "inherit"] });
  const lines = createInterface({ input: p.stdout });
  const waiting = [];
  lines.on("line", (l) => waiting.shift()?.(JSON.parse(l)));
  const ready = new Promise((r) => waiting.push(r));
  return {
    ready,
    ask(cmd) {
      return new Promise((r) => {
        waiting.push(r);
        p.stdin.write(JSON.stringify(cmd) + "\n");
      });
    },
    quit() { p.stdin.end(JSON.stringify({ cmd: "quit" }) + "\n"); },
  };
}

// ── Glim ─────────────────────────────────────────────────────────────────────

function runToExit(args, ms) {
  return new Promise((resolve) => {
    const p = spawn(exe, args, { env, stdio: "ignore" });
    const t = setTimeout(() => { p.kill(); resolve(-1); }, ms);
    p.on("exit", (code) => { clearTimeout(t); resolve(code); });
  });
}

function taskkill(name) {
  return new Promise((r) => spawn("taskkill", ["/IM", name, "/F"], { stdio: "ignore" }).on("exit", r));
}

async function startGlim() {
  const p = spawn(exe, [], { env, stdio: "ignore", detached: false });
  const until = Date.now() + 40_000;
  let up = false;
  while (!up && Date.now() < until) {
    try {
      up = (await fetch(`http://127.0.0.1:${PORT}/json/version`)).ok;
    } catch { /* not up yet */ }
    if (!up) await sleep(300);
  }
  if (!up) {
    const log = join(folders.LOCALAPPDATA, "Glim", "glim.log");
    const tail = existsSync(log) ? readFileSync(log, "utf8").split("\n").slice(-15).join("\n") : "(no log)";
    throw new Error(`no DevTools port ${PORT} within 40 s (glim.exe ${p.exitCode === null ? "running" : `exited ${p.exitCode}`}). Log tail:\n${tail}`);
  }
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
  let island = null;
  for (let i = 0; i < 60 && !island; i++) {
    const pages = browser.contexts().flatMap((c) => c.pages());
    island = pages.find((pg) => /\/(index\.html)?$/.test(new URL(pg.url()).pathname)) ?? null;
    if (!island) await sleep(250);
  }
  if (!island) throw new Error(`no island page over CDP: ${browser.contexts().flatMap((c) => c.pages()).map((pg) => pg.url()).join(", ")}`);
  console.log(`CDP: ${browser.contexts().flatMap((c) => c.pages()).map((pg) => pg.url()).join(", ")}`);
  return { p, browser, island };
}

async function stopGlim(g) {
  await g.browser.close().catch(() => {});
  await taskkill("glim.exe");
  await sleep(1_500);
}

/** Every window of the process, sampled until stopped: a window that is ever
 *  visible but wasn't meant to be (not one of `allowed` at the end) fails. */
function watchWindows(win, pid) {
  const seen = new Map(); // hwnd -> { title, everVisible, visibleAtEnd }
  let on = true;
  const done = (async () => {
    while (on) {
      const list = await win.ask({ cmd: "windows", pid });
      if (Array.isArray(list)) {
        for (const w of list) {
          const s = seen.get(w.hwnd) ?? { title: w.title, cls: w.cls, everVisible: false, samples: 0, visibleSamples: 0 };
          s.title = w.title || s.title;
          s.samples++;
          if (w.visible) { s.everVisible = true; s.visibleSamples++; }
          s.last = w;
          seen.set(w.hwnd, s);
        }
      }
      await sleep(150);
    }
  })();
  return {
    async stop() { on = false; await done; return seen; },
  };
}

function visibleWindows(seen) {
  return [...seen.values()].filter((s) => s.last?.visible);
}

// ── Approvals through the real relay ─────────────────────────────────────────

let reqN = 0;
/** One PermissionRequest through glim-hook, as Claude Code sends it. Resolves
 *  with the relay's stdout once it exits. */
function relay(command) {
  const payload = {
    hook_event_name: "PermissionRequest", session_id: "stage3", transcript_path: "", cwd: "C:\\work\\demo",
    tool_name: "Bash", tool_input: { command, description: `stage 3 #${++reqN}` },
  };
  const p = spawn(hookExe, ["PermissionRequest"], { env, stdio: ["pipe", "pipe", "ignore"] });
  let stdout = "";
  p.stdout.on("data", (d) => { stdout += d; });
  const exited = new Promise((r) => p.on("exit", () => r(stdout.trim())));
  p.stdin.end(JSON.stringify(payload));
  let settled = false;
  exited.then(() => { settled = true; });
  return { exited, settled: () => settled };
}

const decision = (stdout) => {
  try { return JSON.parse(stdout)?.hookSpecificOutput?.decision?.behavior ?? `no decision (${stdout || "empty"})`; } catch { return `not JSON (${stdout})`; }
};

const card = (page) => page.locator(".view.on").filter({ has: page.locator(".code") });

async function cardUp(page, command) {
  const code = card(page).locator(".code");
  await code.filter({ hasText: command.slice(0, 40) }).waitFor({ state: "visible", timeout: 10_000 });
  return code;
}

async function press(win) {
  const r = await win.ask({ cmd: "keys", chord: "ctrl+alt+shift+y" });
  if (!r.sent) throw new Error(`SendInput sent ${r.sent} events (no input desktop?)`);
  await sleep(250);
}

/** Runs one scenario; `steps` presses with the waits before each. */
async function approval(name, g, win, command, steps) {
  const page = g.island;
  const req = relay(command);
  try {
    const code = await cardUp(page, command);
    let presses = 0;
    for (const step of steps) {
      await sleep(step.wait);
      await press(win);
      presses++;
      if (step.check) await step.check(page, code);
      if (!step.last && req.settled()) throw new Error(`answered after press ${presses}, too early`);
    }
    const got = await Promise.race([req.exited, sleep(8_000).then(() => "timeout")]);
    await page.screenshot({ path: join(out, `approval-${name}.png`) }).catch(() => {});
    const d = got === "timeout" ? "no answer within 8 s" : decision(got);
    result(`allow key: ${name}`, d === "allow" ? "PASS" : "FAIL", `${presses} press(es), relay said ${d}`);
  } catch (e) {
    await page.screenshot({ path: join(out, `approval-${name}-failed.png`) }).catch(() => {});
    result(`allow key: ${name}`, String(e.message).includes("SendInput") ? "SKIPPED" : "FAIL", e.message);
    // Free the card for the next one: the Deny key, then wait for the relay.
    await win.ask({ cmd: "keys", chord: "ctrl+alt+shift+n" });
    await Promise.race([req.exited, sleep(5_000)]);
  }
  await sleep(800);
}

const expanded = async (_page, code) => {
  if (!/\bexpanded\b/.test((await code.getAttribute("class")) ?? "")) throw new Error("first press didn't expand the card");
};
const refusedTooSoon = async (page) => {
  const why = card(page).locator('[data-role="allow-why"]');
  await why.filter({ hasText: "Too soon" }).waitFor({ timeout: 2_000 });
};

// ── Capture exclusion ────────────────────────────────────────────────────────

/** While a card is up (the island at its largest, amber), a desktop capture of
 *  the island's window. */
async function captureIsland(name, g, win, pid) {
  const req = relay("echo capture-check");
  try {
    await cardUp(g.island, "echo capture-check");
    await sleep(900);
    const list = await win.ask({ cmd: "windows", pid });
    const island = list.filter((w) => w.visible && w.w > 100 && w.h > 40).sort((a, b) => b.w * b.h - a.w * a.h)[0];
    if (!island) throw new Error("no visible island window");
    const shot = await win.ask({ cmd: "shot", x: island.x, y: island.y, w: island.w, h: island.h, out: join(out, `capture-${name}.png`) });
    return { island, shot };
  } finally {
    await win.ask({ cmd: "keys", chord: "ctrl+alt+shift+n" });
    await Promise.race([req.exited, sleep(5_000)]);
    await sleep(600);
  }
}

// ── Run ──────────────────────────────────────────────────────────────────────

const win = helper();
await win.ready;
let failed = false;
try {
  writeSettings(true);

  // 2. Self-test first.
  const code = await runToExit(["--selftest"], 90_000);
  const report = existsSync(join(folders.LOCALAPPDATA, "Glim", "selftest.txt")) ? readFileSync(join(folders.LOCALAPPDATA, "Glim", "selftest.txt"), "utf8").trim() : "(no report)";
  console.log(report);
  result("selftest", code === 0 ? "PASS" : "FAIL", `exit ${code}`);
  await taskkill("glim.exe");
  await sleep(1_000);

  // 3-4. Glim, the Allow key.
  let g = await startGlim();
  const pid = g.p.pid;
  const watch = watchWindows(win, pid);
  await sleep(3_000);
  await g.island.screenshot({ path: join(out, "island-idle.png") });

  const probe = await win.ask({ cmd: "keys", chord: "shift" });
  if (!probe.sent) {
    result("allow key", "SKIPPED", "SendInput sent 0 events: the runner has no input desktop");
  } else {
    await approval("short", g, win, "npm test", [{ wait: ALLOW_DELAY, last: true }]);
    await approval("too-soon-then-1", g, win, "npm run lint", [
      { wait: 400, check: refusedTooSoon },
      { wait: ALLOW_DELAY, last: true },
    ]);
    await approval("long", g, win, LONG, [
      { wait: ALLOW_DELAY, check: expanded },
      { wait: ALLOW_DELAY, last: true },
    ]);
    await approval("destructive", g, win, LONG_DESTRUCTIVE, [
      { wait: ALLOW_DELAY, check: expanded },
      { wait: ALLOW_DELAY },
      { wait: 300, last: true },
    ]);
  }

  // 5. Capture exclusion, on.
  const on = await captureIsland("on", g, win, pid);
  result("capture exclusion on: affinity", on.island.affinity === 0x11 ? "PASS" : "FAIL", `island affinity 0x${on.island.affinity.toString(16)}`);
  console.log(`capture on: ${JSON.stringify(on.shot)}`);

  // 6. Windows that showed, with the setting on.
  let seen = await watch.stop();
  const allowed = new Set(visibleWindows(seen).map((s) => s.last.hwnd));
  const flashed = [...seen.values()].filter((s) => s.everVisible && !allowed.has(s.last.hwnd));
  result("no hidden window became visible (on)", flashed.length === 0 ? "PASS" : "FAIL",
    flashed.length ? flashed.map((s) => `${s.title || s.cls} (${s.visibleSamples}/${s.samples})`).join(", ") : `${seen.size} windows watched, ${allowed.size} shown`);
  await stopGlim(g);

  // 5. Capture exclusion, off: the same capture shows the island.
  writeSettings(false);
  g = await startGlim();
  const watchOff = watchWindows(win, g.p.pid);
  await sleep(3_000);
  const off = await captureIsland("off", g, win, g.p.pid);
  console.log(`capture off: ${JSON.stringify(off.shot)}`);
  result("capture exclusion off: affinity", off.island.affinity === 0 ? "PASS" : "FAIL", `island affinity 0x${off.island.affinity.toString(16)}`);
  const visibleOff = off.shot.warm > 50;
  result("capture exclusion: on hides the island, off shows it",
    visibleOff && on.shot.warm < off.shot.warm / 20 ? "PASS" : "FAIL",
    `amber pixels sampled: on ${on.shot.warm}, off ${off.shot.warm}`);
  seen = await watchOff.stop();
  const allowedOff = new Set(visibleWindows(seen).map((s) => s.last.hwnd));
  const flashedOff = [...seen.values()].filter((s) => s.everVisible && !allowedOff.has(s.last.hwnd));
  result("no hidden window became visible (off)", flashedOff.length === 0 ? "PASS" : "FAIL",
    flashedOff.length ? flashedOff.map((s) => `${s.title || s.cls}`).join(", ") : `${seen.size} windows watched`);
  await stopGlim(g);
} catch (e) {
  result("stage 3", "FAIL", e.stack ?? String(e));
} finally {
  win.quit();
  await taskkill("glim.exe");
  failed = results.some((r) => r.status === "FAIL");
  const secs = Math.round((Date.now() - started) / 1000);
  const counts = ["PASS", "FAIL", "SKIPPED"].map((s) => `${results.filter((r) => r.status === s).length} ${s.toLowerCase()}`).join(", ");
  const line = `Stage 3, the real app: ${counts}, ${secs}s`;
  console.log(line);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### ${line}\n${results.map((r) => `- ${r.status} ${r.name}${r.detail ? `: ${r.detail.split("\n")[0]}` : ""}`).join("\n")}\n`);
  }
  process.exit(failed ? 1 : 0);
}
