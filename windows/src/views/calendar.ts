// The calendar glance's card (core/calendar.ts): the next event, its time
// and a countdown, where it is, "+1 more at the same time", and Join when
// Rust found a Teams/Zoom link in it (Rust holds the link and opens it). Or,
// once a morning if switched on, the day's events. Read only: nothing here
// changes the calendar, and the island never takes focus.

import { h, clear } from "./dom";
import { card, stack, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { closedCalendar, countdown, nextEvent, timeRange, todays, type CalendarEvent } from "../core/calendar";
import type { AskHost } from "./ask";
import { language, t } from "../i18n/i18n";

/** Events listed in the summary, at most; the rest are counted. */
export const SUMMARY_ROWS = 5;

/** Closes the card: that event stays quiet, the summary goes. */
export function closeCalendar(host: AskHost | null) {
  if (!State.calendar) return;
  State.calendar = closedCalendar(State.calendar, Date.now());
  if (host && State.view === "calendar" && State.mode === "expanded") {
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

/** "9:00 – 9:30 AM · in 12 min", or "All day". */
export function whenText(e: CalendarEvent, now: number): string {
  const range = timeRange(e, language() || undefined);
  if (!range) return t("All day");
  const c = countdown(e, now);
  return `${range} · ${t(c.msg, c.vars)}`;
}

export function buildCalendar(host: AskHost): ViewHost {
  const title = h("div", { class: "title calendar-title" });
  const when = h("div", { class: "calendar-when", "data-role": "calendar-when" });
  const where = h("div", { class: "suggestion-more", "data-role": "calendar-where" });
  const list = h("div", { class: "calendar-list", "data-role": "calendar-list" });
  const row = h("div", { class: "rewrite-actions" });
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, when, where, list, row)));

  const sync = () => {
    const c = State.calendar;
    if (!c) return;
    const now = Date.now();
    clear(list);
    clear(row);
    const close = h("button", { class: "link-btn", type: "button", "data-answer": "close", text: t("Close"), onclick: () => closeCalendar(host) });
    if (c.summaryOpen) {
      // The morning summary: today's events, time and title.
      const today = todays(c.glance.events, now);
      title.textContent = today.length ? t("Today: {count} on your calendar", { count: String(today.length) }) : t("Nothing else on your calendar today");
      when.textContent = "";
      where.textContent = "";
      for (const e of today.slice(0, SUMMARY_ROWS)) {
        list.append(h("div", { class: "calendar-row" }, h("span", { class: "calendar-time", text: timeRange(e, language() || undefined) ?? t("All day") }), h("span", { class: "calendar-name", text: e.title || t("(no title)") })));
      }
      if (today.length > SUMMARY_ROWS) list.append(h("div", { class: "suggestion-more", text: t("+{count} more", { count: String(today.length - SUMMARY_ROWS) }) }));
      list.style.display = "";
      row.append(close);
      return;
    }
    list.style.display = "none";
    const n = nextEvent(c.glance.events, now);
    if (!n) {
      title.textContent = t("Nothing coming up");
      when.textContent = "";
      where.textContent = "";
      row.append(close);
      return;
    }
    title.textContent = n.event.title || t("(no title)");
    when.textContent = whenText(n.event, now);
    const extra = n.same > 0 ? t("+{count} more at the same time", { count: String(n.same) }) : "";
    where.textContent = [n.event.location, extra].filter(Boolean).join(" · ");
    if (n.event.hasJoin) {
      // Rust opens the link it holds for this event, if it is a Teams/Zoom one.
      row.append(h("button", { class: "apply-all", type: "button", "data-answer": "join", text: t("Join"), onclick: () => void Bridge.calendarJoin(c.glance.generation, n.index) }));
    }
    row.append(close);
  };
  return { el, sync };
}
