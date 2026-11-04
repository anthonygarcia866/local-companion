// The live revision's preview (live.html) with the mock bridge: the word diff,
// the three-line cut and "…", the key hint, the note after the accept key, and
// hiding. Stage 2 (owner, 2026-11-03).

import { test, expect } from "@playwright/test";
import { emit, noConsoleErrors, openPage, sent, shot } from "./glim.mjs";

const SHORT = [
  { kind: "same", text: "We " },
  { kind: "del", text: "recieved " },
  { kind: "add", text: "received " },
  { kind: "same", text: "the payment and will deposit it " },
  { kind: "del", text: "tomorow." },
  { kind: "add", text: "tomorrow." },
];
const LONG = [
  { kind: "same", text: "The walkthrough for the unit is on Thursday and the inspector will need the keys, the parking pass and the updated checklist before ten so that " },
  { kind: "del", text: "everythings " },
  { kind: "add", text: "everything is " },
  { kind: "same", text: "ready when the new residents arrive in the afternoon with the movers and the building manager, who asked for a short note about the noise rules as well. " },
  { kind: "add", text: "Thanks." },
];

async function open(page) {
  const problems = await openPage(page, "live", "/live.html");
  await expect.poll(() => page.evaluate(() => window.__glim.listening("live-preview"))).toBe(true);
  return problems;
}

test("a revision shows struck-through removals, highlighted additions and the key hint", async ({ page }) => {
  const problems = await open(page);
  const card = page.locator("[data-role=live-preview]");
  await expect(card).toHaveClass(/is-hidden/);
  await emit(page, "live-preview", { phase: "show", id: 1, ops: SHORT, app: "Word", why: "" });
  await expect(card).not.toHaveClass(/is-hidden/);
  await expect(page.locator("del")).toHaveText(["recieved ", "tomorow."]);
  await expect(page.locator("ins")).toHaveText(["received ", "tomorrow."]);
  await expect(page.locator("[data-role=live-hint]")).toHaveText("Ctrl+Alt+Shift+Enter to apply");
  await expect(page.locator("[data-role=live-more]")).toHaveCount(0);
  // Struck through and highlighted, as drawn.
  expect(await page.locator("del").first().evaluate((e) => getComputedStyle(e).textDecorationLine)).toContain("line-through");
  expect(await page.locator("ins").first().evaluate((e) => getComputedStyle(e).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
  await shot(page, "live-preview-short", card);
  noConsoleErrors(problems);
});

test("past three lines it stops with …, and … asks Rust to make it taller", async ({ page }) => {
  const problems = await open(page);
  await emit(page, "live-preview", { phase: "show", id: 2, ops: LONG, app: "Outlook", why: "" });
  const more = page.locator("[data-role=live-more]");
  await expect(more).toHaveText("…");
  await expect(page.locator("ins", { hasText: "Thanks." })).toHaveCount(0);
  await shot(page, "live-preview-cut", page.locator("[data-role=live-preview]"));
  await more.click();
  expect(await sent(page, "live_expand")).toEqual([{ open: true }]);
  await expect(page.locator("ins", { hasText: "Thanks." })).toHaveCount(1);
  await expect(more).toHaveCount(0);
  await shot(page, "live-preview-expanded", page.locator("[data-role=live-preview]"));
  noConsoleErrors(problems);
});

test("a note after the accept key, then hidden", async ({ page }) => {
  const problems = await open(page);
  await emit(page, "live-preview", { phase: "show", id: 3, ops: SHORT, app: "Word", why: "" });
  await emit(page, "live-preview", { phase: "note", id: 3, ops: [], app: "Word", why: "changed" });
  await expect(page.locator("[data-role=live-note]")).toHaveText("Text changed: wait for a new suggestion");
  await shot(page, "live-preview-changed", page.locator("[data-role=live-preview]"));
  await emit(page, "live-preview", { phase: "note", id: 3, ops: [], app: "Teams", why: "copied" });
  await expect(page.locator("[data-role=live-note]")).toHaveText("Copied: Glim doesn't type in Teams yet. Paste with Ctrl+V.");
  for (const why of ["typing", "field", "dismissed", "timeout", "applied"]) {
    await emit(page, "live-preview", { phase: "show", id: 4, ops: SHORT, app: "Word", why: "" });
    await emit(page, "live-preview", { phase: "hide", id: 4, ops: [], app: "", why });
    await expect(page.locator("[data-role=live-preview]")).toHaveClass(/is-hidden/);
  }
  noConsoleErrors(problems);
});
