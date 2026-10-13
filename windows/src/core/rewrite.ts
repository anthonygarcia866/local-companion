// The Rewrite card (1c, docs/phase1-plan.md §13): its states, and how Rust's
// "writing-rewrite" events move it along. Rust sends phases "sending",
// "done", "failed", "stale" (the answer came for a rewrite no longer asked
// for) and "result" (what "Use this" or "Copy" did).

import { State, type RewriteState } from "./state";
import { Bridge } from "./bridge";

/** The tones, in the card's order; the ids are Rust's (ai/claude.rs Tone).
 *  Casual came with 1d (docs/phase1-plan.md §14.3). */
export const TONES = ["friendlier", "formal", "shorter", "grammar", "casual"] as const;
export type ToneId = (typeof TONES)[number];

export function isTone(s: string): s is ToneId {
  return (TONES as readonly string[]).includes(s);
}

/** The card's tones with the field's default first (1d). */
export function orderedTones(defaultTone: string): ToneId[] {
  return isTone(defaultTone) ? [defaultTone, ...TONES.filter((t) => t !== defaultTone)] : [...TONES];
}

/** One event from Rust. */
export interface RewriteEvent {
  phase: "sending" | "done" | "failed" | "stale" | "result";
  id: number;
  tone: string;
  text: string;
  mode: RewriteState["mode"];
  app: string;
  offerable: boolean;
  model: string;
  costUsd: number;
  code: string;
  retryAfter: number | null;
  outcome: string;
}

/** The card as the hotkey or the Rewrite button opens it. */
export function chooseTone(defaultTone = ""): RewriteState {
  return {
    phase: "choose", id: 0, tone: "", text: "", mode: "view", app: "", offerable: false,
    model: "", costUsd: 0, code: "", retryAfter: null, outcome: "", busy: false, defaultTone,
  };
}

/** A tone was clicked: waiting for Rust (id 0 until it takes the request). */
export function sending(tone: string, defaultTone = ""): RewriteState {
  return { ...chooseTone(defaultTone), phase: "sending", tone };
}

/**
 * Opens the tone card, and asks Rust for the field's default tone (first
 * and highlighted, §14.3). Nothing is sent.
 */
export function openToneCard() {
  State.rewrite = chooseTone(State.rewrite?.defaultTone ?? "");
  void Bridge.writingRewriteDefault().then((tone) => {
    const r = State.rewrite;
    if (!tone || !isTone(tone) || r?.phase !== "choose") return;
    State.rewrite = { ...r, defaultTone: tone };
    State.notify();
  });
}

/** A tone goes to Rust: the card waits for the answer. */
export function sendTone(tone: string) {
  if (State.rewrite?.phase === "sending") return;
  State.rewrite = sending(tone, State.rewrite?.defaultTone ?? "");
  void Bridge.writingRewrite(tone);
  State.notify();
}

/**
 * The rewrite hotkey (owner, 2026-10-12): the first press opens the tone
 * card; a press while it is open on the tones sends the default tone.
 * True when it sent.
 */
export function rewriteKey(cardOpen: boolean): boolean {
  const r = State.rewrite;
  if (cardOpen && r?.phase === "choose" && isTone(r.defaultTone)) {
    sendTone(r.defaultTone);
    return true;
  }
  openToneCard();
  return false;
}

/**
 * The card after an event. A closed card stays closed (Rust was told, and
 * drops the rewrite); events for another rewrite than the one on the card
 * are ignored.
 */
export function nextRewrite(cur: RewriteState | null, e: RewriteEvent): RewriteState | null {
  if (!cur) return null;
  const ours = cur.id === 0 || cur.id === e.id;
  switch (e.phase) {
    case "sending":
      return cur.phase === "sending" && cur.id === 0 ? { ...cur, id: e.id, model: e.model } : cur;
    case "done":
    case "failed":
      if (cur.phase !== "sending" || !ours) return cur;
      return {
        phase: e.phase, id: e.id, tone: cur.tone, text: e.text, mode: e.mode, app: e.app, offerable: e.offerable,
        model: e.model || cur.model, costUsd: e.costUsd, code: e.code, retryAfter: e.retryAfter, outcome: "", busy: false,
        defaultTone: cur.defaultTone,
      };
    case "stale":
      return cur.phase === "sending" && cur.id === e.id ? chooseTone(cur.defaultTone) : cur;
    case "result":
      return cur.phase === "done" && cur.id === e.id ? { ...cur, outcome: e.outcome, app: e.app || cur.app, busy: false } : cur;
  }
  return cur;
}

/** The lantern shows its "data leaving" state while text is on its way to the API: from
 *  Rust taking the request until the answer is in. */
export function sendingToApi(r: RewriteState | null): boolean {
  return r?.phase === "sending" && r.id !== 0;
}

/** "about $0.0001", or "less than $0.0001" for less. */
export function costText(usd: number): string {
  if (!(usd > 0)) return "$0";
  if (usd < 0.0001) return "<$0.0001";
  return `$${usd < 0.01 ? usd.toFixed(4) : usd.toFixed(2)}`;
}
