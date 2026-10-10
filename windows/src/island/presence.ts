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

/** The ember dot and the recording indicator, in the ember-sized window. */
export class PresenceView {
  readonly el: HTMLElement;
  private dot: HTMLElement;
  private indicator = new Lantern({ detail: "notch", state: "idle", heightPx: 22 });

  constructor() {
    this.dot = h("div", { class: "ember-dot" });
    this.indicator.el.classList.add("ember-indicator");
    this.el = h("button", { id: "ember", type: "button", "aria-label": "Glim" }, this.dot, this.indicator.el);
    // Back to the pill. The indicator too: it is the island, just smaller.
    this.el.addEventListener("click", () => void Bridge.setVisibility("normal"));
    this.apply("pill");
  }

  apply(kind: PresenceKind) {
    const look = presenceLook(kind);
    document.documentElement.dataset.presence = kind;
    this.dot.hidden = !look.dot;
    this.indicator.el.style.display = look.record ? "" : "none";
    if (look.record) this.indicator.showReserved(RECORDING_STATE, "screen-recording");
  }
}
