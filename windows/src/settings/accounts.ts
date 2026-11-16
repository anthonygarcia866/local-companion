// Settings → Claude Code & Agents → Accounts (owner 2026-11-15;
// src-tauri/src/accounts.rs): each Claude account on this PC, its Claude
// Code config folder, whether it's signed in (only whether its login file
// exists; Glim never reads it) and the remote courier's cloud client acting
// for it. Prefilled with Claude and Claude2.

import { Bridge, type ClaudeAccountView } from "../core/bridge";
import { clear, h } from "../views/dom";
import { statusDot } from "./parts";
import { t } from "../i18n/i18n";

export function signedInText(a: ClaudeAccountView): string {
  return a.signedIn ? t("Signed in") : t("Needs login");
}

export function accountsSection(): HTMLElement {
  const body = h("div", { "data-role": "accounts" });
  let list: ClaudeAccountView[] = [];
  let note = "";
  let nameDraft = "";
  let folderDraft = "";

  async function act(f: () => Promise<ClaudeAccountView[] | null>) {
    try {
      list = (await f()) ?? list;
      note = "";
    } catch (e) {
      note = String(e).replace(/^Error:\s*/, "");
    }
    draw();
  }

  function draw() {
    clear(body);
    for (const a of list) {
      const remove = h("button", { class: "danger", type: "button", "data-role": "account-remove", text: t("Remove") });
      remove.addEventListener("click", () => void act(() => Bridge.accountsRemove(a.name)));
      body.append(h("div", { "data-role": "account" },
        h("div", { class: "pill-row" }, h("span", { class: "name", text: a.name }), statusDot(a.signedIn), h("span", { class: "state", "data-role": "account-state", text: signedInText(a) }), remove),
        h("div", { class: "path", text: a.configDir }),
        h("div", { class: "hint", text: t("Remote tasks from {client} act as this account.", { client: a.client }) }),
      ));
    }
    const name = h("input", { type: "text", "data-role": "account-name", placeholder: t("Account name"), spellcheck: "false" }) as HTMLInputElement;
    name.value = nameDraft;
    name.addEventListener("input", () => { nameDraft = name.value; });
    const folder = h("input", { type: "text", "data-role": "account-folder", placeholder: t("Config folder, like C:\\Users\\you\\.claude3"), spellcheck: "false" }) as HTMLInputElement;
    folder.value = folderDraft;
    folder.addEventListener("input", () => { folderDraft = folder.value; });
    const add = h("button", { type: "button", "data-role": "account-add", text: t("Add") });
    add.addEventListener("click", () => {
      const [n, f] = [name.value, folder.value];
      void act(async () => {
        const next = await Bridge.accountsAdd(n, f);
        nameDraft = "";
        folderDraft = "";
        return next;
      });
    });
    body.append(h("div", { class: "row" }, name, folder, add));
    if (note) body.append(h("div", { class: "hint", "data-role": "accounts-note", style: "color:#f4505e", text: note }));
  }

  draw();
  void (async () => {
    list = (await Bridge.accountsStatus()) ?? list;
    draw();
  })();
  return h("section", { id: "settings-accounts" },
    h("h2", {}, h("span", { text: t("Accounts") })),
    h("div", { class: "hint", text: t("Each Claude account on this PC and its Claude Code config folder. Keep Claude Code reachable runs one remote control per account, and remote tasks reach only their own account's sessions.") }),
    body,
  );
}
