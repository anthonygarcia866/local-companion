// Glim, the lantern mascot — the approved design (docs/brand/glim-mascot.html)
// as a reusable component. The markup and CSS are generated from that file by
// scripts/port-lantern.mjs (lantern-svg.ts, lantern.css); this module only
// instantiates them. Its look is the design's: don't restyle it here.
//
// - State is one class on the <svg>: s-idle … s-error. CSS transitions and
//   animations do all the motion; changing state is swapping that class.
// - "compact" is added below COMPACT_BELOW_PX of rendered height (fewer
//   details, bigger eyes), as the design specifies.
// - Detail "notch" drops the aura (ribbons) and twinkles (sparkles), which the
//   design reserves for large contexts; "full" keeps everything.
// - Every gradient, filter and clip path id is made unique per instance.
// - prefers-reduced-motion stops every animation (rule in lantern.css).

import { LANTERN_IDS, LANTERN_MARKUP, LANTERN_VIEWBOX } from "./lantern-svg";
import type { BotStateName } from "../core/layout";

export const LANTERN_STATES = [
  "idle", "listen", "think", "suggest", "record", "paused", "delegate", "done", "error",
] as const;
export type LanternState = (typeof LANTERN_STATES)[number];

/**
 * States the design reserves, and the one feature allowed to show each
 * (docs/brand/glim-mascot.html, CLAUDE.md): red s-record is screen recording
 * only; s-delegate is Phase 3 data leaving the machine only. Neither feature
 * exists yet, so no app state maps to them. They can only be shown through
 * `Lantern.showReserved`, which names its owner; tests/lantern.test.mjs checks
 * who calls it.
 */
export const RESERVED_LANTERN_STATES = {
  record: "screen-recording",
  delegate: "phase3-delegation",
} as const;
export type ReservedLanternState = keyof typeof RESERVED_LANTERN_STATES;
/** s-record, for the recording indicator (island/presence.ts), which shows it
 *  through `showReserved` with the "screen-recording" owner. */
export const RECORDING_STATE = "record" satisfies ReservedLanternState;
/** Every state an app state may map to. */
export type AppLanternState = Exclude<LanternState, ReservedLanternState>;

export function isReservedLanternState(s: LanternState): s is ReservedLanternState {
  return s in RESERVED_LANTERN_STATES;
}

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
 * - finished → s-done (happy eyes, a hop)
 * - error → s-error (dim, droopy, worried brows, a sweat drop; never red)
 * - ratelimit → s-paused: the agent hit its usage limit and can't work until it
 *   resets — "not working right now", dimmed with the flame out
 * - sleeping → s-idle: nothing is blocked, Glim is just resting (only the
 *   desktop character ever sets it, after a stretch with no agent activity);
 *   s-idle's dozing eyes and slow breathing are that
 * - dizzy (the shake gag) → s-think (the swaying flame)
 *
 * Never s-record or s-delegate: those belong to features, not app states
 * (RESERVED_LANTERN_STATES), and the return type excludes them.
 */
export function lanternStateFor(state: BotStateName, open: boolean): AppLanternState {
  switch (state) {
    case "idle":
      return open ? "listen" : "idle";
    case "sleeping":
      return "idle";
    case "working":
    case "thinking":
    case "searching":
    case "dizzy":
      return "think";
    case "approval":
    case "question":
      return "suggest";
    case "finished":
      return "done";
    case "error":
      return "error";
    case "ratelimit":
      return "paused";
  }
}

/** How long the attention pop lasts (the design's `.pop`: 400 ms). */
export const POP_MS = 450;
/** The startup ignite: dark for IGNITE_DARK_MS, then the flame lights over
 *  the design's 800 ms `.ignite`. */
export const IGNITE_DARK_MS = 300;
export const IGNITE_MS = 850;

/**
 * Whether a state change gets the attention pop. Every change does except
 * idle ↔ listen (the island opening and closing: far too frequent), and
 * s-record has its own stronger, repeating cue (`pop-record`) instead.
 */
