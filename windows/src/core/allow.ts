// The Allow key (fix e, redesigned 2026-10-17 by the owner; replaces "Allow
// has no key"): Ctrl+Alt+Shift+Y allows the waiting permission on any
// approval card, but only
// - while its card is open on screen (not folded, not queued behind another),
// - showing the whole command (nothing cut off),
// - after that card has been on screen ALLOW_DELAY_MS.
// A destructive command (DESTRUCTIVE: rm -rf, Remove-Item -Recurse -Force,
// git push --force, DROP/TRUNCATE, database tools) takes two presses: the
// first shows "Press again to allow · 3" counting down CONFIRM_WINDOW_MS, a
// second press within it allows, otherwise it resets, as often as it takes. Everything else is a
// single press. Claude Code's own "ask" rules play no part: with the owner's
// settings every card is an ask rule ("let me glance first").
//
// A command the card cuts off (fix 2026-10-25: long bash commands never
// fit one line, so the key refused them all): the first press EXPANDS the
// card to the whole command (it grows, then scrolls inside the box) and
// opens the same CONFIRM_WINDOW_MS window, "Press again to allow · 3". A press
// in it allows (a destructive one then takes its own two presses); too late,
// and the next press only opens the window again. ALLOW_DELAY_MS counts
// from the expansion. Never allowed by key: a command the relay shortened
// ("…": longer than glim-hook's MAX_APPROVAL_LEN), "Too long to show in
// full: click Allow".

/** How long the card must have been on screen before the key works. */
export const ALLOW_DELAY_MS = 1_500;
/** How long the first press of a destructive command waits for the second. */
export const CONFIRM_WINDOW_MS = 3_000;

/** Destructive commands (owner, 2026-10-17), as glob patterns over the
 *  command. Either flag order; anywhere in the command (after `cd x &&`, in a
 *  pipe). Matching ignores case. */
export const DESTRUCTIVE: readonly string[] = [
  "*rm -rf*", "*rm -fr*", "*rm -r -f*", "*rm -f -r*", "*rm --recursive --force*", "*rm --force --recursive*",
  "*Remove-Item*-Recurse*-Force*", "*Remove-Item*-Force*-Recurse*",
  "*rd /s /q*", "*rmdir /s /q*",
  "*git push --force*", "*git push -f*", "*git push * --force*", "*git push * -f *", "*git push * -f",
  "*drop table*", "*drop database*", "*drop schema*", "*drop view*", "*drop index*", "*truncate *",
  // Database tools: psql, supabase and the others the owner might reach for.
  "psql*", "* psql*", "*supabase *", "mysql*", "* mysql*", "sqlite3*", "* sqlite3*", "mongosh*", "* mongosh*",
  "redis-cli*", "* redis-cli*", "sqlcmd*", "* sqlcmd*", "dropdb*", "* dropdb*", "pg_restore*", "* pg_restore*",
  "*prisma migrate*", "*prisma db *", "*drizzle-kit push*",
];

/** A glob over the whole command: `*` is anything (line breaks too), the rest is literal, any case. */
export function globMatches(pattern: string, command: string): boolean {
  const body = pattern.split("*").map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join("[\\s\\S]*");
  return new RegExp(`^${body}$`, "i").test(command);
}

/** The command is destructive (needs two presses). The card's text is
 *  "Bash · rm -rf x" (hooks.ts approvalTarget): matched with and without the
 *  tool's name in front. */
export function isDestructive(command: string): boolean {
  const full = command.trim();
  const sep = full.indexOf(" · ");
  const bare = sep >= 0 ? full.slice(sep + 3).trim() : full;
  return DESTRUCTIVE.some((p) => globMatches(p, full) || globMatches(p, bare));
}

/** Why the key won't allow right now. "ok": allow. "armed": the first of two
 *  presses, the card asks for the second. */
export type AllowCheck = "ok" | "armed" | "expand" | "no-card" | "question" | "not-open" | "too-long" | "too-soon";

/** A cut-off card's first press: the card shows the whole command from `at`;
 *  the window for the press that allows started at `armedAt`. */
export interface Expanded {
  requestId: string;
  at: number;
  armedAt: number;
}

/** The first press of a destructive command: which request, and when. */
export interface Armed {
  requestId: string;
  at: number;
}

export function allowCheck(o: {
  /** The waiting request, if any: its command and whether it's a question. */
  request: { requestId?: string; command?: string; tool?: string; questions?: unknown } | null;
  /** Its card is the view on screen, opened (not folded). */
  open: boolean;
  /** The whole command fits on the card. */
  fullyShown: boolean;
  /** How long this request's card has been on screen. */
  shownMs: number;
  /** The first press, if one is waiting for its second. */
  armed?: Armed | null;
  /** The card was expanded to the whole command (a cut-off one's first press). */
  expanded?: Expanded | null;
  /** Now, on the same clock as `armed.at`. */
  now?: number;
}): AllowCheck {
  if (!o.request) return "no-card";
  if (o.request.questions) return "question";
  if (!o.open) return "not-open";
  const now = o.now ?? 0;
  // The second press within the window: the first one passed every check
  // below (the "Press again" line may squeeze the command's box meanwhile).
  const a = o.armed;
  if (a && a.requestId === o.request.requestId && now - a.at <= CONFIRM_WINDOW_MS) return "ok";
  // Shortened by the relay: nobody can be shown the rest. Click only.
  if (relayShortened(o.request.command)) return "too-long";
  const ex = o.expanded && o.expanded.requestId === o.request.requestId ? o.expanded : null;
  // Cut off on the card: the press shows it all, and opens the window for
  // the press that allows (again, once the window has passed).
  if (!o.fullyShown && (!ex || now - ex.armedAt > CONFIRM_WINDOW_MS)) return "expand";
  const shownMs = ex ? Math.min(o.shownMs, now - ex.at) : o.shownMs;
  if (shownMs < ALLOW_DELAY_MS) return "too-soon";
  return isDestructive(o.request.command ?? o.request.tool ?? "") ? "armed" : "ok";
}

