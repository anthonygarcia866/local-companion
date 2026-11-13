// Settings → Runbooks (local agency v1, part 2; owner 2026-11-12): the
// runbook files in %APPDATA%\Glim\runbooks, each with its state and Run.
//
// Run on a runbook that was never approved, or whose file changed since,
// shows the approval card first: every step (secret NAMES only, Rust wrote
// the lines), what it may touch, "Approve runbook" / "Cancel". A runbook
// with ask = true steps shows those steps and asks again on every run. Then
// the run's progress, step by step, and the last 20 runs.
//
// The step lines and failure details come from Rust in English (the same
// words the command line prints); only this page's own words are
// translated.

import { Bridge, onEvent, type RunbookView, type RunbooksView, type RunbookOutcome } from "../core/bridge";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

/** What the page shows about one runbook right now. */
interface Live {
  /** "approve" | "ask" while a card is up. */
  card: "" | "approve" | "ask";
  lines: string[];
  running: boolean;
  /** Started by voice (the audit log says so). */
  voice: boolean;
}

export function stateText(b: RunbookView): string {
  switch (b.state) {
    case "approved": return t("Approved");
    case "new": return t("Needs approval");
    case "changed": return t("Changed since approved: approve again");
    default: return t("Can't be used: {error}", { error: b.error });
  }
}

export function askSteps(b: RunbookView): number[] {
  return b.steps.map((s, i) => (s.ask ? i + 1 : 0)).filter((n) => n > 0);
}

export function progressLine(o: RunbookOutcome): string {
  return o.ok
    ? t("Step {n} of {total}: OK ({detail})", { n: String(o.step), total: String(o.total), detail: o.detail })
    : t("Step {n} of {total} failed: {detail}", { n: String(o.step), total: String(o.total), detail: o.detail });
}

let listening = false;
let onProgress: ((name: string, o: RunbookOutcome) => void) | null = null;
let onDone: ((name: string, ok: boolean, done: number, total: number, refused: string) => void) | null = null;
let onRequest: ((name: string) => void) | null = null;

