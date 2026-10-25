// Glim learns §3 (docs/glim-learns-plan.md; Rust: src-tauri/src/memory.rs,
// voice/run.rs): the card for what was remembered or refused (and why), the
// correction tip, and Settings → Memory (the lists, Remove, Forget
// everything, and the privacy line).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { nextVoice, cardTimeoutMs, needsCard } = await import("../src/core/voice.ts");
const { buildVoice, voiceShown, voiceText, memoryProblemText, tipText } = await import("../src/views/voice.ts");
const { memorySection } = await import("../src/settings/memory.ts");
const { sent, replies, emit } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const ev = (o) => ({ phase: "idle", level: 0, cleaned: false, text: "", code: "", app: "", ...o });
const at = (o) => nextVoice(null, ev(o));
const tick = () => new Promise((r) => setTimeout(r, 0));

test("what was remembered, and the refusals that say why", () => {
  const rule = at({ phase: "remembered", text: "When I say the office I mean Outlook", code: "rule", mode: "ask" });
  assert.ok(needsCard(rule));
  assert.equal(cardTimeoutMs(rule), 8_000);
  assert.deepEqual(voiceText(rule), { title: "I'll remember that", body: "When I say the office I mean Outlook", note: "Kept on this PC as a rule. It goes to Claude with the Ask Glim requests it matches." });
  assert.match(voiceText(at({ phase: "remembered", text: "Prefers short replies", code: "fact" })).note, /as a fact about you/);
  const refused = voiceText(at({ phase: "failed", code: "memory-health" }));
  assert.deepEqual([refused.title, refused.note], ["Not saved", "Not saved: Glim doesn't keep health details."]);
  // Every code Rust can refuse with has its own words.
  const rs = read("src-tauri/src/memory.rs").split("#[cfg(test)]")[0];
  const codes = [...new Set([...rs.matchAll(/Some\("([a-z-]+)"\)|Err\("([a-z-]+)"\)|"(rules-full|facts-full)"/g)].map((m) => m[1] ?? m[2] ?? m[3]))].filter((c) => c !== "empty");
  assert.deepEqual(codes.sort(), ["customer", "duplicate", "fact-long", "facts-full", "health", "money", "number", "rule-long", "rules-full"]);
  for (const code of codes) assert.notEqual(memoryProblemText(code), "Nothing to remember.", code);
});

test("after a correction the result card adds the tip; an app that opened gets a tip card", () => {
  const heard = at({ phase: "heard", text: "no the other one", tip: true, mode: "ask" });
  State.voice = heard;
  State.view = "voice";
  State.mode = "expanded";
  voiceShown();
  const v = buildVoice({ setView: () => {}, collapse: () => {} });
  v.sync();
  assert.ok(v.el.textContent.includes(tipText()), v.el.textContent);
  assert.equal(tipText(), "Tip: say “Remember: …” and I'll get it right next time.");
  // Without a correction: no tip.
  State.voice = at({ phase: "heard", text: "x", mode: "ask" });
  v.sync();
  assert.ok(!v.el.textContent.includes("Tip:"));
  // Its own card.
  const card = at({ phase: "tip", mode: "ask" });
  assert.deepEqual(voiceText(card), { title: "Tip", body: "", note: tipText() });
  assert.equal(cardTimeoutMs(card), 8_000);
  State.voice = null;
  State.mode = "hidden";
  // Rust: once per correction, never on the dictation key.
  const run = read("src-tauri/src/voice/run.rs");
  // With #57: off through the local match (routine, learned shortcut), armed
  // only for Claude's answer; only the Ask key notes a correction.
  assert.match(run, /let correcting = mode == Mode::Ask && correction_now\(transcript, &mut LAST_ASK\.lock\(\)\.unwrap\(\), Instant::now\(\)\);\n\s+TIP\.store\(false, Ordering::SeqCst\);[\s\S]*?run_routine\(app, mode, routine\)\.await;\n\s+return;[\s\S]*?if replay\([\s\S]*?return;[\s\S]*?TIP\.store\(correcting, Ordering::SeqCst\);\n\s+let cleanup = clean_up\(/);
  assert.match(run, /if takes_tip\(e\.phase\) && TIP\.swap\(false, Ordering::SeqCst\)/);
});

test("Settings → Memory: the lists with Remove, Forget everything, and where they go", async () => {
  replies.set("memory_list", { rules: ["When I say the office I mean Outlook"], facts: ["Prefers short replies"] });
  const el = memorySection();
  await tick();
  const role = (r) => [...el.walk()].filter((e) => e.getAttribute("data-role") === r);
  assert.match(el.textContent, /When I say the office I mean Outlook/);
  assert.match(el.textContent, /Prefers short replies/);
  assert.match(role("memory-privacy")[0].textContent, /go to api\.anthropic\.com with your Ask Glim requests; never with dictation/);
  const removes = role("memory-remove");
  assert.equal(removes.length, 2);
  removes[1].fire("click");
  await tick();
  assert.deepEqual(sent("memory_delete").at(-1), { kind: "fact", text: "Prefers short replies" });
  role("memory-forget-all")[0].fire("click");
  await tick();
  assert.ok(sent("memory_forget_all").length >= 1);
  // Remembered by voice while open: it reloads.
  const before = sent("memory_list").length;
  emit("memory-changed", null);
  await tick();
  assert.ok(sent("memory_list").length > before);
  // Empty: examples, and no Forget everything.
  replies.set("memory_list", { rules: [], facts: [] });
  const empty = memorySection();
  await tick();
  assert.match(empty.textContent, /No rules yet\./);
  assert.match(empty.textContent, /Nothing yet\./);
  assert.equal([...empty.walk()].filter((e) => e.getAttribute("data-role") === "memory-forget-all").length, 0);
  replies.delete("memory_list");
});

test("Rust: the commands are registered and the privacy change is in CLAUDE.md", () => {
  const lib = read("src-tauri/src/lib.rs");
  for (const cmd of ["memory_list", "memory_delete", "memory_forget_all"]) assert.match(lib, new RegExp(`\\n\\s+${cmd},\\n`), cmd);
  const claude = read("../CLAUDE.md");
  assert.ok(claude.includes("Remembered rules and facts (Settings → Memory, `docs/glim-learns-plan.md` §3) go to `api.anthropic.com` inside Ask Glim requests only"));
  const memory = read("src-tauri/src/memory.rs");
  assert.ok(!/net::|reqwest|TcpStream/.test(memory.split("#[cfg(test)]")[0]), "the module never touches the network");
});
