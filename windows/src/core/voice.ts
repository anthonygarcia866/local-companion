// Push-to-talk dictation, the island's half (voice 1.5b,
// docs/voice-actions-plan.md §1, §4.1; Rust: src-tauri/src/voice). Rust sends
// every phase on "voice": "listening" (with the mic level, ~10 a second),
// "transcribing", "cleaning", then one result: "typed", "not-typed",
// "heard", "question" or "failed"; "idle" when nothing came of it (a press
// under 0.4 s). The island only shows them: the lantern while it runs, a card
// for a result that needs one.
//
// Two keys (Decision A, 2026-10-16): Ctrl+Alt+Shift+V dictates, Ctrl+Alt+Shift+A
// asks Glim; the pill names the mode while one is held. While Glim's question
// card waits, the next utterance from either key answers it (Decision B): the
// card stays up, "Listening for your answer…".
//
// Glim learns (docs/glim-learns-plan.md §1-§2): "learned" (a learned shortcut
// ran, Claude wasn't asked: Try again), "learned-new" (Claude's open_app
// worked and was just kept: "Learned: say “…” any time"), "routine-ask" (Save / Cancel on a
// routine's steps), "routine-saved", "routine" (a routine ran, or stopped at
// a step: code "ok" or "failed:k/n").
// Glim learns §3: "remembered" (what Glim kept, or a "failed" memory-… code
// saying why not), and the correction tip: `tip` on a result card, or a
// "tip" card of its own.

import type { PresenceKind } from "../island/presence";
import { N_, t } from "../i18n/i18n";

/** One event from Rust (voice::VoiceEvent). */
/** One step of a routine, or what a shortcut does (learn::StepView). */
export interface StepView {
  tool: "open_app" | "resume_work" | "window";
  /** The app's or the repo's name; "" = every repo. */
  name: string;
  /** window: left | right | maximize | minimize | restore | center. */
  where?: string;
  /** window: "other" = the other screen. */
  display?: string;
}

export interface VoiceEvent {
  phase: "listening" | "transcribing" | "cleaning" | "typed" | "not-typed" | "heard" | "question" | "failed" | "idle"
    | "learned" | "learned-new" | "routine-ask" | "routine-saved" | "routine" | "remembered" | "tip"
    | "watching" | "watched" | "watch-expired" | "watch-stopped";
  /** 0..1, while listening. */
  level: number;
  /** "typed" / "not-typed": false = the raw transcript, not cleaned up. */
  cleaned: boolean;
  /** "heard" / "not-typed": the transcript (to copy); "question": Claude's question. */
  text: string;
  /** "failed" / "not-typed": why; "typed": why it wasn't cleaned up ("" = it was); "heard":
   *  that, or "command" / "no-field" (voice::heard_code). */
  code: string;
  /** The app typed into ("Word"); "remembered": the learned shortcuts the rule removed, quoted. */
  app: string;
  /** Which key ("dictate" / "ask"), or "answer" while a question card waits. */
  mode?: VoiceMode;
  /** "routine-ask" / "routine": the routine's steps. */
  steps?: StepView[];
  /** The user just corrected Glim: the card adds the "Remember" tip. */
  tip?: boolean;
}

export type VoiceMode = "dictate" | "ask" | "answer";

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
  mode: VoiceMode;
  /** The question being answered, while answering one (Decision B); "" otherwise. */
  question: string;
  /** A routine's steps (Glim learns §2); [] otherwise. */
  steps: StepView[];
  /** The correction tip goes on this card (Glim learns §3). */
  tip: boolean;
}

/** The phases while the key is held or the utterance is on its way. */
export function running(v: VoiceState | null): boolean {
  return v?.phase === "listening" || v?.phase === "transcribing" || v?.phase === "cleaning";
}

/** An answer to Glim's question is on its way (spoken, or typed into the card). */
export function answering(v: VoiceState | null): boolean {
  return !!v && running(v) && v.mode === "answer" && v.question !== "";
}

/** The results that open a card. A cleaned-up text typed where the user is
 *  needs none: the text in the field says it. The question card stays while
 *  its answer is on the way. */
