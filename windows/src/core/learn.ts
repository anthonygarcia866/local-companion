// Glim learns (docs/glim-learns-plan.md §1-§2): what Settings → Ask Glim
// lists, as Rust sends it (learn::LearnView). The files are Rust's
// (%LOCALAPPDATA%\Glim\learn); the page only shows them and asks Rust to
// forget, rename or remove.

import type { StepView } from "./voice";
import { t } from "../i18n/i18n";

export interface ShortcutView {
  /** The words it answers to, sorted ("open {app}"). */
  key: string;
  /** open_app with the app as a slot: the same words about any known app. */
  slot: boolean;
  step: StepView;
  /** Days until it's forgotten unless used. */
  daysLeft: number;
}

export interface RoutineView {
  name: string;
  steps: StepView[];
}

export interface LearnView {
  shortcuts: ShortcutView[];
  routines: RoutineView[];
}

/** A shortcut's words, with "any app" for its slot. */
export function keyText(s: ShortcutView): string {
  return s.key.split(" ").map((w) => (w === "{app}" ? t("any app") : w)).join(" ");
}
