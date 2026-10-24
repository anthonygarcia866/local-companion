// The mini lantern's page (1e, docs/phase1-plan.md §7a): the real lantern
// (22 logical px, the suggest state with the count, the data-leaving state during a rewrite) and,
// after a click on it, the popover: the island's own suggestion rows, three a
// page, with Rewrite… and Close in the footer. Rust places the window and says
// where the two parts sit inside it (bubble_window.rs); this page never takes
// focus and registers no keys.
//
// The popover's rules are core/bubble.ts's state machine: a click toggles it,
// a new list starts on page one, an empty list closes it, the field left
// closes it, and 20 s with no pointer over it closes it.

import { h, clear } from "../views/dom";
import { suggestionRow } from "../views/views";
import { DELEGATE_STATE, Lantern } from "../mascot/lantern";
import { State, type WritingFix, type WritingSuggestion } from "../core/state";
import { BUBBLE_CLOSED, bubbleRows, stepBubble, type BubbleEvent, type BubbleState } from "../core/bubble";
import { t, tn } from "../i18n/i18n";

/** What Rust sends on "bubble-state" (bubble_window::State), logical px. */
export interface BubbleWindowState {
  visible: boolean;
  /** "suggest", or the rewrite's data-leaving look (anything else). */
  look: string;
  count: number;
  size: number;
  lanternX: number;
  lanternY: number;
  open: boolean;
  popoverX: number;
  popoverY: number;
  above: boolean;
}

/** The badge's text: "9+" past nine (§7a.2). */
export function bubbleBadge(count: number): string {
  return count > 9 ? "9+" : String(count);
}

/** What the page asks of Rust. */
export interface BubbleHost {
  /** The popover opened or closed: Rust lays the window out again. */
  setOpen(open: boolean): void;
  /** A replacement was clicked: row `index` of the whole list, option `choice`. */
  fix(checkId: number, index: number, choice: number): void;
  copy(checkId: number, index: number, choice: number): void;
  /** Ignore on row `index` (§7a.3). */
  ignore(checkId: number, index: number): void;
  /** Rewrite…: the island's tones, as Ctrl+Alt+Shift+R. */
  rewrite(): void;
  now(): number;
}

export class BubbleView {
  readonly el: HTMLElement;
  private lantern: Pick<Lantern, "el" | "show" | "showReserved" | "setHeight">;
  private lanternBox: HTMLElement;
  private badge: HTMLElement;
  private popover: HTMLElement;
  private title: HTMLElement;
  private list: HTMLElement;
  private range: HTMLElement;
  private prev: HTMLButtonElement;
  private next: HTMLButtonElement;
  private note: HTMLElement;
  state: BubbleState = BUBBLE_CLOSED;
  win: BubbleWindowState | null = null;

  private host: BubbleHost;

  /** `lantern`: the real one in the app; tests pass a stand-in (no innerHTML). */
  constructor(host: BubbleHost, lantern?: Pick<Lantern, "el" | "show" | "showReserved" | "setHeight">) {
    this.host = host;
    this.lantern = lantern ?? new Lantern({ detail: "notch", state: "idle", heightPx: 22 });
    this.badge = h("div", { class: "bubble-badge", role: "status", style: "display:none" });
    const label = t("Writing suggestions");
    this.lanternBox = h("button", { class: "bubble-lantern", type: "button", title: label, "aria-label": label, onclick: () => this.step({ kind: "lantern-click", now: this.host.now() }) },
      this.lantern.el as unknown as HTMLElement, this.badge);
    this.title = h("div", { class: "title one-line" });
    this.list = h("div", { class: "suggestion-list" });
    this.range = h("span", { class: "suggestion-range" });
    this.prev = h("button", { class: "suggestion-page", type: "button", text: "‹", title: t("Previous suggestions"), "aria-label": t("Previous suggestions"), onclick: () => this.step({ kind: "turn", by: -1 }) }) as HTMLButtonElement;
    this.next = h("button", { class: "suggestion-page", type: "button", text: "›", title: t("Next suggestions"), "aria-label": t("Next suggestions"), onclick: () => this.step({ kind: "turn", by: 1 }) }) as HTMLButtonElement;
    this.note = h("div", { class: "suggestion-more", role: "status" });
    const rewrite = h("button", { class: "rewrite-open", type: "button", "data-role": "bubble-rewrite", text: t("Rewrite…"), onclick: () => this.step({ kind: "rewrite" }) });
    const close = h("button", { class: "link-btn", type: "button", "data-role": "bubble-close", text: t("Close"), onclick: () => this.step({ kind: "close" }) });
    this.popover = h("div", { class: "bubble-popover", style: "display:none" },
      this.title, this.list,
      h("div", { class: "suggestion-pager" }, h("div", { class: "suggestion-pages" }, this.prev, this.range, this.next), this.note),
      h("div", { class: "bubble-footer" }, rewrite, close),
    );
    this.popover.addEventListener("mouseenter", () => this.step({ kind: "pointer", over: true, now: this.host.now() }));
    this.popover.addEventListener("mouseleave", () => this.step({ kind: "pointer", over: false, now: this.host.now() }));
    this.el = h("div", { class: "bubble" }, this.lanternBox, this.popover);
  }

