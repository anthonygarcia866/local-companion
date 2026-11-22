// Settings → Remote while it refreshes (owner 2026-11-21): the page reads the
// keep-alive state every 5 s, Remote tasks every 2 s and the pairing every
// 2 s. Typing into a field across several of those ticks must keep the same
// field, its focus and every character typed; what the ticks brought in shows
// once focus leaves. Every reply below changes on each call, so each tick
// really has something to redraw.

import { test, expect } from "@playwright/test";
import { noConsoleErrors, openPage, sent, shot } from "./glim.mjs";

function fakeRemote() {
  const r = window.__glim.replies;
  let ka = 0;
  let rs = 0;
  let qr = 0;
  window.__pairReady = false;
  r.keepalive_status = () => ({
    settings: { on: true, consented: true, folders: [], desktopApp: true },
    // The desktop app's "running since" moves on every read.
    entries: [{ id: "desktop", folder: "", command: "", state: "running", at: 1_763_700_000 + ++ka, lastExit: null, why: "", failedLastHour: 0 }],
  });
  r.remote_status = () => ({
    on: true, consented: true, killed: false, clients: [], relay: "https://relay.example.com", connection: "connected",
    pcPublicKey: "", pcFingerprint: "", pcEncKey: "", pcEncFingerprint: "", requireSealed: false, activity: [],
    runbooksPerHour: 5, runbooksPerHourPc: 10, approvers: [],
    // A different list on every read: a full redraw is due each time.
    remoteRunbooks: [`tick-${++rs}`],
  });
  const svg = (n) => `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" data-n="${n}"><rect width="40" height="40" fill="#888"/></svg>`;
  r.remote_pair_start = () => ({ state: "waiting", url: "https://example.com/pair", qrSvg: svg(0), expires: 0, fingerprint: "", synced: false });
  r.remote_pair_status = () => window.__pairReady
    ? { state: "ready", url: "", qrSvg: "", expires: 0, fingerprint: "AB12-CD34", synced: false }
    : { state: "waiting", url: "https://example.com/pair", qrSvg: svg(++qr), expires: 0, fingerprint: "", synced: false };
}

/** Marks the focused field, so a replaced one (same data-role, new node) is told apart. */
const markFocused = (page) => page.evaluate(() => { document.activeElement.__kept = true; });
const focusedState = (page) => page.evaluate(() => {
  const a = document.activeElement;
  return { role: a?.getAttribute("data-role") ?? null, kept: Boolean(a?.__kept), value: a?.value ?? null };
});
const calls = async (page) => ({ ka: (await sent(page, "keepalive_status")).length, rs: (await sent(page, "remote_status")).length, pair: (await sent(page, "remote_pair_status")).length });

/** Types `text` slowly (~0.4 s a key) into the focused field and checks it was never replaced or blurred. */
async function typeAcrossTicks(page, role, text) {
  const field = page.locator(`[data-role="${role}"]`);
  await field.click();
  await field.press("End");
  const before = await field.inputValue();
  await markFocused(page);
  const c0 = await calls(page);
  for (const ch of text) {
    await page.keyboard.type(ch);
    await page.waitForTimeout(450);
    const now = await focusedState(page);
    expect(now.role, `${role}: focus stayed`).toBe(role);
    expect(now.kept, `${role}: the same field, not a new one`).toBe(true);
  }
  const c1 = await calls(page);
  expect(await focusedState(page)).toEqual({ role, kept: true, value: before + text });
  return { ka: c1.ka - c0.ka, rs: c1.rs - c0.rs, pair: c1.pair - c0.pair };
}

test("settings: Remote keeps focus and text while it refreshes (folder, relay while pairing, approver name)", async ({ page }) => {
  test.setTimeout(120_000);
  const problems = await openPage(page, "settings", "/settings.html", fakeRemote);
  await expect.poll(() => sent(page, "settings_page")).toEqual([{ ok: true }]);
  await page.locator('[data-target="settings-remote"]').click();
  await expect(page.locator('[data-role="remote-folder"]')).toBeVisible();

  // 1. The keep-alive part (5 s refresh) and the Remote tasks below it (2 s).
  const folder = "C:\\work\\example-project\\typed-slowly";
  let ticks = await typeAcrossTicks(page, "remote-folder", folder);
  expect(ticks.ka, "keep-alive read at least twice while typing").toBeGreaterThanOrEqual(2);
  expect(ticks.rs, "Remote tasks read at least 4 times while typing").toBeGreaterThanOrEqual(4);
  console.log(`folder: ${folder.length} keys across ${ticks.ka} keep-alive and ${ticks.rs} status reads; focus and text kept`);

  // 2. Pairing (waiting, a new QR code every 2 s): type the relay address.
  await page.locator('[data-role="remote-pair-start"]').click();
  await expect(page.locator('[data-role="remote-pair-qr"]')).toBeVisible();
  const relayExtra = "/v1-typed-while-pairing";
  ticks = await typeAcrossTicks(page, "remote-relay", relayExtra);
  expect(ticks.pair, "pairing read at least 4 times while typing").toBeGreaterThanOrEqual(4);
  console.log(`relay: ${relayExtra.length} keys across ${ticks.pair} pairing and ${ticks.rs} status reads; focus and text kept`);
  await shot(page, "settings-remote-typing-while-pairing", page.locator("#settings-remote"));

  // 3. The phone is done: the approver's name, while status keeps refreshing.
  await page.evaluate(() => { window.__pairReady = true; });
  // The pairing card waits while a field has focus; leaving it lets it in.
  await page.locator("#settings-remote h2").click();
  const name = page.locator('[data-role="remote-pair-name"]');
  await expect(name).toBeVisible({ timeout: 6_000 });
  ticks = await typeAcrossTicks(page, "remote-pair-name", "My iPhone, typed slowly");
  expect(ticks.rs, "status read at least 4 times while typing").toBeGreaterThanOrEqual(4);
  console.log(`approver name: typed across ${ticks.rs} status reads; focus and text kept`);

  // Leaving the field lets the waiting redraw in; what was typed stays (drafts).
  await page.locator("#settings-remote h2").click();
  const last = (await sent(page, "remote_status")).length;
  await expect.poll(async () => Number(((await page.locator('[data-role="remote-runbooks"]').textContent()) ?? "").match(/tick-(\d+)/)?.[1] ?? 0), { timeout: 6_000 }).toBeGreaterThanOrEqual(last);
  await expect(page.locator('[data-role="remote-folder"]')).toHaveValue(folder);
  await expect(page.locator('[data-role="remote-relay"]')).toHaveValue(`https://relay.example.com${relayExtra}`);
  await expect(name).toHaveValue("My iPhone, typed slowly");
  await shot(page, "settings-remote-after-typing", page.locator("#settings-remote"));
  noConsoleErrors(problems);
});
