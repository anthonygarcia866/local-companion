// Glim learns (docs/glim-learns-plan.md §1-§2; Rust: src-tauri/src/learn,
// voice/run.rs): a learned shortcut's card ("Learned", Try again), a
// routine's card (every step, Save / Cancel), what it says when one ran or
// stopped, the resume card's note, and Settings → Ask Glim (the switch, the
// shortcuts, the routines with Rename and Remove).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State, DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const { nextVoice, cardTimeoutMs, stepText, stoppedAt, needsCard } = await import("../src/core/voice.ts");
const { buildVoice, closeVoice, voiceShown, voiceText, failedText, routineProblemText } = await import("../src/views/voice.ts");
const { buildResume } = await import("../src/views/resume.ts");
const { askSection } = await import("../src/settings/ask.ts");
const { keyText } = await import("../src/core/learn.ts");
const { sent, replies, emit } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const ev = (o) => ({ phase: "idle", level: 0, cleaned: false, text: "", code: "", app: "", ...o });
const at = (o) => nextVoice(null, ev(o));
const tick = () => new Promise((r) => setTimeout(r, 0));
const STEPS = [{ tool: "open_app", name: "Outlook" }, { tool: "open_app", name: "Codex" }, { tool: "resume_work", name: "my-repo" }];

function card() {
  return buildVoice({ setView: () => {}, collapse: () => {} });
}
const buttons = (v) => v.el.find("button").map((b) => b.textContent);

test("steps and stops in words", () => {
  assert.equal(stepText({ tool: "open_app", name: "Outlook" }), "Open Outlook");
  assert.equal(stepText({ tool: "resume_work", name: "my-repo" }), "Pick up my-repo");
  assert.equal(stepText({ tool: "resume_work", name: "" }), "Pick up where you left off");
  assert.deepEqual(stoppedAt("failed:2/3"), [2, 3]);
  assert.equal(stoppedAt("ok"), null);
  assert.equal(keyText({ key: "open {app}", slot: true, step: STEPS[0], daysLeft: 30 }), "open any app");
});

test("a learned shortcut's card says Claude wasn't asked; Try again goes to Rust", async () => {
  const v0 = at({ phase: "learned", app: "Outlook", code: "open_app", mode: "ask" });
  assert.ok(needsCard(v0));
  assert.equal(cardTimeoutMs(v0), 8_000);
  assert.deepEqual(voiceText(v0), { title: "Opened Outlook", body: "", note: "Learned shortcut: Claude wasn't asked." });
  State.voice = v0;
  State.view = "voice";
  State.mode = "expanded";
  voiceShown();
  const v = card();
  v.sync();
  assert.deepEqual(buttons(v), ["Try again", "Close"]);
  const n = sent("voice_try_again").length;
  replies.set("voice_try_again", true);
  v.el.find("button").find((b) => b.textContent === "Try again").fire("click");
  await tick();
  assert.equal(sent("voice_try_again").length, n + 1);
  replies.delete("voice_try_again");
  State.voice = null;
  State.mode = "hidden";
});

test("a routine's card lists every step; Save goes to Rust, Cancel tells Rust nothing is saved", async () => {
  const v0 = at({ phase: "routine-ask", text: "work mode", steps: STEPS, mode: "ask" });
  assert.equal(cardTimeoutMs(v0), 60_000);
  const words = voiceText(v0);
  assert.equal(words.title, "Save the routine “work mode”?");
  assert.equal(words.body, "1. Open Outlook\n2. Open Codex\n3. Pick up my-repo");
  assert.match(words.note, /Nothing runs until you save it/);
  State.voice = v0;
  State.view = "voice";
  State.mode = "expanded";
  voiceShown();
  const v = card();
  v.sync();
  assert.deepEqual(buttons(v), ["Save", "Cancel"]);
  const n = sent("voice_routine_save").length;
  replies.set("voice_routine_save", true);
  v.el.find("button").find((b) => b.textContent === "Save").fire("click");
  await tick();
  assert.equal(sent("voice_routine_save").length, n + 1);
  replies.delete("voice_routine_save");
  // Cancel (or the card closing) drops it in Rust.
  const c = sent("voice_cancel").length;
  closeVoice(null);
  assert.equal(sent("voice_cancel").length, c + 1);
  // A learned card closing doesn't.
  State.voice = at({ phase: "learned", app: "Codex" });
  closeVoice(null);
  assert.equal(sent("voice_cancel").length, c + 1);
  State.mode = "hidden";
});

