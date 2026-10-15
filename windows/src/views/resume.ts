// "Pick up where you left off" (Phase 1.5, the resume reader, PR #21): one
// row per repo with a HANDOFF.md, the most recently updated first, so
// yesterday's repo is on top. Each row says where it stopped and has
// "Open in Claude Code"; nothing opens without that click (owner,
// 2026-10-13). The island sends the repo's name only; Rust finds the folder
// among the repos it reads (resume::folder_for).

import { h, clear } from "./dom";
import { card, stack, type ViewHost } from "./views";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import type { AskHost } from "./ask";
import { t } from "../i18n/i18n";

/** One repo, as Rust sends it (resume::Card). */
export interface ResumeCard {
  repo: string;
  goal: string | null;
  inProgress: string | null;
  branch: string | null;
  lastCommit: string | null;
  hasQuestions: boolean;
  withheld: string[];
  updated: number;
}

/** At most this many repos fit the card. */
export const RESUME_ROWS = 3;

/** The card's note after a click. */
export function openNote(repo: string, opened: boolean): string {
  return opened
    ? t("Opening Claude Code in {repo}…", { repo })
    : t("Couldn't open Claude Code in {repo}. Is it installed?", { repo });
}

/** Closes the card. */
export function closeResume(host: AskHost | null) {
  if (!State.resume) return;
  State.resume = null;
  if (host && State.view === "resume" && State.mode === "expanded") {
    const next = State.lanternView();
    if (next) host.setView(next);
    else host.collapse();
  }
  State.notify();
}

export function buildResume(host: AskHost): ViewHost {
  const title = h("div", { class: "title one-line" });
  const rows = h("div", { class: "resume-rows" });
  const note = h("div", { class: "suggestion-more", role: "status" });
  const close = h("button", { class: "link-btn", type: "button", "data-role": "resume-close", onclick: () => closeResume(host) }) as HTMLButtonElement;
  const el = h("div", { class: "view" }, card(null, stack(98, 16, title, rows, h("div", { class: "rewrite-actions" }, note, close))));
  let busy = false;

  const open = async (repo: string) => {
    if (busy) return;
    busy = true;
    note.textContent = t("Opening Claude Code in {repo}…", { repo });
    const opened = (await Bridge.resumeOpen(repo)) === true;
    busy = false;
    note.textContent = openNote(repo, opened);
    if (opened) setTimeout(() => closeResume(host), 1500);
  };

  const sync = () => {
    const cards = State.resume;
    if (!cards) return;
    title.textContent = t("Pick up where you left off");
    close.textContent = t("Close");
    clear(rows);
    if (cards.length === 0) {
      rows.append(h("div", { class: "hint", text: t("No HANDOFF.md found in the folders under your home folder.") }));
      return;
    }
    for (const c of cards.slice(0, RESUME_ROWS)) {
      const where = c.inProgress ?? c.goal ?? "";
      const meta = [c.branch ? t("Branch {branch}", { branch: c.branch }) : "", c.hasQuestions ? t("Has open questions") : ""].filter(Boolean).join(" · ");
      rows.append(h("div", { class: "resume-row", "data-repo": c.repo },
        h("div", { class: "resume-head" },
          h("span", { class: "resume-repo", text: c.repo }),
          h("button", { class: "apply-all", type: "button", "data-role": "resume-open", text: t("Open in Claude Code"), onclick: () => void open(c.repo) }),
        ),
        h("div", { class: "resume-where", text: where }),
        ...(meta ? [h("div", { class: "resume-meta", text: meta })] : []),
        ...(c.withheld.length ? [h("div", { class: "resume-meta", text: t("Part of it is withheld: it holds a card, ID or bank number.") })] : []),
      ));
    }
  };
  return { el, sync };
}