  /** One popover event; tells Rust when it opened or closed. */
  step(e: BubbleEvent) {
    const was = this.state.open;
    const { state, effect } = stepBubble(this.state, e);
    this.state = state;
    if (effect === "rewrite") this.host.rewrite();
    if (state.open !== was) this.host.setOpen(state.open);
    this.draw();
  }

  /** Rust's placement and look. */
  setWindow(w: BubbleWindowState) {
    this.win = w;
    // Parked (hidden) or the field left: the popover goes.
    if (!w.visible && this.state.open) this.step({ kind: "field-left" });
    this.draw();
  }

  /** The island's list, as Rust sends it to both windows. */
  setList(items: WritingSuggestion[], fix: WritingFix) {
    State.writing = items;
    State.writingFix = fix;
    this.step({ kind: "list", items });
  }

  draw() {
    const w = this.win;
    const visible = !!w?.visible;
    this.lanternBox.style.display = visible ? "" : "none";
    if (!w || !visible) {
      this.popover.style.display = "none";
      return;
    }
    this.lanternBox.style.left = `${w.lanternX}px`;
    this.lanternBox.style.top = `${w.lanternY}px`;
    this.lanternBox.style.width = `${w.size}px`;
    this.lanternBox.style.height = `${w.size}px`;
    this.lantern.setHeight(w.size);
    // A rewrite of this field is on its way: data leaving the machine (§7a.2).
    const sending = w.look !== "suggest";
    if (sending) this.lantern.showReserved(DELEGATE_STATE, "api-request");
    else this.lantern.show("suggest");
    const count = sending ? 0 : w.count;
    this.badge.style.display = count > 0 ? "" : "none";
    this.badge.textContent = bubbleBadge(count);
    this.badge.setAttribute("aria-label", tn("{count} writing suggestion", "{count} writing suggestions", count));
    // The popover shows only once Rust has laid the window out for it.
    const open = this.state.open && w.open;
    this.popover.style.display = open ? "" : "none";
    if (!open) return;
    this.popover.style.left = `${w.popoverX}px`;
    this.popover.style.top = `${w.popoverY}px`;
    const items = State.writing;
    const at = bubbleRows(items, this.state.page);
    this.title.textContent = tn("{count} writing suggestion", "{count} writing suggestions", items.length);
    clear(this.list);
    at.rows.forEach((s, i) => this.list.append(suggestionRow(s, at.from - 1 + i, (index, choice) => this.pick(index, choice), (index) => this.host.ignore(State.writingFix.checkId, index))));
    this.range.textContent = t("{from}–{to} of {total}", { from: at.from, to: at.to, total: at.total });
    (this.range.parentElement as HTMLElement).style.display = at.paged ? "" : "none";
    this.prev.disabled = !at.canPrev;
    this.next.disabled = !at.canNext;
  }

  /** The note under the rows (a fix's outcome), from the island's wording. */
  setNote(text: string) {
    this.note.textContent = text;
  }

  private pick(index: number, choice: number) {
    const fix = State.writingFix;
    if (fix.mode === "apply") this.host.fix(fix.checkId, index, choice);
    else if (fix.mode === "copy") this.host.copy(fix.checkId, index, choice);
  }
}