export function needsCard(v: VoiceState | null): boolean {
  if (!v) return false;
  if (running(v)) return answering(v);
  return v.phase !== "typed" || !v.cleaned;
}

/** The state after an event from Rust. */
export function nextVoice(cur: VoiceState | null, e: VoiceEvent): VoiceState | null {
  if (e.phase === "idle") return null;
  if (e.phase === "listening" && cur?.phase === "listening") return { ...cur, level: clampLevel(e.level) };
  const mode: VoiceMode = e.mode === "ask" || e.mode === "answer" ? e.mode : "dictate";
  const asked = cur?.phase === "question" ? cur.text : (cur?.question ?? "");
  const runningPhase = e.phase === "listening" || e.phase === "transcribing" || e.phase === "cleaning";
  return {
    phase: e.phase,
    level: e.phase === "listening" ? clampLevel(e.level) : 0,
    cleaned: e.cleaned === true,
    text: e.text ?? "",
    code: e.code ?? "",
    app: e.app ?? "",
    copied: "",
    mode,
    question: runningPhase && mode === "answer" ? asked : "",
    steps: Array.isArray(e.steps) ? e.steps : [],
    tip: e.tip === true,
  };
}

/** The pill's label while a voice key is held (Decision A); null otherwise.
 *  Answering a question shows on its card instead. */
export function modeLabel(v: VoiceState | null): "Dictating" | "Ask Glim" | null {
  if (v?.phase !== "listening") return null;
  if (v.mode === "dictate") return N_("Dictating");
  if (v.mode === "ask") return N_("Ask Glim");
  return null;
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

/**
 * The level meter along the pill's bottom edge while the key is held (plan
 * §1.2: it shows Glim is actually hearing something): its width as a share of
 * the pill, from a sliver at silence to 60% at full voice; null when not
 * listening.
 */
export function meterPercent(v: VoiceState | null): number | null {
  if (v?.phase !== "listening") return null;
  return Math.round(6 + Math.min(1, Math.max(0, v.level)) * 54);
}

/** The lantern's "not cleaned up" badge (plan §4.1): the raw transcript was
 *  typed. Shown while that result is (its card, ~8 s); never in the text. */
export function rawBadge(v: VoiceState | null): boolean {
  return v?.phase === "typed" && !v.cleaned;
}

/** How long a result card stays with nobody touching it. A text that wasn't
 *  typed stays longer: it's there to copy. */
export function cardTimeoutMs(v: VoiceState): number {
  if (v.phase === "question" || v.phase === "routine-ask") return 60_000;
  return v.phase === "typed" || v.phase === "learned" || v.phase === "learned-new" || v.phase === "watching" || v.phase === "watch-stopped" || v.phase === "routine-saved" || v.phase === "remembered" || v.phase === "tip" ? 8_000 : 30_000;
}

/** The window action (Rust window_tool.rs), in words. */
function windowText(where: string, app: string): string {
  switch (where) {
    case "left": return t("Put {app} on the left", { app });
    case "right": return t("Put {app} on the right", { app });
    case "maximize": return t("Maximize {app}", { app });
    case "minimize": return t("Minimize {app}", { app });
    case "center": return t("Center {app}", { app });
  }
  return t("Restore {app}", { app });
}

/** A routine's step or a shortcut's call, in words. */
export function stepText(s: StepView): string {
  if (s.tool === "open_app") return t("Open {app}", { app: s.name });
  if (s.tool === "window") {
    const step = windowText(s.where ?? "", s.name);
    return s.display === "other" ? t("{step}, on the other screen", { step }) : step;
  }
  return s.name ? t("Pick up {repo}", { repo: s.name }) : t("Pick up where you left off");
}

/** "failed:2/3" → [2, 3]; null for "ok" or anything else. */
export function stoppedAt(code: string): [number, number] | null {
  const m = /^failed:(\d+)\/(\d+)$/.exec(code);
  return m ? [Number(m[1]), Number(m[2])] : null;
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
