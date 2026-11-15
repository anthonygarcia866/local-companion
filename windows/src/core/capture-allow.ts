// "Allow screenshots for 10 minutes" (owner 2026-11-14): Rust keeps the
// window (screen_capture.rs, memory only) and tells every window its end on
// "capture-allowed" (Unix ms, 0 = hidden again). This is the countdown both
// the island and Settings → Privacy show.

/** "9:41" left until `untilMs`, or "" when it's over (or never started). */
export function countdown(untilMs: number, nowMs: number): string {
  const left = Math.ceil((untilMs - nowMs) / 1000);
  if (!untilMs || left <= 0) return "";
  const m = Math.floor(left / 60);
  const s = left % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}
