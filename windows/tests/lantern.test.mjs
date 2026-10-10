// The lantern mascot (src/mascot): generated from the approved design, unique
// ids per instance, the notch drops the large-context groups, compact below
// 48 px, and every island state maps to one of the design's seven.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { portCss, portSvg } from "../scripts/port-lantern.mjs";
import {
  COMPACT_BELOW_PX,
  LANTERN_STATES,
  RESERVED_LANTERN_STATES,
  lanternClass,
  lanternIds,
  lanternMarkup,
  lanternStateFor,
  shouldPop,
} from "../src/mascot/lantern.ts";

const WINDOWS = join(dirname(fileURLToPath(import.meta.url)), "..");
const DESIGN = readFileSync(join(WINDOWS, "../docs/brand/glim-mascot.html"), "utf8");
const read = (p) => readFileSync(join(WINDOWS, p), "utf8").replace(/\r\n/g, "\n");

test("src/mascot is what the port makes of the approved design", () => {
  assert.equal(read("src/mascot/lantern.css"), portCss(DESIGN));
  assert.equal(read("src/mascot/lantern-svg.ts"), portSvg(DESIGN));
});

test("the port doesn't depend on the design file's line endings", () => {
  // Windows CI checks the design out with CRLF (core.autocrlf); a dev box may have LF.
  const lf = DESIGN.replace(/\r\n/g, "\n");
  const crlf = lf.replace(/\n/g, "\r\n");
  assert.equal(portCss(crlf), portCss(lf));
  assert.equal(portSvg(crlf), portSvg(lf));
});

test("every group and class the design names is in the port", () => {
  const markup = lanternMarkup("t", "full");
  for (const cls of [
    "glow", "tone", "ribs-h", "ribs-v", "inner", "flame-g", "flame", "core", "eyes-wrap", "eyes", "eye", "hl",
    "brows", "happy", "face", "hands", "hand-l", "hand-r", "embers", "hw", "ring", "spark", "lift", "aura", "twinkles",
    "drop",
  ]) {
    assert.match(markup, new RegExp(`class="(?:[^"]* )?${cls}(?: [^"]*)?"`), cls);
  }
});

test("the CSS has a rule for each of the nine states, compact, and reduced motion", () => {
  assert.equal(LANTERN_STATES.length, 9);
  const css = read("src/mascot/lantern.css");
  for (const s of LANTERN_STATES) assert.ok(css.includes(`.lantern.s-${s} `), s);
  assert.ok(css.includes(".lantern.compact "));
  assert.ok(css.includes("@media (prefers-reduced-motion: reduce){.lantern *{animation:none !important}}"));
  // Nothing unscoped could reach the rest of the app.
  for (const rule of css.split("\n").slice(1)) {
    if (!rule || rule.startsWith("@")) continue;
    for (const sel of rule.slice(0, rule.indexOf("{")).split(",")) assert.ok(sel.startsWith(".lantern"), sel);
  }
});

