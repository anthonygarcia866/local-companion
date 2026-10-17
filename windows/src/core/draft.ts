// The "Draft email" card, the island's half (voice 1.5d, decided 2026-10-16;
// Rust: src-tauri/src/voice/draft.rs, draft_worker.rs, outlook.rs). Rust owns
// the card and Outlook; it sends the card on "draft-email" whenever it
// changes. The island sends back only the card's id, indexes and a search's
// text. Glim never sends the email: Create draft saves it in Outlook's Drafts
// and opens it there, and Anthony sends it himself.

import type { PresenceKind } from "../island/presence";

export interface DraftContact {
  name: string;
  address: string;
}

export type DraftFound = { kind: "one"; contact: DraftContact } | { kind: "pick"; choices: DraftContact[] } | { kind: "none" };

export interface DraftRecipient {
  said: string;
  line: "to" | "cc" | "bcc";
  found: DraftFound;
}

/** Rust's voice::draft::Pending. */
export interface DraftCard {
  id: number;
  recipients: DraftRecipient[];
  subject: string;
  body: string;
  /** The recipient "Change recipient" is searching for, and what it found. */
  changing: number | null;
  results: DraftContact[];
  /** A change to the draft Glim just saved (fix b, 2026-10-17): no
   *  recipients, the new subject and text, and Update draft. */
  edit?: boolean;
}

/** One event from Rust (draft_worker::DraftEvent). */
export interface DraftEvent {
  phase: "card" | "created" | "updated" | "closed";
  card: DraftCard | null;
  /** Why the last step didn't work ("no-outlook", "no-account", "no-sender", "outlook-failed"), or "". */
  code: string;
  dryRun: boolean;
  ready: boolean;
}

/** The card as the island keeps it; null when there is none. */
export interface DraftState {
  /** "open": the card; "created": saved and opened in Outlook (a short note);
   *  "failed": Outlook couldn't be reached at all. */
  phase: "open" | "created" | "failed";
  card: DraftCard | null;
  code: string;
  dryRun: boolean;
  ready: boolean;
  /** Create draft was clicked; waiting for Outlook. */
  busy: boolean;
  /** "Change recipient" with several names: which one, picked next. */
  choosing: boolean;
  /** "created" for a change put into the draft Glim saved before. */
  updated?: boolean;
}

/** "me", "Me.", "myself", "my email": the sender (Rust's draft::means_me). */
export function meansMe(said: string): boolean {
  const key = said.trim().replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "").split(/\s+/).join(" ").toLowerCase();
  return ["me", "myself", "my self", "my email", "my address", "my own address"].includes(key);
}

/** The state after an event. "closed" with no reason is just gone. */
export function nextDraft(cur: DraftState | null, e: DraftEvent): DraftState | null {
  switch (e.phase) {
    case "card":
      if (!e.card) return cur;
      return { phase: "open", card: e.card, code: e.code ?? "", dryRun: e.dryRun === true, ready: e.ready === true, busy: false, choosing: cur?.choosing === true && cur.card?.id === e.card.id && e.card.changing === null };
    case "created":
    case "updated":
      return { phase: "created", card: null, code: "", dryRun: e.dryRun === true, ready: false, busy: false, choosing: false, updated: e.phase === "updated" };
    case "closed":
      return e.code ? { phase: "failed", card: null, code: e.code, dryRun: e.dryRun === true, ready: false, busy: false, choosing: false } : null;
  }
  return cur;
}

/**
 * Which recipient "Change recipient" is for: the one that still needs an
 * address, else the only one; null when there are several and the user picks
 * which (their names turn into buttons).
 */
export function changeTarget(card: DraftCard): number | null {
  const open = card.recipients.findIndex((r) => r.found.kind !== "one");
  if (open >= 0) return open;
  return card.recipients.length === 1 ? 0 : null;
}

/** A contact as a chip: "Maria Lopez · maria.lopez@example.com". */
export function chipText(c: DraftContact): string {
  return c.name && c.name.toLowerCase() !== c.address.toLowerCase() ? `${c.name} · ${c.address}` : c.address;
}

/** The card stays until Create draft or Cancel (decided 2026-10-16); the
 *  "created" note and a failure close by themselves after this long. */
export const DRAFT_NOTE_MS = 8_000;

/**
 * Where the card goes, like the voice card: "drop" over fullscreen (the
 * indicator), "queue" behind an approval, else "show" (Hidden too: it came
 * from the push-to-talk key).
 */
export function draftPlacement(o: { presence: PresenceKind; approval: boolean }): "drop" | "queue" | "show" {
  if (o.presence === "indicator") return "drop";
  return o.approval ? "queue" : "show";
}
