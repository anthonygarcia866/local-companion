// The lantern's right-click menu: Settings…, Hide, Ember, and the dock
// position. Drawn in the island's own page, never a native popup: a Windows
// popup menu needs its window in the foreground, which would take focus from
// the app the user is typing in. The island window is WS_EX_NOACTIVATE, so
// clicking in this menu takes no focus (and it has none: no keyboard use).

import { Bridge } from "../core/bridge";
import { DOCKS, type Dock } from "../core/layout";
import { N_, t, tl } from "../i18n/i18n";
import { h } from "../views/dom";

export const MENU_W = 196;
/** Gap kept between the menu and the window's edges. */
const EDGE = 6;

export type MenuAction =
  | { kind: "settings" }
  | { kind: "visibility"; visibility: "ember" | "hidden" }
  | { kind: "dock"; dock: Dock };

/** Docks in the order Settings lists them, with Settings' labels. */
export const DOCK_LABELS: Record<Dock, string> = {
  "top-center": N_("Top centre"),
  "top-left": N_("Top left"),
  "top-right": N_("Top right"),
  left: N_("Left edge (upright)"),
  right: N_("Right edge (upright)"),
};

/** Where the menu opens: at the cursor, kept inside the window. */
export function menuPlacement(
  x: number, y: number, w: number, h: number, winW: number, winH: number,
): { x: number; y: number } {
  const left = Math.max(EDGE, Math.min(x, winW - w - EDGE));
  // Below the cursor when it fits, else above it, else as high as it goes.
  const below = y + h + EDGE <= winH ? y : y - h;
  const top = Math.max(EDGE, Math.min(below, winH - h - EDGE));
  return { x: left, y: top };
}

/** Runs a menu choice through the same paths as Settings and the tray. */
export function runMenuAction(a: MenuAction) {
  switch (a.kind) {
    case "settings":
      void Bridge.openSettingsWindow();
      break;
    case "visibility":
      void Bridge.setVisibility(a.visibility);
      break;
    case "dock":
      void Bridge.setDock(a.dock);
      break;
  }
}

export interface MenuHost {
  /** The menu's rect while it is open (null when closed): it has to take the mouse. */
  setMenuRect(r: { x: number; y: number; w: number; h: number } | null): void;
}

export class LanternMenu {
  readonly el: HTMLElement;
  private host: MenuHost;
  private open = false;
  private rect = { x: 0, y: 0, w: 0, h: 0 };

  constructor(host: MenuHost) {
    this.host = host;
    this.el = h("div", { id: "lantern-menu", role: "menu" });
    this.el.hidden = true;
    // No browser menu on the menu itself.
    this.el.addEventListener("contextmenu", (e) => e.preventDefault());
    // A press anywhere else in the page closes it.
    window.addEventListener("mousedown", (e) => {
      if (this.open && !this.el.contains(e.target as Node)) this.close();
    });
  }

  get isOpen(): boolean {
    return this.open;
  }

  show(x: number, y: number, dock: Dock) {
    const item = (label: string, action: MenuAction, extra = "") =>
      h("button", {
        class: `menu-item${extra}`,
        type: "button",
        role: "menuitem",
        text: label,
        onclick: () => {
          this.close();
          runMenuAction(action);
        },
      });
    this.el.replaceChildren(
      item(t("Settings…"), { kind: "settings" }),
      item(t("Hide"), { kind: "visibility", visibility: "hidden" }),
      item(t("Ember"), { kind: "visibility", visibility: "ember" }),
      h("div", { class: "menu-sep" }),
      h("div", { class: "menu-head", text: tl("Dock position") }),
      ...DOCKS.map((d) => item(t(DOCK_LABELS[d]), { kind: "dock", dock: d }, d === dock ? " on" : "")),
    );
    this.el.hidden = false;
    this.open = true;
    const height = this.el.getBoundingClientRect().height || 250;
    const at = menuPlacement(x, y, MENU_W, height, window.innerWidth, window.innerHeight);
    this.el.style.left = `${at.x}px`;
    this.el.style.top = `${at.y}px`;
    this.rect = { x: at.x, y: at.y, w: MENU_W, h: height };
    this.host.setMenuRect(this.rect);
  }

  close() {
    if (!this.open) return;
    this.open = false;
    this.el.hidden = true;
    this.host.setMenuRect(null);
  }

  /** The cursor (window coordinates): wandering well away from the menu closes it,
   *  since without focus there is no Escape and no click elsewhere reaches us. */
  onCursor(x: number, y: number, nearIsland: boolean) {
    if (!this.open || nearIsland) return;
    const r = this.rect;
    const far = 48;
    if (x < r.x - far || x > r.x + r.w + far || y < r.y - far || y > r.y + r.h + far) this.close();
  }
}
