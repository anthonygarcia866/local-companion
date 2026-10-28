// The browser twin of tests/tauri.mjs: a stand-in for the object Tauri
// injects into the webview, loaded before any page script (Playwright
// addInitScript). The page talks to "Rust" through it unchanged: every command
// it invokes is recorded, replies come from window.__glim.replies, and the
// tests fire the events Rust would emit with window.__glim.emit.
//
// window.__glimLabel (set by an earlier init script) is the window's label:
// "island", "settings"...
(() => {
  const label = window.__glimLabel || "island";
  const calls = [];
  const replies = {};
  const listeners = new Map(); // event -> [handler]
  let next = 1;
  window.__glim = {
    calls,
    replies,
    sent: (cmd) => calls.filter(([name]) => name === cmd).map(([, args]) => args),
    emit: (event, payload) => {
      const hs = listeners.get(event) || [];
      if (!hs.length) throw new Error(`nothing listens to "${event}"`);
      for (const h of hs) h({ event, id: 0, payload });
    },
    listening: (event) => (listeners.get(event) || []).length > 0,
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label }, currentWebview: { label, windowLabel: label } },
    transformCallback: (callback) => callback,
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
    invoke: async (cmd, args) => {
      if (cmd === "plugin:event|listen") {
        const hs = listeners.get(args.event) || [];
        hs.push(args.handler);
        listeners.set(args.event, hs);
        return next++;
      }
      if (cmd.startsWith("plugin:")) return null;
      calls.push([cmd, args]);
      const r = replies[cmd];
      return typeof r === "function" ? r(args) : r ?? null;
    },
  };
})();
