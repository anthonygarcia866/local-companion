// Push-to-talk dictation, the island's half (voice 1.5b,
// docs/voice-actions-plan.md §1, §4.1; Rust: src-tauri/src/voice). Rust sends
// every phase on "voice": "listening" (with the mic level, ~10 a second),
// "transcribing", "cleaning", then one result: "typed", "not-typed",
// "heard", "question" or "failed"; "idle" when nothing came of it (a press
// under 0.4 s). The island only shows them: the lantern while it runs, a card
// for a result that needs one.

import type { PresenceKind } from "../island/presence";

/** One event from Rust (voice::VoiceEvent). */
export interface VoiceEvent {
  phase: "listening" | "transcribing" | "cleaning" | "typed" | "not-typed" | "heard" | "question" | "failed" | "idle";
  /** 0..1, while listening. */
  level: number;
  /** "typed" / "not-typed": false = the raw transcript, not cleaned up. */
  cleaned: boolean;
  /** "heard" / "not-typed": the transcript (to copy); "question": Claude's question. */
  text: string;
  /** "failed" / "not-typed": why; "typed": why it wasn't cleaned up ("" = it was); "heard":
   *  that, or "command" / "no-field" (voice::heard_code). */
  code: string;
  /** The app typed into ("Word"). */
  app: string;
}

export type VoicePhase = Exclude<VoiceEvent["phase"], "idle">;

/** Where dictation is, as the island keeps it; null when nothing is going on. */
export interface VoiceState {
  phase: VoicePhase;
  level: number;
  cleaned: boolean;
  text: string;
  code: string;
  app: string;
  /** "copied" / "clipboard-busy" after a Copy click; "" before. */
  copied: string;
}

/** The phases while the key is held or the utterance is on its way. */
export function running(v: VoiceState | null): boolean {
  return v?.phase === "listening" || v?.phase === "transcribing" || v?.phase === "cleaning";
}

/** The results that open a card. A cleaned-up text typed where the user is
 *  needs none: the text in the field says it. */
export function needsCard(v: VoiceState | null): boolean {
  if (!v || running(v)) return false;
  return v.phase !== "typed" || !v.cleaned;
}

/** The state after an event from Rust. */
export function nextVoice(cur: VoiceState | null, e: VoiceEvent): VoiceState | null {
  if (e.phase === "idle") return null;
  if (e.phase === "listening" && cur?.phase === "listening") return { ...cur, level: clampLevel(e.level) };
  return {
    phase: e.phase,
    level: e.phase === "listening" ? clampLevel(e.level) : 0,
    cleaned: e.cleaned === true,
    text: e.text ?? "",
    code: e.code ?? "",
    app: e.app ?? "",
    copied: "",
  };
}

function clampLevel(n: number): number {
  return Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0;
}

/**
 * The lantern while dictation runs (plan §1-§2): s-hear while the key is
 * held (it wins over everything but a waiting approval, which keeps its
 * card), s-think while transcribing on this PC. Null: the lantern shows what
 * it would anyway (and while Claude cleans it up, `voiceSending`).
 */
export function voiceLantern(v: VoiceState | null): "hear" | "think" | null {
  switch (v?.phase) {
    case "listening": return "hear";
    case "transcribing": return "think";
  }
  return null;
}

/** The transcript is on its way to the Claude API: the lantern shows the
 *  data-leaving state, as for a rewrite (island.ts). */
export function voiceSending(v: VoiceState | null): boolean {
  return v?.phase === "cleaning";
}

/** The lantern's "not cleaned up" badge (plan §4.1): the raw transcript was
 *  typed. Shown while that result is (its card, ~8 s); never in the text. */
export function rawBadge(v: VoiceState | null): boolean {
  return v?.phase === "typed" && !v.cleaned;
}

/** How long a result card stays with nobody touching it. A text that wasn't
 *  typed stays longer: it's there to copy. */
export function cardTimeoutMs(v: VoiceState): number {
  return v.phase === "typed" ? 8_000 : 30_000;
}

/**
 * Where a result card goes (plan §1.7):
 * - "drop": no card for this result, or the island isn't there to show one
 *   (fullscreen's indicator).
 * - "queue": an approval card is up; the result waits behind it.
 * - "show": the card opens now. Hidden presence still shows it, as Hidden
 *   shows an approval (push-to-talk works with the island hidden).
 */
export function voicePlacement(o: { presence: PresenceKind; approval: boolean; v: VoiceState | null }): "drop" | "queue" | "show" {
  if (!needsCard(o.v) || o.presence === "indicator") return "drop";
  return o.approval ? "queue" : "show";
}
