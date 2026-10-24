// The mini lantern's page (1e, docs/phase1-plan.md §7a; src/bubble/view.ts):
// the lantern and its badge where Rust puts them, the popover's rows (the
// island's own), Rewrite… and Close in the footer, the 20 s no-pointer close,
// the field-left close, the data-leaving look during a rewrite, and the
// setting off by default. The window's focus rules are Rust's tests
// (bubble_window.rs).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State, DEFAULT_SETTINGS } = await import("../src/core/state.ts");
const { BubbleView, bubbleBadge } = await import("../src/bubble/view.ts");
const { BUBBLE_AUTO_CLOSE_MS } = await import("../src/core/bubble.ts");
const { bubbleRow } = await import("../src/settings/writing.ts");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const item = (problem) => ({ start: 0, end: problem.length, problem, before: "", after: " x", kind: "Spelling", message: "m", replacements: ["fix"], clear: true });
const placed = (o = {}) => ({ visible: true, look: "suggest", count: 3, size: 22, lanternX: 0, lanternY: 0, open: false, popoverX: 0, popoverY: 28, above: false, ...o });

const looks = [];
const fakeLantern = {
  el: document.createElement("svg"),
  show: (s) => looks.push(s),
  showReserved: (s, owner) => looks.push(`${s}:${owner}`),
  setHeight: () => {},
};

function bubble() {
  let clock = 0;
  const calls = [];
  const view = new BubbleView({
    setOpen: (open) => calls.push(["open", open]),
    fix: (...a) => calls.push(["fix", ...a]),
    copy: (...a) => calls.push(["copy", ...a]),
    ignore: (...a) => calls.push(["ignore", ...a]),
    rewrite: () => calls.push(["rewrite"]),
    now: () => clock,
  }, fakeLantern);
  view.setList(["recieved", "tomorow", "the the", "dont"].map(item), { checkId: 5, mode: "apply", app: "Notepad" });
  view.setWindow(placed());
  return { view, calls, tick: (ms) => { clock += ms; view.step({ kind: "tick", now: clock }); } };
}

const lantern = (view) => view.el.find("button").find((b) => b.getAttribute("class") === "bubble-lantern" || b.className === "bubble-lantern");
const popover = (view) => view.el.querySelector(".bubble-popover");

test("the badge says the count, 9+ past nine", () => {
  assert.equal(bubbleBadge(1), "1");
  assert.equal(bubbleBadge(9), "9");
  assert.equal(bubbleBadge(12), "9+");
  const { view } = bubble();
  assert.equal(view.el.querySelector(".bubble-badge").textContent, "3");
});

test("a click opens the popover with the island's rows, three a page; a click on a replacement fixes by ids", () => {
  const { view, calls } = bubble();
  assert.equal(popover(view).style.display, "none");
  lantern(view).fire("click");
  assert.deepEqual(calls, [["open", true]], "Rust lays the window out for it");
  view.setWindow(placed({ open: true }));
  assert.equal(popover(view).style.display, "");
  assert.equal(view.el.querySelector(".suggestion-list").find(".suggestion").length, 3);
  assert.ok(popover(view).textContent.includes("1–3 of 4"));
  popover(view).find("button").find((b) => b.textContent === "fix").fire("click");
  assert.deepEqual(calls.at(-1), ["fix", 5, 0, 0]);
  // Next page.
  popover(view).find("button").find((b) => b.getAttribute("aria-label") === "Next suggestions").fire("click");
  assert.equal(view.el.querySelector(".suggestion-list").find(".suggestion").length, 1);
});

