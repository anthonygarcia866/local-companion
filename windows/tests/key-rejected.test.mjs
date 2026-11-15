// A 401 from the Claude API (owner 2026-11-14): one clear card on the
// island naming where to fix the key, at most once an hour (Rust throttles,
// ai/key_rejected.rs), waiting while an approval card is up.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const { keyRejectedLine, showWhenFree, KEY_REJECTED_WAIT_MAX_MS, KEY_REJECTED_WAIT_STEP_MS } = await import("../src/core/key-rejected.ts");
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("the card says what happened and where to fix it", () => {
  assert.equal(keyRejectedLine(), "Glim's Claude key was rejected: update it in Settings → Writing → AI (Claude API)");
});

test("shown at once when nothing is up; after an approval otherwise; given up after 10 minutes", () => {
  let shown = 0;
  const queue = [];
  const later = (f) => queue.push(f);
  showWhenFree(() => false, () => shown++, later);
  assert.equal(shown, 1);

  let busy = true;
  showWhenFree(() => busy, () => shown++, later);
  assert.equal(shown, 1, "waits while an approval is up");
  queue.shift()();
  assert.equal(shown, 1);
  busy = false;
  queue.shift()();
  assert.equal(shown, 2, "shown once it's free");
  assert.equal(queue.length, 0);

  showWhenFree(() => true, () => shown++, later);
  let steps = 0;
  while (queue.length) {
    queue.shift()();
    steps++;
  }
  assert.equal(shown, 2);
  assert.equal(steps, KEY_REJECTED_WAIT_MAX_MS / KEY_REJECTED_WAIT_STEP_MS);
});

test("wired: Rust's event reaches the island's note card; claude::send reports every 401", () => {
  const main = read("src/main.ts");
  assert.ok(main.includes('onEvent<null>("api-key-rejected"'));
  assert.ok(main.includes('island.alert("note")'));
  assert.ok(read("src-tauri/src/ai/key_rejected.rs").includes('pub const EVENT: &str = "api-key-rejected";'));
  assert.ok(read("src-tauri/src/ai/claude.rs").includes("super::key_rejected::on_401();"));
});
