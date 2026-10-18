// Push-to-talk's result card (voice 1.5b, core/voice.ts,
// docs/voice-actions-plan.md §4.1): what was typed without the clean-up,
// what was heard but not acted on (with Copy), Claude's question, or why
// nothing happened. The island never takes focus, so the user stays in the
// field they were dictating into; only a click in the question card's answer
// box gives it the keyboard (Decision B: an answer can be typed as well as
// spoken), and Send, Cancel, a spoken answer or the card closing gives it back
// to the user's app (core/keyboard.ts).

import { h, clear } from "./dom";
import { card, stack, outcomeNote, type ViewHost } from "./views";
import { failureText } from "./rewrite";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { AskClock } from "../core/ask";
import { answering, cardTimeoutMs, type VoiceState } from "../core/voice";
import { borrowKeyboard, returnKeyboard } from "../core/keyboard";
import type { AskHost } from "./ask";
import { t } from "../i18n/i18n";

/** Runs while the card is in sight. */
export const voiceClock = new AskClock();
let shown = 0;

/** Closes the card: back to what else is waiting, or out of the way. A
 *  question card closing stops Rust waiting for its answer, so the voice keys
 *  go back to dictating and asking. */
export function closeVoice(host: AskHost | null) {
  if (!State.voice) return;
  voiceClock.stop();
  returnKeyboard("voice");
  const wasQuestion = State.voice.phase === "question" || answering(State.voice);
  State.voice = null;
  if (wasQuestion) void Bridge.voiceCancel();
  if (host && State.view === "voice" && State.mode === "expanded") {
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

/** A new result arrived: its own clock (the old card's is dropped). */
export function voiceShown() {
  shown += 1;
  voiceClock.stop();
}

/** Why the text wasn't cleaned up, from the clean-up's failure code. */
export function notCleanedText(code: string): string {
  if (code === "timeout") return t("Claude didn't answer in time, so it wasn't cleaned up.");
  return t("Not cleaned up: {why}", { why: failureText(code, null) });
}

/** The card's title and lines for a result. */
export function voiceText(v: VoiceState): { title: string; body: string; note: string } {
  switch (v.phase) {
    case "typed":
      return { title: t("Typed in {app}, not cleaned up", { app: v.app }), body: "", note: notCleanedText(v.code) };
    case "not-typed":
      return {
        title: t("Not typed"),
        body: v.text,
        note: outcomeNote(v.code, v.app) ?? t("Glim couldn't type it. Copy it and paste it yourself."),
      };
    case "heard":
      return {
        title: t("Here's what I heard (couldn't act on it)"),
        body: v.text,
        note: heardNote(v.code),
      };
    case "question":
      return { title: v.text, body: "", note: t("Hold either voice shortcut and say your answer, or type it below.") };
    case "failed":
      return { title: v.code === "open-failed" ? t("Not opened") : t("Nothing was typed"), body: "", note: failedText(v.code, v.app) };
  }
  return { title: "", body: "", note: "" };
}

/** The question card while its answer is on the way (Decision B). */
export function answeringNote(v: VoiceState): string {
  return v.phase === "listening" ? t("Listening for your answer…") : t("Working on your answer…");
}

/** Why something heard wasn't acted on (voice::heard_code). */
export function heardNote(code: string): string {
  if (code === "command") return t("Glim can't do that yet, so nothing happened.");
  if (code === "command-words") return t("Not typed: without the clean-up, words that start like a command aren't typed. Copy it if you meant it.");
  if (code === "no-field" || !code) return t("Nothing was typed. Click in a text field first to dictate.");
  return notCleanedText(code);
}

/** Why nothing happened, for a "failed" code from Rust (voice/run.rs, mic.rs, whisper.rs). */
export function failedText(code: string, app = ""): string {
  switch (code) {
    case "nothing-heard": return t("Glim didn't catch any words. Hold the shortcut while you speak.");
    case "no-mic": return t("No microphone found.");
    case "mic-blocked": return t("Windows is blocking the microphone, or another app is using it. Allow Glim in the microphone settings.");
    case "no-whisper": return t("Dictation needs the speech model on this PC. See Settings → Writing → Dictation.");
    case "whisper-start":
    case "whisper": return t("Speech-to-text didn't work this time. Try again.");
    case "sensitive": return failureText("sensitive", null);
    case "open-failed": return t("Couldn't open {app}. Is it installed?", { app });
  }
  return t("Speech-to-text didn't work this time. Try again.");
}

export function buildVoice(host: AskHost): ViewHost {
  const title = h("div", { class: "title one-line" });
  const body = h("div", { class: "rewrite-text" });
  const note = h("div", { class: "suggestion-more", role: "status" });
  const row = h("div", { class: "rewrite-actions" });
  // Decision B: an answer typed into the question card (Enter or Send).
  const answer = h("input", {
    type: "text",
    class: "voice-answer",
    "data-role": "voice-answer",
    maxlength: "1000",
    spellcheck: "false",
    placeholder: t("Type your answer"),
    onkeydown: (e: Event) => {
      if ((e as KeyboardEvent).key === "Enter") void sendTyped();
    },
    // The island never takes focus by itself: a click here borrows the
    // keyboard, or the keys land in the app behind (retest, 2026-10-17).
    onpointerdown: () => {
      borrowKeyboard("voice");
      window.setTimeout(() => answer.focus(), 80);
    },
  }) as HTMLInputElement;
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, body, note, answer, row)));
  let clockFor = -1;

  const sendTyped = async () => {
    const v = State.voice;
    const text = answer.value.trim();
    if (!v || v.phase !== "question" || !text) return;
    answer.value = "";
    returnKeyboard("voice");
    // Rust takes it from here; its "cleaning" phase keeps this card up.
    const ok = await Bridge.voiceAnswer(text);
    if (!ok && State.voice === v) closeVoice(host);
  };

  const copy = async () => {
    const v = State.voice;
    if (!v) return;
    const ok = await Bridge.voiceCopy(v.text);
    if (State.voice === v) {
      State.voice = { ...v, copied: ok ? "copied" : "clipboard-busy" };
      sync();
    }
  };

  const sync = () => {
    const v = State.voice;
    if (!v) return;
    const waiting = answering(v);
    const words = waiting ? { title: v.question, body: "", note: answeringNote(v) } : voiceText(v);
    title.textContent = words.title;
    // A question is never cut off: it wraps (retest, 2026-10-16).
    const asking = v.phase === "question" || waiting;
    title.classList.toggle("one-line", !asking);
    title.classList.toggle("voice-question", asking);
    body.textContent = words.body;
    body.style.display = words.body ? "" : "none";
    note.textContent = v.copied ? (outcomeNote(v.copied, v.app) ?? "") : words.note;
    answer.style.display = v.phase === "question" ? "" : "none";
    // No box to type in (an answer spoken, or not a question): give it back.
    if (v.phase !== "question") returnKeyboard("voice");
    clear(row);
    if (asking) {
      // The answer is on its way, or awaited: Send (typed) and Cancel.
      if (!waiting) row.append(h("button", { class: "apply-all", type: "button", "data-answer": "send", text: t("Send"), onclick: () => void sendTyped() }));
      row.append(h("button", { class: "link-btn", type: "button", "data-answer": "cancel", text: t("Cancel"), onclick: () => closeVoice(host) }));
      // No clock while an answer is on its way; the next result starts one.
      if (waiting) {
        voiceClock.stop();
        clockFor = -1;
        return;
      }
    }
    if (words.body) row.append(h("button", { class: "apply-all", type: "button", "data-answer": "copy", text: t("Copy"), onclick: () => void copy() }));
    // Windows blocks the microphone: its privacy page, one click away (plan §6).
    if (v.phase === "failed" && v.code === "mic-blocked") {
      row.append(h("button", { class: "apply-all", type: "button", "data-answer": "mic-settings", text: t("Open microphone settings"), onclick: () => void Bridge.openMicSettings() }));
    }
    if (!asking) row.append(h("button", { class: "link-btn", type: "button", "data-answer": "close", text: t("Close"), onclick: () => closeVoice(host) }));
    if (State.mode === "expanded" && clockFor !== shown) {
      clockFor = shown;
      voiceClock.start(`voice-${shown}`, () => closeVoice(host), cardTimeoutMs(v));
    }
  };
  return { el, sync };
}
