// Calendar glance, the island's half (Rust: src-tauri/src/calendar/). Rust
// reads classic Outlook's calendar (read only, local COM, only while Outlook
// is running) and sends "calendar": the events of the next 24 h. The island
// shows the next one with its time and a countdown, glows the lantern a few
// minutes before it, offers Join when Rust found a Teams/Zoom link (the link
// itself stays in Rust), and once a morning, if switched on, a summary.
// Titles and locations are shown, never logged or sent.

import { N_ } from "../i18n/i18n";

export interface CalendarEvent {
  title: string;
  location: string;
  /** UTC milliseconds. */
  start: number;
  end: number;
  allDay: boolean;
  hasJoin: boolean;
}

/** Rust's calendar::Glance. */
export interface CalendarGlance {
  available: boolean;
  /** "off", "no-outlook", "failed" or "". */
  code: string;
  events: CalendarEvent[];
  generation: number;
  summary: boolean;
}

export interface CalendarState {
  glance: CalendarGlance;
  /** The morning summary is up. */
  summaryOpen: boolean;
  /** The event whose card was closed (start + title): it doesn't come back by itself. */
  closedFor: string | null;
}

/** An event that started less than this ago is still "now" (Rust's STILL_NOW_MS). */
export const STILL_NOW_MS = 10 * 60_000;
/** Settings → Calendar → lead time choices, in minutes. */
export const LEAD_CHOICES = [5, 10, 15] as const;
export const DEFAULT_LEAD_MIN = 10;

const key = (e: CalendarEvent) => `${e.start}|${e.title}`;

/** The state after Rust's glance. A summary that comes opens; one closed stays closed. */
export function nextCalendar(cur: CalendarState | null, g: CalendarGlance): CalendarState {
  return { glance: g, summaryOpen: g.summary || (cur?.summaryOpen === true && g.available), closedFor: cur?.closedFor ?? null };
}

/** The event to show (Rust's calendar::next): the first timed one not yet
 *  started, or started less than STILL_NOW_MS ago; and how many others start
 *  at the same time. */
export function nextEvent(events: readonly CalendarEvent[], now: number): { event: CalendarEvent; index: number; same: number } | null {
  const index = events.findIndex((e) => !e.allDay && e.end > now && e.start + STILL_NOW_MS > now);
  if (index < 0) return null;
  const event = events[index];
  const same = events.filter((e, i) => i !== index && !e.allDay && e.start === event.start).length;
  return { event, index, same };
}

/** The lantern glows from `leadMin` before the event until it has been on
 *  for STILL_NOW_MS (join late). */
export function glowing(event: CalendarEvent | null, now: number, leadMin: number): boolean {
  if (!event || event.allDay) return false;
  return now >= event.start - leadMin * 60_000 && now < event.start + STILL_NOW_MS && now < event.end;
}

/** The lantern click opens the card: the summary is up, or the next event
 *  glows and its card wasn't closed. */
export function wantsCard(c: CalendarState | null, now: number, leadMin: number): boolean {
  if (!c || !c.glance.available) return false;
  if (c.summaryOpen) return true;
  const n = nextEvent(c.glance.events, now);
  return !!n && glowing(n.event, now, leadMin) && c.closedFor !== key(n.event);
}

/** The card was closed: that event stays quiet (the summary closes too). */
export function closedCalendar(c: CalendarState, now: number): CalendarState {
  const n = nextEvent(c.glance.events, now);
  return { ...c, summaryOpen: false, closedFor: n ? key(n.event) : c.closedFor };
}

/** "in 12 min", "in 1 h 5 min", "now", "started 3 min ago". */
export function countdown(event: CalendarEvent, now: number): { msg: string; vars?: Record<string, string> } {
  const ms = event.start - now;
  if (ms > 0) {
    const mins = Math.ceil(ms / 60_000);
    if (mins < 60) return { msg: N_("in {minutes} min"), vars: { minutes: String(mins) } };
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m === 0 ? { msg: N_("in {hours} h"), vars: { hours: String(h) } } : { msg: N_("in {hours} h {minutes} min"), vars: { hours: String(h), minutes: String(m) } };
  }
  const ago = Math.floor(-ms / 60_000);
  return ago < 1 ? { msg: N_("now") } : { msg: N_("started {minutes} min ago"), vars: { minutes: String(ago) } };
}

/** "9:00 – 9:30 AM" in the PC's time zone and the given locale; all day: null. */
export function timeRange(event: CalendarEvent, locale: string | undefined, timeZone?: string): string | null {
  if (event.allDay) return null;
  const f = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit", timeZone });
  return event.end > event.start ? `${f.format(event.start)} – ${f.format(event.end)}` : f.format(event.start);
}

/** Today's events for the summary (local date of their start, or now if it started earlier). */
export function todays(events: readonly CalendarEvent[], now: number, timeZone?: string): CalendarEvent[] {
  const day = (t: number) => new Intl.DateTimeFormat("en-CA", { year: "numeric", month: "2-digit", day: "2-digit", timeZone }).format(t);
  const today = day(now);
  return events.filter((e) => day(Math.max(e.start, now)) === today);
}

/** The lantern state with the calendar's glow: a resting lantern (idle,
 *  listen, done) takes s-listen's brighter glow. Agent states, writing
 *  suggestions and pause still win (applied after this). */
export function withCalendarGlow<S extends string>(state: S, glow: boolean): S | "listen" {
  return glow && (state === "idle" || state === "listen" || state === "done") ? "listen" : state;
}

/** Where the summary goes (like the Docs notice): never over fullscreen,
 *  behind an approval, else shown. */
export function summaryPlacement(o: { presence: string; approval: boolean }): "drop" | "queue" | "show" {
  if (o.presence === "indicator") return "drop";
  return o.approval ? "queue" : "show";
}
