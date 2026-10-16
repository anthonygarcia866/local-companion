// The "Draft email" card (voice 1.5d, core/draft.ts): To / Cc / Bcc as Rust
// resolved them (an address, chips to pick from, or nothing found), the
// subject, the body to read, and Create draft / Change recipient / Cancel.
// It stays open until one of those (decided 2026-10-16); the email is edited
// and sent in Outlook, never here.
//
// "Change recipient" opens a search of Outlook's address book; typing there is
// the only time this card borrows the keyboard, and it gives it back on a
// pick, on Cancel, or when the card closes.

import { h, clear } from "./dom";
import { card, stack, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { AskClock } from "../core/ask";
import { DRAFT_NOTE_MS, changeTarget, chipText, type DraftCard, type DraftRecipient } from "../core/draft";
import type { AskHost } from "./ask";
import { t } from "../i18n/i18n";

/** Runs only for the "created" note and a failure (the card itself stays). */
export const draftClock = new AskClock();
let focused = false;

function lendFocus(on: boolean) {
  if (focused === on) return;
  focused = on;
  void Bridge.focusWindow(on);
}

/** Closes the card (Cancel tells Rust; a note just goes). */
export function closeDraft(host: AskHost | null) {
  const d = State.draft;
  if (!d) return;
  draftClock.stop();
  lendFocus(false);
  if (d.phase === "open" && d.card) void Bridge.draftCancel(d.card.id);
  State.draft = null;
  if (host && State.view === "draft" && State.mode === "expanded") {
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

/** Why the last step didn't work, in words. */
export function draftCodeText(code: string): string {
  switch (code) {
    case "no-outlook": return t("Classic Outlook isn't installed or didn't start, so there's no draft.");
    case "no-account": return t("None of Outlook's accounts has your address. Check Settings → Email drafts.");
    case "no-sender": return t("Add your email address in Settings → Email drafts first.");
    case "outlook-failed": return t("Outlook couldn't save the draft. Try again.");
  }
  return "";
}

function lineLabel(line: DraftRecipient["line"]): string {
  return line === "cc" ? t("Cc") : line === "bcc" ? t("Bcc") : t("To");
}

export function buildDraft(host: AskHost): ViewHost {
  const title = h("div", { class: "title one-line" });
  const people = h("div", { class: "draft-people" });
  const subject = h("div", { class: "draft-subject one-line" });
  const body = h("div", { class: "rewrite-text draft-body" });
  const search = h("input", { class: "draft-search", type: "text", "data-role": "draft-search", maxlength: "80" }) as HTMLInputElement;
  const results = h("div", { class: "draft-results" });
  const searchBox = h("div", { class: "draft-search-box", style: "display:none" }, search, results);
  const note = h("div", { class: "suggestion-more", role: "status" });
  const row = h("div", { class: "rewrite-actions" });
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, people, subject, searchBox, body, note, row)));
  let typed: ReturnType<typeof setTimeout> | undefined;

  const id = () => State.draft?.card?.id ?? 0;
  const button = (label: string, role: string, onclick: () => void, cls = "apply-all") =>
    h("button", { class: cls, type: "button", "data-role": role, text: label, onclick }) as HTMLButtonElement;

  const change = (index: number) => {
    const d = State.draft;
    if (!d) return;
    State.draft = { ...d, choosing: false };
    void Bridge.draftChange(id(), index);
    lendFocus(true);
    window.setTimeout(() => search.focus(), 80);
  };
  search.addEventListener("input", () => {
    clearTimeout(typed);
    const q = search.value.trim();
    typed = setTimeout(() => {
      if (q) void Bridge.draftSearch(id(), q);
    }, 300);
  });

  const chip = (label: string, role: string, onclick: () => void) => button(label, role, onclick, "fix fix-btn draft-chip");

  const sync = () => {
    const d = State.draft;
    if (!d) return;
    clear(people);
    clear(results);
    clear(row);
    if (d.phase !== "open" || !d.card) {
      // A note: saved and opened, or Outlook couldn't be reached.
      title.textContent = t("Draft email");
      subject.textContent = "";
      body.style.display = "none";
      searchBox.style.display = "none";
      note.textContent = d.phase === "created"
        ? (d.dryRun ? t("Dry run: nothing was saved or opened.") : t("Saved in Outlook's Drafts and opened there. You send it from Outlook."))
        : draftCodeText(d.code);
      row.append(button(t("Close"), "close", () => closeDraft(host), "link-btn"));
      lendFocus(false);
      if (State.mode === "expanded") draftClock.start(`draft-${d.phase}`, () => closeDraft(host), DRAFT_NOTE_MS);
      return;
    }
    draftClock.stop();
    const c: DraftCard = d.card;
    title.textContent = d.dryRun ? t("Draft email (dry run)") : t("Draft email");
    c.recipients.forEach((r, i) => {
      const line = h("div", { class: "draft-line" }, h("span", { class: "draft-label", text: lineLabel(r.line) }));
      const f = r.found;
      if (d.choosing) {
        line.append(chip(f.kind === "one" ? chipText(f.contact) : r.said, "draft-which", () => change(i)));
      } else if (f.kind === "one") {
        line.append(h("span", { class: "draft-addr", text: chipText(f.contact) }));
      } else if (f.kind === "pick") {
        line.append(h("span", { class: "draft-ask", text: t("Which {name}?", { name: r.said }) }));
        f.choices.forEach((ch, k) => line.append(chip(chipText(ch), "draft-choice", () => void Bridge.draftPick(c.id, i, k))));
      } else {
        line.append(h("span", { class: "draft-ask", text: t("No “{name}” in your address book.", { name: r.said }) }));
      }
      people.append(line);
    });
    subject.textContent = c.subject ? t("Subject: {subject}", { subject: c.subject }) : t("No subject");
    body.style.display = "";
    body.textContent = c.body;
    // Change recipient's search.
    const searching = c.changing !== null;
    searchBox.style.display = searching ? "" : "none";
    if (searching) {
      const who = c.recipients[c.changing!]?.said ?? "";
      search.placeholder = t("Search your address book for {name}", { name: who });
      c.results.forEach((r, k) => results.append(chip(chipText(r), "draft-result", () => {
        void Bridge.draftPick(c.id, c.changing!, k);
        search.value = "";
        lendFocus(false);
      })));
    } else {
      search.value = "";
      lendFocus(false);
    }
    note.textContent = d.code ? draftCodeText(d.code)
      : d.choosing ? t("Click the name to change.")
      : d.ready ? t("Create draft saves it in Outlook's Drafts and opens it there. You send it yourself.")
      : t("Pick who it goes to first.");
    const create = button(t("Create draft"), "create", () => {
      if (!State.draft?.ready || State.draft.busy) return;
      State.draft = { ...State.draft, busy: true };
      void Bridge.draftCreate(c.id);
      sync();
    });
    create.disabled = !d.ready || d.busy;
    row.append(create, button(t("Change recipient"), "change", () => {
      const target = changeTarget(c);
      if (target !== null) change(target);
      else if (State.draft) {
        State.draft = { ...State.draft, choosing: true };
        sync();
      }
    }, "link-btn"), button(t("Cancel"), "cancel", () => closeDraft(host), "link-btn"));
  };
  return { el, sync };
}
