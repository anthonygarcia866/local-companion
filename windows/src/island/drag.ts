// Where a press in the open island may start dragging Glim to another dock:
// the lantern (island.ts isBotHit) or the top bar, in every view, cards with
// buttons included. A press on a control never starts a drag, so buttons keep
// working; and the click that ends a drag is swallowed (island.ts), so a drag
// never presses a button either.

/** The top bar of the open island: the 8 px inset plus the 34 px header (layout.ts botPosition). */
export const TOP_BAR_HEIGHT = 42;

const CONTROL_TAGS = new Set(["BUTTON", "A", "INPUT", "TEXTAREA", "SELECT", "LABEL"]);

/** True when `el` or one of its ancestors is something a press operates. */
export function isControl(el: unknown): boolean {
  let node = el as { tagName?: string; getAttribute?(k: string): string | null; isContentEditable?: boolean; parentElement?: unknown } | null;
  while (node) {
    if (node.tagName && CONTROL_TAGS.has(node.tagName.toUpperCase())) return true;
    const role = node.getAttribute?.("role");
    if (role === "button" || role === "link" || role === "slider" || role === "textbox") return true;
    if (node.isContentEditable) return true;
    node = (node.parentElement as typeof node) ?? null;
  }
  return false;
}

/**
 * A left press at `y` (island coordinates: 0 is the island's top) on
 * `target` may start a drag from the top bar.
 */
export function startsBarDrag(button: number, y: number, target: unknown): boolean {
  return button === 0 && y >= 0 && y < TOP_BAR_HEIGHT && !isControl(target);
}
