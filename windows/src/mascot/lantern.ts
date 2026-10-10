// Glim, the lantern mascot — the approved design (docs/brand/glim-mascot.html)
// as a reusable component. The markup and CSS are generated from that file by
// scripts/port-lantern.mjs (lantern-svg.ts, lantern.css); this module only
// instantiates them. Its look is the design's: don't restyle it here.
//
// - State is one class on the <svg>: s-idle … s-delegate. CSS transitions and
//   animations do all the motion; changing state is swapping that class.
// - "compact" is added below COMPACT_BELOW_PX of rendered height (fewer
//   details, bigger eyes), as the design specifies.
// - Detail "notch" drops the aura (ribbons) and twinkles (sparkles), which the
//   design reserves for large contexts; "full" keeps everything.
// - Every gradient, filter and clip path id is made unique per instance.
// - prefers-reduced-motion stops every animation (rule in lantern.css).

import { LANTERN_IDS, LANTERN_MARKUP, LANTERN_VIEWBOX } from "./lantern-svg";
import type { BotStateName } from "../core/layout";

export const LANTERN_STATES = ["idle", "listen", "think", "suggest", "record", "paused", "delegate"] as const;
export type LanternState = (typeof LANTERN_STATES)[number];

export type LanternDetail = "full" | "notch";

/** Rendered height under which the design switches to its compact drawing. */
export const COMPACT_BELOW_PX = 48;

/** The design's lantern spans y = 6 (top of the handle) to y = 105 (bottom of
 * the base) of its 124-unit-tall viewBox; the glow reaches past it. */
export const LANTERN_DRAWN_TOP = 6;
export const LANTERN_DRAWN_BOTTOM = 105;

/** How tall the island draws the lantern (handle to base) for the old orb's
 * diameter: 24 px in the 26 px notch pill (diameter 20). */
export const LANTERN_HEIGHT_PER_DIAMETER = 1.2;

export function isLanternState(s: string): s is LanternState {
  return (LANTERN_STATES as readonly string[]).includes(s);
}

/** The design's markup for one instance: ids suffixed with `uid`, and the
 * large-context groups dropped for the notch. */
export function lanternMarkup(uid: string, detail: LanternDetail): string {
  let markup = LANTERN_MARKUP.replaceAll("{ID}", uid);
  if (detail === "notch") {
    markup = markup.replace(/<g class="aura"[\s\S]*?<\/g>/, "").replace(/<g class="twinkles"[\s\S]*?<\/g>/, "");
  }
  return markup;
}

/** The ids one instance defines. */
export function lanternIds(uid: string): string[] {
  return LANTERN_IDS.map((id) => `${id}-${uid}`);
}

/** The class attribute of the <svg> for a state and a rendered height. */
export function lanternClass(state: LanternState, heightPx: number): string {
  return `lantern s-${state}${heightPx < COMPACT_BELOW_PX ? " compact" : ""}`;
}

/**
 * The lantern state for the island's character state. `open` is whether the
 * island is expanded (Glim attending to you rather than dozing in the notch).
 *
 * - idle → s-idle in the notch, s-listen when the island is open
 * - working, thinking, searching → s-think (an agent is busy)
 * - approval, question → s-suggest (waiting for your answer)
 * - finished → s-delegate (happy eyes, both hands up: done)
 * - error → s-record (the red hue and pulsing ring are the design's only alert)
 * - ratelimit, sleeping → s-paused (lantern dimmed, flame out)
 * - dizzy (the shake gag) → s-think (the swaying flame)
 */
export function lanternStateFor(state: BotStateName, open: boolean): LanternState {
  switch (state) {
    case "idle":
      return open ? "listen" : "idle";
    case "working":
    case "thinking":
    case "searching":
    case "dizzy":
      return "think";
    case "approval":
    case "question":
      return "suggest";
    case "finished":
      return "delegate";
    case "error":
      return "record";
    case "ratelimit":
    case "sleeping":
      return "paused";
  }
}

let instances = 0;
const SVG_NS = "http://www.w3.org/2000/svg";

/** One lantern on the page. */
export class Lantern {
  readonly el: SVGSVGElement;
  readonly uid: string;
  private stateName: LanternState;
  private heightPx = 0;

  constructor(opts: { detail: LanternDetail; state?: LanternState; heightPx?: number }) {
    this.uid = `g${++instances}`;
    this.stateName = opts.state ?? "idle";
    this.el = document.createElementNS(SVG_NS, "svg");
    this.el.setAttribute("viewBox", LANTERN_VIEWBOX);
    this.el.setAttribute("aria-hidden", "true");
    this.el.innerHTML = lanternMarkup(this.uid, opts.detail);
    this.setHeight(opts.heightPx ?? 0);
  }

  get state(): LanternState {
    return this.stateName;
  }

  set state(next: LanternState) {
    if (next === this.stateName) return;
    this.stateName = next;
    this.apply();
  }

  /** The <svg>'s rendered height in CSS pixels; picks compact or not. */
  setHeight(px: number) {
    this.heightPx = px;
    this.apply();
  }

  private apply() {
    const cls = lanternClass(this.stateName, this.heightPx);
    if (this.el.getAttribute("class") !== cls) this.el.setAttribute("class", cls);
  }
}
