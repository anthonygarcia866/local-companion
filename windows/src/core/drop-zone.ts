// The file drop zone (1d retest, 2026-10-14). Rust arms it only while a file
// drag is over the pill or the ember, or within 40 px of it
// (island.rs drop_zone_armed); anywhere else Glim's window stays click-through
// so the drop reaches the folder or app under it. When the drag leaves that
// area, is dropped elsewhere or cancelled, the bucket folds away at once.

import type { IslandViewName } from "./layout";

/**
 * Where the island goes when a file drag leaves the zone: back to a card
 * waiting for an answer (or the suggestions), else folded. Only the bucket
 * itself ("upload") folds; a file already being taken ("uploading") stays.
 */
export function afterDragLeave(view: IslandViewName, expanded: boolean, waiting: IslandViewName | null): IslandViewName | "collapse" | "stay" {
  if (view !== "upload" || !expanded) return "stay";
  return waiting ?? "collapse";
}
