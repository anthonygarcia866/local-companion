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
// or refusal reasons; never a payload). Since the courier (owner 2026-11-14,
// remote/courier.rs): the relay's address and the connection's state, and
// "Set up the relay", which makes this PC's relay token and result key.
// Cloud clients (owner 2026-11-15): one per Claude account, each with its
// name, its signer's fingerprint and the date it was added; "Add cloud
// client" takes the client's PUBLIC key and makes its own relay token,
// shown exactly once; "Remove" refuses its tasks at once.

import { Bridge, type KeepAliveEntry, type KeepAliveStatus, type RemoteActivity, type RemoteClient, type RemoteStatus, type PairView } from "../core/bridge";
import { h, clear } from "../views/dom";
import { deferWhileTyping, redrawKeeping } from "../views/typing";
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
  // Remote tasks sits after the keep-alive part and is never detached by its
  // redraw: moving a node takes focus from the field being typed in
  // (owner 2026-11-21: fields lost focus every 5 s while pairing a phone).
  const tasks = remoteTasks(toggle);
  const section = h(
    "section",
    { id: "settings-remote" },
    h("h2", {}, h("span", { text: t("Remote") })),
    h("div", {
      class: "hint",
      text: t("Keeps Claude Code's remote control running for the folders below, so you can reach this PC's Claude Code from your phone or claude.ai. Each one starts when Glim starts, in a terminal window you can see, and starts again if it stops. Nothing else is ever run."),
    }),
    h("div", { style: "display:flex;flex-direction:column;gap:12px" }, body, tasks),
  );
  let status: KeepAliveStatus = { settings: { on: false, consented: false, folders: [], desktopApp: false }, entries: [] };
  let consenting = false;
  let note = "";
  /** The folder typed but not added yet: kept across redraws. */
  let folderDraft = "";
  /** The 5 s refresh waits while a field here has focus. */
  const later = deferWhileTyping(body);

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
    redrawKeeping(body, fill);
  }

  function fill() {
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
        h("div", { class: "pill-row" }, h("span", { class: "name", text: e.account ? `${e.folder} · ${e.account}` : e.folder }), remove),
        h("div", { class: "path", text: e.command }),
        h("div", { class: "hint", "data-role": "remote-state", text: entryState(e, s.on && s.consented) }),
        ...(e.failedLastHour >= 3 ? [h("div", { class: "hint", style: "color:#f4505e", text: t("Stopped {n} times in the last hour.", { n: String(e.failedLastHour) }) })] : []),
      ));
    }
    const input = h("input", { type: "text", "data-role": "remote-folder", placeholder: t("Project folder, like C:\\path\\to\\project"), spellcheck: "false" }) as HTMLInputElement;
    input.value = folderDraft;
    input.addEventListener("input", () => { folderDraft = input.value; });
    const add = h("button", { type: "button", "data-role": "remote-add", text: t("Add") });
    add.addEventListener("click", () => {
      const folder = input.value;
      void act(async () => {
        const next = await Bridge.keepAliveAdd(folder);
        folderDraft = "";
        return next;
      });
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
  }

  /** The 5 s refresh: redraws only when something changed, and never under a field with focus. */
  async function load() {
    const next = (await Bridge.keepAliveStatus()) ?? status;
    if (JSON.stringify(next) === JSON.stringify(status)) return;
    status = next;
    later(draw);
  }

  draw();
  void load();
  if (timer) clearInterval(timer);
  timer = setInterval(() => void load(), 5000);
  // Never what keeps a process alive (Node runs this page's tests).
  (timer as { unref?: () => void }).unref?.();
  return section;
}

/** One activity line: "14:05 account-2 · Accepted: session-status" / "14:06 Refused: expired". */
export function activityLine(a: RemoteActivity): string {
  if (a.kind === "kill-switch") return `${clock(a.at)} ${t("Stop remote tasks")}`;
  if (a.kind === "allowed-again") return `${clock(a.at)} ${t("Allow again")}`;
  // Leases (owner 2026-11-21): granted, and what the client did once it ended.
  if (a.kind === "lease-granted" || a.kind === "lease-ended") return `${clock(a.at)} ${a.client} · ${a.summary ?? ""}`;
  const who = a.client ? `${a.client} · ` : "";
  const kind = a.runbook ? `${a.kind} ${a.runbook}` : a.kind;
  return `${clock(a.at)} ${who}${a.ok ? t("Accepted: {kind}", { kind }) : t("Refused: {reason}", { reason: a.reason })}`;
}

