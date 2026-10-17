// A box on a card borrows the keyboard: the question card's answer (Decision
// B) and the draft card's address-book search (1.5d). The island never takes
// focus by itself; a click in one of these boxes lends it the keyboard, and
// Send, Cancel, a pick or the card closing gives it back to the window the
// user was typing in before (Rust remembers which, lend_keyboard).
//
// One borrower at a time: a card only gives back what it borrowed, so the
// voice card closing never takes the keyboard from the draft card's search.

import { Bridge } from "./bridge";

export type Borrower = "voice" | "draft";

let holder: Borrower | null = null;

/** Who has the keyboard now, if anyone. */
export function keyboardHolder(): Borrower | null {
  return holder;
}

/** `who` takes the keyboard (a click in its box). Asked every time: the island
 *  folding away releases focus on its own, and Rust keeps the window to give
 *  back only when it isn't the island itself, so asking twice is harmless. */
export function borrowKeyboard(who: Borrower) {
  holder = who;
  void Bridge.lendKeyboard(true);
}

/** `who` is done typing: the keyboard goes back, if `who` had it. */
export function returnKeyboard(who: Borrower) {
  if (holder !== who) return;
  holder = null;
  void Bridge.lendKeyboard(false);
}
