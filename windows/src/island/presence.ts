// How present the island is (src-tauri/src/presence.rs decides, this draws):
// the pill, the ember (a small glowing dot at the dock's edge, click it to get
// the pill back), hidden (the window is gone: nothing to draw), or the
// recording indicator, which stands in the ember's place whenever the island
// would otherwise be hidden while the screen is being recorded.

import { Bridge } from "../core/bridge";
import { Lantern, RECORDING_STATE } from "../mascot/lantern";
import { h } from "../views/dom";
import { DRAG_THRESHOLD } from "../mochi/desktop-logic";
import { tn } from "../i18n/i18n";

export type PresenceKind = "pill" | "ember" | "hidden" | "indicator";

export interface PresencePayload {
  kind: PresenceKind;
  visibility: "normal" | "ember" | "hidden";
  recording: boolean;
}

export function isPresenceKind(s: string): s is PresenceKind {
  return s === "pill" || s === "ember" || s === "hidden" || s === "indicator";
}

/** What the page draws for each kind. The indicator is the lantern in the
 *  reserved recording state (RECORDING_STATE), and nothing else ever is. */
export function presenceLook(kind: PresenceKind): { island: boolean; dot: boolean; record: boolean } {
  switch (kind) {
    case "pill":
      return { island: true, dot: false, record: false };
    case "ember":
      return { island: false, dot: true, record: false };
    case "indicator":
      return { island: false, dot: false, record: true };
    case "hidden":
      return { island: false, dot: false, record: false };
  }
}

/** What the island does for presence: bring the pill out. */
export interface PresenceHost {
  /** The compact pill on screen, from wherever the island's own state is. */
  showPill(): void;
  /** The island open on the writing suggestions (after an ember click). */
  openSuggestions(): void;
}

/**
 * The page's half, without the DOM. Pill means the pill is drawn: the island
 * is told to show it on every switch to Pill, whatever its own state machine
 * did meanwhile (it used to sit folded in its invisible wake strip, so Pill,
 * the hotkey's Hidden → Normal and the ember's click all showed nothing).
 */
export class PresenceModel {
  kind: PresenceKind = "pill";
  /** The writing checker's count, shown on the ember. */
  writing = 0;
  /** The ember was clicked to see the suggestions: the pill is out for that
   *  only, and Ember comes back when the island closes. */
  peeking = false;
  private openOnPill = false;
  private host: PresenceHost;

  constructor(host: PresenceHost) {
    this.host = host;
  }

  apply(kind: PresenceKind) {
    this.kind = kind;
    if (kind === "pill") {
      this.host.showPill();
      if (this.openOnPill) {
        this.openOnPill = false;
        this.host.openSuggestions();
      }
    } else {
      // Back to the ember (or hidden, or fullscreen): any peek is over.
      this.peeking = false;
      this.openOnPill = false;
    }
    return presenceLook(kind);
  }

  /**
   * A click on the ember or the recording indicator. With writing suggestions
   * waiting, the ember opens the island on them and comes back afterwards
   * (a peek, never saved); otherwise, back to the pill (Normal).
   */
  restore() {
    if (this.kind === "ember" && this.writing > 0) {
      this.peeking = true;
      this.openOnPill = true;
      void Bridge.setPeek(true);
      return;
    }
    void Bridge.setVisibility("normal");
  }

  /** The island closed: a peek ends, back to the ember. */
  islandClosed() {
    if (!this.peeking) return;
    this.peeking = false;
    void Bridge.setPeek(false);
  }
}

/** The ember's look with writing suggestions: brighter, with a count (no
 *  bounce). Nine and up show "9+": the window is 28 px. */
export function emberWriting(count: number): { lit: boolean; badge: string } {
  if (count <= 0) return { lit: false, badge: "" };
  return { lit: true, badge: count > 9 ? "9+" : String(count) };
}

/**
 * The ember's mouse: a click brings the pill back; a press that moves past
 * DRAG_THRESHOLD is a drag instead (Rust moves the window and snaps it to the
 * nearest dock), and the click that follows its release is not a restore.
 */
export class EmberGesture {
  private press: { x: number; y: number } | null = null;
  private dragged = false;

  down(x: number, y: number, button: number) {
    if (button !== 0) return;
    this.press = { x, y };
    this.dragged = false;
  }

  /** True when this move starts the drag. */
  move(x: number, y: number, buttons: number): boolean {
    const p = this.press;
    if (!p) return false;
    if (!(buttons & 1)) {
      this.press = null;
      return false;
    }
    if (Math.hypot(x - p.x, y - p.y) <= DRAG_THRESHOLD) return false;
    this.press = null;
    this.dragged = true;
    return true;
  }

  /** True when the click restores the pill (it wasn't the end of a drag). */
  click(): boolean {
    this.press = null;
    const restore = !this.dragged;
    this.dragged = false;
    return restore;
  }
}

/** The ember dot and the recording indicator, in the ember-sized window. */
export class PresenceView {
  readonly el: HTMLElement;
  readonly model: PresenceModel;
  private dot: HTMLElement;
  private badge: HTMLElement;
  private indicator = new Lantern({ detail: "notch", state: "idle", heightPx: 22 });

  constructor(host: PresenceHost) {
    this.model = new PresenceModel(host);
    this.dot = h("div", { class: "ember-dot" });
    this.badge = h("div", { class: "ember-count", "aria-hidden": "true" });
    this.indicator.el.classList.add("ember-indicator");
    this.el = h("button", { id: "ember", type: "button", "aria-label": "Glim" }, this.dot, this.badge, this.indicator.el);
    // Click: back to the pill (the indicator too: it is the island, just
    // smaller). Drag: move it to another dock.
    const gesture = new EmberGesture();
    this.el.addEventListener("mousedown", (e) => gesture.down(e.clientX, e.clientY, e.button));
    window.addEventListener("mousemove", (e) => {
      if (gesture.move(e.clientX, e.clientY, e.buttons)) void Bridge.dockDragStart();
    });
    this.el.addEventListener("click", () => {
      if (gesture.click()) this.model.restore();
    });
    // No browser menu on the dot.
    this.el.addEventListener("contextmenu", (e) => e.preventDefault());
    this.apply("pill");
  }

  apply(kind: PresenceKind) {
    const look = this.model.apply(kind);
    document.documentElement.dataset.presence = kind;
    this.dot.hidden = !look.dot;
    this.indicator.el.style.display = look.record ? "" : "none";
    if (look.record) this.indicator.showReserved(RECORDING_STATE, "screen-recording");
    this.drawWriting();
  }

  /** The writing checker's count changed. */
  setWriting(count: number) {
    this.model.writing = count;
    this.drawWriting();
  }

  private drawWriting() {
    const { lit, badge } = emberWriting(this.model.writing);
    const shown = lit && this.model.kind === "ember";
    this.el.classList.toggle("lit", shown);
    this.badge.textContent = badge;
    this.badge.hidden = !shown;
    this.el.setAttribute(
      "aria-label",
      shown ? `Glim: ${tn("{count} writing suggestion", "{count} writing suggestions", this.model.writing)}` : "Glim",
    );
  }
}