/** The first free "account-N" (account-1, account-2 ...). */
export function nextClientName(clients: RemoteClient[]): string {
  for (let n = 1; ; n++) if (!clients.some((c) => c.name === `account-${n}`)) return `account-${n}`;
}

/** "Added Nov 15, 2026", or the first setup's for the client from before clients. */
export function addedText(c: RemoteClient): string {
  if (!c.added) return t("Added with the first setup");
  return t("Added {date}", { date: new Date(c.added * 1000).toLocaleDateString() });
}

/** The relay connection's state code, in words. */
export function connectionText(code: string): string {
  switch (code) {
    case "off": return t("Off");
    case "connecting": return t("Connecting…");
    case "connected": return t("Connected");
    case "reconnecting":
    case "woke-reconnecting": return t("Reconnecting…");
    case "unauthorized": return t("The relay refused this PC's token.");
    case "not-set-up": return t("Set up the relay first.");
    case "no-relay": return t("Add the relay address first.");
    default: return t("Can't reach the relay ({code}).", { code });
  }
}

/** Commands refused at the gate without a Claude account (remote::ACCOUNT_COMMANDS). */
export const SESSION_COMMANDS = ["transcript-tail", "start-session", "send-to-session"];
/** Commands that do anything only with a Claude account (the others answer not-your-session). */
export const NEEDS_ACCOUNT = [...SESSION_COMMANDS, "session-status", "keepalive-restart"];

/** How often Remote tasks reads the connection's state again. */
export const REMOTE_POLL_MS = 2000;
/** How long "Saved" stays beside Save key / Save address. */
export const SAVED_MS = 3000;
let tasksTimer: ReturnType<typeof setInterval> | null = null;

