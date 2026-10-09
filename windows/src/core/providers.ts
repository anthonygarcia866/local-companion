// Who the chat can talk to — the island's side of chat.rs: a model server on
// this machine, Ollama or LM Studio. There are no cloud providers (net/ in Rust
// refuses anything but 127.0.0.1 and localhost). Pure data and helpers, so they
// can be tested without a webview.

import type { Settings } from "./state";
export type ProviderId = "ollama" | "lmstudio";

export interface ProviderDef {
  id: ProviderId;
  /** Shown on the chip in the chat's model picker. */
  name: string;
  accent: string;
  /** Settings field holding the server's address. */
  urlField: "ollamaUrl" | "lmstudioUrl";
}

export const PROVIDERS: readonly ProviderDef[] = [
  { id: "ollama", name: "Ollama", accent: "#FACC15", urlField: "ollamaUrl" },
  { id: "lmstudio", name: "LM Studio", accent: "#A3E635", urlField: "lmstudioUrl" },
];

export function providerDef(id: string): ProviderDef {
  return PROVIDERS.find((p) => p.id === id) ?? PROVIDERS[0];
}

/** The model the chat uses for the active provider. */
export function activeModel(settings: Settings): string {
  const p = providerDef(settings.chatProvider);
  return settings.chatModels[p.id] || "";
}

/** `settings` with `model` picked for `provider`. */
export function withModel(settings: Settings, provider: ProviderId, model: string): Settings {
  return { ...settings, chatModels: { ...settings.chatModels, [provider]: model } };
}

/**
 * The chips of the picker: a model server once it is connected — or while it
 * is the active one, so the picker never hides where the chat goes.
 */
export function visibleProviders(settings: Settings): ProviderDef[] {
  return PROVIDERS.filter((p) => settings[p.urlField] !== "" || providerDef(settings.chatProvider).id === p.id);
}

/** The model to keep once the list arrives: the saved one if offered, else a sensible one. */
export function pickModel(_provider: ProviderDef, offered: string[], current: string): string | null {
  if (offered.length === 0) return null;
  return offered.includes(current) ? current : offered[0];
}

// ── Where an address points ───────────────────────────────────────────────────

/**
 * Names Rust's normalise_server_url turns into 127.0.0.1 before checking the
 * allowlist (net/mod.rs), so they count as this machine here too.
 */
export function isLoopbackHost(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, "").toLowerCase();
  return h === "localhost" || h === "127.0.0.1" || h === "0.0.0.0" || h === "::1" || h === "::";
}

/**
 * Where a server address points: this machine, another one (which Rust
 * refuses), or nowhere valid. An empty field means the usual address on this
 * machine.
 */
export type Exposure = "local" | "remote" | "invalid";

export function urlExposure(raw: string): Exposure {
  const text = raw.trim();
  if (!text) return "local";
  let url: URL;
  try {
    url = new URL(text.includes("://") ? text : `http://${text}`);
  } catch {
    return "invalid";
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return "invalid";
  if (!url.hostname || url.username || url.password) return "invalid";
  return isLoopbackHost(url.hostname) ? "local" : "remote";
}
