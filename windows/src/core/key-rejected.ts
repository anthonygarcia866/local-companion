// The Claude API refused Glim's key (a 401; ai/key_rejected.rs, owner
// 2026-11-14): one clear card on the island, at most once an hour (Rust
// decides when), instead of only a line in glim.log.

import { t } from "../i18n/i18n";

/** The card closes by itself after this long. */
export const KEY_REJECTED_MS = 20_000;
/** While an approval card is up, it waits its turn (checked this often, this long at most). */
export const KEY_REJECTED_WAIT_STEP_MS = 2_000;
export const KEY_REJECTED_WAIT_MAX_MS = 10 * 60_000;

export function keyRejectedLine(): string {
  return t("Glim's Claude key was rejected: update it in Settings → Writing → AI (Claude API)");
}

/**
 * Shows the card through `show` once nothing more urgent (an approval) is up,
 * checking every KEY_REJECTED_WAIT_STEP_MS for at most KEY_REJECTED_WAIT_MAX_MS.
 */
export function showWhenFree(busy: () => boolean, show: () => void, later: (f: () => void, ms: number) => void, waited = 0): void {
  if (!busy()) return show();
  if (waited >= KEY_REJECTED_WAIT_MAX_MS) return;
  later(() => showWhenFree(busy, show, later, waited + KEY_REJECTED_WAIT_STEP_MS), KEY_REJECTED_WAIT_STEP_MS);
}