export function shouldPop(prev: LanternState, next: LanternState): boolean {
  if (prev === next) return false;
  if (next === "record") return false;
  const calm = new Set<LanternState>(["idle", "listen"]);
  return !(calm.has(prev) && calm.has(next));
}

/** The lantern's state through a file drop (src/upload): listening for the
 *  file, thinking while it loads, done at the check mark. */
export function uploadLanternState(f: { check: number; barAlpha: number; progress: number }): AppLanternState {
  if (f.check > 0) return "done";
  if (f.barAlpha > 0 && f.progress > 0) return "think";
  return "listen";
}

let instances = 0;
const SVG_NS = "http://www.w3.org/2000/svg";

/** One lantern on the page. */
export class Lantern {
  readonly el: SVGSVGElement;
  readonly uid: string;
  private stateName: LanternState;
  private heightPx = 0;
  /** Classes on top of the state: "pop", "pop-record", "ignite". */
  private extra = new Set<string>();
  private popTimer = 0;
  /** While igniting, app states wait: the startup sequence plays out first. */
  private igniting = false;
  /** The latest app state asked for while igniting: shown once it's over. */
  private pending: AppLanternState | null = null;
  private igniteTimers: number[] = [];

  constructor(opts: { detail: LanternDetail; state?: AppLanternState; heightPx?: number }) {
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

  /** Shows an app state. Reserved states can't be passed here. */
  show(next: AppLanternState) {
    if (this.igniting) {
      this.pending = next;
      return;
    }
    this.set(next);
  }

  /** Whether the startup ignite is still playing. */
  get isIgniting(): boolean {
    return this.igniting;
  }

  /**
   * Shows a reserved state, for its owner only: `record` for screen recording,
   * `delegate` for Phase 3 delegation, either for the dev preview switch
   * (`--mascot-state`). tests/lantern.test.mjs fails on any other caller.
   */
  showReserved<S extends ReservedLanternState>(state: S, owner: (typeof RESERVED_LANTERN_STATES)[S] | "dev-preview") {
    void owner;
    // A reserved state is a feature speaking (recording above all): it never
    // waits for the startup sequence.
    this.stopIgnite();
    this.set(state);
  }

  /** The startup sequence: dark (s-paused), then the flame lights and the
   *  lantern settles into s-idle. App states resume afterwards. */
  ignite() {
    this.stopIgnite();
    this.igniting = true;
    this.stateName = "paused";
    this.extra.clear();
    this.apply();
    this.igniteTimers.push(window.setTimeout(() => {
      this.stateName = "idle";
      this.extra.add("ignite");
      this.apply();
      this.igniteTimers.push(window.setTimeout(() => {
        this.stopIgnite();
        if (this.pending) this.set(this.pending);
        this.pending = null;
      }, IGNITE_MS));
    }, IGNITE_DARK_MS));
  }

  private stopIgnite() {
    for (const id of this.igniteTimers) window.clearTimeout(id);
    this.igniteTimers = [];
    this.igniting = false;
    this.extra.delete("ignite");
    this.apply();
  }

  private set(next: LanternState) {
    if (next === this.stateName) return;
    const prev = this.stateName;
    this.stateName = next;
    if (next === "record") this.extra.add("pop-record");
    else this.extra.delete("pop-record");
    if (shouldPop(prev, next)) this.pop();
    this.apply();
  }

  /** The attention pop: a brief bounce and glow flare, then it settles. */
  private pop() {
    window.clearTimeout(this.popTimer);
    // Restart the animation even if a pop is still running.
    this.extra.delete("pop");
    this.apply();
    void this.el.getBoundingClientRect();
    this.extra.add("pop");
    this.popTimer = window.setTimeout(() => {
      this.extra.delete("pop");
      this.apply();
    }, POP_MS);
  }

  /** The <svg>'s rendered height in CSS pixels; picks compact or not. */
  setHeight(px: number) {
    this.heightPx = px;
    this.apply();
  }

  private apply() {
    const extra = [...this.extra].join(" ");
    const cls = lanternClass(this.stateName, this.heightPx) + (extra ? ` ${extra}` : "");
    if (this.el.getAttribute("class") !== cls) this.el.setAttribute("class", cls);
  }
}
