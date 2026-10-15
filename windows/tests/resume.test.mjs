// "Pick up where you left off" (views/resume.ts; src-tauri/src/resume/):
// the card lists repos newest first, a click on a row's button opens Claude
// Code there by the repo's name only, nothing opens without that click, and
// the card says when it couldn't.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { State } = await import("../src/core/state.ts");
const { buildResume, closeResume, openNote, RESUME_ROWS } = await import("../src/views/resume.ts");
const { sent } = await import("./tauri.mjs");

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const repo = (name, o = {}) => ({ repo: name, goal: `${name} goal`, inProgress: `${name}: the next step`, branch: "main", lastCommit: "abc1234", hasQuestions: false, withheld: [], updated: 0, ...o });

function card(cards) {
  State.resume = cards;
  State.view = "resume";
  State.mode = "expanded";
  const calls = [];
  const v = buildResume({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  v.sync();
  return { v, calls };
}

test("one row per repo in Rust's order (newest first), at most three, saying where it stopped", () => {
  const { v } = card([repo("local-companion", { hasQuestions: true, branch: "ai-check" }), repo("dale-cuts"), repo("crm"), repo("old")]);
  const rows = v.el.find(".resume-row");
  assert.equal(rows.length, RESUME_ROWS);
  assert.deepEqual(rows.map((r) => r.getAttribute("data-repo")), ["local-companion", "dale-cuts", "crm"]);
  assert.ok(rows[0].textContent.includes("local-companion: the next step"));
  assert.ok(rows[0].textContent.includes("Branch ai-check · Has open questions"));
  assert.ok(v.el.textContent.includes("Pick up where you left off"));
  State.mode = "hidden";
});

test("nothing opens until a row's button is clicked, and then only by the repo's name", async () => {
  const before = sent("resume_open").length;
  const { v } = card([repo("local-companion")]);
  assert.equal(sent("resume_open").length, before, "showing the card opens nothing");
  v.el.find("button").find((b) => b.getAttribute("data-role") === "resume-open").fire("click");
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent("resume_open").at(-1), { repo: "local-companion" });
  State.mode = "hidden";
});

test("the notes, the withheld line, an empty list, and Close", () => {
  assert.equal(openNote("dale-cuts", true), "Opening Claude Code in dale-cuts…");
  assert.equal(openNote("dale-cuts", false), "Couldn't open Claude Code in dale-cuts. Is it installed?");
  let { v } = card([repo("x", { withheld: ["in_progress"] })]);
  assert.ok(v.el.textContent.includes("Part of it is withheld"));
  ({ v } = card([]));
  assert.ok(v.el.textContent.includes("No HANDOFF.md found"));
  const { calls } = card([repo("x")]);
  closeResume({ setView: (n) => calls.push(["view", n]), collapse: () => calls.push(["collapse"]) });
  assert.equal(State.resume, null);
  assert.deepEqual(calls.at(-1), ["collapse"]);
  State.mode = "hidden";
});

test("Rust: the folder comes from the repos Glim reads, never from the island; the dev switch is one of its triggers", () => {
  const lib = read("src-tauri/src/lib.rs");
  assert.ok(lib.includes("fn resume_open(repo: String) -> bool"));
  assert.ok(lib.includes("resume::folder_for(&repo, &resume::repos(&[]))"));
  assert.ok(lib.includes('argv.iter().any(|a| a == "--resume-card")'));
  assert.ok(lib.includes("if dev_session() && argv"), "dev-only until the everyday trigger is decided");
  const win = read("src-tauri/src/platform/windows.rs");
  const fn = win.slice(win.indexOf("pub fn open_claude_code_in"), win.indexOf("pub fn find_on_path"));
  assert.ok(!/cmd\.exe|powershell|"\/c"/.test(fn), "no shell");
  assert.ok(fn.includes(".arg(prompt)"));
});

test("the card's triggers (decided 2026-10-15): the tray item, the resume event, the dev switch; never by itself", () => {
  const tray = read("src-tauri/src/tray.rs");
  assert.match(tray, /\("resume", n_\("Pick up where you left off"\)\)/);
  assert.match(tray, /\(id == "open" \|\| id == "resume"\) && crate::presence::current\(app\)/, "it brings the pill back from Hidden, as Open does");
  const main = read("src/main.ts");
  // showResume is called only from the tray's "resume" and the "resume-show" event.
  const calls = [...main.matchAll(/showResume\(\)/g)].map((m) => main.slice(Math.max(0, m.index - 120), m.index));
  assert.equal(calls.length, 2);
  assert.ok(calls.some((c) => c.includes('case "resume":')));
  assert.ok(calls.some((c) => c.includes('"resume-show"')));
  // Rust emits "resume-show" only for the dev switch on this branch (voice adds resume_work).
  const lib = read("src-tauri/src/lib.rs");
  assert.equal([...lib.matchAll(/"resume-show"/g)].length, 1);
});
