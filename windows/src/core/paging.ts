// Writing suggestions a page: the island's list and the mini lantern's
// popover (1e, docs/phase1-plan.md §7a.3) page the same way.

/** Suggestions shown at once; Previous / Next page through the rest. */
export const SUGGESTIONS_SHOWN = 3;

/**
 * Which suggestions a page shows: `page` clamped to the pages there are,
 * and `from`-`to` (1-based, for "1–3 of 12"). Paging is by buttons only: the
 * island's window never takes focus, so the mouse wheel can't be relied on.
 */
export function suggestionPage(total: number, page: number): { page: number; pages: number; from: number; to: number } {
  const pages = Math.max(1, Math.ceil(total / SUGGESTIONS_SHOWN));
  const p = Math.min(Math.max(0, page), pages - 1);
  const from = total === 0 ? 0 : p * SUGGESTIONS_SHOWN + 1;
  return { page: p, pages, from, to: Math.min(total, (p + 1) * SUGGESTIONS_SHOWN) };
}
