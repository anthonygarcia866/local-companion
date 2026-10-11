// The per-session minis (pills and the compact 2×2 grid): a small lantern per
// agent session, in the state that session is in. They replaced upstream's
// mini characters (MiniBotCanvasView), which drew a placeholder.
//
// The lantern animates itself through CSS, so nothing ticks them; a session's
// colour is a soft glow behind its lantern, so sessions stay told apart.

import type { AgentTask } from "../core/state";
import { LANTERN_DRAWN_BOTTOM, LANTERN_DRAWN_TOP, LANTERN_HEIGHT_PER_DIAMETER, Lantern, lanternStateFor } from "./lantern";

const live = new Map<string, { lantern: Lantern; slot: HTMLElement }[]>();

/** A mini lantern for `task`, in a square slot `size` CSS pixels across. */
export function createMiniLantern(task: AgentTask, size: number): HTMLElement {
  const slot = document.createElement("span");
  slot.className = "mini";
  slot.style.width = `${size}px`;
  slot.style.height = `${size}px`;

  // As in the notch: the lantern stands 1.2× the slot, its base at the slot's bottom.
  const unit = (size * LANTERN_HEIGHT_PER_DIAMETER) / (LANTERN_DRAWN_BOTTOM - LANTERN_DRAWN_TOP);
  const lantern = new Lantern({ detail: "notch", state: lanternStateFor(task.state, false), heightPx: 124 * unit });
  lantern.el.style.width = `${100 * unit}px`;
  lantern.el.style.height = `${124 * unit}px`;
  lantern.el.style.bottom = `${-(124 - LANTERN_DRAWN_BOTTOM) * unit}px`;
  tint(lantern, task.color);
  slot.append(lantern.el);

  const list = live.get(task.id) ?? [];
  list.push({ lantern, slot });
  live.set(task.id, list);
  return slot;
}

function tint(lantern: Lantern, color: string) {
  lantern.el.style.filter = `drop-shadow(0 0 2px ${color})`;
}

/** Forgets the minis no longer in the document (views are rebuilt wholesale). */
export function pruneMiniLanterns() {
  for (const [id, list] of live) {
    const kept = list.filter((m) => m.slot.isConnected);
    if (kept.length) live.set(id, kept);
    else live.delete(id);
  }
}

/** Follows each session's state and colour. */
export function syncMiniLanterns(tasks: AgentTask[]) {
  for (const [id, list] of live) {
    const task = tasks.find((t) => t.id === id);
    if (!task) continue;
    for (const m of list) {
      m.lantern.show(lanternStateFor(task.state, false));
      tint(m.lantern, task.color);
    }
  }
}

export const miniLanternCount = () => [...live.values()].reduce((n, l) => n + l.length, 0);
