// The mini lantern's popover (1e, docs/phase1-plan.md §7a.3): its state
// machine only. No window, no DOM: the window glue (the next 1e step, after
// the owner's live session) feeds it events and acts on its effect.
//
// Decided 2026-10-13 (owner): the footer has "Rewrite…", and the popover
// closes after 20 s with no pointer over it.
//
// - Opens and closes on a click on the mini lantern; Close closes it.
// - The rows are the checker's suggestions as the island has them (Rust
//   sends them in reading order); three a page, as the island's list.
// - A new list (a new check) starts again on the first page; an empty one
//   (the last fixed or ignored) closes it.
// - The caret leaving the field, or the field losing focus, closes it.
// - 20 s without the pointer over it closes it; the clock starts when it
//   opens and again each time the pointer leaves, and never runs while the
//   pointer is over it.
// - "Rewrite…" closes it and asks for the island's Rewrite card: the same
//   path as Ctrl+Alt+Shift+R (Rust's `shortcuts::dispatch(app, "rewrite")`),
//   so nothing about 1c changes and nothing is sent until a tone is picked.

import { suggestionPage } from "./paging";

/** No pointer over the open popover for this long closes it. */
export const BUBBLE_AUTO_CLOSE_MS = 20_000;

export interface BubbleState {
  open: boolean;
  page: number;
  /** The list the page belongs to (compared by identity, as the island does). */
  listOf: readonly unknown[] | null;
  pointerOver: boolean;
  /** When the no-pointer clock started (ms); meaningful while open and not over. */
  idleSince: number;
}

export type BubbleEvent =
  | { kind: "lantern-click"; now: number }
  | { kind: "close" }
  | { kind: "turn"; by: number }
  | { kind: "pointer"; over: boolean; now: number }
  | { kind: "tick"; now: number }
  | { kind: "list"; items: readonly unknown[] }
  | { kind: "field-left" }
  | { kind: "rewrite" };

/** What the window glue must do after a step. */
export type BubbleEffect = "rewrite" | null;

export const BUBBLE_CLOSED: BubbleState = { open: false, page: 0, listOf: null, pointerOver: false, idleSince: 0 };

function closed(s: BubbleState): BubbleState {
  return { ...s, open: false, pointerOver: false };
}

export function stepBubble(s: BubbleState, e: BubbleEvent): { state: BubbleState; effect: BubbleEffect } {
  const done = (state: BubbleState, effect: BubbleEffect = null) => ({ state, effect });
  switch (e.kind) {
    case "lantern-click":
      if (s.open) return done(closed(s));
      // Nothing to show: it doesn't open (the lantern only shows with
      // suggestions, but a click can race the last fix).
      if (s.listOf !== null && s.listOf.length === 0) return done(s);
      return done({ ...s, open: true, page: 0, pointerOver: false, idleSince: e.now });
    case "close":
    case "field-left":
      return done(s.open ? closed(s) : s);
    case "turn": {
      if (!s.open) return done(s);
      const at = suggestionPage(s.listOf?.length ?? 0, s.page + e.by);
      return done({ ...s, page: at.page });
    }
    case "pointer":
      if (!s.open) return done(s);
      return done({ ...s, pointerOver: e.over, idleSince: e.over ? s.idleSince : e.now });
    case "tick":
      if (s.open && !s.pointerOver && e.now - s.idleSince >= BUBBLE_AUTO_CLOSE_MS) return done(closed(s));
      return done(s);
    case "list": {
      const fresh = e.items !== s.listOf;
      const next = { ...s, listOf: e.items, page: fresh ? 0 : s.page };
      return done(e.items.length === 0 && s.open ? closed(next) : next);
    }
    case "rewrite":
      return done(closed(s), "rewrite");
  }
}

/** The rows the open popover shows, in the list's (reading) order, with the
 * "1–3 of 12" numbers and whether Previous / Next can be pressed. */
export function bubbleRows<T>(items: readonly T[], page: number) {
  const at = suggestionPage(items.length, page);
  return {
    rows: items.slice(Math.max(0, at.from - 1), at.to),
    from: at.from,
    to: at.to,
    total: items.length,
    canPrev: at.page > 0,
    canNext: at.page < at.pages - 1,
    paged: at.pages > 1,
  };
}
