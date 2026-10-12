// A usage or spend limit that stopped an agent: Claude Code's StopFailure
// carries `error: "rate_limit"` (the plan's 5-hour or weekly limit) or
// `"billing_error"` (a spend or billing limit), and the turn's last message
// says when it resets: "You've hit your limit · resets 2pm (America/Los_Angeles)".
// Only the reset time is read from that text; nothing else is kept.

export type LimitKind = "usage" | "spend";

export interface AgentLimit {
  kind: LimitKind;
  /** The reset as Claude Code wrote it, tidied ("2 PM", "Oct 14, 9 AM"); null when not said. */
  resets: string | null;
  /** When it resets, if the text gives a time of day this computer can place (epoch ms). */
  resetsAt: number | null;
}

/** The StopFailure errors that are limits, not failures. */
export function limitKind(error: unknown): LimitKind | null {
  if (error === "rate_limit") return "usage";
  if (error === "billing_error") return "spend";
  return null;
}

/** True for a Notification that says the agent hit a rate limit. */
export function isLimitNotice(message: string): boolean {
  const lower = message.toLowerCase();
  return lower.includes("rate limit") || lower.includes("usage limit") || lower.includes("limite d") || /\bhit your\b.*\blimit\b/.test(lower);
}

const CLOCK = /^(\d{1,2})(?::(\d{2}))?\s*(am|pm)$/i;

/** "2pm" → "2 PM", "9:30am" → "9:30 AM"; anything else as it was. */
function tidyClock(text: string): string {
  return text.replace(/\b(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b/gi, (_, h, m, ap) => `${h}${m ? `:${m}` : ""} ${ap.toUpperCase()}`);
}

/**
 * The next moment after `now` at that clock time, in this computer's time
 * zone. Only when the text names no zone or this computer's own: a time in
 * another zone is shown as written but not placed.
 */
function nextAt(clock: string, zone: string | null, localZone: string, now: number): number | null {
  if (zone && zone !== localZone) return null;
  const m = CLOCK.exec(clock.trim());
  if (!m) return null;
  let hour = Number(m[1]) % 12;
  if (m[3].toLowerCase() === "pm") hour += 12;
  const minute = m[2] ? Number(m[2]) : 0;
  if (minute > 59) return null;
  const at = new Date(now);
  at.setHours(hour, minute, 0, 0);
  if (at.getTime() <= now) at.setDate(at.getDate() + 1);
  return at.getTime();
}

/**
 * Reads the limit from the StopFailure (or Notification) texts. `texts` are
 * searched in order for "resets <when> (<zone>)"; the first that has one wins.
 */
export function parseLimit(
  kind: LimitKind,
  texts: (string | undefined)[],
  now: number,
  localZone: string = Intl.DateTimeFormat().resolvedOptions().timeZone,
): AgentLimit {
  for (const text of texts) {
    if (!text) continue;
    const m = /\bresets?\s+(?:at\s+|on\s+)?([^·\n()]+?)\s*(?:\(([^)\n]+)\))?\s*(?:[·.\n]|$)/i.exec(text);
    if (!m) continue;
    const when = m[1].trim();
    if (!when || when.length > 40) continue;
    const zone = m[2]?.trim() || null;
    const shownZone = zone && zone !== localZone ? ` (${zone})` : "";
    return { kind, resets: tidyClock(when) + shownZone, resetsAt: nextAt(when, zone, localZone, now) };
  }
  return { kind, resets: null, resetsAt: null };
}

/** True once the reset time has passed (never for a limit whose time isn't known). */
export function limitHasReset(limit: AgentLimit, now: number): boolean {
  return limit.resetsAt != null && now >= limit.resetsAt;
}
