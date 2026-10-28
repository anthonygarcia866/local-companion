// Settings page (settings.html) with the mock bridge.

import { test, expect } from "@playwright/test";
import { emit, noConsoleErrors, openPage, sent, shot } from "./glim.mjs";

/** 3 mailboxes × 3 calendars (example addresses only: the repo is public). */
const MAILBOXES = ["alex@example.com", "team@example.com", "home@example.org"];
const CALENDARS = MAILBOXES.flatMap((mailbox, m) =>
  ["Calendar", "Birthdays", "Holidays"].map((name, c) => ({ id: `cal-${m}-${c}`, name, mailbox, default: m === 0 && c === 0 })));

/** The replies Settings needs, kept as state so delete and rename stick. */
function fakeRust(calendars) {
  const memory = { rules: ["when I say the office I mean Outlook", "open Claude means the desktop app"], facts: ["I prefer short replies"] };
  let routines = [
    { name: "work mode", steps: [{ tool: "open_app", name: "Outlook" }, { tool: "open_app", name: "Claude" }] },
    { name: "wrap up", steps: [{ tool: "resume_work", name: "" }] },
  ];
  const r = window.__glim.replies;
  r.calendar_calendars = calendars;
  // Rust's built-in lists (writing::apps, writing::tones, capture::sensitive), a few each.
  r.writing_known_apps = [
    { exe: "outlook.exe", name: "Outlook", defaultOn: true, chat: false, fixes: true },
    { exe: "ms-teams.exe", name: "Teams", defaultOn: true, chat: true, fixes: true },
    { exe: "winword.exe", name: "Word", defaultOn: false, chat: false, fixes: false },
  ];
  r.writing_tone_defaults = [["email", "formal"], ["chat", "casual"]];
  r.writing_pause_builtins = { apps: ["1password.exe", "keepass.exe"], sites: ["bank", "paypal.com"] };
  r.memory_list = () => structuredClone(memory);
  r.memory_delete = ({ kind, text }) => {
    const list = kind === "rule" ? memory.rules : memory.facts;
    const at = list.indexOf(text);
    if (at >= 0) list.splice(at, 1);
    return at >= 0;
  };
  r.learn_list = () => ({ shortcuts: [], routines: structuredClone(routines) });
  r.learn_routine_rename = ({ name, to }) => {
    routines = routines.map((x) => (x.name === name ? { ...x, name: to } : x));
    return null;
  };
  r.learn_routine_delete = ({ name }) => {
    routines = routines.filter((x) => x.name !== name);
    return true;
  };
}

async function openSettings(page) {
  const problems = await openPage(page, "settings", "/settings.html", fakeRust, CALENDARS);
  // The page reports it drew its sections (webview_health.rs: "loaded").
  await expect.poll(() => sent(page, "settings_page")).toEqual([{ ok: true }]);
  return problems;
}

test("settings: every section renders, no console errors", async ({ page }) => {
  const problems = await openSettings(page);
  const targets = await page.locator("[data-target]").evaluateAll((els) => els.map((e) => e.getAttribute("data-target")));
  expect(targets.length).toBeGreaterThanOrEqual(14);
  for (const id of targets) {
    await page.locator(`[data-target="${id}"]`).first().click();
    const section = page.locator(`#${id}`);
    await expect(section, id).toBeVisible();
    // A sub-entry (Writing → Apps) points at a heading inside its section:
    // what counts is what follows it there.
    const text = await section.evaluate((el) => {
      if (!/^H\d$/.test(el.tagName)) return el.innerText.trim();
      let out = el.innerText;
      for (let n = el.nextElementSibling; n && !/^H\d$/.test(n.tagName); n = n.nextElementSibling) out += n.innerText;
      return out.trim();
    });
    expect(text.length, `${id} has content`).toBeGreaterThan(10);
    await shot(page, `settings-${id.replace("settings-", "")}`);
  }
  console.log(`settings: ${targets.length} sections rendered`);
  noConsoleErrors(problems);
});

