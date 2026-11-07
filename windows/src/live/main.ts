// The live revision's preview window: wires LiveView to Rust. Listens only
// (capabilities/live.json); acts only through Glim's own live_expand command,
// and says once that it listens (live_page_ready: the first revision of a
// run came while this window was being made).

import "../style.css";
import "./live.css";
import { Bridge, onEvent } from "../core/bridge";
import { State } from "../core/state";
import { SHORTCUTS, effective } from "../core/shortcuts";
import { resolveLanguage, setLanguage, systemLanguages } from "../i18n/i18n";
import { LiveView, type LivePreviewEvent } from "./view";

async function main() {
  const boot = await Bridge.boot();
  if (boot) State.settings = { ...State.settings, ...boot.settings };
  setLanguage(resolveLanguage(State.settings.language, systemLanguages()));
  const accept = SHORTCUTS.find((d) => d.id === "liveAccept")!;
  const view = new LiveView({
    expand: (open) => void Bridge.liveExpand(open),
    acceptKeys: () => effective(accept, State.settings.shortcuts).keys,
  });
  document.getElementById("live-root")!.append(view.el);
  // Each revision drawn is answered (live_page_ready): Rust reloads or
  // rebuilds a page that stops answering (live_window.rs, 2026-11-07).
  await onEvent<LivePreviewEvent>("live-preview", (e) => {
    view.on(e);
    if (e.phase === "show") void Bridge.livePageReady();
  });
  void Bridge.livePageReady();
  await onEvent<typeof State.settings>("settings-changed", (s) => { State.settings = { ...State.settings, ...s }; });
}

void main();
