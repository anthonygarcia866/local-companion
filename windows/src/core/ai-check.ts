// "Check with AI" (docs/phase1-plan.md §15.1; src-tauri/src/ai/check.rs).
// A click at the foot of the suggestions list (or, with the setting on, a
// pause in typing) sends the paragraph to Claude (Haiku). Rust says what is
// happening on "writing-ai-check": phases and numbers only. The AI's
// suggestions themselves arrive with Harper's on "writing-suggestions",
// marked kind "AI".

import { t, tn } from "../i18n/i18n";
import { failureText } from "../views/rewrite";

/** The suggestion kind Rust gives the AI's findings (writing::AI_KIND). */
export const AI_KIND = "AI";

export interface AiCheckEvent {
  phase: "sending" | "done" | "failed";
  id: number;
  /** The automatic check (the setting), not a click. */
  auto: boolean;
  /** "done": how many suggestions the AI added. */
  added: number;
  /** "failed": why. */
  code: string;
  retryAfter: number | null;
}

/** Text is on its way to the Claude API: the lantern shows the reserved data-leaving state. */
export function aiSending(e: AiCheckEvent | null): boolean {
  return e?.phase === "sending";
}

/**
 * What the island keeps from an event. A failed automatic check says nothing
 * (it wasn't asked for; the log has it): only the lantern stops delegating.
 */
export function nextAiCheck(e: AiCheckEvent): AiCheckEvent | null {
  return e.phase === "failed" && e.auto ? null : e;
}

/** The line under the list for the last check; "" for none. */
export function aiCheckNote(e: AiCheckEvent | null): string {
  if (!e) return "";
  if (e.phase === "sending") return t("Checking with Claude…");
  if (e.phase === "done") {
    return e.added > 0
      ? tn("Claude found {count} more suggestion.", "Claude found {count} more suggestions.", e.added)
      : t("Claude found nothing more to fix.");
  }
  switch (e.code) {
    case "no-paragraph":
    case "empty": return t("Click in the paragraph you want checked first.");
    case "too-long": return t("That paragraph is too long to check (over 2,000 characters).");
    case "changed": return t("The paragraph changed while Claude was checking. Check again.");
    case "refused": return t("Claude declined to check this.");
    case "truncated":
    case "bad-answer": return t("Claude's answer couldn't be used. Try again.");
    default: return failureText(e.code, e.retryAfter);
  }
}
