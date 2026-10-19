// Settings → Phone (phone companion step 1, docs/phone-plan.md §6): pushes to
// the owner's phone through Pushover, off by default. The keys go to Rust
// once, when saved, and into Windows Credential Manager; this page only ever
// gets back hints ("…abcd"). The fields are emptied as soon as they are sent.

import { Bridge, type PhoneKeysStatus } from "../core/bridge";
import type { Settings } from "../core/state";
import {
  APPROVAL_MAX_SEC, APPROVAL_MIN_SEC, IDLE_MAX_MIN, IDLE_MIN_MIN, approvalDelaySec, idleAfterMin, minutes, phoneSettings, type PhoneSettings,
} from "../core/phone";
import { h, clear } from "../views/dom";
import { statusDot } from "./parts";
import { t } from "../i18n/i18n";

/** What the section needs from the Settings page. */
export interface PhoneHost {
  settings(): Settings;
  save(): Promise<void>;
  toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement;
}

export async function phoneSection(host: PhoneHost): Promise<HTMLElement> {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    { id: "settings-phone" },
    h("h2", {}, h("span", { text: t("Phone") })),
    h("div", {
      class: "hint",
      text: t("Short status messages to your phone through Pushover, like \"Claude Code in local-companion needs approval\". Never commands, code, file names, prompts or anything you typed."),
    }),
    body,
  );
  let keys: PhoneKeysStatus = (await Bridge.phoneKeysStatus()) ?? { set: false, userHint: "", tokenHint: "" };
  let note = "";

  const p = (): PhoneSettings => phoneSettings(host.settings().phone);
  const change = (patch: Partial<PhoneSettings>) => {
    host.settings().phone = { ...p(), ...patch };
    void host.save();
  };
  const redraw = () => {
    clear(body);
    draw();
  };

  function switchRow(role: string, label: string, on: boolean, set: (v: boolean) => void, hint?: string, disabled = false): HTMLElement {
    const sw = host.toggle(on, set);
    if (disabled) sw.setAttribute("disabled", "");
    return h("div", { class: "row", "data-role": role }, h("label", { text: label }), sw, ...(hint ? [h("span", { class: "hint", text: hint })] : []));
  }

  function keysRow(): HTMLElement {
    if (keys.set) {
      const remove = h("button", { class: "danger", text: t("Remove") });
      remove.addEventListener("click", async () => {
        try {
          keys = (await Bridge.phoneKeysRemove()) ?? keys;
          note = "";
        } catch (e) {
          note = String(e);
        }
        redraw();
      });
      return h("div", { class: "row", "data-role": "phone-keys" },
        h("label", { text: t("Pushover keys") }),
        h("span", { class: "path", text: `${keys.userHint} · ${keys.tokenHint}` }),
        statusDot(true),
        remove,
      );
    }
    const user = h("input", { type: "password", placeholder: t("User key"), autocomplete: "off", spellcheck: "false" }) as HTMLInputElement;
    const token = h("input", { type: "password", placeholder: t("App token"), autocomplete: "off", spellcheck: "false" }) as HTMLInputElement;
    const saveBtn = h("button", { class: "primary", text: t("Save keys") });
    saveBtn.addEventListener("click", async () => {
      const [u, k] = [user.value, token.value];
      // Out of the page as soon as they are sent.
      user.value = "";
      token.value = "";
      try {
        keys = (await Bridge.phoneKeysSave(u, k)) ?? keys;
        note = "";
      } catch (e) {
        note = String(e);
      }
      redraw();
    });
    return h("div", { class: "row", "data-role": "phone-keys" }, h("label", { text: t("Pushover keys") }), user, token, saveBtn);
  }

  function delayRow(): HTMLElement {
    const input = h("input", { type: "number", min: String(APPROVAL_MIN_SEC), max: String(APPROVAL_MAX_SEC), value: String(approvalDelaySec(p())), style: "width:5em" }) as HTMLInputElement;
    input.addEventListener("change", () => {
      change({ approvalAfterSec: approvalDelaySec({ ...p(), approvalAfterSec: Number(input.value) }) });
      input.value = String(approvalDelaySec(p()));
    });
    return h("div", { class: "row", "data-role": "phone-delay" }, h("label", { text: t("Seconds before an approval pings") }), input);
  }

  function idleRow(): HTMLElement {
    const input = h("input", { type: "number", min: String(IDLE_MIN_MIN), max: String(IDLE_MAX_MIN), value: String(idleAfterMin(p())), style: "width:5em" }) as HTMLInputElement;
    if (!p().idleOnly) input.setAttribute("disabled", "");
    input.addEventListener("change", () => {
      change({ idleAfterMin: idleAfterMin({ ...p(), idleAfterMin: Number(input.value) }) });
      input.value = String(idleAfterMin(p()));
    });
    return h("div", { class: "row", "data-role": "phone-idle" },
      h("label", { text: t("Only when I'm away from this PC") }),
      host.toggle(p().idleOnly, (v) => {
        change({ idleOnly: v });
        redraw();
      }),
      input,
      h("span", { class: "hint", text: t("minutes without keyboard or mouse input") }),
    );
  }

  function quietRow(): HTMLElement {
    const time = (value: string, set: (v: string) => void) => {
      const input = h("input", { type: "time", value }) as HTMLInputElement;
      input.addEventListener("change", () => {
        if (minutes(input.value) !== null) set(input.value);
        else input.value = value;
      });
      return input;
    };
    return h("div", { class: "row", "data-role": "phone-quiet" },
      h("label", { text: t("Quiet hours") }),
      host.toggle(p().quietHours, (v) => change({ quietHours: v })),
      h("span", { class: "hint", text: t("from") }),
      time(p().quietFrom, (v) => change({ quietFrom: v })),
      h("span", { class: "hint", text: t("to") }),
      time(p().quietTo, (v) => change({ quietTo: v })),
    );
  }

  function testRow(): HTMLElement {
    const test = h("button", { text: t("Send test notification") }) as HTMLButtonElement;
    if (!p().enabled || !keys.set) test.setAttribute("disabled", "");
    test.addEventListener("click", async () => {
      try {
        await Bridge.phoneTest();
        note = t("Sent. Check your phone.");
      } catch (e) {
        note = String(e);
      }
      redraw();
    });
    return h("div", { class: "row", "data-role": "phone-test" }, test);
  }

  function draw() {
    const s = p();
    body.append(
      switchRow("phone-enabled", t("Send notifications to my phone"), s.enabled, (v) => {
        change({ enabled: v });
        redraw();
      }, t("Off by default. While it's off, Glim can't reach Pushover at all.")),
      keysRow(),
      h("div", { class: "hint", text: t("Make a user key and an app token at pushover.net (the iPhone app is a one-time $4.99).") }),
      h("div", { class: "hint", text: t("Notify me when:") }),
      switchRow("phone-approval", t("An approval waits"), s.approval, (v) => change({ approval: v })),
      delayRow(),
      switchRow("phone-finished", t("A session finishes"), s.finished, (v) => change({ finished: v })),
      switchRow("phone-limit", t("A usage or spend limit is hit"), s.limit, (v) => change({ limit: v })),
      switchRow("phone-meeting", t("My next meeting is in 10 minutes"), false, () => {}, t("Needs the calendar glance, not built in yet."), true),
      switchRow("phone-project", t("Include the project name"), s.includeProject, (v) => change({ includeProject: v }), t("Off: \"Claude Code needs approval\".")),
      idleRow(),
      quietRow(),
      testRow(),
    );
    if (note) body.append(h("div", { class: "notice", "data-role": "phone-note", text: note }));
  }

  draw();
  return section;
}
