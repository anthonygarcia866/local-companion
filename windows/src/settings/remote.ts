// Settings → Remote (local agency v1, part 3; owner 2026-11-12): "Keep
// Claude Code reachable". Off by default; the first time it is turned on, a
// card says what it does and asks once ("Turn on" / "Not now"). The list is
// Rust's (keepalive.rs): project folders, each with the FIXED command shown
// here, and optionally the Claude desktop app. Each entry's status comes
// from Rust every 5 s while the page is open.
//
// Below it, "Remote tasks" (remote courier core, owner 2026-11-14; remote/):
// off by default with its own one-time card, the signer's public key
// (pasted; only its fingerprint is ever shown back), the kill switch (also
// in the tray) and the Remote activity list (times, outcomes, command kinds
// or refusal reasons; never a payload). No transport is wired yet.

import { Bridge, type KeepAliveEntry, type KeepAliveStatus, type RemoteActivity, type RemoteStatus } from "../core/bridge";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

/** "14:05" from seconds since 1970, local time. */
export function clock(secs: number): string {
  const d = new Date(secs * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function entryState(e: KeepAliveEntry, on: boolean): string {
  if (!on) return t("Off");
  switch (e.state) {
    case "running": return t("Running since {time}", { time: clock(e.at) });
    case "waiting": return t("Stopped (exit code {code}); starting again at {time}", { code: String(e.lastExit ?? "?"), time: clock(e.at) });
    case "start-failed": return t("Couldn't start ({why}); trying again at {time}", { why: e.why, time: clock(e.at) });
    default: return t("Starting…");
  }
}

let timer: ReturnType<typeof setInterval> | null = null;

export function remoteSection(toggle: (on: boolean, onChange: (v: boolean) => void) => HTMLElement): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    { id: "settings-remote" },
    h("h2", {}, h("span", { text: t("Remote") })),
    h("div", {
      class: "hint",
      text: t("Keeps Claude Code's remote control running for the folders below, so you can reach this PC's Claude Code from your phone or claude.ai. Each one starts when Glim starts, in a terminal window you can see, and starts again if it stops. Nothing else is ever run."),
    }),
    body,
  );
  let status: KeepAliveStatus = { settings: { on: false, consented: false, folders: [], desktopApp: false }, entries: [] };
  let consenting = false;
  let note = "";
  const tasks = remoteTasks(toggle);

  async function act(f: () => Promise<KeepAliveStatus | null>) {
    try {
      status = (await f()) ?? status;
      note = "";
    } catch (e) {
      note = String(e);
    }
    draw();
  }

  function consentCard(): HTMLElement {
    const yes = h("button", { class: "primary", type: "button", "data-role": "remote-consent-yes", text: t("Turn on") });
    const no = h("button", { type: "button", "data-role": "remote-consent-no", text: t("Not now") });
    yes.addEventListener("click", () => {
      consenting = false;
      void act(() => Bridge.keepAliveSet(true, true));
    });
    no.addEventListener("click", () => {
      consenting = false;
      draw();
    });
    return h("div", { class: "card", "data-role": "remote-consent" },
      h("div", { class: "name", text: t("Keep Claude Code reachable?") }),
      h("div", { class: "hint", text: t("Glim will start Claude Code's remote control for each folder on the list whenever Glim starts, and restart it if it stops. Remote control connects to Anthropic so that you, signed in to your Claude account, can use these sessions from another device. Glim itself sends nothing. You can turn it off here at any time.") }),
      h("div", { class: "row" }, yes, no),
    );
  }

  function draw() {
    clear(body);
    const s = status.settings;
    const sw = toggle(s.on, (v) => {
      if (v && !s.consented) {
        consenting = true;
        draw();
        return;
      }
      void act(() => Bridge.keepAliveSet(v, false));
    });
    body.append(h("div", { class: "row", "data-role": "remote-on" }, h("label", { text: t("Keep Claude Code reachable") }), sw));
    if (consenting) body.append(consentCard());

    const list = h("div", { class: "pill-group", "data-role": "remote-list" });
    list.append(h("h3", { text: t("Folders") }));
    if (s.folders.length === 0) list.append(h("div", { class: "hint", text: t("No folders yet.") }));
    for (const e of status.entries.filter((x) => x.folder)) {
      const remove = h("button", { class: "danger", type: "button", "data-role": "remote-remove", text: t("Remove") });
      remove.addEventListener("click", () => void act(() => Bridge.keepAliveRemove(e.folder)));
      list.append(h("div", { "data-role": "remote-entry" },
        h("div", { class: "pill-row" }, h("span", { class: "name", text: e.folder }), remove),
        h("div", { class: "path", text: e.command }),
        h("div", { class: "hint", "data-role": "remote-state", text: entryState(e, s.on && s.consented) }),
        ...(e.failedLastHour >= 3 ? [h("div", { class: "hint", style: "color:#f4505e", text: t("Stopped {n} times in the last hour.", { n: String(e.failedLastHour) }) })] : []),
      ));
    }
    const input = h("input", { type: "text", "data-role": "remote-folder", placeholder: t("Project folder, like C:\\path\\to\\project"), spellcheck: "false" }) as HTMLInputElement;
    const add = h("button", { type: "button", "data-role": "remote-add", text: t("Add") });
    add.addEventListener("click", () => {
      const folder = input.value;
      void act(() => Bridge.keepAliveAdd(folder));
    });
    list.append(h("div", { class: "row" }, input, add));
    if (note) list.append(h("div", { class: "hint", "data-role": "remote-note", style: "color:#f4505e", text: note }));
    body.append(list);

    const desktop = status.entries.find((x) => !x.folder);
    body.append(h("div", { class: "row", "data-role": "remote-desktop" },
      h("label", { text: t("Also keep the Claude desktop app open") }),
      toggle(s.desktopApp, (v) => void act(() => Bridge.keepAliveDesktop(v))),
      ...(desktop ? [h("span", { class: "hint", text: entryState(desktop, s.on && s.consented) })] : []),
    ));
    body.append(tasks);
  }

  async function load() {
    status = (await Bridge.keepAliveStatus()) ?? status;
    draw();
  }

  draw();
  void load();
  if (timer) clearInterval(timer);
  timer = setInterval(() => void load(), 5000);
  // Never what keeps a process alive (Node runs this page's tests).
  (timer as { unref?: () => void }).unref?.();
  return section;
}

