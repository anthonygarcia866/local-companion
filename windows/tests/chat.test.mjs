// The chat view (src/views/chat.ts) on a fake DOM, through the real bridge:
// the model switcher asks a local server for its models only once it is
// picked, picking saves the settings, and a streamed answer grows in place.

import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";
import { calls, emit, internals, sent } from "./tauri.mjs";

installFakeDom();
const { buildPrompt } = await import("../src/views/chat.ts");
const { DEFAULT_SETTINGS, State } = await import("../src/core/state.ts");

/** What the mocked Rust side answers, by command. */
let answers;
const plainInvoke = internals.invoke;
internals.invoke = async (cmd, args) => {
  const result = await plainInvoke(cmd, args);
  if (cmd in answers) return typeof answers[cmd] === "function" ? answers[cmd](args) : answers[cmd];
  return result;
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

let view;
beforeEach(() => {
  calls.length = 0;
  answers = {};
  State.settings = { ...DEFAULT_SETTINGS, chatModels: {} };
  State.chatHistory = [];
  State.stateOverride = null;
  State.view = "prompt";
  State.droppedFile = null;
  view = buildPrompt(() => {});
  view.sync();
});

const $ = (cls) => view.el.querySelector(cls);
const chips = () => view.el.find(".picker-chip").map((c) => c.textContent);
const models = () => view.el.find(".picker-model").map((m) => m.textContent);

test("the model button shows the active server's model", () => {
  assert.equal($(".model-name").textContent, "Choose a model");
  State.settings = { ...State.settings, chatModels: { ollama: "llama3.2" } };
  view.sync();
  assert.equal($(".model-name").textContent, "llama3.2");
});

test("opening the picker asks the active server only, and never for a key", async () => {
  answers.chat_models = [{ id: "llama3.2", label: "llama3.2" }, { id: "qwen2.5", label: "qwen2.5" }];
  $(".model-btn").fire("click");
  await flush();
  assert.ok($(".chat-body").classList.contains("picking"));
  assert.deepEqual(chips(), ["Ollama"]);
  assert.deepEqual(sent("secret_present"), []);
  assert.deepEqual(sent("chat_models"), [{ provider: "ollama" }]);
  // Nothing was saved yet: the first model offered is kept, and saved.
  assert.equal(State.settings.chatModels.ollama, "llama3.2");
  assert.deepEqual(models(), ["llama3.2", "qwen2.5"]);
});

test("picking a model saves it", async () => {
  answers.chat_models = [{ id: "llama3.2", label: "llama3.2" }, { id: "qwen2.5", label: "qwen2.5" }];
  $(".model-btn").fire("click");
  await flush();
  view.el.find(".picker-model")[1].fire("click");
  assert.equal(State.settings.chatModels.ollama, "qwen2.5");
  assert.equal(sent("save_settings").at(-1).settings.chatModels.ollama, "qwen2.5");
  assert.ok(!$(".chat-body").classList.contains("picking"));
  assert.equal($(".model-name").textContent, "qwen2.5");
});

test("switching server saves it and asks the new server only", async () => {
  State.settings = { ...State.settings, lmstudioUrl: "http://127.0.0.1:1234" };
  answers.chat_models = (args) => (args.provider === "lmstudio" ? [{ id: "qwen2.5", label: "qwen2.5" }] : []);
  $(".model-btn").fire("click");
  await flush();
  assert.deepEqual(chips(), ["Ollama", "LM Studio"]);
  view.el.find(".picker-chip")[1].fire("click");
  await flush();
  assert.equal(State.settings.chatProvider, "lmstudio");
  assert.deepEqual(sent("chat_models").at(-1), { provider: "lmstudio" });
  assert.equal(State.settings.chatModels.lmstudio, "qwen2.5");
  assert.deepEqual(models(), ["qwen2.5"]);
});

test("a local answer streams into one reply, then the finished text replaces it", async () => {
  let finish;
  answers.chat_send = () => new Promise((resolve) => (finish = resolve));
  const input = $(".chat-input");
  input.value = "hello";
  $(".send-btn").fire("click");
  await flush();
  view.sync(); // what State.notify() does in the island
  assert.ok($(".model-btn").disabled, "no switching mid-answer");
  assert.ok($(".typing"), "dots until the first visible text");

  emit("chat-delta", ""); // still thinking
  assert.ok($(".typing"));
  emit("chat-delta", "Hel");
  emit("chat-delta", "Hello **there**");
  view.sync(); // another view update mid-stream must not wipe the live answer
  assert.equal($(".typing"), null);
  const replies = view.el.find(".reply");
  assert.equal(replies.length, 1);
  assert.equal(replies[0].find("STRONG")[0].textContent, "there");

  finish({ text: "Hello **there**!" });
  await flush();
  view.sync();
  assert.equal(view.el.find(".reply").length, 1);
  assert.equal(view.el.find(".reply")[0].textContent, "Hello there!");
  assert.deepEqual(State.chatHistory.map((m) => m.role), ["user", "assistant"]);
  assert.ok(!$(".model-btn").disabled);
});
