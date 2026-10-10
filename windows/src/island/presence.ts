// How present the island is (src-tauri/src/presence.rs decides, this draws):
// the pill, the ember (a small glowing dot at the dock's edge, click it to get
// the pill back), hidden (the window is gone: nothing to draw), or the
// recording indicator, which stands in the ember's place whenever the island
// would otherwise be hidden while the screen is being recorded.

import { Bridge } from "../core/bridge";
import { Lantern, RECORDING_STATE } from "../mascot/lantern";
import { h } from "../views/dom";

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
}

/**
 * The page's half, without the DOM. Pill means the pill is drawn: the island
 * is told to show it on every switch to Pill, whatever its own state machine
 * did meanwhile (it used to sit folded in its invisible wake strip, so Pill,
 * the hotkey's Hidden → Normal and the ember's click all showed nothing).
 */
export class PresenceModel {
  kind: PresenceKind = "pill";
  private host: PresenceHost;

  constructor(host: PresenceHost) {
    this.host = host;
  }

  apply(kind: PresenceKind) {
    this.kind = kind;
    if (kind === "pill") this.host.showPill();
    return presenceLook(kind);
  }

  /** A click on the ember or the recording indicator: back to the pill. */
  restore() {
    void Bridge.setVisibility("normal");
  }
}

/** The ember dot and the recording indicator, in the ember-sized window. */
export class PresenceView {
  readonly el: HTMLElement;
  readonly model: PresenceModel;
  private dot: HTMLElement;
  private indicator = new Lantern({ detail: "notch", state: "idle", heightPx: 22 });

  constructor(host: PresenceHost) {
    this.model = new PresenceModel(host);
    this.dot = h("div", { class: "ember-dot" });
    this.indicator.el.classList.add("ember-indicator");
    this.el = h("button", { id: "ember", type: "button", "aria-label": "Glim" }, this.dot, this.indicator.el);
    // Back to the pill. The indicator too: it is the island, just smaller.
    this.el.addEventListener("click", () => this.model.restore());
    this.apply("pill");
  }

  apply(kind: PresenceKind) {
    const look = this.model.apply(kind);
    document.documentElement.dataset.presence = kind;
    this.dot.hidden = !look.dot;
    this.indicator.el.style.display = look.record ? "" : "none";
    if (look.record) this.indicator.showReserved(RECORDING_STATE, "screen-recording");
  }
}
