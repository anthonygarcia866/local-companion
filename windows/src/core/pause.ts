// The user's pause entries (1d, docs/phase1-plan.md §14.5), checked as they
// are typed. The same rules as Rust's capture::sensitive::user_app and
// user_site, which check them again when settings.json is read; both are
// tested against tests/fixtures/pause-entries.json.

export type PauseRefusal = "empty" | "not-exe" | "bad-name" | "built-in" | "bad-site" | "bad-host" | "bad-path" | "bad-segment";

export type Checked = { ok: true; entry: string } | { ok: false; reason: PauseRefusal };

const BAD_NAME = /[\u0000-\u001f\u007f\\/:*?"<>|]/;

/** An app entry: the executable's file name, trimmed. */
export function userApp(entry: string, builtIn: readonly string[]): Checked {
  const e = entry.trim();
  if (!e) return { ok: false, reason: "empty" };
  if (!/\.exe$/i.test(e) || !e.slice(0, -4).trim()) return { ok: false, reason: "not-exe" };
  if ([...e].length > 128 || BAD_NAME.test(e)) return { ok: false, reason: "bad-name" };
  if (builtIn.some((a) => a.toLowerCase() === e.toLowerCase())) return { ok: false, reason: "built-in" };
  return { ok: true, entry: e };
}

const WORD = /^[a-z0-9\-_.~%]+$/;
const LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;

/** A site entry: "example.com", "example.com/path" or "*\/segment", lower-cased, without the scheme or a trailing "/". */
export function userSite(entry: string, builtIn: readonly string[]): Checked {
  let e = entry.trim().toLowerCase();
  for (const scheme of ["https://", "http://"]) if (e.startsWith(scheme)) e = e.slice(scheme.length);
  e = e.replace(/\/+$/, "");
  if (!e) return { ok: false, reason: "empty" };
  if (/[\s\u0000-\u001f\u007f?#@:\\]/.test(e)) return { ok: false, reason: "bad-site" };
  const slash = e.indexOf("/");
  const host = slash < 0 ? e : e.slice(0, slash);
  const path = slash < 0 ? "" : e.slice(slash + 1);
  let site: string;
  if (host === "*") {
    if (!WORD.test(path)) return { ok: false, reason: "bad-segment" };
    site = `*/${path}`;
  } else if (e.startsWith("*")) {
    return { ok: false, reason: "bad-segment" };
  } else {
    const labels = host.split(".");
    const tld = labels[labels.length - 1];
    if (labels.length < 2 || !labels.every((l) => LABEL.test(l)) || !/^[a-z]{2,}$/.test(tld)) return { ok: false, reason: "bad-host" };
    if (path && !path.split("/").every((s) => WORD.test(s))) return { ok: false, reason: "bad-path" };
    site = path ? `${host}/${path}` : host;
  }
  if (builtIn.includes(site)) return { ok: false, reason: "built-in" };
  return { ok: true, entry: site };
}

/** `list` with `entry` added once (any case), or as it was. */
export function addEntry(list: readonly string[], entry: string): string[] {
  return list.some((x) => x.toLowerCase() === entry.toLowerCase()) ? [...list] : [...list, entry];
}

export function removeEntry(list: readonly string[], entry: string): string[] {
  return list.filter((x) => x !== entry);
}
