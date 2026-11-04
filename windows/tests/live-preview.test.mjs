// The live revision's preview (src/live/view.ts): the word diff, three
// lines at most until "…", the accept key's hint, notes after the key, and
// the window wiring (vite input, a listen-only capability).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { LiveView, truncate, noteText, MAX_CHARS } = await import("../src/live/view.ts");
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

const OPS = [
  { kind: "same", text: "We " },
  { kind: "del", text: "recieved " },
  { kind: "add", text: "received " },
  { kind: "same", text: "the payment." },
];

function view() {
  const calls = [];
  const v = new LiveView({ expand: (open) => calls.push(open), acceptKeys: () => "Ctrl+Alt+Shift+Enter" });
  return { v, calls };
}

test("a revision shows struck-through removals, highlighted additions and the key hint", () => {
  const { v } = view();
  assert.ok(v.el.classList.contains("is-hidden"), "hidden until a revision comes");
  v.on({ phase: "show", id: 1, ops: OPS, app: "Word", why: "" });
  assert.ok(!v.el.classList.contains("is-hidden"));
  assert.equal(v.el.find("del").map((d) => d.textContent).join(""), "recieved ");
  assert.equal(v.el.find("ins").map((d) => d.textContent).join(""), "received ");
  const hint = v.el.find("div").find((d) => d.getAttribute("data-role") === "live-hint");
  assert.equal(hint.textContent, "Ctrl+Alt+Shift+Enter to apply");
  assert.equal(v.el.find("button").length, 0, "short: no …");
  v.on({ phase: "hide", id: 1, ops: [], app: "", why: "typing" });
  assert.ok(v.el.classList.contains("is-hidden"));
});

test("past three lines it stops at a word with …, and … expands it", () => {
  const long = [{ kind: "same", text: "word ".repeat(80) }, { kind: "add", text: "end." }];
  const cut = truncate(long);
  assert.ok(cut.cut);
  const drawn = cut.ops.map((o) => o.text).join("");
  assert.ok(drawn.length <= MAX_CHARS && drawn.endsWith(" "), "cut at a word's end");
  assert.deepEqual(truncate(OPS), { ops: OPS, cut: false });
  const { v, calls } = view();
  v.on({ phase: "show", id: 2, ops: long, app: "Word", why: "" });
  const more = v.el.find("button").find((b) => b.getAttribute("data-role") === "live-more");
  assert.equal(more.textContent, "…");
  assert.equal(more.getAttribute("tabindex"), "-1", "never takes the keyboard");
  more.fire("click");
  assert.deepEqual(calls, [true]);
  assert.ok(v.el.find("ins").some((i) => i.textContent === "end."), "the whole revision once expanded");
  // A new revision starts short again.
  v.on({ phase: "show", id: 3, ops: long, app: "Word", why: "" });
  assert.ok(v.el.find("button").some((b) => b.getAttribute("data-role") === "live-more"));
});

test("notes after the accept key", () => {
  assert.equal(noteText("changed", "Word"), "Text changed: wait for a new suggestion");
  assert.equal(noteText("copied", "Teams"), "Copied: Glim doesn't type in Teams yet. Paste with Ctrl+V.");
  assert.equal(noteText("unconfirmed", "Word"), "Couldn't apply it in Word.");
  const { v } = view();
  v.on({ phase: "note", id: 1, ops: [], app: "Word", why: "changed" });
  assert.ok(!v.el.classList.contains("is-hidden"));
  assert.equal(v.el.find("div").find((d) => d.getAttribute("data-role") === "live-note").textContent, "Text changed: wait for a new suggestion");
});

test("the window is built like the mini lantern's and only listens", () => {
  assert.match(read("vite.config.ts"), /live: resolve\(__dirname, "live\.html"\)/);
  const cap = JSON.parse(read("src-tauri/capabilities/live.json"));
  assert.deepEqual(cap.windows, ["live"]);
  assert.deepEqual(cap.permissions, ["core:event:allow-listen", "core:event:allow-unlisten"]);
  const rs = read("src-tauri/src/live_window.rs");
  assert.ok(rs.includes(".focusable(false)") && rs.includes("make_non_activating"));
});
