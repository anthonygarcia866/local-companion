// The calendar glance, the island's half (core/calendar.ts, views/calendar.ts,
// settings/calendar.ts; Rust: src-tauri/src/calendar/). Fake events only:
// recurring occurrences, all-day, overlapping, time zones, Join.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State, DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const { STILL_NOW_MS, LEAD_CHOICES, DEFAULT_LEAD_MIN, nextCalendar, nextEvent, glowing, wantsCard, closedCalendar, countdown, timeRange, todays, withCalendarGlow, summaryPlacement } = await import("../src/core/calendar.ts");
const { buildCalendar, closeCalendar, whenText } = await import("../src/views/calendar.ts");
const { calendarSection, chosenIds, reloadCalendars } = await import("../src/settings/calendar.ts");
const { internals, sent, emit } = await import("./tauri.mjs");
const { t } = await import("../src/i18n/i18n.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const M = 60_000;
const H = 60 * M;
/** 2026-10-17 15:00 UTC = 08:00 in Los Angeles. */
const NOW = 1_792_249_200_000;
const ev = (title, start, mins, o = {}) => ({ title, location: "", start, end: start + mins * M, allDay: false, hasJoin: false, ...o });
const glance = (events, o = {}) => ({ available: true, code: "", events, generation: 4, summary: false, ...o });

test("the next event: not started, or started under 10 minutes ago; all-day never; same-time ones counted", () => {
  assert.equal(STILL_NOW_MS, 10 * M);
  const allDay = ev("Holiday", NOW - 8 * H, 24 * 60, { allDay: true });
  // A daily stand-up: each occurrence is its own event (Rust/Outlook expand them).
  const events = [allDay, ev("Stand-up", NOW - 5 * M, 15), ev("Board", NOW + H, 60), ev("Audit", NOW + H, 30), ev("Stand-up", NOW + 23 * H, 15)];
  const n = nextEvent(events, NOW);
  assert.equal(n.event.title, "Stand-up", "started 5 min ago: still now");
  assert.equal(n.event.start, NOW - 5 * M);
  const later = nextEvent(events, NOW + 11 * M);
  assert.deepEqual([later.event.title, later.index, later.same], ["Board", 2, 1], "overlapping: Board and Audit at the same time");
  assert.equal(nextEvent(events, NOW + 23 * H + 20 * M), null, "the last one is over");
  assert.equal(nextEvent([allDay], NOW), null);
});

test("the lantern glows from the lead time until 10 minutes in", () => {
  const e = ev("Rent review", NOW + 10 * M, 30);
  assert.equal(glowing(e, NOW - 1, 10), false);
  assert.equal(glowing(e, NOW, 10), true);
  assert.equal(glowing(e, NOW, 5), false, "5-minute lead");
  assert.equal(glowing(e, NOW + 15 * M, 10), true, "5 minutes in");
  assert.equal(glowing(e, NOW + 20 * M, 10), false, "10 minutes in: done glowing");
  assert.equal(glowing(ev("Short", NOW, 5), NOW + 6 * M, 10), false, "an event that ended");
  assert.equal(glowing(ev("Day", NOW, 600, { allDay: true }), NOW, 10), false);
  assert.equal(glowing(null, NOW, 10), false);
  // A resting lantern glows; agent states and suggestions are left alone.
  assert.equal(withCalendarGlow("idle", true), "listen");
  assert.equal(withCalendarGlow("done", true), "listen");
  assert.equal(withCalendarGlow("idle", false), "idle");
  for (const s of ["think", "suggest", "error", "paused"]) assert.equal(withCalendarGlow(s, true), s);
  assert.deepEqual([...LEAD_CHOICES], [5, 10, 15]);
  assert.equal(DEFAULT_LEAD_MIN, 10);
  assert.equal(DEFAULT_SETTINGS.calendarLeadMin, 10);
  assert.equal(DEFAULT_SETTINGS.calendarOn, false, "off until the live test passes");
  assert.equal(DEFAULT_SETTINGS.calendarHideAllDay, true);
  assert.equal(DEFAULT_SETTINGS.calendarSummary, false);
  assert.deepEqual(DEFAULT_SETTINGS.calendarIds, []);
});

test("countdown and time range, in the PC's time zone", () => {
  const e = ev("x", NOW + 12 * M, 30);
  const text = (c) => t(c.msg, c.vars);
  assert.equal(text(countdown(e, NOW)), "in 12 min");
  assert.equal(text(countdown(e, NOW + 11 * M + 30_000)), "in 1 min");
  assert.equal(text(countdown(ev("x", NOW + 65 * M, 30), NOW)), "in 1 h 5 min");
  assert.equal(text(countdown(ev("x", NOW + 2 * H, 30), NOW)), "in 2 h");
  assert.equal(text(countdown(e, NOW + 12 * M + 20_000)), "now");
  assert.equal(text(countdown(e, NOW + 15 * M)), "started 3 min ago");
  // The same event at 08:12 in Los Angeles is 00:12 the next day in Tokyo.
  assert.equal(timeRange(e, "en-US", "America/Los_Angeles"), "8:12 AM – 8:42 AM");
  assert.equal(timeRange(e, "en-US", "Asia/Tokyo"), "12:12 AM – 12:42 AM");
  assert.equal(timeRange(e, "en-US", "UTC"), "3:12 PM – 3:42 PM");
  assert.equal(timeRange(ev("reminder", NOW, 0), "en-US", "UTC"), "3:00 PM", "a zero-length event shows one time");
  assert.equal(timeRange(ev("Day", NOW, 600, { allDay: true }), "en-US", "UTC"), null);
  // "Today" is the local date: 23:30 in Los Angeles is today there, tomorrow in UTC.
  const late = ev("Late", NOW + 15 * H + 30 * M, 30);
  assert.deepEqual(todays([late], NOW, "America/Los_Angeles").map((x) => x.title), ["Late"]);
  assert.deepEqual(todays([late], NOW, "UTC"), []);
});

test("the card: title, time, countdown, place, the others at the same time, and Join only with a link", () => {
  State.mode = "expanded";
  State.view = "calendar";
  State.settings = { ...DEFAULT_SETTINGS, calendarOn: true };
  const soon = Date.now() + 8 * M;
  State.calendar = nextCalendar(null, glance([ev("Rent review with Gabriela", soon, 30, { location: "Room 12C", hasJoin: true }), ev("Audit", soon, 30)]));
  const calls = [];
  const v = buildCalendar({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  v.sync();
  assert.match(v.el.textContent, /Rent review with Gabriela/);
  assert.match(v.el.textContent, /in 8 min/);
  assert.match(v.el.textContent, /Room 12C · \+1 more at the same time/);
  const buttons = () => v.el.find("button").map((b) => b.textContent);
  assert.deepEqual(buttons(), ["Join", "Close"]);
  const n = sent("calendar_join").length;
  v.el.find("button")[0].fire("click");
  assert.deepEqual(sent("calendar_join").slice(n), [{ generation: 4, index: 0 }], "by read and index: the link stays in Rust");
  // No link: no Join.
  State.calendar = nextCalendar(State.calendar, glance([ev("Lunch", soon, 60)]));
  v.sync();
  assert.deepEqual(buttons(), ["Close"]);
  // The lantern click opens the card only while it glows, and not again once closed.
  assert.equal(wantsCard(State.calendar, Date.now(), 10), true);
  assert.equal(State.lanternView(), "calendar");
  closeCalendar({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  assert.equal(wantsCard(State.calendar, Date.now(), 10), false, "closed: that event stays quiet");
  assert.deepEqual(calls.at(-1), ["collapse"]);
  assert.equal(wantsCard(nextCalendar(null, glance([ev("Later", Date.now() + 3 * H, 30)])), Date.now(), 10), false, "not yet");
  assert.equal(wantsCard(nextCalendar(null, glance([], { available: false, code: "no-outlook" })), Date.now(), 10), false);
  assert.equal(whenText(ev("Day", NOW, 600, { allDay: true }), NOW), "All day");
  State.calendar = null;
  State.mode = "hidden";
});

test("the morning summary: today's list, opened once, closed for good", () => {
  State.mode = "expanded";
  // 09:00 local, so the day's events don't run past midnight on any runner
  // (CI runs in UTC late in the day: 2026-10-17's first run failed on that).
  const realNow = Date.now;
  const now = new Date(2026, 9, 17, 9, 0).getTime();
  Date.now = () => now;
  const many = Array.from({ length: 7 }, (_, i) => ev(`Meeting ${i + 1}`, now + (i + 1) * 20 * M, 15));
  State.calendar = nextCalendar(null, glance(many, { summary: true }));
  assert.equal(State.calendar.summaryOpen, true);
  const v = buildCalendar({ setView() {}, collapse() {} });
  v.sync();
  assert.match(v.el.textContent, /Today: 7 on your calendar/);
  assert.match(v.el.textContent, /Meeting 1/);
  assert.match(v.el.textContent, /\+2 more/, "5 listed, the other 2 counted");
  // The next read doesn't reopen it; Close closes it.
  State.calendar = nextCalendar(State.calendar, glance(many));
  assert.equal(State.calendar.summaryOpen, true, "still up until closed");
  State.calendar = closedCalendar(State.calendar, now);
  assert.equal(State.calendar.summaryOpen, false);
  State.calendar = nextCalendar(State.calendar, glance(many));
  assert.equal(State.calendar.summaryOpen, false);
  assert.equal(summaryPlacement({ presence: "indicator", approval: false }), "drop", "never over fullscreen");
  assert.equal(summaryPlacement({ presence: "pill", approval: true }), "queue");
  assert.equal(summaryPlacement({ presence: "hidden", approval: false }), "show");
  Date.now = realNow;
  State.calendar = null;
  State.mode = "hidden";
});

test("Settings → Calendar: switches, lead time and calendars are stored; Outlook not running says so", async () => {
  const settings = { ...DEFAULT_SETTINGS };
  let saves = 0;
  const original = internals.invoke;
  let folders = [{ id: "\\\\me@example.com\\Calendar", name: "Calendar", mailbox: "me@example.com", default: true }, { id: "\\\\shared@example.com\\Calendar", name: "Calendar", mailbox: "shared@example.com", default: false }];
  internals.invoke = async (cmd, args) => (cmd === "calendar_calendars" ? folders : original(cmd, args));
  try {
    const section = calendarSection({ settings: () => settings, save: async () => { saves++; } });
    await new Promise((r) => setTimeout(r, 0));
    const role = (r) => [...section.walk()].filter((e) => e.getAttribute("data-role") === r);
    role("calendar-on")[0].fire("click");
    assert.equal(settings.calendarOn, true);
    role("calendar-hide-all-day")[0].fire("click");
    assert.equal(settings.calendarHideAllDay, false);
    role("calendar-summary")[0].fire("click");
    assert.equal(settings.calendarSummary, true);
    const lead = role("calendar-lead")[0];
    assert.equal(lead.value, "10");
    lead.value = "5";
    lead.fire("change");
    assert.equal(settings.calendarLeadMin, 5);
    const boxes = role("calendar-folder");
    assert.equal(boxes.length, 2);
    // One calendar per row, labelled with its mailbox, the default marked:
    // two calendars both named "Calendar" are told apart.
    const rows = role("calendar-row");
    assert.equal(rows.length, 2);
    assert.match(rows[0].textContent, /Calendar \(default\)/);
    assert.match(rows[0].textContent, /me@example\.com/);
    assert.doesNotMatch(rows[1].textContent, /default/);
    assert.match(rows[1].textContent, /shared@example\.com/);
    assert.deepEqual(boxes.map((b) => b.checked), [true, false], "nothing picked = the default");
    boxes[1].checked = true;
    boxes[1].fire("change");
    assert.deepEqual(settings.calendarIds, folders.map((f) => f.id));
    boxes[1].checked = false;
    boxes[1].fire("change");
    assert.deepEqual(settings.calendarIds, [], "only the default again = empty");
    assert.equal(saves, 6, "one save per change");
    // Outlook not running: no list, and a line saying Glim won't start it.
    folders = [];
    const closed = calendarSection({ settings: () => settings, save: async () => {} });
    await new Promise((r) => setTimeout(r, 0));
    const status = [...closed.walk()].find((e) => e.getAttribute("data-role") === "calendar-folders-status");
    assert.match(status.textContent, /Open classic Outlook to choose calendars\. Glim never starts it for this\./);
  } finally {
    internals.invoke = original;
  }
  assert.deepEqual(chosenIds([], "d"), []);
  assert.deepEqual(chosenIds(["d"], "d"), []);
  assert.deepEqual(chosenIds(["x"], "d"), ["x"]);
});

test("Settings → Calendar: an empty list at launch fills in when Outlook is ready, when Settings opens, or from the sidebar", async () => {
  const settings = { ...DEFAULT_SETTINGS };
  const original = internals.invoke;
  let folders = [];
  let asks = 0;
  internals.invoke = async (cmd, args) => (cmd === "calendar_calendars" ? (asks++, folders) : original(cmd, args));
  const tick = () => new Promise((r) => setTimeout(r, 0));
  try {
    const section = calendarSection({ settings: () => settings, save: async () => {} });
    await tick();
    const role = (r) => [...section.walk()].filter((e) => e.getAttribute("data-role") === r);
    const status = () => role("calendar-folders-status")[0].textContent;
    assert.equal(asks, 1, "asked once when built (at launch)");
    assert.equal(role("calendar-folder").length, 0);
    assert.match(status(), /Open classic Outlook to choose calendars/);
    // Outlook answers after Rust's retry: Rust says the count changed.
    folders = [{ id: "\\\\me@example.com\\Calendar", name: "Calendar", default: true }, { id: "\\\\me@example.com\\Calendar\\Team", name: "Team", default: false }];
    emit("calendar-folders", 2);
    await tick();
    assert.equal(asks, 2);
    assert.equal(role("calendar-folder").length, 2, "the list filled in");
    assert.match(status(), /The default calendar is used when none is picked\./);
    // Outlook closed, then Settings opened again: asked again, empty again.
    folders = [];
    emit("settings-opened");
    await tick();
    assert.equal(asks, 3);
    assert.equal(role("calendar-folder").length, 0);
    // Calendar picked in the sidebar.
    folders = [{ id: "\\\\me@example.com\\Calendar", name: "Calendar", default: true }];
    reloadCalendars();
    await tick();
    assert.equal(role("calendar-folder").length, 1);
    // Two asks in a row: only the newer answer is drawn.
    let release;
    internals.invoke = async (cmd, args) => {
      if (cmd !== "calendar_calendars") return original(cmd, args);
      if (!release) return new Promise((r) => { release = () => r([]); });
      return [{ id: "a", name: "A", default: true }, { id: "b", name: "B", default: false }, { id: "c", name: "C", default: false }];
    };
    reloadCalendars();
    reloadCalendars();
    await tick();
    assert.equal(role("calendar-folder").length, 3);
    release();
    await tick();
    assert.equal(role("calendar-folder").length, 3, "the older, empty answer is dropped");
  } finally {
    internals.invoke = original;
  }
  const main = read("src/settings/main.ts");
  assert.ok(main.includes('if (id === "settings-calendar") reloadCalendars();'), "the sidebar asks again");
  const lib = read("src-tauri/src/lib.rs");
  const show = lib.slice(lib.indexOf("pub fn show_settings_window"), lib.indexOf("fn open_settings_window"));
  assert.ok(show.includes('app.emit_to("settings", "settings-opened", ())'), "Rust says Settings opened");
  assert.ok(read("src-tauri/src/calendar/worker.rs").includes('pub const FOLDERS_EVENT: &str = "calendar-folders";'), "same event name as Rust");
});

test("titles and locations are never logged or sent by the island; Rust and the island agree on the setting keys", () => {
  for (const f of ["src/core/calendar.ts", "src/views/calendar.ts", "src/settings/calendar.ts"]) {
    const src = read(f);
    for (const banned of ["console.", "Bridge.log(", "localStorage"]) assert.ok(!src.includes(banned), `${f}: ${banned}`);
  }
  const main = read("src/main.ts");
  const handler = main.slice(main.indexOf('onEvent<CalendarGlance>("calendar"'), main.indexOf("window.setInterval"));
  assert.ok(!handler.includes("Bridge."), "the handler sends nothing back");
  const rust = read("src-tauri/src/settings.rs");
  for (const k of ["calendarOn", "calendarLeadMin", "calendarHideAllDay", "calendarSummary", "calendarIds", "calendarSummaryShown"]) assert.ok(rust.includes(`"${k}"`), k);
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(/\n\s+calendar_calendars,\n\s+calendar_join,\n/.test(lib), "registered");
  assert.ok(lib.includes("settings.calendar_summary_shown = current.calendar_summary_shown.clone();"), "the summary date is Rust's");
});