test("the pop, the recording cue and the ignite are separate classes, still under reduced motion", () => {
  const css = read("src/mascot/lantern.css");
  for (const cls of ["pop", "pop-record", "ignite"]) assert.match(css, new RegExp(`\\.lantern\\.${cls}[{ ]`), cls);
  // About 1.15x over about 400 ms, as asked.
  assert.match(css, /\.lantern\.pop\{[^}]*animation:lantern-pop \.4s/);
  assert.match(css, /@keyframes lantern-pop\{[^\n]*scale\(1\.15\)/);
  // Reduced motion: no bounce, the glow alone changes.
  assert.ok(css.includes("@media (prefers-reduced-motion: reduce){.lantern.pop,.lantern.pop-record{animation:none}"));
  // The approved state rules don't mention them: the classes only add.
  for (const rule of css.split("\n")) {
    if (/\.s-[a-z]+/.test(rule)) assert.ok(!/\.(pop|ignite)\b/.test(rule), rule);
  }
});

test("every state change pops except idle and listen, and recording has its own cue", () => {
  assert.equal(shouldPop("idle", "listen"), false);
  assert.equal(shouldPop("listen", "idle"), false);
  assert.equal(shouldPop("idle", "idle"), false);
  for (const [a, b] of [["idle", "think"], ["listen", "suggest"], ["think", "done"], ["think", "error"], ["done", "idle"], ["paused", "idle"]]) {
    assert.equal(shouldPop(a, b), true, `${a} -> ${b}`);
  }
  // s-record repeats its stronger cue instead of a one-off pop.
  assert.equal(shouldPop("idle", "record"), false);
});

test("ids are unique per instance and every reference follows", () => {
  const a = lanternMarkup("a1", "full");
  const b = lanternMarkup("b2", "full");
  for (const id of lanternIds("a1")) {
    assert.ok(a.includes(`id="${id}"`), id);
    assert.ok(!b.includes(`id="${id}"`), id);
  }
  assert.ok(!a.includes("{ID}"));
  // No reference is left pointing at an id this instance doesn't define.
  for (const [, ref] of a.matchAll(/url\(#([^)]+)\)/g)) assert.ok(lanternIds("a1").includes(ref), ref);
});

test("the notch sprite has no aura or twinkles; large contexts keep them", () => {
  const notch = lanternMarkup("n", "notch");
  const full = lanternMarkup("f", "full");
  assert.ok(!notch.includes('class="aura"') && !notch.includes('class="twinkles"'));
  assert.ok(full.includes('class="aura"') && full.includes('class="twinkles"'));
  // Everything else is identical.
  const strip = (m) => m.replace(/<g class="aura"[\s\S]*?<\/g>/, "").replace(/<g class="twinkles"[\s\S]*?<\/g>/, "");
  assert.equal(notch, strip(lanternMarkup("n", "full")));
});

test("compact below 48 px of rendered height", () => {
  assert.equal(COMPACT_BELOW_PX, 48);
  assert.equal(lanternClass("think", 47.9), "lantern s-think compact");
  assert.equal(lanternClass("think", 48), "lantern s-think");
});

/** Every island state, read from the BotStateName union in core/layout.ts, so a
 * state added later is checked too. */
function botStates() {
  const src = read("src/core/layout.ts");
  const union = src.match(/export type BotStateName =([^;]+);/)[1];
  return [...union.matchAll(/"([a-z]+)"/g)].map((m) => m[1]);
}

test("every island state maps to one of the design's states", () => {
  const states = botStates();
  assert.equal(states.length, 11);
  const want = {
    idle: "idle", working: "think", thinking: "think", searching: "think", approval: "suggest", question: "suggest",
    error: "error", finished: "done", ratelimit: "paused", sleeping: "idle", dizzy: "think",
  };
  for (const s of states) assert.equal(lanternStateFor(s, false), want[s], s);
  assert.equal(lanternStateFor("idle", true), "listen", "an open island listens");
  for (const s of states) assert.ok(LANTERN_STATES.includes(lanternStateFor(s, true)));
});

// ── Reserved states ───────────────────────────────────────────────────────────
// Red s-record is screen recording only; s-delegate is Phase 3 data leaving the
// machine only (docs/brand/glim-mascot.html, CLAUDE.md). Neither feature exists
// yet. When one lands, add its file to SHOWS_RESERVED below with the owner it
// passes, and nowhere else.

const SHOWS_RESERVED = {
  // The dev-only `--mascot-state` switch (GLIM_DEV=1) previews any state.
  "src/island/island.ts": ["dev-preview"],
};

function sources(dir = "src", out = []) {
  for (const e of readdirSync(join(WINDOWS, dir), { withFileTypes: true })) {
    const p = `${dir}/${e.name}`;
    if (e.isDirectory()) sources(p, out);
    else if (/\.ts$/.test(e.name)) out.push(p);
  }
  return out;
}

test("no app state maps to a reserved state", () => {
  assert.deepEqual(Object.keys(RESERVED_LANTERN_STATES).sort(), ["delegate", "record"]);
  for (const s of botStates()) {
    for (const open of [false, true]) {
      const got = lanternStateFor(s, open);
      assert.ok(!(got in RESERVED_LANTERN_STATES), `${s} (open=${open}) maps to reserved s-${got}`);
    }
  }
});

test("reserved states are named only in the mascot module", () => {
  const mascot = ["src/mascot/lantern.ts", "src/mascot/lantern-svg.ts"];
  const named = /["'`](?:s-)?(?:record|delegate)["'`]|\bs-(?:record|delegate)\b/;
  for (const file of sources()) {
    if (mascot.includes(file)) continue;
    const lines = read(file).split("\n");
    lines.forEach((line, i) => assert.ok(!named.test(line), `${file}:${i + 1} names a reserved state: ${line.trim()}`));
  }
});

test("only the allowed owners call showReserved", () => {
  for (const file of sources()) {
    if (file === "src/mascot/lantern.ts") continue;
    const calls = [...read(file).matchAll(/showReserved\(([^)]*)\)/g)];
    if (!calls.length) continue;
    const allowed = SHOWS_RESERVED[file];
    assert.ok(allowed, `${file} calls showReserved but isn't an owner`);
    for (const [, args] of calls) {
      const owner = args.match(/["']([a-z0-9-]+)["']\s*$/)?.[1];
      assert.ok(allowed.includes(owner), `${file}: showReserved owner ${owner ?? args} isn't allowed there`);
    }
  }
});
