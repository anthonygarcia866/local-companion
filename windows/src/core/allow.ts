// The Allow key (fix e, owner 2026-10-17; replaces "Allow has no key"):
// Ctrl+Alt+Shift+Y allows the waiting permission, but only
// - while its card is open on screen (not folded, not queued behind another),
// - showing the whole command (nothing cut off),
// - after that card has been on screen ALLOW_DELAY_MS,
// and never for a command the owner's Claude Code "ask" rules name: those
// stay click-only, and the card says so.
//
// The risky list is built in (the owner's ask rules on 2026-10-17, with the
// flags in either order) and topped up with the live rules Rust reads from
// every config folder's settings.json (hooks::ask_patterns), so a rule added
// later counts too. Matching ignores case: stricter than Claude Code's.

/** How long the card must have been on screen before the key works. */
export const ALLOW_DELAY_MS = 1_500;

/** The owner's ask rules (2026-10-17), as glob patterns over the command. */
export const BUILT_IN_RISKY: readonly string[] = [
  "supabase *", "npx supabase *", "bunx supabase *", "psql *",
  "*drop table*", "*truncate table*",
  "rm -rf *", "rm -fr *", "*rm -rf *", "*rm -fr *",
  "*Remove-Item* -Recurse*-Force*", "*Remove-Item* -Force*-Recurse*",
  "git push --force*", "git push -f *", "*git push --force*", "*git push -f *",
  "gh pr merge*", "*gh pr merge*",
  "git merge*", "*git merge*",
];

/** A glob over the whole command: `*` is anything (line breaks too), the rest is literal, any case. */
export function globMatches(pattern: string, command: string): boolean {
  const body = pattern.split("*").map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join("[\\s\\S]*");
  return new RegExp(`^${body}$`, "i").test(command);
}

/** The command matches an ask rule (built in, or one of `live`). The card's
 *  text is "Bash · git merge x" (hooks.ts approvalTarget): matched with and
 *  without the tool's name in front. */
export function isRisky(command: string, live: readonly string[] = []): boolean {
  const full = command.trim();
  const sep = full.indexOf(" · ");
  const bare = sep >= 0 ? full.slice(sep + 3).trim() : full;
  return [...BUILT_IN_RISKY, ...live].some((p) => globMatches(p, full) || globMatches(p, bare));
}

/** Why the key won't allow right now ("ok" when it will). */
export type AllowCheck = "ok" | "no-card" | "question" | "not-open" | "cut-off" | "too-soon" | "risky";

export function allowCheck(o: {
  /** The waiting request, if any: its command and whether it's a question. */
  request: { command?: string; tool?: string; questions?: unknown } | null;
  /** Its card is the view on screen, opened (not folded). */
  open: boolean;
  /** The whole command fits on the card. */
  fullyShown: boolean;
  /** How long this request's card has been on screen. */
  shownMs: number;
  /** The live ask rules (hooks::ask_patterns). */
  live: readonly string[];
}): AllowCheck {
  if (!o.request) return "no-card";
  if (o.request.questions) return "question";
  if (!o.open) return "not-open";
  // The risk comes first: such a card never takes the key, however long it waits.
  if (isRisky(o.request.command ?? o.request.tool ?? "", o.live)) return "risky";
  // Not the whole command: the box cuts it off, or the relay shortened it ("…").
  if (!o.fullyShown || (o.request.command ?? "").trimEnd().endsWith("…")) return "cut-off";
  if (o.shownMs < ALLOW_DELAY_MS) return "too-soon";
  return "ok";
}

// ── What the card on screen reports (views.ts buildApproval) ──────────────────

let shownFor: string | null = null;
let shownSince = 0;
let fully = false;
let live: string[] = [];

/** The approval card is on screen for `requestId` (called on every sync while
 *  it is): its clock starts when it first shows, and again after it was away. */
export function cardOnScreen(requestId: string, now: number, fullyShown: boolean) {
  if (requestId !== shownFor) {
    shownFor = requestId;
    shownSince = now;
  }
  fully = fullyShown;
}

/** The card left the screen (folded, another view, closed): the clock stops. */
export function cardOffScreen() {
  shownFor = null;
  fully = false;
}

/** How long `requestId`'s card has been on screen, and whether it shows the whole command. */
export function onScreen(requestId: string, now: number): { shownMs: number; fullyShown: boolean } {
  return shownFor === requestId ? { shownMs: now - shownSince, fullyShown: fully } : { shownMs: 0, fullyShown: false };
}

/** The live ask rules, as Rust last read them. */
export function liveRules(): readonly string[] {
  return live;
}

export function setLiveRules(rules: unknown) {
  live = Array.isArray(rules) ? rules.filter((r): r is string => typeof r === "string" && r.trim() !== "") : [];
}
