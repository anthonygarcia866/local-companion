// Push-to-talk, the island's half (voice 1.5b; core/voice.ts, views/voice.ts,
// docs/voice-actions-plan.md §1, §4.1): Rust's phases become the lantern's
// state and, for a result that needs one, a card; the card words each
// result, copies what was heard, and closes by itself.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { nextVoice, needsCard, running, voiceLantern, voiceSending, voicePlacement, cardTimeoutMs, answering, modeLabel } = await import("../src/core/voice.ts");
const { buildVoice, closeVoice, voiceClock, voiceShown, voiceText, failedText, notCleanedText, heardNote } = await import("../src/views/voice.ts");
const { sent } = await import("./tauri.mjs");
const { keyboardHolder } = await import("../src/core/keyboard.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const ev = (o) => ({ phase: "idle", level: 0, cleaned: false, text: "", code: "", app: "", ...o });
const at = (o) => nextVoice(null, ev(o));

test("Rust's phases become the island's state; idle clears it", () => {
  let v = at({ phase: "listening", level: 0.4 });
  assert.equal(v.phase, "listening");
  assert.equal(v.level, 0.4);
  v = nextVoice(v, ev({ phase: "listening", level: 7 }));
  assert.equal(v.level, 1, "the level is clamped");
  v = nextVoice(v, ev({ phase: "listening", level: Number.NaN }));
  assert.equal(v.level, 0);
  v = nextVoice(v, ev({ phase: "transcribing" }));
  assert.deepEqual([v.phase, v.level], ["transcribing", 0]);
  v = nextVoice({ ...at({ phase: "heard", text: "open outlook" }), copied: "copied" }, ev({ phase: "failed", code: "whisper" }));
  assert.deepEqual([v.phase, v.code, v.text, v.copied], ["failed", "whisper", "", ""], "a new result starts clean");
  assert.equal(nextVoice(v, ev({ phase: "idle" })), null);
});

test("the lantern: s-hear while held, thinking while transcribing, data leaving only while Claude has it", () => {
  assert.equal(voiceLantern(at({ phase: "listening" })), "hear");
  assert.equal(voiceLantern(at({ phase: "transcribing" })), "think");
  for (const phase of ["cleaning", "typed", "heard", "question", "failed", "not-typed"]) {
    assert.equal(voiceLantern(at({ phase })), null, phase);
    assert.equal(voiceSending(at({ phase })), phase === "cleaning", phase);
  }
  assert.equal(voiceLantern(null), null);
  assert.equal(voiceSending(null), false);
  for (const phase of ["listening", "transcribing", "cleaning"]) assert.equal(running(at({ phase })), true, phase);
});

test("a cleaned-up text typed in the field needs no card; every other result does", () => {
  assert.equal(needsCard(at({ phase: "typed", cleaned: true })), false);
  assert.equal(needsCard(at({ phase: "typed", cleaned: false, code: "no-key" })), true, "not cleaned up: Glim says so");
  for (const phase of ["not-typed", "heard", "question", "failed"]) assert.equal(needsCard(at({ phase })), true, phase);
  for (const phase of ["listening", "transcribing", "cleaning"]) assert.equal(needsCard(at({ phase })), false, phase);
  assert.equal(needsCard(null), false);
});

test("the card shows with the pill or hidden, waits behind an approval, and not over fullscreen", () => {
  const v = at({ phase: "heard", text: "open outlook" });
  const place = (o) => voicePlacement({ presence: "pill", approval: false, v, ...o });
  assert.equal(place({}), "show");
  assert.equal(place({ presence: "hidden" }), "show", "push-to-talk works with Glim hidden");
  assert.equal(place({ presence: "ember" }), "show");
  assert.equal(place({ approval: true }), "queue");
  assert.equal(place({ presence: "indicator" }), "drop");
  assert.equal(place({ v: at({ phase: "typed", cleaned: true }) }), "drop");

  // Behind an approval and the writing questions, before the suggestions.
  State.voice = v;
  State.writing = [{ id: 1 }];
  State.pendingApproval = { questions: null };
  assert.equal(State.lanternView(), "approval");
  State.pendingApproval = null;
  State.writingDocsTip = true;
  assert.equal(State.lanternView(), "writingDocsTip");
  State.writingDocsTip = false;
  assert.equal(State.lanternView(), "voice");
  State.voice = at({ phase: "typed", cleaned: true });
  assert.equal(State.lanternView(), "suggestions");
  State.voice = null;
  State.writing = [];
});

test("each result in words: not cleaned up says why, a command is never presented as typed", () => {
  const typed = voiceText(at({ phase: "typed", app: "Word", code: "no-key" }));
  assert.equal(typed.title, "Typed in Word, not cleaned up");
  assert.match(typed.note, /Anthropic API key/);
  assert.equal(notCleanedText("timeout"), "Claude didn't answer in time, so it wasn't cleaned up.");
  assert.match(notCleanedText("cap"), /spending cap/);

  const heard = voiceText(at({ phase: "heard", text: "open outlook", code: "command" }));
  assert.equal(heard.title, "Here's what I heard (couldn't act on it)");
  assert.equal(heard.body, "open outlook");
  assert.equal(heard.note, "Glim can't do that yet, so nothing happened.");
  assert.match(voiceText(at({ phase: "heard", text: "hi", code: "no-field" })).note, /Click in a text field first/);
  const rust = read("src-tauri/src/voice/mod.rs");
  for (const code of ['"command"', '"no-field"']) assert.ok(rust.includes(code), code);
  assert.match(voiceText(at({ phase: "heard", text: "x", code: "network" })).note, /Not cleaned up: Couldn't reach Claude/);

  const notTyped = voiceText(at({ phase: "not-typed", text: "hello", code: "paused", app: "Word" }));
  assert.deepEqual([notTyped.title, notTyped.body, notTyped.note], ["Not typed", "hello", "Glim is paused."]);
  assert.match(voiceText(at({ phase: "not-typed", text: "hi", code: "?" })).note, /Copy it and paste it yourself/);

  assert.equal(voiceText(at({ phase: "question", text: "Which Maria?" })).title, "Which Maria?");
  assert.match(failedText("sensitive"), /^Not sent: .*card, ID or bank number/);
  assert.match(failedText("no-whisper"), /Settings → Writing → Dictation/);
  assert.match(failedText("mic-blocked"), /microphone settings/);
  assert.match(failedText("nothing-heard"), /didn't catch any words/);
  assert.equal(failedText("anything-else"), failedText("whisper"));
  const notOpened = voiceText(at({ phase: "failed", code: "open-failed", app: "Outlook" }));
  assert.deepEqual([notOpened.title, notOpened.note], ["Not opened", "Couldn't open Outlook. Is it installed?"]);
});

function card() {
  const calls = [];
  const v = buildVoice({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  return { v, calls };
}
const buttons = (v) => v.el.find("button").map((b) => b.textContent);

test("Copy puts what was heard on the clipboard through Rust; Close closes; the clock closes it too", async () => {
  State.voice = at({ phase: "heard", text: "open outlook" });
  State.view = "voice";
  State.mode = "expanded";
  voiceShown();
  const { v, calls } = card();
  v.sync();
  assert.deepEqual(buttons(v), ["Copy", "Close"]);
  assert.match(v.el.textContent, /open outlook/);
  assert.equal(voiceClock.running(), true);
  const n = sent("voice_copy").length;
  v.el.find("button").find((b) => b.textContent === "Copy").fire("click");
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent("voice_copy").slice(n), [{ text: "open outlook" }]);
  // The fake bridge answers null: the clipboard was busy, and it says so.
  assert.equal(State.voice.copied, "clipboard-busy");
  assert.match(v.el.textContent, /another app is using the clipboard/);

  v.el.find("button").find((b) => b.textContent === "Close").fire("click");
  assert.equal(State.voice, null);
  assert.deepEqual(calls, [["collapse"]]);
  assert.equal(voiceClock.running(), false);

  // Nothing to copy: Close only.
  State.voice = at({ phase: "failed", code: "no-mic" });
  voiceShown();
  const other = card();
  other.v.sync();
  assert.deepEqual(buttons(other.v), ["Close"]);
  closeVoice(null);
  assert.equal(State.voice, null);
  State.mode = "hidden";
});

test("the microphone blocked: one click opens Windows' microphone privacy page, a fixed link in Rust", () => {
  State.voice = at({ phase: "failed", code: "mic-blocked" });
  State.mode = "expanded";
  voiceShown();
  const { v } = card();
  v.sync();
  assert.deepEqual(buttons(v), ["Open microphone settings", "Close"]);
  const n = sent("open_mic_settings").length;
  v.el.find("button").find((b) => b.textContent === "Open microphone settings").fire("click");
  assert.equal(sent("open_mic_settings").length, n + 1);
  // No mic at all: nothing to open.
  State.voice = at({ phase: "failed", code: "no-mic" });
  v.sync();
  assert.deepEqual(buttons(v), ["Close"]);
  closeVoice(null);
  State.mode = "hidden";
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(lib.includes('const MIC_SETTINGS: &str = "ms-settings:privacy-microphone";'));
  assert.ok(lib.includes("fn open_mic_settings() {"), "it takes nothing from the island");
});

test("the lantern's not-cleaned-up badge: only while a raw transcript's result is up", async () => {
  const { rawBadge } = await import("../src/core/voice.ts");
  assert.equal(rawBadge(at({ phase: "typed", cleaned: false, code: "cap" })), true);
  assert.equal(rawBadge(at({ phase: "typed", cleaned: true })), false);
  for (const phase of ["heard", "not-typed", "failed", "listening"]) assert.equal(rawBadge(at({ phase })), false, phase);
  assert.equal(rawBadge(null), false);
  const island = read("src/island/island.ts");
  assert.match(island, /const raw = rawBadge\(State\.voice\) && State\.mode !== "hidden" && !State\.mochiOnDesktop;/);
  assert.match(island, /const shown = !raw && writing > 0/, "it takes the count's spot");
});

test("the level meter: only while listening, a sliver at silence, 60% of the pill at full voice", async () => {
  const { meterPercent } = await import("../src/core/voice.ts");
  assert.equal(meterPercent(at({ phase: "listening", level: 0 })), 6);
  assert.equal(meterPercent(at({ phase: "listening", level: 0.5 })), 33);
  assert.equal(meterPercent(at({ phase: "listening", level: 1 })), 60);
  assert.equal(meterPercent({ ...at({ phase: "listening" }), level: 9 }), 60);
  for (const phase of ["transcribing", "cleaning", "typed", "failed"]) assert.equal(meterPercent(at({ phase })), null, phase);
  assert.equal(meterPercent(null), null);
  const island = read("src/island/island.ts");
  assert.match(island, /this\.voiceMeter\.style\.display = meter === null \|\| State\.mode === "hidden" \? "none" : "";/);
});

test("a typed result closes sooner than one there to copy; a question waits longest", () => {
  assert.equal(cardTimeoutMs(at({ phase: "typed" })), 8_000);
  for (const phase of ["heard", "not-typed", "failed"]) assert.equal(cardTimeoutMs(at({ phase })), 30_000, phase);
  assert.equal(cardTimeoutMs(at({ phase: "question" })), 60_000);
});

test("the island never logs or stores what was heard; the handler sends nothing to Rust", () => {
  for (const file of ["src/core/voice.ts", "src/views/voice.ts"]) {
    const src = read(file);
    for (const banned of ["console.", "localStorage", "sessionStorage", "addEventListener"]) assert.ok(!src.includes(banned), `${file}: ${banned}`);
  }
  assert.deepEqual(read("src/views/voice.ts").match(/Bridge\.\w+\([^)]*\)/g), ["Bridge.voiceCancel()", "Bridge.voiceAnswer(text)", "Bridge.voiceCopy(v.text)", "Bridge.openMicSettings()"]);
  const main = read("src/main.ts");
  const handler = main.slice(main.indexOf('"voice"'));
  const body = handler.slice(0, handler.indexOf("\n  });") + 5);
  for (const banned of ["Bridge.", "console.", "localStorage"]) assert.ok(!body.includes(banned), banned);
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(lib.includes("fn voice_copy(text: String) -> bool {"));
  assert.ok(/\n\s+voice_copy,\n/.test(lib), "registered");
  // Rust logs only that a copy happened, never the text.
  const fn = lib.slice(lib.indexOf("fn voice_copy("), lib.indexOf("fn voice_copy(") + 600);
  assert.ok(!/log::line\([^)]*text/.test(fn));
});

test("Settings → Writing → Dictation: the model and the words are stored, and the files hint follows the model", async () => {
  const { dictationGroup, modelFile } = await import("../src/settings/writing.ts");
  const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");
  assert.equal(DEFAULT_SETTINGS.voiceModel, "base");
  assert.equal(DEFAULT_SETTINGS.voiceWords, "");
  // The same file names Rust looks for.
  const rust = read("src-tauri/src/voice/whisper.rs");
  for (const size of ["base", "small"]) assert.ok(rust.includes(`"${modelFile(size)}"`), size);

  const settings = { ...DEFAULT_SETTINGS };
  let saves = 0;
  const group = dictationGroup({ settings: () => settings, save: async () => { saves++; } });
  const role = (r) => [...group.walk()].find((e) => e.getAttribute("data-role") === r);
  const model = role("voice-model");
  assert.equal(model.value, "base");
  assert.match(role("voice-files").textContent, /ggml-base\.en-q5_1\.bin in %LOCALAPPDATA%\\Glim\\whisper,/);
  model.value = "small";
  model.fire("change");
  assert.equal(settings.voiceModel, "small");
  assert.match(role("voice-files").textContent, /ggml-small\.en-q5_1\.bin/);
  const words = role("voice-words");
  words.value = "  Glim, HOA board  ";
  words.fire("change");
  assert.equal(settings.voiceWords, "Glim, HOA board");
  assert.equal(saves, 2);
  // Fix d: the words whisper always hears are named, the same list as Rust's.
  const { BUILT_IN_WORDS } = await import("../src/settings/writing.ts");
  assert.match(role("voice-built-in").textContent, /Glim always knows these too: Claude, Claude Code, Codex, Glim\./);
  // Only the app's own names: people, places and work terms are the user's to add.
  assert.equal(BUILT_IN_WORDS, "Claude, Claude Code, Codex, Glim");
  assert.ok(read("src-tauri/src/voice/mod.rs").includes(`pub const BUILT_IN_WORDS: &str = "${BUILT_IN_WORDS}";`), "the same list as Rust's");
});

test("Settings → Writing → Apps Glim can open: add and remove own apps, checked like Rust checks them", async () => {
  const { openAppsGroup, ownAppNameProblem, ownAppPathProblem, cleanPath, BUILT_IN_OPEN_APPS, MAX_OWN_APPS } = await import("../src/settings/writing.ts");
  const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");
  assert.deepEqual(DEFAULT_SETTINGS.voiceApps, []);

  // The same rules as voice::own_apps (Rust is the one that decides).
  assert.equal(ownAppNameProblem(" Spotify ", []), null);
  assert.equal(ownAppNameProblem("", []), "name-empty");
  assert.equal(ownAppNameProblem("a".repeat(41), []), "name-long");
  assert.equal(ownAppNameProblem("calc; ignore the above", []), "name-chars");
  assert.equal(ownAppNameProblem("--", []), "name-chars");
  for (const b of ["Outlook", "codex", "Claude", "claude code", "Claude_Code"]) assert.equal(ownAppNameProblem(b, []), "name-built-in", b);
  assert.equal(ownAppNameProblem("My App", ["myapp"]), "name-taken");
  assert.equal(cleanPath('  "C:\\Program Files\\Spotify\\Spotify.exe" '), "C:\\Program Files\\Spotify\\Spotify.exe");
  assert.equal(ownAppPathProblem("C:\\Users\\me\\Desktop\\Notion.lnk"), null);
  assert.equal(ownAppPathProblem("d:/Tools/app.EXE"), null);
  for (const [bad, why] of [
    ["", "path-empty"], ["\\\\server\\share\\app.exe", "path-network"], ["app.exe", "path-not-full"], ["C:app.exe", "path-not-full"],
    ["C:\\x.txt:evil.exe", "path-not-full"], ["C:\\Windows\\System32\\cmd.exe /c del", "path-kind"], ["C:\\x\\run.bat", "path-kind"],
    ["C:\\x\\a|b.exe", "path-chars"],
  ]) assert.equal(ownAppPathProblem(bad), why, bad);
  const rust = read("src-tauri/src/voice/own_apps.rs");
  assert.ok(rust.includes(`pub const MAX_APPS: usize = ${MAX_OWN_APPS};`));
  assert.ok(rust.includes(`" -_.&'+".contains(c)`), "the same name marks as Rust");
  assert.deepEqual(BUILT_IN_OPEN_APPS, ["Outlook", "Codex", "Claude", "Claude Code"]);

  const settings = { ...DEFAULT_SETTINGS, voiceApps: [] };
  let saves = 0;
  const group = openAppsGroup({ settings: () => settings, save: async () => { saves++; } });
  const role = (r) => [...group.walk()].find((e) => e.getAttribute("data-role") === r);
  const all = (r) => [...group.walk()].filter((e) => e.getAttribute("data-role") === r);
  assert.match(role("open-apps-built-in").textContent, /Glim knows Outlook, Codex, Claude, Claude Code\./);
  assert.match(role("own-apps").textContent, /No apps of your own yet\./);
  const name = role("own-app-name");
  const path = role("own-app-path");
  // A bad path: nothing stored, the reason shown.
  name.value = "Spotify";
  path.value = "C:\\Windows\\System32\\cmd.exe /c del";
  role("own-app-add").fire("click");
  assert.deepEqual(settings.voiceApps, []);
  assert.equal(saves, 0);
  assert.match(role("own-app-why").textContent, /\.exe file or a shortcut/);
  // A good one (Explorer's quotes dropped): stored, saved, listed, fields cleared.
  path.value = '"C:\\Apps\\Spotify\\Spotify.exe"';
  role("own-app-add").fire("click");
  assert.deepEqual(settings.voiceApps, [{ name: "Spotify", path: "C:\\Apps\\Spotify\\Spotify.exe" }]);
  assert.equal(saves, 1);
  assert.equal(role("own-app-why").textContent, "");
  assert.equal(name.value, "");
  assert.match(role("own-apps").textContent, /Spotify → C:\\Apps\\Spotify\\Spotify\.exe/);
  // The same name again (any case) is refused.
  name.value = "spotify";
  path.value = "C:\\Other\\Spotify.exe";
  role("own-app-add").fire("click");
  assert.equal(settings.voiceApps.length, 1);
  assert.match(role("own-app-why").textContent, /already have an app with that name/);
  // Remove.
  all("own-app-remove")[0].fire("click");
  assert.deepEqual(settings.voiceApps, []);
  assert.equal(saves, 2);
  // At most MAX_OWN_APPS.
  settings.voiceApps = Array.from({ length: MAX_OWN_APPS }, (_, i) => ({ name: `App ${i}`, path: "C:\\a.exe" }));
  name.value = "One more";
  path.value = "C:\\b.exe";
  role("own-app-add").fire("click");
  assert.equal(settings.voiceApps.length, MAX_OWN_APPS);
  assert.match(role("own-app-why").textContent, /up to 20 apps/);
  // It sits on the Writing page, after Dictation.
  const writing = read("src/settings/writing.ts");
  assert.ok(writing.indexOf("dictationGroup(host),\n    openAppsGroup(host),") > 0);
});

// Retest defect (2026-10-16): the question sat in a one-line title and was
// cut off ("open an ap…", "draft it as…"). A question always shows in full:
// it wraps, and only a very long one scrolls inside a capped box.
test("a long question wraps and shows in full, never cut off", () => {
  const long = "Do you want me to open an app, or should I draft it as an email to the tenant about the updated lease before Friday afternoon, with the signature added?";
  State.voice = at({ phase: "question", text: long });
  State.view = "voice";
  State.mode = "expanded";
  const { v } = card();
  v.sync();
  const q = v.el.find(".voice-question")[0];
  assert.ok(q, "the question has its own wrapping line");
  assert.equal(q.textContent, long, "the whole question, not a shortened one");
  assert.equal(q.classList.contains("one-line"), false);
  const title = v.el.find(".title")[0];
  assert.equal(title.classList.contains("one-line"), false, "no ellipsis on a question");
  const css = read("src/style.css");
  const rule = css.slice(css.indexOf(".voice-question {"), css.indexOf("}", css.indexOf(".voice-question {")));
  assert.match(rule, /white-space: normal/);
  assert.match(rule, /overflow-wrap: anywhere/);
  assert.match(rule, /max-height: \d+px/, "a sensible cap");
  assert.match(rule, /overflow-y: auto/, "scrolls only past the cap");
  assert.doesNotMatch(rule, /ellipsis|nowrap/);
  // Back to a short-title result: the one-line title returns.
  State.voice = at({ phase: "heard", text: "open outlook" });
  v.sync();
  assert.equal(v.el.find(".title")[0].classList.contains("one-line"), true);
});

test("the card grows for a question, so a wrapped one fits before it ever scrolls", async () => {
  const { VOICE_QUESTION_H, VIEW_LAYOUTS } = await import("../src/core/layout.ts");
  assert.ok(VOICE_QUESTION_H > VIEW_LAYOUTS.voice.height);
  assert.ok(VOICE_QUESTION_H <= 320, "still inside the island's window");
  const island = read("src/island/island.ts");
  assert.ok(island.includes('State.view === "voice" && (State.voice?.phase === "question" || answering(State.voice))'));
  assert.match(island, /h = VOICE_QUESTION_H;/);
});

// ── Decision A (2026-10-16): two keys, V dictates, A asks Glim ───────────────

test("the pill names the held key: Dictating or Ask Glim, only while listening", async () => {
  assert.equal(modeLabel(at({ phase: "listening", mode: "dictate" })), "Dictating");
  assert.equal(modeLabel(at({ phase: "listening", mode: "ask" })), "Ask Glim");
  assert.equal(modeLabel(at({ phase: "listening" })), "Dictating", "an event without a mode is dictation");
  assert.equal(modeLabel(at({ phase: "listening", mode: "answer" })), null, "answering shows on its card");
  for (const phase of ["transcribing", "cleaning", "typed", "heard"]) assert.equal(modeLabel(at({ phase, mode: "ask" })), null, phase);
  assert.equal(modeLabel(null), null);
  const island = read("src/island/island.ts");
  assert.match(island, /id: "voice-mode"/);
  assert.match(island, /const label = modeLabel\(State\.voice\);/);
  assert.match(read("src/style.css"), /#voice-mode \{/);
  // Both keys are listed in Settings → Shortcuts, rebindable, with these defaults.
  const { SHORTCUTS } = await import("../src/core/shortcuts.ts");
  const keys = Object.fromEntries(SHORTCUTS.map((d) => [d.id, d.defaultKeys]));
  assert.equal(keys.pushToTalk, "Ctrl+Alt+Shift+V");
  assert.equal(keys.askGlim, "Ctrl+Alt+Shift+A");
  const rust = read("src-tauri/src/shortcuts.rs");
  assert.ok(rust.includes('action("askGlim", "Ctrl+Alt+Shift+A", true, true),'));
  assert.ok(rust.includes('if action == "pushToTalk" || action == "askGlim" {'));
});

test("dictation without a clean-up that starts like a command says why it wasn't typed", () => {
  assert.match(heardNote("command-words"), /words that start like a command aren't typed/);
  assert.ok(read("src-tauri/src/voice/mod.rs").includes('"command-words"'));
});

// ── Decision B (2026-10-16): a voice answer goes into the waiting card ───────

test("while a question waits, an answer from either key keeps the card up", () => {
  const q = at({ phase: "question", text: "Which app: Outlook, Codex or Claude Code?", mode: "ask" });
  assert.equal(q.question, "", "the question itself is in text");
  let v = nextVoice(q, ev({ phase: "listening", mode: "answer" }));
  assert.equal(answering(v), true);
  assert.equal(v.question, "Which app: Outlook, Codex or Claude Code?");
  assert.equal(needsCard(v), true, "the card stays while the answer is on its way");
  assert.equal(voiceLantern(v), "hear");
  v = nextVoice(v, ev({ phase: "transcribing", mode: "answer" }));
  v = nextVoice(v, ev({ phase: "cleaning", mode: "answer" }));
  assert.equal(v.question, "Which app: Outlook, Codex or Claude Code?", "kept through every phase");
  assert.equal(answering(v), true);
  // The answer's result replaces it.
  v = nextVoice(v, ev({ phase: "heard", text: "codex", mode: "answer" }));
  assert.deepEqual([answering(v), v.question], [false, ""]);
  // A plain press (no question waiting) is not answering, and needs no card while it runs.
  const plain = at({ phase: "listening", mode: "dictate" });
  assert.deepEqual([answering(plain), needsCard(plain)], [false, false]);
});

test("the question card: Listening for your answer…, Cancel, and a typed answer goes to Rust", async () => {
  State.voice = at({ phase: "question", text: "Which Maria?", mode: "ask" });
  State.view = "voice";
  State.mode = "expanded";
  voiceShown();
  const { v } = card();
  v.sync();
  assert.deepEqual(buttons(v), ["Send", "Cancel"]);
  assert.match(v.el.textContent, /Hold either voice shortcut and say your answer, or type it below\./);
  const box = v.el.find(".voice-answer")[0];
  assert.ok(box, "there is an answer box");
  assert.equal(box.style.display, "");
  assert.equal(voiceClock.running(), true);
  // Typed: Enter sends it through Rust (voice_answer).
  const n = sent("voice_answer").length;
  box.value = "  Maria Lopez  ";
  box.fire("keydown", { key: "Enter" });
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent("voice_answer").slice(n), [{ text: "Maria Lopez" }]);
  // Spoken: Rust's answer phases keep the card, which says it is listening.
  State.voice = nextVoice(at({ phase: "question", text: "Which Maria?" }), ev({ phase: "listening", mode: "answer" }));
  v.sync();
  assert.match(v.el.textContent, /Listening for your answer…/);
  assert.match(v.el.textContent, /Which Maria\?/);
  assert.deepEqual(buttons(v), ["Cancel"]);
  assert.equal(box.style.display, "none");
  assert.equal(voiceClock.running(), false, "no clock while the answer is on its way");
  State.voice = nextVoice(State.voice, ev({ phase: "cleaning", mode: "answer" }));
  v.sync();
  assert.match(v.el.textContent, /Working on your answer…/);
  // Cancel stops Rust waiting (voice_cancel) and closes the card.
  const c = sent("voice_cancel").length;
  v.el.find("button").find((b) => b.textContent === "Cancel").fire("click");
  assert.equal(State.voice, null);
  assert.equal(sent("voice_cancel").length, c + 1);
  State.mode = "hidden";
});

test("the answer box borrows the keyboard on a click and gives it back on Send, a spoken answer, Cancel or close (retest, 2026-10-17)", async () => {
  const lent = () => sent("lend_keyboard").map((c) => c.on);
  State.view = "voice";
  State.mode = "expanded";
  // Typed and sent with Enter.
  State.voice = at({ phase: "question", text: "What should it say?", mode: "ask" });
  voiceShown();
  const { v } = card();
  v.sync();
  const box = v.el.find(".voice-answer")[0];
  let n = sent("lend_keyboard").length;
  v.sync();
  assert.deepEqual(lent().slice(n), [], "showing the card never takes the keyboard");
  box.fire("pointerdown");
  assert.deepEqual(lent().slice(n), [true]);
  assert.equal(keyboardHolder(), "voice");
  box.value = "the shift moved to Tuesday";
  box.fire("keydown", { key: "Enter" });
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(lent().slice(n), [true, false], "Send gives it back");
  assert.equal(keyboardHolder(), null);
  // Clicked, then answered by voice instead: the box goes, so does the keyboard.
  State.voice = at({ phase: "question", text: "Which app?", mode: "ask" });
  v.sync();
  n = sent("lend_keyboard").length;
  box.fire("pointerdown");
  State.voice = nextVoice(State.voice, ev({ phase: "listening", mode: "answer" }));
  v.sync();
  assert.deepEqual(lent().slice(n), [true, false]);
  // Clicked, then Cancel.
  State.voice = at({ phase: "question", text: "Which app?", mode: "ask" });
  v.sync();
  n = sent("lend_keyboard").length;
  box.fire("pointerdown");
  v.el.find("button").find((b) => b.textContent === "Cancel").fire("click");
  assert.deepEqual(lent().slice(n), [true, false]);
  // Closed without a click: nothing was borrowed, nothing is given back.
  State.voice = at({ phase: "question", text: "Which app?", mode: "ask" });
  n = sent("lend_keyboard").length;
  closeVoice(null);
  assert.deepEqual(lent().slice(n), []);
  State.mode = "hidden";
  // Rust: island only, gives the keyboard back to the window it came from.
  const lib = read("src-tauri/src/lib.rs");
  const fn = lib.slice(lib.indexOf("fn lend_keyboard("), lib.indexOf("fn dock_drag_start("));
  assert.ok(fn.includes("caller.label() != island::WINDOW_LABEL"), "only the island may ask");
  assert.ok(fn.includes("platform::foreground_other_than(&win)") && fn.includes("platform::give_foreground_back(&win, to)"));
  assert.ok(/\n\s+focus_window,\n\s+lend_keyboard,\n/.test(lib), "registered");
});

test("a spoken never mind: Rust ends the question and the island leaves the card, no follow-up (fix c, 2026-10-17)", () => {
  // Rust sends "idle" (voice) and "closed" (draft); the island leaves those
  // views instead of showing them emptied, and gives back a borrowed keyboard.
  const main = read("src/main.ts");
  const gone = main.slice(main.indexOf("const cardGone = "), main.indexOf('await onEvent<VoiceEvent>("voice"'));
  for (const step of ["returnKeyboard(view);", "State.view !== view", "State.lanternView()", "island.setView(next)", "island.collapse()"]) assert.ok(gone.includes(step), step);
  assert.ok(main.includes('if (e.phase === "idle" && was) cardGone("voice");'));
  assert.ok(main.includes('if (had && !State.draft) cardGone("draft");'));
  assert.equal(nextVoice(at({ phase: "question", text: "Is there something else?" }), ev({ phase: "idle" })), null, "idle clears the card");
});

test("closing a question card tells Rust; other cards don't", () => {
  const cases = [
    [at({ phase: "question", text: "Which app?" }), 1],
    [nextVoice(at({ phase: "question", text: "Which app?" }), ev({ phase: "listening", mode: "answer" })), 1],
    [at({ phase: "heard", text: "x" }), 0],
    [at({ phase: "failed", code: "no-mic" }), 0],
  ];
  for (const [voice, tells] of cases) {
    State.voice = voice;
    const c = sent("voice_cancel").length;
    closeVoice(null);
    assert.equal(sent("voice_cancel").length - c, tells, voice.phase);
  }
  // Rust: the commands exist and are registered; the typed answer is never logged.
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(lib.includes("fn voice_answer(app: AppHandle, text: String) -> bool {"));
  assert.ok(lib.includes("fn voice_cancel() {"));
  assert.ok(/\n\s+voice_answer,\n\s+voice_cancel,\n/.test(lib), "registered");
  const run = read("src-tauri/src/voice/run.rs");
  const typed = run.slice(run.indexOf("pub fn answer_typed("), run.indexOf("async fn process("));
  assert.ok(!/log::line\([^;]*\{text\}/.test(typed), "the typed answer is never logged");
  assert.match(run, /pub const QUESTION_TTL: Duration = Duration::from_secs\(120\);/);
});