export function runbooksSection(): HTMLElement {
  const listBox = h("div", { class: "pill-group", "data-role": "runbooks-list" });
  const runsBox = h("div", { class: "pill-group", "data-role": "runbooks-runs" });
  const folderHint = h("div", { class: "hint" });
  const section = h(
    "section",
    { id: "settings-runbooks" },
    h("h2", {}, h("span", { text: t("Runbooks") })),
    folderHint,
    listBox,
    runsBox,
  );

  let view: RunbooksView = { folder: "", books: [], runs: [] };
  const live = new Map<string, Live>();
  const of = (name: string): Live => {
    let l = live.get(name);
    if (!l) {
      l = { card: "", lines: [], running: false, voice: false };
      live.set(name, l);
    }
    return l;
  };

  async function start(b: RunbookView, confirmed: number[]) {
    const l = of(b.name);
    l.card = "";
    l.lines = [];
    l.running = true;
    draw();
    try {
      await Bridge.runbookRun(b.name, b.hash, confirmed, l.voice);
    } catch (e) {
      l.running = false;
      l.lines = [String(e)];
      draw();
    }
  }

  /** Run, or the voice asking: approval first, then the ask steps, then run. */
  function begin(b: RunbookView, voice = false) {
    const l = of(b.name);
    l.voice = voice;
    if (b.state === "invalid") {
      l.lines = [stateText(b)];
    } else if (b.state !== "approved") {
      l.card = "approve";
    } else if (askSteps(b).length > 0) {
      l.card = "ask";
    } else {
      void start(b, []);
      return;
    }
    draw();
  }

  function stepList(b: RunbookView, only?: number[]): HTMLElement {
    const ol = h("ol", { "data-role": "runbook-steps" });
    b.steps.forEach((s, i) => {
      if (only && !only.includes(i + 1)) return;
      const li = h("li", { text: s.text });
      if (s.ask) li.append(h("span", { class: "hint", text: ` · ${t("asks every run")}` }));
      ol.append(li);
    });
    return ol;
  }

  function card(b: RunbookView, l: Live): HTMLElement {
    if (l.card === "approve") {
      const yes = h("button", { class: "primary", type: "button", "data-role": "runbook-approve", text: t("Approve runbook") });
      const no = h("button", { type: "button", "data-role": "runbook-cancel", text: t("Cancel") });
      yes.addEventListener("click", async () => {
        try {
          await Bridge.runbookApprove(b.name, b.hash);
        } catch (e) {
          l.card = "";
          l.lines = [String(e)];
          await load();
          return;
        }
        l.card = "";
        await load();
        const fresh = view.books.find((x) => x.name === b.name);
        if (fresh && fresh.state === "approved") begin(fresh, l.voice);
      });
      no.addEventListener("click", () => {
        l.card = "";
        draw();
      });
      const allowed: HTMLElement[] = [];
      const row = (label: string, items: string[]) => {
        if (items.length) allowed.push(h("div", { class: "hint", text: `${label} ${items.join(" · ")}` }));
      };
      row(t("May write in or run in:"), b.folders);
      row(t("May start:"), b.apps);
      row(t("May run exactly:"), b.commands);
      return h("div", { class: "card", "data-role": "runbook-approval" },
        h("div", { class: "name", text: b.state === "changed" ? t("{name} changed since you approved it. It will now:", { name: b.name }) : t("Approve {name}? It will:", { name: b.name }) }),
        stepList(b),
        ...allowed,
        h("div", { class: "hint", text: t("Secrets are shown by name only. Any change to the file means approving it again.") }),
        h("div", { class: "row" }, yes, no),
      );
    }
    const asks = askSteps(b);
    const go = h("button", { class: "primary", type: "button", "data-role": "runbook-ask-run", text: t("Run now") });
    const no = h("button", { type: "button", "data-role": "runbook-cancel", text: t("Cancel") });
    go.addEventListener("click", () => void start(b, asks));
    no.addEventListener("click", () => {
      l.card = "";
      draw();
    });
    return h("div", { class: "card", "data-role": "runbook-ask" },
      h("div", { class: "name", text: t("These steps ask every run:") }),
      stepList(b, asks),
      h("div", { class: "row" }, go, no),
    );
  }

  function draw() {
    folderHint.textContent = t("Runbook files (.toml) are in {folder}. Steps run in order and stop at the first failure. Run one here, say \"run <name>\" with Ask Glim, or use glim.exe --run <name>.", { folder: view.folder || "%APPDATA%\\Glim\\runbooks" });
    clear(listBox);
    if (view.books.length === 0) listBox.append(h("div", { class: "hint", text: t("No runbooks yet. Put a .toml file in that folder, then reopen Settings.") }));
    for (const b of view.books) {
      const l = of(b.name);
      const run = h("button", { type: "button", "data-role": "runbook-run", text: t("Run") });
      if (l.running || b.state === "invalid") run.setAttribute("disabled", "");
      run.addEventListener("click", () => begin(b));
      const box = h("div", { "data-role": "runbook", "data-name": b.name },
        h("div", { class: "pill-row" },
          h("span", { class: "name", text: b.name }),
          h("span", { class: "hint", "data-role": "runbook-state", text: stateText(b) }),
          run,
        ),
      );
      if (b.description) box.append(h("div", { class: "hint", text: b.description }));
      if (l.card) box.append(card(b, l));
      for (const line of l.lines) box.append(h("div", { class: "hint", "data-role": "runbook-line", text: line }));
      listBox.append(box);
    }
    clear(runsBox);
    runsBox.append(h("h3", { text: t("Last runs") }));
    if (view.runs.length === 0) runsBox.append(h("div", { class: "hint", text: t("No runs yet.") }));
    for (const r of view.runs) {
      const outcome = r.refused ? t("Refused") : r.ok ? t("OK") : t("Failed");
      const steps = r.total > 0 ? ` ${r.done}/${r.total}` : "";
      runsBox.append(h("div", { class: "pill-row", "data-role": "runbook-history" },
        h("span", { class: "hint", text: r.time }),
        h("span", { class: "name", text: r.name }),
        h("span", { class: "hint", text: `${outcome}${steps} · ${r.trigger}` }),
      ));
    }
  }

  async function load() {
    view = (await Bridge.runbooksList()) ?? { folder: "", books: [], runs: [] };
    draw();
  }

  async function requested(name: string) {
    await load();
    const b = view.books.find((x) => x.name === name);
    if (b) begin(b, true);
  }

  onProgress = (name, o) => {
    of(name).lines.push(progressLine(o));
    draw();
  };
  onDone = (name, ok, done, total, refused) => {
    const l = of(name);
    l.running = false;
    l.lines.push(refused ? t("Didn't run: {why}", { why: refused }) : ok ? t("Done: all {total} steps OK.", { total: String(total) }) : t("Stopped after {done} of {total} steps.", { done: String(done), total: String(total) }));
    void load();
  };
  onRequest = (name) => {
    // Taken here so the next Settings load doesn't run it again.
    void Bridge.runbookTakeRequest().then(() => requested(name));
  };
  if (!listening) {
    listening = true;
    void onEvent<RunbookOutcome & { name: string }>("runbook-progress", (p) => onProgress?.(p.name, p));
    void onEvent<{ name: string; ok: boolean; done: number; total: number; refused: string }>("runbook-done", (d) => onDone?.(d.name, d.ok, d.done, d.total, d.refused));
    void onEvent<string>("runbook-request", (name) => onRequest?.(name));
  }

  draw();
  void load().then(async () => {
    const asked = await Bridge.runbookTakeRequest();
    if (asked) await requested(asked);
  });
  return section;
}