test("Ignore on a popover row sends the list and row ids; Rust's new list recounts and pages it", () => {
  const { view, calls } = bubble();
  lantern(view).fire("click");
  view.setWindow(placed({ open: true }));
  const ignores = popover(view).find("button").filter((b) => b.getAttribute("data-role") === "ignore");
  assert.equal(ignores.length, 3, "one per row");
  assert.equal(ignores[1].getAttribute("aria-label"), "Ignore “tomorow” until Glim quits");
  ignores[1].fire("click");
  assert.deepEqual(calls.at(-1), ["ignore", 5, 1]);
  // Rust sends the list again without it: three left, one page, still open.
  view.setList(["recieved", "the the", "dont"].map(item), { checkId: 6, mode: "apply", app: "Notepad" });
  assert.equal(view.el.querySelector(".suggestion-list").find(".suggestion").length, 3);
  assert.ok(popover(view).textContent.includes("3 writing suggestions"));
  // The last one ignored: the list empties and the popover closes.
  view.setList([], { checkId: 0, mode: "view", app: "" });
  assert.equal(popover(view).style.display, "none");
});

test("the footer: Rewrite… asks for the tones and closes; Close closes", () => {
  const { view, calls } = bubble();
  lantern(view).fire("click");
  view.setWindow(placed({ open: true }));
  popover(view).find("button").find((b) => b.getAttribute("data-role") === "bubble-rewrite").fire("click");
  assert.deepEqual(calls.slice(-2), [["rewrite"], ["open", false]]);
  lantern(view).fire("click");
  view.setWindow(placed({ open: true }));
  popover(view).find("button").find((b) => b.getAttribute("data-role") === "bubble-close").fire("click");
  assert.deepEqual(calls.at(-1), ["open", false]);
});

test("20 s with no pointer over it closes it; the pointer over it keeps it", () => {
  assert.equal(BUBBLE_AUTO_CLOSE_MS, 20_000);
  const { view, calls, tick } = bubble();
  lantern(view).fire("click");
  view.setWindow(placed({ open: true }));
  popover(view).fire("mouseenter");
  tick(60_000);
  assert.equal(view.state.open, true, "pointer over it");
  popover(view).fire("mouseleave");
  tick(19_000);
  assert.equal(view.state.open, true);
  tick(1_000);
  assert.equal(view.state.open, false);
  assert.deepEqual(calls.at(-1), ["open", false]);
});

test("the field left, or the lantern parked, closes the popover; a rewrite shows no count", () => {
  const { view, calls } = bubble();
  lantern(view).fire("click");
  view.step({ kind: "field-left" });
  assert.deepEqual(calls.at(-1), ["open", false]);
  lantern(view).fire("click");
  view.setWindow(placed({ visible: false }));
  assert.equal(view.state.open, false);
  assert.equal(lantern(view).style.display, "none");
  view.setWindow(placed({ look: "delegate", count: 0 }));
  assert.equal(view.el.querySelector(".bubble-badge").style.display, "none");
  assert.equal(looks.at(-1), "delegate:api-request", "a rewrite: the data-leaving look");
  view.setWindow(placed());
  assert.equal(looks.at(-1), "suggest");
});

test("the setting is off by default and the switch stores it", () => {
  assert.equal(DEFAULT_SETTINGS.writingBubble, false);
  const settings = { ...DEFAULT_SETTINGS };
  let saves = 0;
  const row = bubbleRow({ settings: () => settings, save: async () => { saves++; } });
  const sw = row.find("button").find((b) => b.getAttribute("data-role") === "writing-bubble");
  assert.equal(sw.getAttribute("aria-pressed"), "false");
  sw.fire("click");
  assert.equal(settings.writingBubble, true);
  assert.equal(saves, 1);
});

test("the page registers no keys and the window is its own vite entry with a listen-only capability", () => {
  for (const f of ["src/bubble/view.ts", "src/bubble/main.ts"]) {
    const code = read(f);
    for (const banned of ["keydown", "keyup", "focus()", "localStorage"]) assert.ok(!code.includes(banned), `${f}: ${banned}`);
  }
  assert.match(read("vite.config.ts"), /bubble: resolve\(__dirname, "bubble\.html"\)/);
  const cap = JSON.parse(read("src-tauri/capabilities/bubble.json"));
  assert.deepEqual(cap.windows, ["bubble"]);
  assert.deepEqual(cap.permissions, ["core:event:allow-listen", "core:event:allow-unlisten"]);
});