test("settings: calendar list, one row per calendar with its mailbox", async ({ page }) => {
  const problems = await openSettings(page);
  await page.locator('[data-target="settings-calendar"]').click();
  const rows = page.locator('[data-role="calendar-row"]');
  await expect(rows).toHaveCount(9);
  for (let i = 0; i < 9; i++) {
    const row = rows.nth(i);
    const { name, mailbox } = await page.evaluate((k) => window.__glim.replies.calendar_calendars[k], i);
    await expect(row.locator(".calendar-name")).toHaveText(i === 0 ? `${name} (default)` : name);
    await expect(row.locator(".calendar-mailbox")).toHaveText(mailbox);
    // One per row: rows don't share a line.
    if (i > 0) {
      const [a, b] = [await rows.nth(i - 1).boundingBox(), await row.boundingBox()];
      expect(b.y, `row ${i} below row ${i - 1}`).toBeGreaterThanOrEqual(a.y + a.height - 1);
    }
  }
  await shot(page, "settings-calendar-list", page.locator("#settings-calendar"));
  console.log("calendar: 9 rows (3 mailboxes x 3 calendars), each with its mailbox");
  noConsoleErrors(problems);
});

test("settings: Memory lists rules and facts, Remove deletes one", async ({ page }) => {
  const problems = await openSettings(page);
  await page.locator('[data-target="settings-memory"]').click();
  const rules = page.locator('[data-role="memory-rules"] .pill-row');
  const facts = page.locator('[data-role="memory-facts"] .pill-row');
  await expect(rules).toHaveCount(2);
  await expect(facts).toHaveCount(1);
  await shot(page, "settings-memory-before", page.locator("#settings-memory"));
  await rules.filter({ hasText: "when I say the office" }).locator('[data-role="memory-remove"]').click();
  await expect(rules).toHaveCount(1);
  await expect(rules.first()).toContainText("open Claude means the desktop app");
  expect(await sent(page, "memory_delete")).toEqual([{ kind: "rule", text: "when I say the office I mean Outlook" }]);
  // Something remembered by voice while Settings is open shows up.
  await page.evaluate(() => {
    const before = window.__glim.replies.memory_list();
    window.__glim.replies.memory_list = () => ({ ...before, facts: [...before.facts, "I work in Denver"] });
  });
  await emit(page, "memory-changed", null);
  await expect(facts).toHaveCount(2);
  await shot(page, "settings-memory-after", page.locator("#settings-memory"));
  console.log("memory: 2 rules + 1 fact listed; Remove deleted 1 rule; memory-changed reloads");
  noConsoleErrors(problems);
});

test("settings: routines list, rename and remove", async ({ page }) => {
  const problems = await openSettings(page);
  await page.locator('[data-target="settings-ask"]').click();
  const routines = page.locator(".learn-routine");
  await expect(routines).toHaveCount(2);
  await expect(page.locator('[data-routine="work mode"]')).toContainText("1. Open Outlook");
  await expect(page.locator('[data-routine="work mode"]')).toContainText("2. Open Claude");
  await shot(page, "settings-routines-before", page.locator('[data-role="learn-routine-list"]'));
  const work = page.locator('[data-routine="work mode"]');
  await work.locator('[data-role="learn-rename-input"]').fill("office mode");
  await work.locator('[data-role="learn-rename"]').click();
  await expect(page.locator('[data-routine="office mode"]')).toHaveCount(1);
  expect(await sent(page, "learn_routine_rename")).toEqual([{ name: "work mode", to: "office mode" }]);
  await page.locator('[data-routine="wrap up"] [data-role="learn-remove"]').click();
  await expect(routines).toHaveCount(1);
  expect(await sent(page, "learn_routine_delete")).toEqual([{ name: "wrap up" }]);
  await shot(page, "settings-routines-after", page.locator('[data-role="learn-routine-list"]'));
  console.log("routines: 2 listed with steps; renamed work mode -> office mode; removed wrap up");
  noConsoleErrors(problems);
});
