// Live pages that redraw on a timer must never take a field away from the
// person typing in it (owner 2026-11-21: Settings → Remote lost focus every
// few seconds while pairing a phone). Two rules for every such page:
//
//  1. A timer-driven redraw waits while a field inside it has focus
//     (`deferWhileTyping`), and runs once focus leaves the part.
//  2. Any redraw (also one a button asks for) puts focus, caret and text back
//     into the new field with the same data-role (`redrawKeeping`).
//
// Unsaved text in a field that does NOT have focus is the page's own job: it
// keeps a draft and writes it back on redraw.

type Field = HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;

const NOT_TEXT = new Set(["button", "submit", "reset", "checkbox", "radio", "file", "image", "range", "color"]);

function isField(el: Element | null | undefined): el is Field {
  if (!el) return false;
  const tag = el.tagName?.toLowerCase();
  if (tag === "textarea" || tag === "select") return true;
  if (tag !== "input") return false;
  return !NOT_TEXT.has(((el as HTMLInputElement).type || "text").toLowerCase());
}

/** The field inside `root` that has focus, if any. */
export function typingIn(root: Element): Field | null {
  const a = (globalThis.document as Document | undefined)?.activeElement;
  if (!isField(a)) return null;
  return root.contains?.(a) ? a : null;
}

/**
 * Runs `fill` (which clears `root` and draws it again). If a field inside
 * `root` had focus, the new field with the same data-role gets its text,
 * caret and focus back, so typing goes on where it was.
 */
export function redrawKeeping(root: HTMLElement, fill: () => void): void {
  const was = typingIn(root);
  const role = was?.getAttribute("data-role");
  const value = was?.value;
  let start: number | null = null;
  let end: number | null = null;
  try {
    start = (was as HTMLInputElement | null)?.selectionStart ?? null;
    end = (was as HTMLInputElement | null)?.selectionEnd ?? null;
  } catch {
    // number inputs have no selection
  }
  fill();
  if (!was || !role) return;
  if (root.contains(was)) return; // not replaced: nothing to do
  const now = Array.from(root.querySelectorAll(`[data-role="${role}"]`)).filter(isField);
  if (now.length !== 1) return;
  const field = now[0];
  if (value !== undefined && field.value !== value) field.value = value;
  field.focus();
  if (start !== null && end !== null) {
    try {
      (field as HTMLInputElement).setSelectionRange(start, end);
    } catch {
      // not a text field
    }
  }
}

/**
 * For timer-driven redraws of `root`: `later(fn)` runs `fn` now, or, while a
 * field inside `root` has focus, once focus has left `root` (only the
 * latest `fn` runs).
 */
export function deferWhileTyping(root: HTMLElement): (fn: () => void) => void {
  let pending: (() => void) | null = null;
  root.addEventListener("focusout", () => {
    // Focus is still moving: look once it has landed.
    setTimeout(() => {
      if (!pending || typingIn(root)) return;
      const f = pending;
      pending = null;
      f();
    }, 0);
  });
  return (fn) => {
    if (typingIn(root)) pending = fn;
    else {
      pending = null;
      fn();
    }
  };
}