/** One activity line: "14:05 Accepted: session-status" / "14:06 Refused: expired". */
export function activityLine(a: RemoteActivity): string {
  if (a.kind === "kill-switch") return `${clock(a.at)} ${t("Stop remote tasks")}`;
  if (a.kind === "allowed-again") return `${clock(a.at)} ${t("Allow again")}`;
  return `${clock(a.at)} ${a.ok ? t("Accepted: {kind}", { kind: a.kind }) : t("Refused: {reason}", { reason: a.reason })}`;
}

/** Settings → Remote → Remote tasks (remote/). */
export function remoteTasks(toggle: (on: boolean, onChange: (v: boolean) => void) => HTMLElement): HTMLElement {
  const box = h("div", { class: "pill-group", "data-role": "remote-tasks" });
  let status: RemoteStatus = { on: false, consented: false, killed: false, fingerprint: "", activity: [] };
  let consenting = false;
  let note = "";

  async function act(f: () => Promise<RemoteStatus | null>) {
    try {
      status = (await f()) ?? status;
      note = "";
    } catch (e) {
      note = String(e);
    }
    draw();
  }

  function draw() {
    clear(box);
    box.append(h("h3", { text: t("Remote tasks") }));
    box.append(h("div", { class: "hint", text: t("Lets tasks you sign on your phone reach this PC (not connected yet). Only tasks signed with the key below are accepted, each once and within 10 minutes.") }));
    const sw = toggle(status.on, (v) => {
      if (v && !status.consented) {
        consenting = true;
        draw();
        return;
      }
      void act(() => Bridge.remoteSet(v, false));
    });
    box.append(h("div", { class: "row", "data-role": "remote-tasks-on" }, h("label", { text: t("Accept remote tasks") }), sw));
    if (consenting) {
      const yes = h("button", { class: "primary", type: "button", "data-role": "remote-tasks-yes", text: t("Turn on") });
      const no = h("button", { type: "button", "data-role": "remote-tasks-no", text: t("Not now") });
      yes.addEventListener("click", () => {
        consenting = false;
        void act(() => Bridge.remoteSet(true, true));
      });
      no.addEventListener("click", () => {
        consenting = false;
        draw();
      });
      box.append(h("div", { class: "card", "data-role": "remote-tasks-consent" },
        h("div", { class: "name", text: t("Accept remote tasks?") }),
        h("div", { class: "hint", text: t("Glim will accept tasks signed with your key: a session's status, the end of its transcript, starting or messaging a Claude Code session in an allowed project, and runbooks you approved. At most 5 new sessions and 30 tasks an hour. Stop them any time from the tray or here.") }),
        h("div", { class: "row" }, yes, no),
      ));
    }

    const input = h("input", { type: "text", "data-role": "remote-key", placeholder: t("Paste the 64 hex characters"), spellcheck: "false", autocomplete: "off" }) as HTMLInputElement;
    const save = h("button", { type: "button", "data-role": "remote-key-save", text: t("Save key") });
    save.addEventListener("click", () => {
      const key = input.value;
      void act(() => Bridge.remoteKey(key));
    });
    box.append(h("label", { text: t("Signer's public key") }), h("div", { class: "row" }, input, save));
    if (status.fingerprint) box.append(h("div", { class: "hint", "data-role": "remote-fingerprint", text: t("Fingerprint: {fingerprint}", { fingerprint: status.fingerprint }) }));

    if (status.killed) {
      const again = h("button", { type: "button", "data-role": "remote-allow", text: t("Allow again") });
      again.addEventListener("click", () => void act(() => Bridge.remoteKill(false)));
      box.append(h("div", { class: "row", "data-role": "remote-killed" }, h("span", { class: "hint", style: "color:#f4505e", text: t("Remote tasks are stopped.") }), again));
    } else {
      const stop = h("button", { class: "danger", type: "button", "data-role": "remote-kill", text: t("Stop remote tasks") });
      stop.addEventListener("click", () => void act(() => Bridge.remoteKill(true)));
      box.append(h("div", { class: "row" }, stop));
    }
    if (note) box.append(h("div", { class: "hint", "data-role": "remote-tasks-note", style: "color:#f4505e", text: note }));

    const list = h("div", { "data-role": "remote-activity" }, h("h3", { text: t("Remote activity") }));
    if (status.activity.length === 0) list.append(h("div", { class: "hint", text: t("Nothing yet.") }));
    for (const a of status.activity.slice(0, 20)) list.append(h("div", { class: "hint", "data-role": "remote-activity-line", text: activityLine(a) }));
    box.append(list);
  }

  draw();
  void (async () => {
    status = (await Bridge.remoteStatus()) ?? status;
    draw();
  })();
  return box;
}
