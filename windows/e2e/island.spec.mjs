// The island (index.html) with the mock bridge: approval cards and the Allow
// key (#59, core/allow.ts), and the voice cards (Glim learns).

import { test, expect } from "@playwright/test";
import { emit, expectOnScreen, noConsoleErrors, openPage, sent, shot } from "./glim.mjs";

const ALLOW_DELAY_MS = 1_500; // core/allow.ts
const AFTER_DELAY = ALLOW_DELAY_MS + 200;
/** The island opens and grows with a spring: let it settle, as a person's
 *  first look at the card would (well under the 1.5 s). */
const SETTLE = 700;

let n = 0;
/** A permission request, as the relay sends it (hooks.ts). */
function ask(page, command) {
  const id = `r${++n}`;
  return emit(page, "hook", {
    hook_event_name: "PermissionRequest", request_id: id, session_id: "s1", tool_name: "Bash",
    tool_input: { command }, cwd: "C:/work/demo",
  }).then(() => id);
}

/** The Allow key, as Rust's global shortcut sends it. */
const press = (page) => emit(page, "shortcut", "allowApproval");

async function allowed(page, id) {
  return (await sent(page, "approval_decision")).some((d) => d.requestId === id && d.decision === "allow");
}

const card = (page) => page.locator(".view").filter({ has: page.locator(".code") });
const code = (page) => card(page).locator(".code");
const why = (page) => card(page).locator('[data-role="allow-why"]');
const confirm = (page) => card(page).locator('[data-role="allow-confirm"]');

/** The command box and both buttons are inside the card, none cut off by its
 *  edge (a box that won't shrink pushed them out once, 2026-10-28). */
async function nothingCutOff(page) {
  const out = await card(page).locator(".code").evaluate((box) => {
    const edge = box.closest(".stack").parentElement.getBoundingClientRect();
    const parts = [box, ...box.closest(".stack").querySelectorAll(".actions button")];
    return parts.filter((p) => {
      const r = p.getBoundingClientRect();
      return r.top < edge.top - 1 || r.bottom > edge.bottom + 1;
    }).map((p) => p.textContent.slice(0, 20));
  });
  expect(out, "cut off by the card's edge").toEqual([]);
}

/** Rust's answers to the voice card's buttons: done. */
function fakeRust() {
  window.__glim.replies.voice_routine_save = true;
  window.__glim.replies.voice_try_again = true;
}

async function openIsland(page) {
  const problems = await openPage(page, "island", "/index.html", fakeRust);
  await expect.poll(() => page.evaluate(() => window.__glim.listening("hook") && window.__glim.listening("shortcut") && window.__glim.listening("voice"))).toBe(true);
  return problems;
}

/** A card up, waited past the 1.5 s delay. */
async function cardFor(page, command) {
  const id = await ask(page, command);
  await expect(code(page)).toHaveText(`Bash · ${command}`);
  await expectOnScreen(code(page));
  await page.waitForTimeout(AFTER_DELAY);
  return id;
}

const LONG = `npm run build -- --filter ${Array.from({ length: 12 }, (_, i) => `packages/module-${i}`).join(" ")} --verbose`;
const LONG_DESTRUCTIVE = `rm -rf build && ${LONG}`;

test("approval: short command, one press allows", async ({ page }) => {
  const problems = await openIsland(page);
  const id = await cardFor(page, "npm test");
  await shot(page, "approval-short");
  await press(page);
  await expect.poll(() => allowed(page, id)).toBe(true);
  console.log("approval short: 1 press allowed");
  noConsoleErrors(problems);
});

test("approval: a press before 1.5 s is refused, with the message", async ({ page }) => {
  const problems = await openIsland(page);
  const id = await ask(page, "npm test");
  await expect(code(page)).toHaveText("Bash · npm test");
  await page.waitForTimeout(SETTLE);
  await press(page);
  await expect(why(page)).toHaveText("Too soon: read it, then press again.");
  expect(await allowed(page, id), "refused before 1.5 s").toBe(false);
  await shot(page, "approval-short-too-soon");
  console.log("approval too soon: refused at 0.7 s with the message");
  noConsoleErrors(problems);
});

// Found by this test (2026-10-28), fixed the same day: the "Too soon" line
// took room from the command box, so even "npm test" was clipped and the next
// press expanded the card instead of allowing it (3 presses). The box no
// longer shrinks (style.css .code).
test("approval: after a too-soon press, the next press (1.5 s on) allows a short command", async ({ page }) => {
  const problems = await openIsland(page);
  const id = await ask(page, "npm test");
  await expect(code(page)).toHaveText("Bash · npm test");
  await page.waitForTimeout(SETTLE);
  await press(page);
  await expect(why(page)).toBeVisible();
  await nothingCutOff(page);
  await page.waitForTimeout(AFTER_DELAY);
  await press(page);
  await expect.poll(() => allowed(page, id), { timeout: 2_000 }).toBe(true);
  noConsoleErrors(problems);
});

