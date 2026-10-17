// The one-time "Check writing in [app]?" prompt (1d, docs/phase1-plan.md
// §14.2). Rust offers the question ("writing-ask") after 2 s in an editable
// field of an app on neither list; the island decides whether it can be seen
// now, and the answer goes back as the app's exe and a word, nothing else.

import type { PresenceKind } from "../island/presence";

/** The question: the exe the answer is stored under, and the app's name. */
export interface WritingAsk {
  exe: string;
  name: string;
}

export type AskAnswer = "yes" | "later" | "never";

/** The card closes by itself after this long, which counts as "Not now". */
export const ASK_TIMEOUT_MS = 30_000;

/**
 * What to do with a question from Rust:
 * - "drop": not now (Hidden, fullscreen, recording indicator, paused, or
 *   another app's question is up). Nothing is recorded, so it is asked
 *   again the next time the user types there with Glim in sight.
 * - "keep": this app's question is already up.
 * - "ember": the ember shows "?"; clicking it opens the card.
 * - "queue": an approval or question card is up; this one waits behind it.
 * - "show": the card opens now.
 */
export function askPlacement(o: {
  presence: PresenceKind;
  paused: boolean;
  approval: boolean;
  current: WritingAsk | null;
  incoming: WritingAsk;
}): "drop" | "keep" | "ember" | "queue" | "show" {
  if (o.current) return o.current.exe.toLowerCase() === o.incoming.exe.toLowerCase() ? "keep" : "drop";
  if (o.paused || o.presence === "hidden" || o.presence === "indicator") return "drop";
  if (o.presence === "ember") return "ember";
  return o.approval ? "queue" : "show";
}

/** The card's clock: 30 s from when the question is first in sight. */
export class AskClock {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private exe = "";

  /** Starts it for `exe`, unless it already runs for that app. */
  start(exe: string, fire: () => void, ms = ASK_TIMEOUT_MS) {
    if (this.timer !== undefined && this.exe === exe) return;
    this.stop();
    this.exe = exe;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      fire();
    }, ms);
  }

  stop() {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.exe = "";
  }

  running(): boolean {
    return this.timer !== undefined;
  }
}