test("window steps and window cards, in words", () => {
  assert.equal(stepText({ tool: "window", name: "Outlook", where: "left" }), "Put Outlook on the left");
  assert.equal(stepText({ tool: "window", name: "Chrome", where: "right" }), "Put Chrome on the right");
  assert.equal(stepText({ tool: "window", name: "Chrome", where: "maximize", display: "other" }), "Maximize Chrome, on the other screen");
  assert.equal(stepText({ tool: "window", name: "Codex", where: "restore" }), "Restore Codex");
  assert.equal(voiceText(at({ phase: "learned", code: "window", app: "Outlook" })).title, "Moved Outlook");
  const missing = voiceText(at({ phase: "failed", code: "window-missing", app: "Outlook" }));
  assert.deepEqual(missing, { title: "Window not moved", body: "", note: "Outlook isn't open, so there's no window to move." });
  assert.match(voiceText(at({ phase: "failed", code: "window-refused", app: "1Password" })).note, /password manager, an app running as administrator, or Glim itself/);
  assert.equal(failedText("window-no-other-display"), "There's only one screen.");
  assert.equal(failedText("window-failed", "Chrome"), "Couldn't move Chrome's window.");
});

test("a shortcut just learned says what to say, and goes like other result cards", () => {
  const v = at({ phase: "learned-new", app: "Outlook", text: "Open Outlook" });
  assert.deepEqual(voiceText(v), { title: "Opened Outlook", body: "", note: "Learned: say “Open Outlook” any time. Claude won't be asked." });
  assert.equal(cardTimeoutMs(v), 8_000);
});

test("saved, ran, stopped at a step, refused: each in words", () => {
  assert.deepEqual(voiceText(at({ phase: "routine-saved", text: "work mode" })), { title: "Saved “work mode”", body: "", note: "Say “work mode” with Ask Glim to run it." });
  assert.equal(voiceText(at({ phase: "routine", code: "ok", text: "work mode", steps: STEPS.slice(0, 2) })).title, "Ran “work mode”");
  const stopped = voiceText(at({ phase: "routine", code: "failed:2/3", text: "work mode", steps: [STEPS[1]] }));
  assert.equal(stopped.title, "“work mode” stopped");
  assert.equal(stopped.note, "Step 2 of 3 (Open Codex) didn't work. The steps after it didn't run.");
  const refused = voiceText(at({ phase: "failed", code: "routine-name-taken" }));
  assert.deepEqual([refused.title, refused.note], ["Routine not saved", "You already have a routine with that name."]);
  // Every code Rust can send has its words.
  const rs = read("src-tauri/src/learn/mod.rs");
  const codes = [...new Set([...rs.matchAll(/Some\("((?:name|no|too)-[a-z-]+)"\)|then_some\("([a-z-]+)"\)|Err\("([a-z-]+)"\)/g)].map((m) => m[1] ?? m[2] ?? m[3]))].filter((c) => c !== "gone");
  assert.ok(codes.length >= 9, codes.join());
  for (const code of codes) assert.notEqual(routineProblemText(code), "Glim couldn't save that routine.", code);
  assert.equal(failedText("routine-too-many-steps"), "A routine can have at most 5 steps.");
});

