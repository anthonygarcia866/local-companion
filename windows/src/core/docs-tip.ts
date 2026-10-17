// The Google Docs tip (1d retest, 2026-10-14; src-tauri/src/writing/docs_tip.rs).
// Docs draws its pages on a canvas, so without screen reader support Glim
// finds no text to check there. Rust offers the tip once per run, after 3 s
// in a Docs field with no text; "Got it" stores that it was dismissed and it
// never comes back. "Not now", or 30 s with no answer, just closes it.

import type { PresenceKind } from "../island/presence";

/** The card closes by itself after this long, as "Not now". */
export const DOCS_TIP_TIMEOUT_MS = 30_000;

/**
 * What to do with the tip when Rust offers it:
 * - "drop": not now (Hidden, Ember, fullscreen, paused, already showing, or
 *   the "Check writing in…" question is up). It may come back next run.
 * - "queue": an approval card is up; the tip waits behind it.
 * - "show": the card opens now.
 */
export function docsTipPlacement(o: {
  presence: PresenceKind;
  paused: boolean;
  approval: boolean;
  ask: boolean;
  showing: boolean;
}): "drop" | "queue" | "show" {
  if (o.showing || o.ask || o.paused || o.presence !== "pill") return "drop";
  return o.approval ? "queue" : "show";
}
