// Settings → Privacy (1d, docs/phase1-plan.md §14.5): the user's pause
// entries are checked as Rust checks them (the shared fixture), added once,
// refused with the reason, and removed; the built-in ones are shown locked
// and can't be removed.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { userApp, userSite, addEntry, removeEntry } = await import("../src/core/pause.ts");
const { privacySection, refusalText } = await import("../src/settings/privacy.ts");
const { DEFAULT_SETTINGS } = await import("../src/core/state.ts");

const FIXTURE = JSON.parse(readFileSync(new URL("./fixtures/pause-entries.json", import.meta.url), "utf8"));
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");

/** The built-in lists, read from Rust's source so the two can't drift. */
function rustList(name) {
  const src = read("src-tauri/src/capture/sensitive.rs");
  const body = src.slice(src.indexOf(`pub const ${name}`)).split("];")[0];
  return [...body.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}
const BUILTIN = { apps: rustList("PAUSED_APPS"), sites: rustList("PAUSED_SITES") };

test("entries are checked exactly as Rust checks them", () => {
  assert.equal(BUILTIN.apps.length, 12);
  assert.equal(BUILTIN.sites.length, 33);
  for (const c of FIXTURE.apps) {
    const got = userApp(c.in, BUILTIN.apps);
    assert.deepEqual(got, c.out !== undefined ? { ok: true, entry: c.out } : { ok: false, reason: c.reason }, JSON.stringify(c.in));
  }
  for (const c of FIXTURE.sites) {
    const got = userSite(c.in, BUILTIN.sites);
    assert.deepEqual(got, c.out !== undefined ? { ok: true, entry: c.out } : { ok: false, reason: c.reason }, JSON.stringify(c.in));
  }
  for (const c of [...FIXTURE.apps, ...FIXTURE.sites].filter((c) => c.reason)) assert.ok(refusalText(c.reason), c.reason);
});

test("an entry is added once and removed", () => {
  assert.deepEqual(addEntry(["Quicken.exe"], "QUICKEN.EXE"), ["Quicken.exe"]);
  assert.deepEqual(addEntry([], "a.com"), ["a.com"]);
  assert.deepEqual(removeEntry(["a.com", "b.com"], "a.com"), ["b.com"]);
});

test("the section adds, refuses and removes the user's entries; built-ins are locked", () => {
  const settings = { ...DEFAULT_SETTINGS, pauseApps: ["Quicken.exe"], pauseSites: [] };
  let saves = 0;
  const el = privacySection({ settings: () => settings, save: async () => { saves++; } }, BUILTIN);
  const input = (role) => el.find("input").find((i) => i.getAttribute("data-role") === role);
  const button = (role) => el.find("button").find((b) => b.getAttribute("data-role") === role);

  // A site, typed with its scheme: stored in the built-ins' form.
  input("pause-sites").value = "https://Portal.Example.com/";
  button("pause-sites-add").fire("click");
  assert.deepEqual(settings.pauseSites, ["portal.example.com"]);

  // A refused one says why and stores nothing.
  input("pause-apps").value = "C:\\Apps\\qw.exe";
  input("pause-apps").fire("input");
  button("pause-apps-add").fire("click");
  assert.deepEqual(settings.pauseApps, ["Quicken.exe"]);
  assert.ok(el.textContent.includes("Just the file name, without a folder."), el.textContent);

  // Built-ins can't be added again, and have no Remove button.
  input("pause-apps").value = "KeePass.exe";
  button("pause-apps-add").fire("click");
  assert.deepEqual(settings.pauseApps, ["Quicken.exe"]);
  const removable = el.find("button").filter((b) => b.textContent === "Remove").map((b) => b.getAttribute("data-entry"));
  assert.deepEqual(removable, ["Quicken.exe", "portal.example.com"]);
  assert.ok(el.textContent.includes("1Password.exe") && el.textContent.includes("paypal.com"), "the locked list is shown");

  el.find("button").find((b) => b.getAttribute("data-entry") === "Quicken.exe").fire("click");
  assert.deepEqual(settings.pauseApps, []);
  assert.equal(saves, 2);
});
