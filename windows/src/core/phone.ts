// Phone notifications (phone companion step 1, docs/phone-plan.md §5): when
// the island asks Rust to push to the owner's phone. Off by default. Only a
// kind, a pill id and the pill's project name ever go to Rust; phone.rs
// builds the text from fixed templates, checks Settings again and keeps the
// rate limit (one every 10 s, 30 an hour). Quiet hours are checked here,
// where the local time is.

import type { LimitKind } from "./limit";

/** Settings → Phone (settings.rs PhoneSettings). */
export interface PhoneSettings {
  enabled: boolean;
  approval: boolean;
  approvalAfterSec: number;
  finished: boolean;
  limit: boolean;
  meeting: boolean;
  includeProject: boolean;
  quietHours: boolean;
  quietFrom: string;
  quietTo: string;
}

export const DEFAULT_PHONE: PhoneSettings = {
  enabled: false,
  approval: true,
  approvalAfterSec: 60,
  finished: true,
  limit: true,
  meeting: false,
  includeProject: true,
  quietHours: false,
  quietFrom: "22:00",
  quietTo: "07:00",
};

/** The approval delay's range: the relay hands a card to the terminal after ~108 s. */
export const APPROVAL_MIN_SEC = 15;
export const APPROVAL_MAX_SEC = 100;

export type PingKind = "approval" | "finished" | "usageLimit" | "spendLimit";

/** One push for Rust (Bridge.phoneNotify). */
export interface Ping {
  kind: PingKind;
  pillId: string;
  project: string;
}

/** Saved settings over the defaults (an older settings.json has none). */
export function phoneSettings(saved: Partial<PhoneSettings> | undefined): PhoneSettings {
  return { ...DEFAULT_PHONE, ...(saved ?? {}) };
}

/** The approval delay in seconds, kept in range. */
export function approvalDelaySec(s: PhoneSettings): number {
  const n = Math.round(Number(s.approvalAfterSec));
  return Number.isFinite(n) ? Math.min(APPROVAL_MAX_SEC, Math.max(APPROVAL_MIN_SEC, n)) : DEFAULT_PHONE.approvalAfterSec;
}

/** "HH:MM" as minutes after midnight, or null. */
export function minutes(hhmm: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm.trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
}

/** Inside quiet hours at `now` (local time). From = to, or a bad time: never. */
export function inQuietHours(s: PhoneSettings, now: Date): boolean {
  if (!s.quietHours) return false;
  const from = minutes(s.quietFrom);
  const to = minutes(s.quietTo);
  if (from === null || to === null || from === to) return false;
  const at = now.getHours() * 60 + now.getMinutes();
  return from < to ? at >= from && at < to : at >= from || at < to;
}

/** The pill's project name (its folder's display name); phone.rs cleans and shortens it. */
export function projectOf(tasks: { id: string; name: string }[], pillId: string): string {
  return tasks.find((t) => t.id === pillId)?.name ?? "";
}

/** A request waiting for an answer, as the island knows it. */
export interface Waiting {
  requestId: string;
  pillId: string;
  project: string;
  waitedMs: number;
}

export class PhoneWatcher {
  /** Requests already pinged (kept only while they wait). */
  private pinged = new Set<string>();
  /** Limits already pinged: pill and reset. */
  private limits = new Set<string>();

  private allowed(s: PhoneSettings, now: Date): boolean {
    return s.enabled && !inQuietHours(s, now);
  }

  /** Every second (island.ts tickWait): requests that have waited long enough, once each. */
  approvals(waiting: Waiting[], s: PhoneSettings, now: Date): Ping[] {
    const ids = new Set(waiting.map((w) => w.requestId));
    for (const id of this.pinged) if (!ids.has(id)) this.pinged.delete(id);
    if (!s.approval) return [];
    const due = waiting.filter((w) => !this.pinged.has(w.requestId) && w.waitedMs >= approvalDelaySec(s) * 1000);
    // Counted as pinged even in quiet hours: nothing is sent late.
    for (const w of due) this.pinged.add(w.requestId);
    if (!this.allowed(s, now)) return [];
    return due.map((w) => ({ kind: "approval", pillId: w.pillId, project: w.project }));
  }

  /** A session's turn ended (Stop). Not while one of its requests still waits: that has its own. */
  finished(pillId: string, project: string, requestWaiting: boolean, s: PhoneSettings, now: Date): Ping | null {
    if (!s.finished || requestWaiting || !this.allowed(s, now)) return null;
    return { kind: "finished", pillId, project };
  }

  /** A usage or spend limit stopped a session: once per pill and reset. */
  limit(pillId: string, project: string, kind: LimitKind, resets: string | null, s: PhoneSettings, now: Date): Ping | null {
    const key = `${pillId}|${kind}|${resets ?? ""}`;
    if (!s.limit || this.limits.has(key)) return null;
    this.limits.add(key);
    if (!this.allowed(s, now)) return null;
    return { kind: kind === "spend" ? "spendLimit" : "usageLimit", pillId, project };
  }
}

/** The island's one watcher. */
export const phone = new PhoneWatcher();
