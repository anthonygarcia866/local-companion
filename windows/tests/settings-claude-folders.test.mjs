// Settings → Claude Code: every Claude config folder is listed with its state,
// and the one button says Install, Update (a folder behind) or Reinstall.
// Also: a rewrite blocked for a card or ID number says so (owner, 2026-10-13).

import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { folderRows, folderState, installLabel } = await import("../src/settings/claude-folders.ts");
const { failureText } = await import("../src/views/rewrite.ts");

const folder = (settingsPath, installed, upToDate) => ({ settingsPath, installed, upToDate });
const status = (folders) => ({
  installed: folders.some((f) => f.installed),
  allUpToDate: folders.every((f) => f.upToDate),
  folders,
  planRelayInstalled: false, settingsPath: "", hookPath: "", hookReady: true,
});

test("each folder is listed with its settings.json and its state", () => {
  const s = status([
    folder("C:\\Users\\me\\.claude\\settings.json", true, true),
    folder("C:\\Users\\me\\.claude2\\settings.json", true, false),
    folder("C:\\Users\\me\\.claude-work\\settings.json", false, false),
  ]);
  const rows = folderRows(s);
  const listed = rows.filter((r) => r.getAttribute("data-folder")).map((r) => [r.getAttribute("data-folder"), r.textContent]);
  assert.deepEqual(listed.map(([p]) => p), s.folders.map((f) => f.settingsPath));
  assert.ok(listed[0][1].includes("Hooks installed"));
  assert.ok(listed[1][1].includes("Hooks out of date"));
  assert.ok(listed[2][1].includes("No hooks"));
  assert.ok(rows.some((r) => r.textContent.includes("Update hooks to bring them all in line")), "says some are behind");
  assert.equal(folderState(folder("x", false, false)), "No hooks");
});

test("one button for every folder: Install, Update when one is behind, else Reinstall", () => {
  assert.equal(installLabel(status([folder("a", false, false), folder("b", false, false)])), "Install hooks…");
  assert.equal(installLabel(status([folder("a", true, true), folder("b", false, false)])), "Update hooks…");
  assert.equal(installLabel(status([folder("a", true, true), folder("b", true, true)])), "Reinstall hooks…");
  // All in line: no warning.
  assert.ok(!folderRows(status([folder("a", true, true)])).some((r) => r.textContent.includes("bring them all in line")));
});

test("a rewrite blocked for a card or ID number says so plainly", () => {
  assert.match(failureText("sensitive", null), /card, ID or bank number/);
  assert.notEqual(failureText("sensitive", null), failureText("no-paragraph", null));
});
