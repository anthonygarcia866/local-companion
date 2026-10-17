// Settings → Claude Code: every Claude config folder Glim's hooks go into
// (~/.claude, CLAUDE_CONFIG_DIR, other ~/.claude* folders with a
// settings.json; src-tauri/src/hooks.rs config_dirs), each with its state.
// One Install / Update / Uninstall covers them all, after one diff.

import type { HookStatus } from "../core/bridge";
import { h } from "../views/dom";
import { statusDot } from "./parts";
import { t } from "../i18n/i18n";

/** A folder's state, in words. */
export function folderState(f: HookStatus["folders"][number]): string {
  if (f.installed && f.upToDate) return t("Hooks installed");
  if (f.installed) return t("Hooks out of date");
  return t("No hooks");
}

/** The install button's label: Install, Update (some folder behind), or Reinstall. */
export function installLabel(status: HookStatus): string {
  if (!status.installed) return t("Install hooks…");
  return status.allUpToDate ? t("Reinstall hooks…") : t("Update hooks…");
}

/** One row per folder: its settings.json, a dot, and its state. */
export function folderRows(status: HookStatus): HTMLElement[] {
  const rows = status.folders.map((f) =>
    h("div", { class: "row", "data-folder": f.settingsPath },
      h("span", { class: "path", text: f.settingsPath }),
      statusDot(f.installed && f.upToDate),
      h("span", { class: "state", text: folderState(f) }),
    ),
  );
  if (status.installed && !status.allUpToDate) {
    rows.push(h("div", {
      class: "notice warn",
      text: t("Not every Claude config folder has Glim's current hooks. Update hooks to bring them all in line."),
    }));
  }
  return rows;
}
