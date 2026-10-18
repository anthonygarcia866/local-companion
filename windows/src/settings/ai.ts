// Settings → AI (1c, docs/phase1-plan.md §13): the brain, the Anthropic API
// key (masked, with Remove), the model, the monthly cap and this month's
// spend. The key goes to Rust once, when it is saved, and straight into
// Windows Credential Manager; this page only ever gets back a hint
// ("sk-ant-…abcd"). The field is emptied as soon as the key is sent.

import { Bridge, type AiKeyStatus, type AiModel, type AiSpend } from "../core/bridge";
import type { Settings } from "../core/state";
import { h, clear } from "../views/dom";
import { statusDot } from "./parts";
import { t, tn } from "../i18n/i18n";

/** What the section needs from the Settings page. */
export interface AiHost {
  settings(): Settings;
  save(): Promise<void>;
}

/** "$0.10 / $0.50 per million tokens". */
export function priceText(m: AiModel): string {
  const usd = (n: number) => `$${n.toFixed(2)}`;
  return t("{input} / {output} per million tokens", { input: usd(m.inputUsdPerMtok), output: usd(m.outputUsdPerMtok) });
}

/** A typed cap: a positive number of dollars, else 0 (no cap). */
export function parseCap(raw: string): number {
  const n = Number(raw.trim().replace(/^\$/, ""));
  return Number.isFinite(n) && n > 0 ? Math.round(n * 100) / 100 : 0;
}

/**
 * This month's spend with enough digits to mean something (a request costs
 * a fraction of a cent): "$0.00" for nothing, "under $0.0001", four decimals
 * below 10 cents ("$0.0042", "$0.0123"), cents from there ("$0.47").
 */
export function spendCost(usd: number): string {
  if (!(usd > 0)) return "$0.00";
  if (usd < 0.0001) return t("under {cost}", { cost: "$0.0001" });
  return `$${usd < 0.1 ? usd.toFixed(4) : usd.toFixed(2)}`;
}

/** "This month: $0.0042 (3 requests)". */
export function spendText(s: AiSpend): string {
  return t("This month: {cost} ({count})", {
    cost: spendCost(s.usd),
    count: tn("{count} request", "{count} requests", s.requests),
  });
}

export async function aiSection(host: AiHost): Promise<HTMLElement> {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("AI (Claude API)") })),
    h("div", { class: "hint", text: t("Rewrites go to Anthropic's Claude API, only when you click a tone in the Rewrite card. Your API key stays in Windows Credential Manager.") }),
    body,
  );
  const models = (await Bridge.aiModels()) ?? [];
  let key: AiKeyStatus = (await Bridge.aiKeyStatus()) ?? { set: false, hint: "" };
  let spend = await Bridge.aiSpend();
  let error = "";

  const redraw = () => {
    clear(body);
    draw();
  };

  function brainRow(): HTMLElement {
    const select = h("select", {}) as HTMLSelectElement;
    select.append(
      h("option", { value: "claude", text: t("Claude API (default)") }),
      h("option", { value: "local", text: t("Local model (not built yet)") }),
    );
    select.value = host.settings().aiBrain === "local" ? "local" : "claude";
    select.addEventListener("change", () => {
      host.settings().aiBrain = select.value === "local" ? "local" : "claude";
      void host.save();
    });
    return h("div", { class: "row" }, h("label", { text: t("Brain") }), select);
  }

  function keyRow(): HTMLElement {
    if (key.set) {
      const remove = h("button", { class: "danger", text: t("Remove") });
      remove.addEventListener("click", async () => {
        try {
          key = (await Bridge.aiKeyRemove()) ?? key;
          error = "";
        } catch (e) {
          error = String(e);
        }
        redraw();
      });
      return h("div", { class: "row" }, h("label", { text: t("API key") }), h("span", { class: "path", text: key.hint }), statusDot(true), remove);
    }
    const input = h("input", { type: "password", placeholder: "sk-ant-…", autocomplete: "off", spellcheck: "false" }) as HTMLInputElement;
    const saveBtn = h("button", { class: "primary", text: t("Save key") });
    saveBtn.addEventListener("click", async () => {
      const typed = input.value;
      // Out of the page as soon as it is sent.
      input.value = "";
      try {
        key = (await Bridge.aiKeySave(typed)) ?? key;
        error = "";
      } catch (e) {
        error = String(e);
      }
      redraw();
    });
    return h("div", { class: "row" }, h("label", { text: t("API key") }), input, saveBtn);
  }

  function modelRow(): HTMLElement {
    const select = h("select", {}) as HTMLSelectElement;
    for (const m of models) {
      const label = `${m.name} · ${priceText(m)}${m.id === models[0]?.id ? ` · ${t("default")}` : ""}`;
      select.append(h("option", { value: m.id, text: label }));
    }
    select.value = host.settings().aiModel;
    if (select.selectedIndex < 0 && models[0]) select.value = models[0].id;
    select.addEventListener("change", () => {
      host.settings().aiModel = select.value;
      void host.save();
    });
    return h("div", { class: "row" }, h("label", { text: t("Model") }), select);
  }

  function capRow(): HTMLElement {
    const cap = host.settings().aiMonthlyCapUsd;
    const input = h("input", { type: "text", inputmode: "decimal", placeholder: t("No cap"), value: cap > 0 ? String(cap) : "" }) as HTMLInputElement;
    input.addEventListener("change", async () => {
      host.settings().aiMonthlyCapUsd = parseCap(input.value);
      input.value = host.settings().aiMonthlyCapUsd > 0 ? String(host.settings().aiMonthlyCapUsd) : "";
      await host.save();
      spend = await Bridge.aiSpend();
      redraw();
    });
    return h("div", { class: "row" }, h("label", { text: t("Monthly cap ($)") }), input);
  }

  function draw() {
    body.append(brainRow(), keyRow());
    if (error) body.append(h("div", { class: "hint", style: "color:#f4505e", text: error }));
    if (!key.set) body.append(h("div", { class: "hint", text: t("Get a key at console.anthropic.com → API Keys, then paste it here.") }));
    body.append(modelRow(), capRow());
    if (spend) {
      body.append(h("div", { class: "row" }, h("label", { text: t("Spend") }), h("span", { text: spendText(spend) })));
      if (spend.atCap) body.append(h("div", { class: "hint", style: "color:#f2b14c", text: t("The cap is reached: nothing more is sent this month.") }));
    }
    body.append(h("div", { class: "hint", text: t("Glim's own estimate from token counts. Your Anthropic Console bill is the real number.") }));
  }

  draw();
  return section;
}
