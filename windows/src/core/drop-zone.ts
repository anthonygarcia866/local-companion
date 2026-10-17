// The file drop zone (1d retest, 2026-10-14). Rust arms it only while a file
// drag is over the pill or the ember, or within 40 px of it
// (island.rs drop_zone_armed); anywhere else Glim's window stays click-through
// so the drop reaches the folder or app under it. When the drag leaves that
// area, is dropped elsewhere or cancelled, the bucket folds away at once.

import type { IslandViewName } from "./layout";

/**
 * One file drag near the pill. `enter` notes how the island looked before
 * the bucket opened; `leave` (the drag left the arming area, dropped
 * elsewhere, or Esc) says where to go back to: folded if it was folded, else
 * the same view. Never to "what's waiting": the writing suggestions are
 * nearly always waiting, and the pill stayed open (retest, 2026-10-16).
 * Only the bucket itself ("upload") folds; a file already being taken
 * ("uploading") stays.
 */
export class DropZoneSession {
  private before: { expanded: boolean; view: IslandViewName } | null = null;

  enter(expanded: boolean, view: IslandViewName) {
    if (!this.before) this.before = { expanded, view };
  }

  leave(view: IslandViewName, expanded: boolean): IslandViewName | "collapse" | "stay" {
    const before = this.before;
    this.before = null;
    if (view !== "upload" || !expanded) return "stay";
    if (!before || !before.expanded || before.view === "upload" || before.view === "uploading") return "collapse";
    return before.view;
  }

  /** The file landed on the bucket: the drop sequence takes it from here. */
  drop() {
    this.before = null;
  }
}
