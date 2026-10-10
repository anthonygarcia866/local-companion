// Dev-only debug panel for the Phase 0b text-capture spike (GLIM_DEV=1).
// Shows the latest reading of the focused field in another app, live. The
// text exists only in this page's memory while it is shown: nothing here
// stores it (no localStorage, no files, no console logging).

import { onEvent } from "../core/bridge";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface CaptureMeta {
  app: string;
  controlType: string;
  pattern: string;
  readable: boolean;
  password: boolean;
  charCount: number;
  caret: number | null;
}

interface Capture {
  meta: CaptureMeta;
  windowTitle: string;
  text: string | null;
  via: string;
}

const root = document.getElementById("capture-root")!;

function row(label: string, value: string, strong = false): HTMLElement {
  const el = document.createElement("div");
  el.style.cssText = "display:flex;gap:10px;padding:2px 0";
  const k = document.createElement("span");
  k.textContent = label;
  k.style.cssText = "width:110px;color:#8b90a0;flex:none";
  const v = document.createElement("span");
  v.textContent = value;
  if (strong) v.style.fontWeight = "600";
  el.append(k, v);
  return el;
}

let lastTitle = "";

function render(c: Capture) {
  const m = c.meta;
  const box = document.createElement("div");
  box.style.cssText = "padding:12px 14px";
  const verdict = m.password ? "skipped (password field)" : m.readable ? "yes" : "no";
  box.append(
    row("process", m.app, true),
    row("window", c.windowTitle),
    row("control type", m.controlType),
    row("pattern", m.pattern),
    row("readable", verdict, true),
    row("chars", String(m.charCount)),
    row("caret", m.caret == null ? "—" : String(m.caret)),
    row("via", c.via),
  );
  const text = document.createElement("pre");
  text.style.cssText =
    "margin:10px 0 0;padding:8px;background:#16181d;border-radius:6px;white-space:pre-wrap;word-break:break-word;max-height:190px;overflow:auto;font:12px/1.4 ui-monospace,Consolas,monospace";
  text.textContent = m.password ? "(not read)" : c.text ?? "(nothing readable)";
  box.append(text);
  root.replaceChildren(box);
  // The window title carries the metadata only (never the text), so the
  // panel can be checked from outside without capturing its pixels.
  const title = `Capture debug — ${m.app} | ${m.controlType} | ${m.pattern} | readable=${m.readable} | chars=${m.charCount} | via=${c.via}`;
  if (title !== lastTitle) {
    lastTitle = title;
    void getCurrentWindow().setTitle(title);
  }
}

void onEvent<Capture>("capture-debug", render);
