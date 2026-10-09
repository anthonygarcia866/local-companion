// The island's provider table (src/core/providers.ts): which model the chat
// uses, which chips the picker shows, and what a server address exposes.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  PROVIDERS, activeModel, isLoopbackHost, pickModel, providerDef, urlExposure, visibleProviders, withModel,
} from "../src/core/providers.ts";
import { DEFAULT_SETTINGS } from "../src/core/state.ts";

const settings = (over = {}) => ({ ...DEFAULT_SETTINGS, ...over });

test("only the local model servers are left, and an unknown id falls back to Ollama", () => {
  assert.deepEqual(PROVIDERS.map((p) => p.id), ["ollama", "lmstudio"]);
  assert.deepEqual(PROVIDERS.map((p) => p.urlField), ["ollamaUrl", "lmstudioUrl"]);
  // A cloud provider left in an older settings.json reads as Ollama.
  for (const old of ["anthropic", "openai", "google", "openrouter", "custom", "nope"]) {
    assert.equal(providerDef(old).id, "ollama", old);
  }
});

test("the model is kept per provider, and empty until one is picked", () => {
  assert.equal(activeModel(settings()), "");
  let s = withModel(settings(), "ollama", "llama3.2");
  assert.equal(activeModel(s), "llama3.2");
  s = withModel({ ...s, chatProvider: "lmstudio" }, "lmstudio", "qwen2.5");
  assert.equal(activeModel(s), "qwen2.5");
  assert.equal(s.chatModels.ollama, "llama3.2");
  // withModel never changes the object it was given.
  assert.deepEqual(DEFAULT_SETTINGS.chatModels, {});
});

test("model servers show in the picker once connected, or while in use", () => {
  const ids = (s) => visibleProviders(s).map((p) => p.id);
  assert.deepEqual(ids(settings()), ["ollama"]);
  assert.deepEqual(ids(settings({ lmstudioUrl: "http://127.0.0.1:1234" })), ["ollama", "lmstudio"]);
  assert.deepEqual(ids(settings({ chatProvider: "lmstudio" })), ["lmstudio"]);
});

test("the saved model is kept when offered, else the first one", () => {
  const ollama = providerDef("ollama");
  assert.equal(pickModel(ollama, ["llama3.2", "qwen"], "qwen"), "qwen");
  assert.equal(pickModel(ollama, ["llama3.2", "qwen"], "gone"), "llama3.2");
  assert.equal(pickModel(ollama, [], "x"), null);
});

test("the names Rust maps to 127.0.0.1 count as this machine", () => {
  for (const host of ["localhost", "LOCALHOST", "127.0.0.1", "[::1]", "::1", "0.0.0.0", "[::]"]) {
    assert.ok(isLoopbackHost(host), host);
  }
  for (const host of ["example.com", "192.168.1.2", "localhost.example.com", "app.localhost", "127.0.0.2", "[2001:db8::1]"]) {
    assert.ok(!isLoopbackHost(host), host);
  }
});

test("an address on another machine is flagged: Rust refuses it", () => {
  assert.equal(urlExposure(""), "local");
  assert.equal(urlExposure("http://localhost:11434"), "local");
  assert.equal(urlExposure("127.0.0.1:1234/v1"), "local");
  assert.equal(urlExposure("http://[::1]:8000"), "local");
  assert.equal(urlExposure("https://llm.example.com"), "remote");
  assert.equal(urlExposure("http://gpu-box.lan:8000"), "remote");
  assert.equal(urlExposure("192.168.1.20:8000"), "remote");
  assert.equal(urlExposure("ftp://example.com"), "invalid");
  assert.equal(urlExposure("http://user:pw@example.com"), "invalid");
  assert.equal(urlExposure("http://"), "invalid");
});