/** Settings → Remote → Remote tasks (remote/). */
export function remoteTasks(toggle: (on: boolean, onChange: (v: boolean) => void) => HTMLElement, pollMs = REMOTE_POLL_MS): HTMLElement {
  const box = h("div", { class: "pill-group", "data-role": "remote-tasks" });
  let status: RemoteStatus = { on: false, consented: false, killed: false, clients: [], relay: "", connection: "off", pcPublicKey: "", pcFingerprint: "", pcEncKey: "", pcEncFingerprint: "", requireSealed: false, activity: [] };
  let consenting = false;
  let note = "";
  let renewing = false;
  /** A new cloud client's token, right after "Add cloud client" only; forgotten on "Done". */
  let cloudToken = "";
  /** A client was just removed: the runbook reminder. */
  let removed = false;
  /** An agent client was just added: the runbook reminder. */
  let agentAdded = false;
  /** What was typed but not saved yet: kept across redraws. */
  let keyDraft = "";
  let nameDraft: string | null = null;
  let relayDraft: string | null = null;
  /** Which Save just worked ("key" / "relay"), shown as "Saved" for a moment. */
  let saved = "";
  let savedTimer: ReturnType<typeof setTimeout> | null = null;
  /** Updated in place by the poll, so typing isn't interrupted. */
  let connectionLine: HTMLElement | null = null;
  let activityBox: HTMLElement | null = null;
  /** Runbook limits typed but not saved yet (they save on change). */
  let perClientDraft: string | null = null;
  let perPcDraft: string | null = null;
  /** Polls (status 2 s, pairing 2 s) wait while a field here has focus. */
  const later = deferWhileTyping(box);

  async function act(f: () => Promise<RemoteStatus | null>, what = "") {
    try {
      status = (await f()) ?? status;
      note = "";
      if (what) {
        saved = what;
        if (what === "key") {
          keyDraft = "";
          nameDraft = null;
        }
        if (what === "relay") relayDraft = null;
        if (savedTimer) clearTimeout(savedTimer);
        savedTimer = setTimeout(() => {
          saved = "";
          later(draw);
        }, SAVED_MS);
        (savedTimer as { unref?: () => void }).unref?.();
      }
    } catch (e) {
      note = String(e);
      saved = "";
    }
    draw();
  }

  function savedMark(what: string): HTMLElement[] {
    return saved === what ? [h("span", { class: "hint", "data-role": `remote-${what}-saved`, role: "status", style: "color:#3fb950", text: t("Saved") })] : [];
  }

  let pair: PairView | null = null;
  let pairName = "";
  let pairTimer: ReturnType<typeof setInterval> | null = null;
  let pairNote = "";

  function stopPairPoll() {
    if (pairTimer) clearInterval(pairTimer);
    pairTimer = null;
  }

  async function pollPair() {
    let next: PairView | null;
    try {
      next = await Bridge.remotePairStatus();
    } catch {
      return;
    }
    if (!next || next.state !== "waiting") stopPairPoll();
    // Still waiting with the same QR code: nothing to redraw.
    if (JSON.stringify(next) === JSON.stringify(pair)) return;
    pair = next;
    later(draw);
  }

  /**
   * An agent client's lease (owner 2026-11-21): locked, or unlocked until a
   * time with "Lock now". Only the owner's phone grants one.
   */
  function leaseRow(c: RemoteClient): HTMLElement {
    const until = c.leaseUntil ?? 0;
    if (!until) {
      return h("div", { class: "hint", "data-role": "remote-lease", text: t("Locked: only its ticked commands. It can ask for a lease, which only your phone grants.") });
    }
    const lock = h("button", { class: "danger", type: "button", "data-role": "remote-lease-lock", text: t("Lock now") });
    lock.addEventListener("click", () => void act(() => Bridge.remoteLeaseLock(c.name)));
    return h("div", { class: "row", "data-role": "remote-lease" }, h("span", { class: "hint", text: t("Unlocked until {time}", { time: clock(until) }) }), lock);
  }

  /**
   * A client's commands (owner 2026-11-21): a checkbox per command, saved at
   * once; refused ones answer not-allowed. Its Claude account: session
   * commands need one (no-account otherwise).
   */
  function clientAccess(c: RemoteClient): HTMLElement {
    const all = status.commands ?? [];
    const mine = new Set(c.commands ?? all);
    const row = h("div", { class: "row", "data-role": "remote-client-commands", style: "flex-wrap:wrap;gap:4px 12px" });
    const boxes: HTMLInputElement[] = [];
    for (const kind of all) {
      const box = h("input", { type: "checkbox", "data-role": "remote-client-command", "data-command": kind }) as HTMLInputElement;
      box.checked = mine.has(kind);
      boxes.push(box);
      box.addEventListener("change", () => {
        // From the boxes as they are now (review 2026-11-21): two quick
        // clicks can't send a list that brings back the first one's change.
        const next = boxes.filter((b) => b.checked).map((b) => b.getAttribute("data-command") ?? "");
        void act(() => Bridge.remoteClientCommands(c.name, next));
      });
      row.append(h("label", { style: "display:inline-flex;gap:4px;align-items:center" }, box, h("span", { text: kind })));
    }
    const pick = h("select", { "data-role": "remote-client-account" }) as HTMLSelectElement;
    pick.append(h("option", { value: "", text: t("None") }));
    for (const a of status.accounts ?? []) {
      const taken = a.client && a.client !== c.name ? ` (${t("now {client}'s", { client: a.client })})` : "";
      pick.append(h("option", { value: a.name, text: `${a.name}${taken}` }));
    }
    pick.value = c.account ?? "";
    // An agent client never acts as one of the owner's Claude accounts.
    if (c.agent) pick.disabled = true;
    pick.addEventListener("change", () => void act(() => Bridge.remoteClientAccount(c.name, pick.value)));
    const noAccount = !c.account && [...mine].some((k) => NEEDS_ACCOUNT.includes(k));
    return h("div", { "data-role": "remote-client-access" },
      h("label", { text: t("Commands it may send") }),
      row,
      h("div", { class: "row" }, h("label", { text: t("Claude account for its sessions") }), pick),
      ...(noAccount ? [h("div", { class: "hint", "data-role": "remote-client-no-account", text: t("No Claude account: its transcript-tail, start-session and send-to-session are refused, and session-status and keepalive-restart answer not-your-session.") })] : []),
    );
  }

  function approversBox(): HTMLElement {
    const box = h("div", { "data-role": "remote-approvers" }, h("h3", { text: t("Phone approvers") }));
    const list = status.approvers ?? [];
    if (list.length === 0) box.append(h("div", { class: "hint", text: t("No approver yet: a paused remote runbook step can't be approved.") }));
    for (const a of list) {
      const rm = h("button", { type: "button", "data-role": "remote-approver-remove", text: t("Remove") });
      rm.addEventListener("click", () => void act(() => Bridge.remoteApproverRemove(a.id)));
      box.append(h("div", { class: "row", "data-role": "remote-approver" },
        h("span", { text: `${a.name} · ${a.fingerprint}` }),
        ...(a.synced ? [h("span", { class: "hint", text: ` · ${t("synced passkey")}` })] : []),
        rm,
      ));
    }
    if (!pair || pair.state === "none") {
      const start = h("button", { type: "button", "data-role": "remote-pair-start", text: t("Pair a phone") });
      start.addEventListener("click", async () => {
        pairNote = "";
        try {
          pair = await Bridge.remotePairStart();
          stopPairPoll();
          pairTimer = setInterval(() => void pollPair(), 2000);
        } catch (e) {
          pairNote = String(e);
        }
        draw();
      });
      box.append(h("div", { class: "row" }, start));
    } else {
      const cancel = h("button", { type: "button", "data-role": "remote-pair-cancel", text: t("Cancel") });
      cancel.addEventListener("click", async () => {
        stopPairPoll();
        pair = await Bridge.remotePairCancel().catch(() => null);
        draw();
      });
      const card = h("div", { class: "card", "data-role": "remote-pair" });
      if (pair.state === "waiting") {
        const qr = h("div", { "data-role": "remote-pair-qr" });
        // Rust drew this SVG (qrcode crate) from the page's address only.
        qr.innerHTML = pair.qrSvg;
        card.append(
          h("div", { class: "hint", text: t("Scan this with your iPhone's camera, then follow the page. It needs iOS 18.4 or later and a Face ID passkey.") }),
          qr,
          h("div", { class: "hint", text: t("Waiting for your phone…") }),
          h("div", { class: "row" }, cancel),
        );
      } else if (pair.state === "ready") {
        const name = h("input", { type: "text", "data-role": "remote-pair-name", value: pairName, placeholder: t("Name for this approver") }) as HTMLInputElement;
        name.addEventListener("input", () => (pairName = name.value));
        const yes = h("button", { class: "primary", type: "button", "data-role": "remote-pair-confirm", text: t("Fingerprints match: add") });
        yes.addEventListener("click", async () => {
          await act(() => Bridge.remotePairConfirm(name.value));
          pair = null;
          pairName = "";
          draw();
        });
        card.append(
          h("div", { class: "name", "data-role": "remote-pair-fingerprint", text: t("Fingerprint on your phone: {fingerprint}. Add it only if it's the same on both screens.", { fingerprint: pair.fingerprint }) }),
          ...(pair.synced ? [h("div", { class: "hint", text: t("Synced passkey: it also works on your other Apple devices signed into the same Apple ID.") })] : []),
          name,
          h("div", { class: "row" }, yes, cancel),
        );
      } else {
        card.append(h("div", { class: "hint", style: "color:#f4505e", text: pair.state === "expired" ? t("Pairing expired: start again.") : t("Pairing failed: start again.") }), h("div", { class: "row" }, cancel));
      }
      box.append(card);
    }
    if (pairNote) box.append(h("div", { class: "hint", style: "color:#f4505e", text: pairNote }));
    return box;
  }

  function runbookLimits(): HTMLElement {
    const perClient = h("input", { type: "number", min: "1", max: "20", "data-role": "remote-runbooks-client", value: perClientDraft ?? String(status.runbooksPerHour ?? 5), style: "width:5em" }) as HTMLInputElement;
    const perPc = h("input", { type: "number", min: "1", max: "50", "data-role": "remote-runbooks-pc", value: perPcDraft ?? String(status.runbooksPerHourPc ?? 10), style: "width:5em" }) as HTMLInputElement;
    perClient.addEventListener("input", () => { perClientDraft = perClient.value; });
    perPc.addEventListener("input", () => { perPcDraft = perPc.value; });
    const save = () => void act(async () => {
      const next = await Bridge.remoteRunbookLimits(Number(perClient.value) || 5, Number(perPc.value) || 10);
      perClientDraft = null;
      perPcDraft = null;
      return next;
    });
    perClient.addEventListener("change", save);
    perPc.addEventListener("change", save);
    return h("div", { "data-role": "remote-runbook-limits" },
      h("div", { class: "row" }, h("label", { text: t("Remote runbooks an hour, per cloud client") }), perClient),
      h("div", { class: "row" }, h("label", { text: t("Remote runbooks an hour, this PC") }), perPc),
    );
  }

  function fillActivity(list: HTMLElement) {
    clear(list);
    list.append(h("h3", { text: t("Remote activity") }));
    if (status.activity.length === 0) list.append(h("div", { class: "hint", text: t("Nothing yet.") }));
    for (const a of status.activity.slice(0, 20)) list.append(h("div", { class: "hint", "data-role": "remote-activity-line", text: activityLine(a) }));
  }

  /** The poll: the connection line and activity change in place; anything else redraws (drafts kept). */
  async function poll() {
    let next: RemoteStatus | null = null;
    try {
      next = await Bridge.remoteStatus();
    } catch {
      return;
    }
    if (!next) return;
    const prev = status;
    status = next;
    const same = (k: keyof RemoteStatus) => JSON.stringify(prev[k]) === JSON.stringify(next![k]);
    const rest = (["on", "consented", "killed", "clients", "relay", "pcPublicKey", "pcFingerprint", "pcEncKey", "pcEncFingerprint", "requireSealed", "runbooksPerHour", "runbooksPerHourPc", "remoteRunbooks", "approvers"] as const).every(same);
    if (!rest) return later(draw);
    if (connectionLine && !same("connection")) connectionLine.textContent = t("Relay: {state}", { state: connectionText(status.connection) });
    if (activityBox && !same("activity")) fillActivity(activityBox);
  }

  function draw() {
    redrawKeeping(box, fill);
  }

  function fill() {
    clear(box);
    box.append(h("h3", { text: t("Remote tasks") }));
    box.append(h("div", { class: "hint", text: t("Lets tasks you sign reach this PC through your relay. Only tasks signed with the key below are accepted, each once and within 10 minutes; every answer is signed with this PC's own key.") }));
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
        h("div", { class: "hint", text: t("Glim will connect to your relay and accept tasks signed with your key: a session's status, the end of its transcript, starting or messaging a Claude Code session in an allowed project, and runbooks you approved. At most 5 new sessions and 30 tasks an hour. Stop them any time from the tray or here.") }),
        h("div", { class: "row" }, yes, no),
      ));
    }

    const clients = h("div", { "data-role": "remote-clients" }, h("label", { text: t("Cloud clients") }));
    if (status.clients.length === 0) clients.append(h("div", { class: "hint", text: t("No cloud clients yet.") }));
    for (const c of status.clients) {
      const remove = h("button", { class: "danger", type: "button", "data-role": "remote-client-remove", text: t("Remove") });
      remove.addEventListener("click", () => void act(async () => {
        const next = await Bridge.remoteClientRemove(c.name);
        cloudToken = "";
        removed = true;
        return next;
      }));
      clients.append(h("div", { "data-role": "remote-client" },
        h("div", { class: "pill-row" }, h("span", { class: "name", text: c.name }), ...(c.agent ? [h("span", { class: "hint", "data-role": "remote-client-agent", text: t("agent client · public key only") })] : []), remove),
        h("div", { class: "hint", "data-role": "remote-fingerprint", text: t("Fingerprint: {fingerprint}", { fingerprint: c.fingerprint }) }),
        h("div", { class: "hint", text: addedText(c) }),
        ...(c.agent ? [leaseRow(c)] : []),
        clientAccess(c),
      ));
    }
    box.append(clients);
    if (removed) box.append(h("div", { class: "hint", "data-role": "remote-client-removed", text: t("Removed. Run the relay-tokens runbook so the relay stops taking its token.") }));
    const name = h("input", { type: "text", "data-role": "remote-client-name", placeholder: t("Client name, like account-2"), spellcheck: "false", autocomplete: "off" }) as HTMLInputElement;
    name.value = nameDraft ?? nextClientName(status.clients);
    name.addEventListener("input", () => { nameDraft = name.value; });
    const input = h("input", { type: "text", "data-role": "remote-key", placeholder: t("Paste the 64 hex characters"), spellcheck: "false", autocomplete: "off" }) as HTMLInputElement;
    input.value = keyDraft;
    input.addEventListener("input", () => { keyDraft = input.value; });
    const save = h("button", { type: "button", "data-role": "remote-client-add", text: t("Add cloud client") });
    save.addEventListener("click", () => {
      const [n, key] = [name.value, input.value];
      void act(async () => {
        const r = await Bridge.remoteClientAdd(n, key);
        cloudToken = r.cloudToken;
        removed = false;
        agentAdded = false;
        return r.status;
      }, "key");
    });
    // Token-less agent clients (owner 2026-11-21): the same name and key,
    // no token; the relay checks a signature on each of its requests.
    const saveAgent = h("button", { type: "button", "data-role": "remote-agent-client-add", text: t("Add agent client (public key only)") });
    saveAgent.addEventListener("click", () => {
      const [n, key] = [name.value, input.value];
      void act(async () => {
        const next = await Bridge.remoteAgentClientAdd(n, key);
        cloudToken = "";
        removed = false;
        agentAdded = true;
        return next;
      }, "key");
    });
    box.append(h("label", { text: t("Signer's public key") }), h("div", { class: "row" }, name, input, save, saveAgent, ...savedMark("key")));
    if (agentAdded) box.append(h("div", { class: "hint", "data-role": "remote-agent-added", text: t("Agent client added: no token. Run the relay-tokens runbook so the relay knows its key, then set RELAY_CLIENT to its name in the agent's environment.") }));

    const relay = h("input", { type: "text", "data-role": "remote-relay", placeholder: "https://name.subdomain.workers.dev", spellcheck: "false", autocomplete: "off" }) as HTMLInputElement;
    relay.value = relayDraft ?? status.relay;
    relay.addEventListener("input", () => { relayDraft = relay.value; });
    const saveRelay = h("button", { type: "button", "data-role": "remote-relay-save", text: t("Save address") });
    saveRelay.addEventListener("click", () => {
      const address = relay.value;
      void act(() => Bridge.remoteRelay(address), "relay");
    });
    box.append(h("label", { text: t("Relay address") }), h("div", { class: "row" }, relay, saveRelay, ...savedMark("relay")));
    connectionLine = h("div", { class: "hint", "data-role": "remote-connection", "aria-live": "polite", text: t("Relay: {state}", { state: connectionText(status.connection) }) });
    box.append(connectionLine);

    if (status.pcPublicKey && status.pcFingerprint) {
      box.append(h("label", { text: t("This PC's result key") }), h("div", { class: "path", "data-role": "remote-pc-key", text: status.pcPublicKey }));
      box.append(h("div", { class: "hint", "data-role": "remote-pc-fingerprint", text: t("Fingerprint: {fingerprint}", { fingerprint: status.pcFingerprint }) }));
    }
    // Store-and-forward (owner 2026-11-17): tasks are sealed to this key;
    // each cloud environment needs it as GLIM_PC_ENC_KEY. Public, so it may be copied.
    if (status.pcEncKey && status.pcEncFingerprint) {
      const encKey = status.pcEncKey;
      const copyEnc = h("button", { type: "button", "data-role": "remote-pc-enc-copy", text: t("Copy") });
      copyEnc.addEventListener("click", () => void navigator.clipboard?.writeText(encKey));
      box.append(
        h("label", { text: t("This PC's encryption key (GLIM_PC_ENC_KEY)") }),
        h("div", { class: "row" }, h("div", { class: "path", "data-role": "remote-pc-enc-key", text: encKey }), copyEnc),
        h("div", { class: "hint", "data-role": "remote-pc-enc-fingerprint", text: t("Fingerprint: {fingerprint}", { fingerprint: status.pcEncFingerprint }) }),
      );
      // "Require sealed tasks" (owner 2026-11-19): off until every cloud
      // environment has GLIM_PC_ENC_KEY and the async cloud test passed.
      const sealedSw = toggle(Boolean(status.requireSealed), (v) => void act(() => Bridge.remoteRequireSealed(v)));
      box.append(
        h("div", { class: "row", "data-role": "remote-require-sealed" }, h("label", { text: t("Require sealed tasks") }), sealedSw),
        h("div", { class: "hint", text: t("Refuse any task that isn't sealed to this PC's encryption key. Turn on once every cloud environment has GLIM_PC_ENC_KEY.") }),
      );
    }
    const setup = h("button", { type: "button", "data-role": "remote-setup", text: status.pcPublicKey ? t("Make new tokens and key") : t("Set up the relay") });
    setup.addEventListener("click", () => {
      if (status.pcPublicKey) {
        renewing = true;
        draw();
      } else void runSetup();
    });
    box.append(h("div", { class: "row" }, setup));
    if (renewing) {
      const yes = h("button", { class: "danger", type: "button", "data-role": "remote-renew-yes", text: t("Make new ones") });
      const no = h("button", { type: "button", "data-role": "remote-renew-no", text: t("Not now") });
      yes.addEventListener("click", () => {
        renewing = false;
        void runSetup();
      });
      no.addEventListener("click", () => {
        renewing = false;
        draw();
      });
      box.append(h("div", { class: "card", "data-role": "remote-renew" },
        h("div", { class: "name", text: t("Make a new PC token and result key?") }),
        h("div", { class: "hint", text: t("The old ones stop working at once: put the new result key in each cloud environment and run the relay-tokens runbook again.") }),
        h("div", { class: "row" }, yes, no),
      ));
    }
    if (cloudToken) {
      const field = h("input", { type: "text", "data-role": "remote-cloud-token", spellcheck: "false", autocomplete: "off" }) as HTMLInputElement;
      field.value = cloudToken;
      field.readOnly = true;
      const copy = h("button", { type: "button", "data-role": "remote-cloud-copy", text: t("Copy") });
      copy.addEventListener("click", () => void navigator.clipboard?.writeText(field.value));
      const done = h("button", { class: "primary", type: "button", "data-role": "remote-cloud-done", text: t("Done") });
      done.addEventListener("click", () => {
        cloudToken = "";
        draw();
      });
      box.append(h("div", { class: "card", "data-role": "remote-cloud" },
        h("div", { class: "hint", text: t("This client's relay token, shown only this once: put it in that account's cloud environment as the relay's network secret, then run the relay-tokens runbook.") }),
        h("div", { class: "row" }, field, copy, done),
      ));
    }

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

    // Remote runbooks (owner 2026-11-20, docs/remote-runbooks.md): the
    // limits are set here, on the PC, only; the list is read only.
    box.append(runbookLimits());
    // Phone approvers (owner 2026-11-21): enrolled and removed only here.
    box.append(approversBox());
    const remoteBooks = status.remoteRunbooks ?? [];
    box.append(h("div", { class: "hint", "data-role": "remote-runbooks", text: remoteBooks.length ? t("Runbooks cloud Glim may start: {names}", { names: remoteBooks.join(", ") }) : t("No runbook may run remotely. A runbook's file sets it: [remote] level.") }));

    activityBox = h("div", { "data-role": "remote-activity" });
    fillActivity(activityBox);
    box.append(activityBox);
  }

  async function runSetup() {
    try {
      status = await Bridge.remoteSetup();
      note = "";
    } catch (e) {
      note = String(e);
    }
    draw();
  }

  draw();
  void (async () => {
    status = (await Bridge.remoteStatus()) ?? status;
    later(draw);
  })();
  // The connection changes on its own (connects, drops, the PC wakes): read
  // it again every 2 s while this page is open.
  if (tasksTimer) clearInterval(tasksTimer);
  tasksTimer = setInterval(() => void poll(), pollMs);
  // Never what keeps a process alive (Node runs this page's tests).
  (tasksTimer as { unref?: () => void }).unref?.();
  return box;
}
