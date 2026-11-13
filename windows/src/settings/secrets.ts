// Settings → Secrets (local agency v1, part 1; owner 2026-11-12): "Add a
// secret" (a name and a masked value), and the saved ones by name with the
// date each was saved, Replace and Delete (Delete asks first).
//
// The value goes to Rust once, on Save, into Windows Credential Manager
// (secrets.rs), and the field is emptied before the answer comes back. This
// page never gets a value back: only names and dates. A runbook's env_secret
// step is the only thing that reads one (runbooks, part 2).

import { Bridge, type SecretInfo } from "../core/bridge";
import { h, clear } from "../views/dom";
import { t } from "../i18n/i18n";

/** "saved 2026-11-12 14:05", in local time. */
export function savedText(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return t("saved {date}", { date: "?" });
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  const date = `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  return t("saved {date}", { date });
}

export function secretsSection(): HTMLElement {
  const listBox = h("div", { class: "pill-group", "data-role": "secrets-list" });
  const form = h("div", { class: "pill-group", "data-role": "secrets-add" });
  const section = h(
    "section",
    { id: "settings-secrets" },
    h("h2", {}, h("span", { text: t("Secrets") })),
    h("div", {
      class: "hint",
      text: t("Kept in Windows Credential Manager on this PC only. Glim never shows a secret again, never logs it and never sends it to the AI or anywhere else: a runbook you approved writes it into the file you chose."),
    }),
    listBox,
    form,
  );

  let secrets: SecretInfo[] = [];
  let replacing: string | null = null;
  let confirming: string | null = null;
  let note = "";

  function drawList() {
    clear(listBox);
    listBox.append(h("h3", { text: t("Saved secrets") }));
    if (secrets.length === 0) listBox.append(h("div", { class: "hint", text: t("No secrets yet.") }));
    for (const s of secrets) {
      if (confirming === s.name) {
        const yes = h("button", { class: "danger", type: "button", "data-role": "secrets-delete-yes", text: t("Delete") });
        const no = h("button", { type: "button", "data-role": "secrets-delete-no", text: t("Cancel") });
        yes.addEventListener("click", async () => {
          confirming = null;
          try {
            await Bridge.secretsDelete(s.name);
            note = "";
          } catch (e) {
            note = String(e);
          }
          await load();
        });
        no.addEventListener("click", () => {
          confirming = null;
          drawList();
        });
        listBox.append(h("div", { class: "pill-row", "data-role": "secrets-confirm" },
          h("span", { class: "name", text: t("Delete {name}? Runbooks that use it will stop at that step.", { name: s.name }) }),
          yes,
          no,
        ));
        continue;
      }
      const replace = h("button", { type: "button", "data-role": "secrets-replace", text: t("Replace") });
      const del = h("button", { class: "danger", type: "button", "data-role": "secrets-delete", text: t("Delete") });
      replace.addEventListener("click", () => {
        replacing = s.name;
        drawForm();
      });
      del.addEventListener("click", () => {
        confirming = s.name;
        drawList();
      });
      listBox.append(h("div", { class: "pill-row", "data-role": "secrets-row" },
        h("span", { class: "name", text: s.name }),
        h("span", { class: "hint", text: savedText(s.savedMs) }),
        replace,
        del,
      ));
    }
  }

  function drawForm() {
    clear(form);
    form.append(h("h3", { text: replacing ? t("Replace {name}", { name: replacing }) : t("Add a secret") }));
    const name = h("input", {
      type: "text", "data-role": "secrets-name", placeholder: t("Name, like EXAMPLE_TOKEN"), autocomplete: "off", spellcheck: "false", maxlength: "64",
    }) as HTMLInputElement;
    if (replacing) {
      name.value = replacing;
      name.setAttribute("disabled", "");
    }
    const value = h("input", {
      type: "password", "data-role": "secrets-value", placeholder: t("Secret"), autocomplete: "off", spellcheck: "false",
    }) as HTMLInputElement;
    const save = h("button", { class: "primary", type: "button", "data-role": "secrets-save", text: t("Save") });
    save.addEventListener("click", async () => {
      const [n, v] = [replacing ?? name.value, value.value];
      // Out of the page as soon as it is sent.
      value.value = "";
      try {
        await Bridge.secretsSave(n, v);
        note = "";
        replacing = null;
      } catch (e) {
        note = String(e);
      }
      await load();
    });
    const buttons: HTMLElement[] = [save];
    if (replacing) {
      const cancel = h("button", { type: "button", "data-role": "secrets-replace-cancel", text: t("Cancel") });
      cancel.addEventListener("click", () => {
        replacing = null;
        drawForm();
      });
      buttons.push(cancel);
    }
    form.append(h("div", { class: "row" }, name, value, ...buttons));
    if (note) form.append(h("div", { class: "hint", "data-role": "secrets-note", style: "color:#f4505e", text: note }));
  }

  async function load() {
    secrets = (await Bridge.secretsList()) ?? [];
    drawList();
    drawForm();
  }

  drawList();
  drawForm();
  void load();
  return section;
}
