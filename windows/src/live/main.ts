// The live revision's preview window: wires LiveView to Rust. Listens only
// (capabilities/live.json); acts only through Glim's own live_expand command.

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
  await onEvent<LivePreviewEvent>("live-preview", (e) => view.on(e));
  await onEvent<typeof State.settings>("settings-changed", (s) => { State.settings = { ...State.settings, ...s }; });
}

void main();
