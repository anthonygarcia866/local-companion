// What every page test shares: opening a Glim page with the mock bridge,
// collecting console errors (a check fails on any; warnings are logged),
// screenshots.

import { expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const SHOTS = join(here, "..", "e2e-results", "shots");
mkdirSync(SHOTS, { recursive: true });

/** The browser asks for /favicon.ico by itself; WebView2 never does. */
const IGNORED = [/\/favicon\.ico$/];

/**
 * Opens `path` as Glim's `label` window. `setup(args)` runs in the page
 * before any page script (after the mock): it fills window.__glim.replies.
 * Returns the console problems seen, for `noConsoleErrors`.
 */
export async function openPage(page, label, path, setup, args) {
  const problems = [];
  page.on("console", (m) => {
    if (m.type() !== "error" && m.type() !== "warning") return;
    if (IGNORED.some((r) => r.test(m.location().url ?? ""))) return;
    // Warnings are logged, not failed (errors fail the check).
    if (m.type() === "warning") console.log(`  console warning: ${m.text().split("\n")[0]}`);
    else problems.push(`${m.type()}: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
  await page.addInitScript((l) => { window.__glimLabel = l; }, label);
  await page.addInitScript({ path: join(here, "mock-bridge.js") });
  if (setup) await page.addInitScript(setup, args);
  await page.goto(path);
  return problems;
}

export function noConsoleErrors(problems) {
  expect(problems, problems.join("\n")).toEqual([]);
}

export async function shot(page, name, locator) {
  const path = join(SHOTS, `${name}.png`);
  if (locator) await locator.screenshot({ path });
  else await page.screenshot({ path });
}

/** Fires an event the way Rust would. */
export function emit(page, event, payload) {
  return page.evaluate(([e, p]) => window.__glim.emit(e, p), [event, payload]);
}

/** The arguments of every `cmd` the page invoked. */
export function sent(page, cmd) {
  return page.evaluate((c) => window.__glim.sent(c), cmd);
}

/**
 * `locator` is really on screen: in the viewport, with a size, and fully
 * opaque through all its ancestors (a card fading in, or folded away, is not).
 * Waits for the island's spring and fades to finish.
 */
export async function expectOnScreen(locator) {
  await expect(locator).toBeInViewport();
  await expect.poll(() => locator.evaluate((el) => {
    const r = el.getBoundingClientRect();
    if (r.width < 4 || r.height < 4) return 0;
    let o = 1;
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      const s = getComputedStyle(n);
      if (s.visibility === "hidden" || s.display === "none") return 0;
      o *= parseFloat(s.opacity);
    }
    return o;
  }), { timeout: 4_000 }).toBeGreaterThan(0.98);
}
