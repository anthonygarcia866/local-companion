// The live revision's preview (writing/live.rs, live_window.rs): the
// word-level diff (removed words struck through, added ones highlighted),
// three lines at most until "…" is clicked, and the accept key's hint.
// The window never takes focus or keys: clicking "…" only asks Rust to make
// it taller; the accept and dismiss keys are global shortcuts in Rust.

import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

export interface DiffOp {
  kind: "same" | "del" | "add";
  text: string;
}

export interface LivePreviewEvent {
  /** "show" a revision, "hide" it, or a "note" after the accept key. */
  phase: "show" | "hide" | "note";
  id: number;
  ops: DiffOp[];
  app: string;
  /** "hide": why; "note": the outcome code (changed, copied, blocked …). */
  why: string;
}

/** About three lines of the preview at its width (440 px, 13 px text). */
export const MAX_CHARS = 186;

/** The ops cut to `max` characters of what is drawn; whether anything was left out. */
export function truncate(ops: DiffOp[], max = MAX_CHARS): { ops: DiffOp[]; cut: boolean } {
  const out: DiffOp[] = [];
  let left = max;
  for (const op of ops) {
    if (left <= 0) return { ops: out, cut: true };
    if (op.text.length <= left) {
      out.push(op);
      left -= op.text.length;
    } else {
      // Cut at a word's end where there is one.
      const part = op.text.slice(0, left);
      const space = part.lastIndexOf(" ");
      out.push({ kind: op.kind, text: space > 0 ? part.slice(0, space + 1) : part });
      return { ops: out, cut: true };
    }
  }
  return { ops: out, cut: false };
}

/** What a note after the accept key says. */
export function noteText(code: string, app: string): string {
  switch (code) {
    case "changed": return t("Text changed: wait for a new suggestion");
    case "copied": return t("Copied: Glim doesn't type in {app} yet. Paste with Ctrl+V.", { app });
    default: return t("Couldn't apply it in {app}.", { app });
  }
}

export interface LiveHost {
  /** "…" clicked: Rust makes the window taller (or back). */
  expand(open: boolean): void;
  /** The accept key as the user has it ("Ctrl+Alt+Shift+Enter"). */
  acceptKeys(): string;
}

export class LiveView {
  readonly el = h("div", { class: "live-preview is-hidden", "data-role": "live-preview" });
  private current: LivePreviewEvent | null = null;
  private open = false;

  private host: LiveHost;

  constructor(host: LiveHost) {
    this.host = host;
  }

  on(e: LivePreviewEvent): void {
    if (e.phase === "hide") {
      this.current = null;
      this.el.classList.add("is-hidden");
      clear(this.el);
      return;
    }
    if (e.phase === "note") {
      clear(this.el);
      this.el.append(h("div", { class: "live-note", role: "status", "data-role": "live-note", text: noteText(e.why, e.app) }));
      this.el.classList.remove("is-hidden");
      return;
    }
    this.current = e;
    this.open = false;
    this.draw();
  }

  private draw(): void {
    const e = this.current;
    if (!e) return;
    clear(this.el);
    const { ops, cut } = this.open ? { ops: e.ops, cut: false } : truncate(e.ops);
    const text = h("div", { class: "live-diff", "data-role": "live-diff" });
    for (const op of ops) {
      text.append(op.kind === "same" ? h("span", { text: op.text }) : h(op.kind === "del" ? "del" : "ins", { class: `live-${op.kind}`, text: op.text }));
    }
    if (cut) {
      const more = h("button", { class: "live-more", type: "button", tabindex: "-1", "data-role": "live-more", "aria-label": t("Show all"), text: "…" });
      more.addEventListener("click", () => {
        this.open = true;
        this.host.expand(true);
        this.draw();
      });
      text.append(more);
    }
    this.el.append(text, h("div", { class: "live-hint", "data-role": "live-hint", text: t("{keys} to apply", { keys: this.host.acceptKeys() }) }));
    this.el.classList.remove("is-hidden");
  }
}
