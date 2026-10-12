// The island's agent cards: what their buttons open.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { buildError, errorOpenLabel } = await import("../src/views/views.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

const task = (id, source, extra = {}) => ({
  id, name: id, color: "#fff", state: "error", stepIndex: 0, steps: ["boom"], source, isIntegration: false, ...extra,
});

function errorCard(t) {
  const calls = [];
  const actions = new Proxy({}, { get: (_, name) => (...args) => calls.push([name, ...args]) });
  State.tasks = [t];
  State.setFocus(t.id);
  const view = buildError(actions);
  view.sync();
  const buttons = view.el.find("button").filter((b) => b.style.display !== "none");
  return { view, calls, buttons, label: (b) => b.children[0].textContent };
}

test("a Claude Code session's error card opens its terminal, not n8n", () => {
  const { buttons, calls, label } = errorCard(task("integration_vscode", "claudeCode", { sessionId: "s1", sessionCwd: "C:/repo" }));
  assert.deepEqual(buttons.map(label), ["Retry", "Open terminal"]);
  buttons[1].fire("click");
  assert.deepEqual(calls, [["openTerminal"]]);
});

test("a Claude desktop session's error card opens Claude; other agents get Open terminal", () => {
  assert.equal(errorOpenLabel(task("agent_claude-desktop", "agent")), "Open Claude");
  assert.equal(errorOpenLabel(task("agent_codex", "agent")), "Open terminal");
});

test("a local chat pill's error card has no open button (nothing to open)", () => {
  assert.equal(errorOpenLabel(task("local_ollama", "n8n")), null);
  const { buttons, label } = errorCard(task("local_ollama", "n8n"));
  assert.deepEqual(buttons.map(label), ["Retry"]);
});

test("no card offers Open in n8n any more", () => {
  assert.doesNotMatch(read("src/views/views.ts"), /Open in n8n/);
});