test("the resume card from a learned shortcut says so, with Try again", async () => {
  State.resume = [{ repo: "my-repo", goal: "g", inProgress: "next", branch: "main", lastCommit: "abc", hasQuestions: false, withheld: [], updated: 0 }];
  State.resumeLearned = true;
  const r = buildResume({ setView: () => {}, collapse: () => {} });
  r.sync();
  assert.match(r.el.textContent, /Learned shortcut: Claude wasn't asked\./);
  const n = sent("voice_try_again").length;
  r.el.find("button").find((b) => b.getAttribute("data-role") === "resume-try-again").fire("click");
  await tick();
  assert.equal(sent("voice_try_again").length, n + 1);
  assert.equal(State.resume, null, "the card closes");
  assert.equal(State.resumeLearned, false);
  // A resume card from the tray or Claude has no note.
  State.resume = [{ repo: "my-repo", goal: "g", inProgress: "next", branch: "main", lastCommit: "abc", hasQuestions: false, withheld: [], updated: 0 }];
  const plain = buildResume({ setView: () => {}, collapse: () => {} });
  plain.sync();
  assert.doesNotMatch(plain.el.textContent, /Learned shortcut/);
  State.resume = null;
});

test("Settings → Ask Glim: learning on by default, shortcuts forgotten, routines renamed and removed", async () => {
  const settings = { ...DEFAULT_SETTINGS };
  delete settings.learnShortcuts; // a settings.json from before the switch
  assert.equal(DEFAULT_SETTINGS.learnShortcuts, true);
  let saves = 0;
  replies.set("learn_list", {
    shortcuts: [{ key: "open {app}", slot: true, step: STEPS[0], daysLeft: 29 }, { key: "my-repo was where", slot: false, step: STEPS[2], daysLeft: 3 }],
    routines: [{ name: "work mode", steps: STEPS }],
  });
  const el = askSection({ settings: () => settings, save: async () => { saves++; } });
  await tick();
  const role = (r) => [...el.walk()].filter((e) => e.getAttribute("data-role") === r);
  const sw = role("learn-shortcuts")[0];
  assert.ok(sw.classList.contains("on"));
  sw.fire("click");
  assert.equal(settings.learnShortcuts, false);
  assert.equal(saves, 1);
  assert.match(el.textContent, /“open any app” → Open Outlook/);
  assert.match(el.textContent, /“my-repo was where” → Pick up my-repo/);
  assert.match(el.textContent, /1\. Open Outlook {2}2\. Open Codex {2}3\. Pick up my-repo/);

  role("learn-forget")[0].fire("click");
  await tick();
  assert.deepEqual(sent("learn_forget_shortcut").at(-1), { key: "open {app}", slot: true });
  role("learn-forget-all")[0].fire("click");
  await tick();
  assert.ok(sent("learn_forget_shortcuts").length >= 1);

  // A refused rename says why, in words; nothing else changes.
  replies.set("learn_routine_rename", "name-command");
  const input = role("learn-rename-input")[0];
  input.value = "open";
  role("learn-rename")[0].fire("click");
  await tick();
  assert.deepEqual(sent("learn_routine_rename").at(-1), { name: "work mode", to: "open" });
  assert.match(el.textContent, /That's one of Glim's command words\. Pick another name\./);
  replies.delete("learn_routine_rename");
  role("learn-remove")[0].fire("click");
  await tick();
  assert.deepEqual(sent("learn_routine_delete").at(-1), { name: "work mode" });

  // Rust tells the page when something was learned or saved: it reloads.
  const before = sent("learn_list").length;
  emit("learn-changed", null);
  await tick();
  assert.ok(sent("learn_list").length > before);

  // Nothing yet: it says so.
  replies.set("learn_list", { shortcuts: [], routines: [] });
  const empty = askSection({ settings: () => settings, save: async () => {} });
  await tick();
  assert.match(empty.textContent, /Nothing learned yet\./);
  assert.match(empty.textContent, /No routines yet\./);
  replies.delete("learn_list");
});

test("Rust: the commands are registered, the learn files stay local, the dictation key never learns", () => {
  const lib = read("src-tauri/src/lib.rs");
  for (const cmd of ["voice_try_again", "voice_routine_save", "learn_list", "learn_forget_shortcut", "learn_forget_shortcuts", "learn_routine_rename", "learn_routine_delete"]) {
    assert.match(lib, new RegExp(`\\n\\s+${cmd},\\n`), cmd);
  }
  const run = read("src-tauri/src/voice/run.rs");
  // Matching happens only for the Ask key without a waiting question.
  assert.ok(run.includes("let gist = (mode == Mode::Ask && asked.is_none() && !crate::capture::sensitive::contains_sensitive_number(transcript))"));
  // The learn module never touches the network.
  const learn = read("src-tauri/src/learn/mod.rs");
  assert.ok(!/net::|reqwest|TcpStream/.test(learn));
  assert.ok(learn.includes('crate::platform::local_dir().join("learn")'));
});

test("'tell me when …': the cards say what Glim waits for, what happened, and why not", async () => {
  const { watchText } = await import("../src/views/voice.ts");
  assert.equal(voiceText(at({ phase: "watching", code: "claude_code" })).note, "I'll tell you when Claude Code finishes.");
  assert.equal(voiceText(at({ phase: "watched", code: "codex" })).title, "Codex finished");
  assert.equal(watchText("window", "Chrome", "build passed", false), "I'll tell you when “build passed” shows in Chrome.");
  assert.equal(watchText("window-gone", "Outlook", "Uploading", true), "“Uploading” is gone from Outlook");
  assert.equal(voiceText(at({ phase: "watch-expired" })).note, "Nothing happened in 3 hours, so Glim stopped watching.");
  assert.equal(voiceText(at({ phase: "watch-stopped", code: "0" })).note, "Glim wasn't watching anything.");
  assert.deepEqual(voiceText(at({ phase: "failed", code: "watch-too-many" })), { title: "Not watching", body: "", note: "Glim is already watching 3 things. Say “stop watching” first." });
  assert.equal(cardTimeoutMs(at({ phase: "watching", code: "codex" })), 8_000);
  assert.equal(cardTimeoutMs(at({ phase: "watched", code: "codex" })), 30_000, "what happened stays longer");
});
