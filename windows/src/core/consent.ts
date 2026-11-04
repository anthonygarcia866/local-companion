// Ask first (owner, 2026-11-04; Rust: src-tauri/src/consent.rs). Live
// revisions, polished dictation and "Check with AI automatically" send text
// by themselves, so each starts off: the first time one would send, Rust
// asks the island to show a one-time card with exactly what it sends, and
// nothing is sent until "Turn on". Settings' switches show the same answer.

import type { Settings } from "./state";
import { N_ } from "../i18n/i18n";

export type CloudFeature = "live" | "polish" | "autoCheck";

export const CLOUD_FEATURES: readonly CloudFeature[] = ["live", "polish", "autoCheck"];

export function isCloudFeature(v: unknown): v is CloudFeature {
  return typeof v === "string" && (CLOUD_FEATURES as readonly string[]).includes(v);
}

export type ConsentAnswer = "unasked" | "on" | "off";

/** As Rust's consent::answer_in: a file from before the cards keeps what the user chose. */
export function consentAnswer(s: Settings, f: CloudFeature): ConsentAnswer {
  const stored = s.cloudConsent?.[f];
  if (stored === "on" || stored === "off") return stored;
  if (f === "autoCheck" && s.aiAutoCheck === true) return "on";
  if (f === "live" && s.liveRevisions === false) return "off";
  return "unasked";
}

export function consentOn(s: Settings, f: CloudFeature): boolean {
  return consentAnswer(s, f) === "on";
}

/** As Rust's consent::record: the answer, and the feature's own switch with it. */
export function setConsent(s: Settings, f: CloudFeature, on: boolean) {
  s.cloudConsent = { ...(s.cloudConsent ?? {}), [f]: on ? "on" : "off" };
  if (f === "live" && on) s.liveRevisions = true;
  if (f === "autoCheck") s.aiAutoCheck = on;
}

/** The card's lines: exactly what goes, as PRIVACY.md's table says (English; the card translates). */
export const WHAT_IS_SENT: Record<CloudFeature, { title: string; lines: string[] }> = {
  live: {
    title: N_("Turn on live revisions?"),
    lines: [
      N_("The paragraph you're editing, when you pause typing"),
      N_("The paragraph before it, for context"),
      N_("Up to 5 of your remembered rules that match it"),
    ],
  },
  polish: {
    title: N_("Turn on polished dictation?"),
    lines: [
      N_("What you said, in its typed form"),
      N_("The text just before the cursor, and the message you're replying to in that field"),
      N_("Your dictation word list and up to 5 remembered rules that match"),
    ],
  },
  autoCheck: {
    title: N_("Check with AI automatically?"),
    lines: [N_("The paragraph you're typing in, after a 2-second pause, once per version")],
  },
};

export const SENT_FOOTER =
  N_("Only to Claude (api.anthropic.com). Names, emails, phone numbers, addresses and unit numbers become placeholders on this PC first; signatures are left out; card, ID and bank numbers are never sent.");
