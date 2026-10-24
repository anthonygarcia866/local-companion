// Settings → Calendar (calendar glance; core/calendar.ts, Rust calendar/):
// on/off (off until the owner's live test passes), how many minutes before
// an event the lantern glows, all-day events hidden, the morning summary
// (off by default), and which of Outlook's calendars. Glim only reads the
// calendar, only while classic Outlook is running, and never starts it.

import { h, clear } from "../views/dom";
import { Bridge, onEvent } from "../core/bridge";
import { DEFAULT_LEAD_MIN, LEAD_CHOICES } from "../core/calendar";
import type { Settings } from "../core/state";
import { t } from "../i18n/i18n";

export interface CalendarHost {
  settings(): Settings;
  save(): Promise<void>;
}

type Flag = "calendarOn" | "calendarHideAllDay" | "calendarSummary";

/** An on/off row bound to a boolean setting. */
function switchRow(host: CalendarHost, flag: Flag, label: string, role: string): HTMLElement {
  const on = host.settings()[flag] === true;
  const sw = h("button", { class: on ? "switch on" : "switch", type: "button", "aria-pressed": String(on), "data-role": role, "aria-label": label }) as HTMLButtonElement;
  sw.addEventListener("click", () => {
    const next = !sw.classList.contains("on");
    sw.classList.toggle("on", next);
    sw.setAttribute("aria-pressed", String(next));
    host.settings()[flag] = next;
    void host.save();
  });
  return h("div", { class: "row" }, h("label", { text: label }), sw);
}

/** The Calendar section's list loader (the newest section built). */
let reloadList: (() => void) | null = null;
let listening = false;

/** Asks Rust for Outlook's calendars again and redraws the list. */
export function reloadCalendars(): void {
  reloadList?.();
}

/** The calendars to read: empty = the default one. Checking only the default keeps it empty. */
export function chosenIds(checked: string[], defaultId: string | null): string[] {
  if (checked.length === 0) return [];
  return checked.length === 1 && checked[0] === defaultId ? [] : checked;
}

export function calendarSection(host: CalendarHost): HTMLElement {
  const lead = h("select", { id: "calendar-lead", "data-role": "calendar-lead" }) as HTMLSelectElement;
  for (const m of LEAD_CHOICES) lead.append(h("option", { value: String(m), text: t("{minutes} minutes before", { minutes: String(m) }) }));
  const cur = host.settings().calendarLeadMin;
  lead.value = String(LEAD_CHOICES.includes(cur as (typeof LEAD_CHOICES)[number]) ? cur : DEFAULT_LEAD_MIN);
  lead.addEventListener("change", () => {
    host.settings().calendarLeadMin = Number(lead.value) || DEFAULT_LEAD_MIN;
    void host.save();
  });

  // Outlook's calendars (none if Outlook isn't running). Asked when the page
  // is built (at launch), and again when Settings opens, when Calendar is
  // picked in the sidebar, and when Rust sees the count change (Outlook
  // ready after a retry, started later, or closed): reloadCalendars().
  const list = h("div", { class: "calendar-folders", "data-role": "calendar-folders" });
  const status = h("div", { class: "hint", "data-role": "calendar-folders-status", text: t("Looking for Outlook's calendars…") });
  let asked = 0;
  const load = () => {
    const mine = ++asked;
    void Bridge.calendarCalendars().then((found) => {
      if (mine !== asked) return; // a newer ask is on its way
      fill(found);
    });
  };
  const fill = (found: { id: string; name: string; mailbox?: string; default: boolean }[] | null) => {
    clear(list);
    const folders = Array.isArray(found) ? found : [];
    if (folders.length === 0) {
      status.textContent = t("Open classic Outlook to choose calendars. Glim never starts it for this.");
      return;
    }
    status.textContent = t("The default calendar is used when none is picked.");
    const defaultId = folders.find((f) => f.default)?.id ?? null;
    const picked = host.settings().calendarIds ?? [];
    const boxes: [string, HTMLInputElement][] = [];
    for (const f of folders) {
      const box = h("input", { type: "checkbox", "data-role": "calendar-folder" }) as HTMLInputElement;
      box.checked = picked.length ? picked.includes(f.id) : f.default;
      box.addEventListener("change", () => {
        host.settings().calendarIds = chosenIds(boxes.filter(([, b]) => b.checked).map(([id]) => id), defaultId);
        void host.save();
      });
      boxes.push([f.id, box]);
      // One calendar per row: its name (the default marked), then the
      // mailbox it belongs to. Shown here only, never logged.
      list.append(
        h("label", { class: "check-row calendar-row", "data-role": "calendar-row" },
          box,
          h("span", { class: "calendar-name", text: f.default ? t("{name} (default)", { name: f.name }) : f.name }),
          f.mailbox ? h("span", { class: "hint calendar-mailbox", text: f.mailbox }) : null,
        ),
      );
    }
  };
  reloadList = load;
  if (!listening) {
    listening = true;
    void onEvent<number>("calendar-folders", () => reloadCalendars());
    void onEvent("settings-opened", () => reloadCalendars());
  }
  load();

  return h("section", {},
    h("h2", {}, h("span", { text: t("Calendar") })),
    h("div", { class: "hint", text: t("Your next Outlook event on the island, and the lantern glows before it. Glim only reads classic Outlook's calendar on this PC, while Outlook is open; nothing is changed, answered or sent, and titles are never logged.") }),
    switchRow(host, "calendarOn", t("Show my next event"), "calendar-on"),
    h("div", { class: "row" }, h("label", { text: t("Glow before an event"), for: "calendar-lead" }), lead),
    switchRow(host, "calendarHideAllDay", t("Hide all-day events"), "calendar-hide-all-day"),
    switchRow(host, "calendarSummary", t("Morning summary of the day"), "calendar-summary"),
    h("div", { class: "row" }, h("label", { text: t("Calendars") })),
    list,
    status,
  );
}