test("approval: long command, 1st press expands, 2nd (after 1.5 s) allows", async ({ page }) => {
  const problems = await openIsland(page);
  const id = await cardFor(page, LONG);
  await shot(page, "approval-long-folded");
  await press(page);
  await expect(code(page)).toHaveClass(/expanded/);
  await expect(confirm(page)).toContainText("Press again to allow");
  expect(await allowed(page, id)).toBe(false);
  await page.waitForTimeout(SETTLE);
  await shot(page, "approval-long-expanded");
  // The 1.5 s starts again once expanded.
  await press(page);
  await expect(why(page)).toHaveText("Too soon: read it, then press again.");
  expect(await allowed(page, id), "too soon after expanding").toBe(false);
  await page.waitForTimeout(AFTER_DELAY);
  await press(page);
  await expect.poll(() => allowed(page, id)).toBe(true);
  console.log("approval long: expand, refused <1.5 s after expanding, allowed on the next press");
  noConsoleErrors(problems);
});

test("approval: long destructive command, expand then 2 presses", async ({ page }) => {
  const problems = await openIsland(page);
  const id = await cardFor(page, LONG_DESTRUCTIVE);
  await press(page);
  await expect(code(page)).toHaveClass(/expanded/);
  await expect(confirm(page)).toHaveText("Read it, then press twice to allow");
  await shot(page, "approval-destructive-expanded");
  await page.waitForTimeout(AFTER_DELAY);
  await press(page);
  await expect(confirm(page)).toContainText("Press again to allow");
  expect(await allowed(page, id), "one press after expanding is not enough").toBe(false);
  await shot(page, "approval-destructive-armed");
  await press(page);
  await expect.poll(() => allowed(page, id)).toBe(true);
  console.log("approval destructive: expand + 2 presses");
  noConsoleErrors(problems);
});

test("approval: a command the relay cut short is refused, with the message", async ({ page }) => {
  const problems = await openIsland(page);
  const id = await cardFor(page, `${LONG} --and-more…`);
  await expect(why(page)).toHaveText("Too long to show in full: click Allow.");
  for (let i = 0; i < 3; i++) {
    await press(page);
    await page.waitForTimeout(AFTER_DELAY);
  }
  expect(await allowed(page, id), "never by key").toBe(false);
  await expect(why(page)).toHaveText("Too long to show in full: click Allow.");
  await shot(page, "approval-truncated");
  // The button still works.
  await card(page).getByRole("button", { name: /Allow/ }).click();
  await expect.poll(() => allowed(page, id)).toBe(true);
  console.log("approval truncated: 3 presses refused with the message; the Allow button allows");
  noConsoleErrors(problems);
});

// 91e9654 (security review of #59): the box is measured as it is on screen,
// with the "Press again" line taking its room. A command exactly at the
// expanded box's limit is allowed; one character more is refused until
// scrolled to its end.
test("approval: the boundary, a command at the expanded box's exact limit", async ({ page }) => {
  const problems = await openIsland(page);
  // Find the limit on a real expanded card, with "Press again" showing.
  const probe = await cardFor(page, `echo ${"x".repeat(3000)}`);
  await press(page);
  await expect(code(page)).toHaveClass(/expanded/);
  await expect(confirm(page)).toBeVisible();
  await page.waitForTimeout(SETTLE);
  const limit = await code(page).evaluate((el) => {
    const keep = el.textContent;
    const fits = (len) => {
      el.textContent = `Bash · echo ${"x".repeat(len)}`;
      return el.scrollHeight <= el.clientHeight + 1 && el.scrollWidth <= el.clientWidth + 1;
    };
    let lo = 1, hi = 3000; // fits(lo), !fits(hi)
    while (hi - lo > 1) {
      const mid = (lo + hi) >> 1;
      if (fits(mid)) lo = mid; else hi = mid;
    }
    el.textContent = keep;
    return lo;
  });
  expect(limit).toBeGreaterThan(100);
  await emit(page, "shortcut", "denyApproval");
  await expect.poll(async () => (await sent(page, "approval_decision")).some((d) => d.requestId === probe)).toBe(true);

  // At the limit: expand, then allowed.
  const at = await cardFor(page, `echo ${"x".repeat(limit)}`);
  await press(page);
  await expect(code(page)).toHaveClass(/expanded/);
  await page.waitForTimeout(AFTER_DELAY);
  await press(page);
  await expect.poll(() => allowed(page, at)).toBe(true);

  // One more character: its last line is out of view, so refused.
  const over = await cardFor(page, `echo ${"x".repeat(limit + 1)}`);
  await press(page);
  await expect(code(page)).toHaveClass(/expanded/);
  await page.waitForTimeout(AFTER_DELAY);
  await press(page);
  await expect(why(page)).toHaveText("Scroll to the end of the command, then press again.");
  expect(await allowed(page, over), "last line out of view").toBe(false);
  await shot(page, "approval-boundary-over-refused");
  await nothingCutOff(page);
  // Scrolled to its end: allowed.
  await code(page).evaluate((el) => { el.scrollTop = el.scrollHeight; });
  await press(page);
  await expect.poll(() => allowed(page, over)).toBe(true);
  console.log(`approval boundary: ${limit} chars fit and are allowed; ${limit + 1} refused until scrolled, then allowed`);
  noConsoleErrors(problems);
});

