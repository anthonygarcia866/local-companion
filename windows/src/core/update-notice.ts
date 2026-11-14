// The last update's result (src-tauri/src/update_result.rs; runbooks round 2,
// owner 2026-11-13). An updater outside the repo rebuilds and reinstalls Glim
// and leaves update-result.json behind; the new Glim shows it once on the
// island ("Updated to <commit>: self-test 6/6", or what failed) and Settings →
// Runbooks shows the last one. The step details are the script's own short
// English words.

import { t } from "../i18n/i18n";

export interface UpdateStep {
  name: string;
  ok: boolean;
  ms: number;
  detail: string;
}

export interface UpdateResult {
  runId: string;
  commit: string;
  ok: boolean;
  selftest: string;
  failedStep: string;
  error: string;
  finishedAt: string;
  totalMs: number;
  steps: UpdateStep[];
}

/** The island's closes by itself after this long. */
export const UPDATE_NOTICE_MS = 12_000;

/** One line: the commit and the self-test, or the step that failed and why. */
export function updateLine(u: UpdateResult): string {
  const commit = u.commit || "?";
  if (u.ok) return t("Updated to {commit}: self-test {selftest}", { commit, selftest: u.selftest || "?" });
  return t("Update to {commit} failed at {step}: {error}", { commit, step: u.failedStep || "?", error: u.error || "?" });
}

/** "pack 8 min 20 s · install 12 s …": each step's time, for Settings. */
export function updateTimings(u: UpdateResult): string {
  const secs = (ms: number) => {
    const s = Math.round(ms / 1000);
    return s >= 60 ? `${Math.floor(s / 60)} min ${s % 60} s` : `${s} s`;
  };
  return [...u.steps.map((s) => `${s.name} ${secs(s.ms)}${s.ok ? "" : " ✗"}`), `${t("total")} ${secs(u.totalMs)}`].join(" · ");
}

/** Shown on the island only on the pill with no approval waiting; Settings has it anyway. */
export function updateNoticeShows(o: { presence: string; paused: boolean; approval: boolean }): boolean {
  return o.presence === "pill" && !o.approval;
}
