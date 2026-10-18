// Waiting-approval escalation (owner, 2026-10-17, with the Allow key's
// redesign): a permission or question that has waited ESCALATE_AFTER_MS makes
// the lantern pulse (the design's attention pop, again every PULSE_EVERY_MS)
// until it's answered, and, if Settings → Claude Code → "Play a soft sound"
// is on (off by default), one soft chime. Every request's wait is logged when
// it ends (counts and times only, never the command).

/** How long a card waits before Glim calls for attention. */
export const ESCALATE_AFTER_MS = 30_000;
/** How often the lantern pulses after that. */
export const PULSE_EVERY_MS = 4_000;

/** What the island does after a tick. */
export interface WaitTick {
  /** Pop the lantern now. */
  pulse: boolean;
  /** Play the soft sound now (once per request; the island checks the setting). */
  chime: boolean;
  /** Requests that stopped waiting since the last tick, with how long they waited. */
  ended: { requestId: string; waitedMs: number; how: string; escalated: boolean }[];
}

export class ApprovalWait {
  private since = new Map<string, number>();
  private escalated = new Set<string>();
  private how = new Map<string, string>();
  private lastPulse = -Infinity;

  /** How it was answered ("allow (key)", "deny (click)"…), for the log line. */
  decided(requestId: string, how: string) {
    this.how.set(requestId, how);
  }

  /** How long `requestId` has waited, or 0 if it isn't waiting. */
  waited(requestId: string, now: number): number {
    const t = this.since.get(requestId);
    return t === undefined ? 0 : now - t;
  }

  /** `waiting`: the ids waiting now (the card on screen and the queue). */
  tick(waiting: readonly string[], now: number): WaitTick {
    const out: WaitTick = { pulse: false, chime: false, ended: [] };
    for (const id of waiting) if (!this.since.has(id)) this.since.set(id, now);
    for (const [id, t] of [...this.since]) {
      if (waiting.includes(id)) continue;
      out.ended.push({ requestId: id, waitedMs: now - t, how: this.how.get(id) ?? "answered elsewhere", escalated: this.escalated.has(id) });
      this.since.delete(id);
      this.escalated.delete(id);
      this.how.delete(id);
    }
    let overdue = false;
    for (const [id, t] of this.since) {
      if (now - t < ESCALATE_AFTER_MS) continue;
      overdue = true;
      if (!this.escalated.has(id)) {
        this.escalated.add(id);
        out.chime = true;
      }
    }
    if (overdue && now - this.lastPulse >= PULSE_EVERY_MS) {
      out.pulse = true;
      this.lastPulse = now;
    }
    return out;
  }
}

/** The soft sound: two quiet sine notes, made in code (no audio file, as
 *  Phase 4's sounds will be). Nothing if the webview has no audio. */
export function playSoftChime() {
  const Ctx = (globalThis as { AudioContext?: typeof AudioContext }).AudioContext;
  if (!Ctx) return;
  try {
    const ctx = new Ctx();
    const t0 = ctx.currentTime;
    for (const [i, freq] of [660, 880].entries()) {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = freq;
      const start = t0 + i * 0.16;
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(0.06, start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.35);
      osc.connect(gain).connect(ctx.destination);
      osc.start(start);
      osc.stop(start + 0.4);
    }
    window.setTimeout(() => void ctx.close(), 1_000);
  } catch {
    // No sound device: the pulse is still there.
  }
}