// ── Voice cards (Glim learns) ────────────────────────────────────────────────

/** A voice event, as Rust sends it (voice::VoiceEvent). */
const voice = (page, e) => emit(page, "voice", { level: 0, cleaned: false, text: "", code: "", app: "", mode: "ask", ...e });
/** The voice card showing `text`. */
const voiceCard = (page, text) => page.locator(".view").filter({ has: page.locator(".rewrite-actions") }).filter({ hasText: text });

test("voice: the routine card lists every step; Save and Cancel go to Rust", async ({ page }) => {
  const problems = await openIsland(page);
  const steps = [{ tool: "open_app", name: "Outlook" }, { tool: "open_app", name: "Claude" }];
  await voice(page, { phase: "routine-ask", text: "work mode", steps });
  const c = voiceCard(page, "Save the routine “work mode”?");
  await expect(c).toHaveCount(1);
  await expect(c).toContainText("1. Open Outlook");
  await expect(c).toContainText("2. Open Claude");
  await expect(c).toContainText("Nothing runs until you save it");
  await expectOnScreen(c.locator(".title"));
  await shot(page, "voice-routine-card");
  await c.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await sent(page, "voice_routine_save")).length).toBe(1);
  // Again, cancelled this time: Rust is told nothing is saved.
  await voice(page, { phase: "routine-ask", text: "home", steps: steps.slice(0, 1) });
  const home = voiceCard(page, "Save the routine “home”?");
  await expect(home).toHaveCount(1);
  const cancels = (await sent(page, "voice_cancel")).length;
  await home.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await sent(page, "voice_cancel")).length).toBe(cancels + 1);
  console.log("voice routine card: 2 steps listed; Save -> voice_routine_save; Cancel -> voice_cancel");
  noConsoleErrors(problems);
});

test("voice: the Learned card says Claude wasn't asked, with Try again", async ({ page }) => {
  const problems = await openIsland(page);
  await voice(page, { phase: "learned", app: "Outlook", code: "open_app" });
  const c = voiceCard(page, "Opened Outlook");
  await expect(c).toHaveCount(1);
  await expect(c).toContainText("Learned shortcut: Claude wasn't asked.");
  await expectOnScreen(c.locator(".title"));
  await shot(page, "voice-learned");
  await c.getByRole("button", { name: "Try again" }).click();
  await expect.poll(async () => (await sent(page, "voice_try_again")).length).toBe(1);
  console.log("voice learned: card + Try again -> voice_try_again");
  noConsoleErrors(problems);
});

test("voice: a remembered rule that replaced a learned shortcut says so (#62)", async ({ page }) => {
  const problems = await openIsland(page);
  await voice(page, { phase: "remembered", code: "rule", text: "when I say mail I mean Outlook", app: "“open outlook”" });
  const c = voiceCard(page, "I'll remember that");
  await expect(c).toHaveCount(1);
  await expect(c).toContainText("Removed the learned shortcut for “open outlook”.");
  await expectOnScreen(c.locator(".suggestion-more"));
  await shot(page, "voice-removed-shortcut");
  console.log("voice remembered: 'Removed the learned shortcut' line shown");
  noConsoleErrors(problems);
});

test("voice: a long question wraps and shows in full", async ({ page }) => {
  const problems = await openIsland(page);
  const long = "Do you want me to open an app, or should I draft it as an email to the tenant about the updated lease before Friday afternoon, with the signature added?";
  await voice(page, { phase: "question", text: long });
  const q = page.locator(".voice-question").filter({ visible: true });
  await expect(q).toHaveText(long);
  await expectOnScreen(q);
  const box = await q.evaluate((el) => ({
    lines: Math.round(el.getBoundingClientRect().height / parseFloat(getComputedStyle(el).lineHeight || "16")),
    cutX: el.scrollWidth > el.clientWidth + 1,
    cutY: el.scrollHeight > el.clientHeight + 1,
    ellipsis: getComputedStyle(el).textOverflow,
  }));
  expect(box.lines, "wraps over several lines").toBeGreaterThan(1);
  expect(box.cutX, "nothing cut off sideways").toBe(false);
  expect(box.cutY, "fits without scrolling").toBe(false);
  expect(box.ellipsis).not.toBe("ellipsis");
  await shot(page, "voice-question-long");
  console.log(`voice question: wraps over ${box.lines} lines, nothing cut off`);
  noConsoleErrors(problems);
});