/** The relay cut the command short (glim-hook adds "…" past MAX_APPROVAL_LEN). */
export function relayShortened(command: string | undefined): boolean {
  return (command ?? "").trimEnd().endsWith("…");
}

// ── What the card on screen reports (views.ts buildApproval) ──────────────────

let shownFor: string | null = null;
let shownSince = 0;
let fully: () => boolean = () => false;
let armed: Armed | null = null;
let expanded: Expanded | null = null;
/** The last refusal, for the card's one-line reason (REASON_MS). */
let refused: { requestId: string; check: AllowCheck; at: number } | null = null;

/** How long a refusal's reason stays on the card. */
export const REASON_MS = 3_000;

/** The approval card is on screen for `requestId` (called on every sync while
 *  it is): its clock starts when it first shows, and again after it was away.
 *  `fullyShown` is asked at the press, not at the sync (bug 2, 2026-10-18): a
 *  redraw can measure the box mid-expand, or squeezed by the "Press again"
 *  line it is about to remove, and "cut-off" then stuck until the next one. */
export function cardOnScreen(requestId: string, now: number, fullyShown: boolean | (() => boolean)) {
  if (requestId !== shownFor) {
    shownFor = requestId;
    shownSince = now;
    armed = null;
    expanded = null;
    refused = null;
  }
  fully = typeof fullyShown === "function" ? fullyShown : () => fullyShown;
}

/** The card left the screen (folded, another view, closed): the clock stops,
 *  and a first press is forgotten. */
export function cardOffScreen() {
  shownFor = null;
  fully = () => false;
  armed = null;
  // Opened again, it starts small and must be expanded (and read) again.
  expanded = null;
}

/** The card shows the whole command for `requestId` (expanded by a press). */
export function expandedFor(requestId: string | null | undefined): Expanded | null {
  return requestId && expanded?.requestId === requestId ? expanded : null;
}

/** The window opened by an expanding press, while it is open. */
export function expandWindow(requestId: string | null | undefined, now: number): Expanded | null {
  const ex = expandedFor(requestId);
  return ex && now - ex.armedAt <= CONFIRM_WINDOW_MS ? ex : null;
}

/** Why the last press on `requestId` was refused, while the card says so. */
export function refusalFor(requestId: string | null | undefined, now: number): AllowCheck | null {
  return requestId && refused?.requestId === requestId && now - refused.at <= REASON_MS ? refused.check : null;
}

/** How long `requestId`'s card has been on screen, and whether it shows the whole command. */
export function onScreen(requestId: string, now: number): { shownMs: number; fullyShown: boolean } {
  return shownFor === requestId ? { shownMs: now - shownSince, fullyShown: fully() } : { shownMs: 0, fullyShown: false };
}

/** The first press of a destructive command, while its window is open
 *  (null once CONFIRM_WINDOW_MS has passed: the card goes back). */
export function armedFor(now: number): Armed | null {
  if (armed && now - armed.at > CONFIRM_WINDOW_MS) armed = null;
  return armed;
}

/** Whole seconds left for the second press (3, 2, 1), for the card's countdown. */
export function secondsLeft(a: Armed, now: number): number {
  return Math.max(1, Math.ceil((CONFIRM_WINDOW_MS - (now - a.at)) / 1_000));
}

/** Remembers a first press (allowCheck said "armed"). */
export function arm(requestId: string, now: number) {
  armed = { requestId, at: now };
}

/** A press: what the key does now. Arms on a destructive command's first
 *  press, clears the arming when it allows. */
export function press(request: Parameters<typeof allowCheck>[0]["request"], open: boolean, now: number): AllowCheck {
  const id = request?.requestId;
  const seen = id ? onScreen(id, now) : { shownMs: 0, fullyShown: false };
  const check = allowCheck({ request, open, ...seen, armed: armedFor(now), expanded: expandedFor(id), now });
  if (check === "armed" && id) arm(id, now);
  // Shown in full from the first expansion; the window from this press.
  if (check === "expand" && id) expanded = { requestId: id, at: expanded?.requestId === id ? expanded.at : now, armedAt: now };
  if (check === "ok") {
    armed = null;
    expanded = null;
  }
  refused = id && check !== "ok" && check !== "armed" && check !== "expand" ? { requestId: id, check, at: now } : null;
  return check;
}
