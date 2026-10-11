// Island geometry — ported from IslandTypes.swift + IslandWindowController.islandSize
// + IslandRootView.botPosition. All values are logical pixels, identical to the
// macOS app's points.

export type IslandMode = "hidden" | "compact" | "expanded";

export type IslandViewName =
  | "overview"
  | "empty"
  | "approval"
  | "question"
  | "error"
  | "finished"
  | "confused"
  | "upload"
  | "uploading"
  | "choose"
  | "mail"
  | "prompt"
  | "searching"
  | "result"
  | "note"
  | "settings"
  | "recap"
  | "suggestions";

export type BotStateName =
  | "idle"
  | "working"
  | "thinking"
  | "searching"
  | "approval"
  | "question"
  | "error"
  | "finished"
  | "ratelimit"
  | "sleeping"
  | "dizzy";

export type BotEmoteName = "love" | "surprised" | "proud" | "wink" | "yawn" | "happy" | "annoyed";

export type AgentLayoutMode = "none" | "grid" | "pills" | "column";

export interface ViewLayout {
  height: number;
  botX: number;
  botY: number | null; // null = auto-centred
  botDiameter: number;
  agentMode: AgentLayoutMode;
}

// The window is a fixed 720×320 (largest view) like the macOS panel; the island is
// drawn inside it, glued to the top edge and horizontally centred.
export const PANEL_W = 720;
export const PANEL_H = 320;

// No notch on a PC: these are the hidden/compact sizes from docs/upstream/SPEC.md.
export const NOTCH_W = 184;
export const NOTCH_H = 32;
export const COMPACT_W = 288; // NOTCH_W + 104
export const EXPANDED_W = 640;

export const ROUNDED_CORNER = 14; // hidden / compact

/** The notch pill's size setting. */
export type PillSize = "small" | "medium" | "large";
export const PILL_SIZES: readonly PillSize[] = ["small", "medium", "large"];

/** The collapsed pill for a size: its box, and where the lantern sits in it.
 *  The lantern stands 1.2 × `diameter` tall (island.ts), its base at
 *  `botCy + diameter / 2`: 24 px (small, compact drawing), 36 px (medium,
 *  still compact) and 48 px (large: the full drawing with face, brows and
 *  hands, since the lantern's own box is 60 px ≥ COMPACT_BELOW_PX). */
export interface PillGeometry {
  w: number;
  h: number;
  botCx: number;
  botCy: number;
  diameter: number;
}

export function pillGeometry(size: PillSize | string): PillGeometry {
  switch (size) {
    case "small":
      return { w: 288, h: 32, botCx: 40, botCy: 16, diameter: 20 };
    case "medium":
      return { w: 320, h: 44, botCx: 46, botCy: 25, diameter: 30 };
    default:
      return { w: 352, h: 56, botCx: 52, botCy: 32, diameter: 40 };
  }
}
export const EXPANDED_CORNER = 22;

/** Where the island is docked on its display (src-tauri/src/placement.rs). */
export type Dock = "top-center" | "top-left" | "top-right" | "left" | "right";
export const DOCKS: readonly Dock[] = ["top-center", "top-left", "top-right", "left", "right"];

export function isDock(s: string): s is Dock {
  return (DOCKS as readonly string[]).includes(s);
}

/** Upright along the left or right edge: the pill stands vertically and the
 *  island opens sideways, away from the edge. */
export function isVerticalDock(d: Dock): boolean {
  return d === "left" || d === "right";
}

/** The window's height when docked upright (placement.rs PANEL_V_H). */
export const PANEL_V_H = 400;
/** Gap between a top-corner island and the display's side edge. */
export const CORNER_GAP = 12;

/**
 * The island's drawn size for a dock: upright docks turn the pill (and the
 * retracted island) on its side; the open island keeps its size.
 */
export function dockedSize(dock: Dock, mode: IslandMode, size: { w: number; h: number }): { w: number; h: number } {
  return isVerticalDock(dock) && mode !== "expanded" ? { w: size.h, h: size.w } : size;
}

/** The island's frame in its window (window-logical px), and its corners. */
export function islandFrame(dock: Dock, w: number, h: number, r: number): {
  x: number; y: number; w: number; h: number; radius: string;
} {
  switch (dock) {
    case "top-left":
      return { x: CORNER_GAP, y: 0, w, h, radius: `0 0 ${r}px ${r}px` };
    case "top-right":
      return { x: PANEL_W - CORNER_GAP - w, y: 0, w, h, radius: `0 0 ${r}px ${r}px` };
    case "left":
      return { x: 0, y: (PANEL_V_H - h) / 2, w, h, radius: `0 ${r}px ${r}px 0` };
    case "right":
      return { x: PANEL_W - w, y: (PANEL_V_H - h) / 2, w, h, radius: `${r}px 0 0 ${r}px` };
    default:
      return { x: (PANEL_W - w) / 2, y: 0, w, h, radius: `0 0 ${r}px ${r}px` };
  }
}

/** The lantern's place in a docked island: an upright pill carries it at the
 *  top, where a level pill has it on the left. */
export function dockedBot(dock: Dock, mode: IslandMode, p: BotPlacement): BotPlacement {
  return isVerticalDock(dock) && mode !== "expanded" ? { ...p, cx: p.cy, cy: p.cx } : p;
}

/** Invisible hover strip that wakes the island when hidden. */
export const WAKE_STRIP_W = 240;
export const WAKE_STRIP_H = 6;

export const VIEW_LAYOUTS: Record<IslandViewName, ViewLayout> = {
  overview: { height: 160, botX: 68, botY: null, botDiameter: 58, agentMode: "pills" },
  empty: { height: 160, botX: 70, botY: null, botDiameter: 62, agentMode: "none" },
  approval: { height: 160, botX: 62, botY: null, botDiameter: 56, agentMode: "column" },
  question: { height: 160, botX: 62, botY: null, botDiameter: 56, agentMode: "column" },
  error: { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  finished: { height: 160, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  confused: { height: 160, botX: 76, botY: null, botDiameter: 66, agentMode: "column" },
  upload: { height: 176, botX: 140, botY: 104, botDiameter: 62, agentMode: "column" },
  // botY 103 = bar top (42 + 58) + 3, so the dot really rides the bar. The Swift
  // layout says 118 while its own comment says 103; the comment matches the spec.
  uploading: { height: 176, botX: 46, botY: 103, botDiameter: 20, agentMode: "none" },
  choose: { height: 176, botX: 60, botY: 101, botDiameter: 52, agentMode: "column" },
  mail: { height: 240, botX: 56, botY: null, botDiameter: 46, agentMode: "column" },
  prompt: { height: 160, botX: 52, botY: null, botDiameter: 44, agentMode: "column" },
  searching: { height: 160, botX: 52, botY: null, botDiameter: 44, agentMode: "column" },
  result: { height: 160, botX: 52, botY: null, botDiameter: 44, agentMode: "column" },
  note: { height: 160, botX: 60, botY: null, botDiameter: 50, agentMode: "column" },
  settings: { height: 160, botX: 54, botY: null, botDiameter: 46, agentMode: "none" },
  // Mac: 160. The extra 24 hold the two lines with top agent, project, busiest
  // day, longest session, permissions and questions, which the Mac card leaves
  // to the shared image.
  recap: { height: 184, botX: 62, botY: null, botDiameter: 58, agentMode: "column" },
  // The writing checker's list (src/views/views.ts buildSuggestions): a title,
  // a page of three suggestions (~57 px each) and the pager row. At 212 the
  // third row and the pager were cut off, and the window can't scroll.
  suggestions: { height: 290, botX: 52, botY: null, botDiameter: 44, agentMode: "none" },
};

// The upload views above are only the fallback geometry. Once a file is actually
// dropped the whole sequence — Mochi included — is drawn by src/upload, which
// owns its own constants (USC) straight from UploadSequenceEngine.swift.

/** The question view with options to pick from: room for two rows of them. */
export const QUESTION_PICKER_H = 200;

/** Chat view grows with the conversation — IslandContainer.chatPromptHeight. */
export function chatPromptHeight(messageCount: number): number {
  return Math.min(300, 240 + messageCount * 40);
}

export function islandSize(
  mode: IslandMode,
  view: IslandViewName,
  chatCount = 0,
  pill: PillGeometry = pillGeometry("small"),
): { w: number; h: number } {
  switch (mode) {
    case "hidden":
      // No notch to hide inside on a PC: the island retracts to zero height and
      // slides into the top edge of the screen instead of sitting there as a bar.
      return { w: NOTCH_W, h: 0 };
    case "compact":
      return { w: pill.w, h: pill.h };
    case "expanded": {
      const h = view === "prompt" ? chatPromptHeight(chatCount) : VIEW_LAYOUTS[view].height;
      return { w: EXPANDED_W, h };
    }
  }
}

export interface BotPlacement {
  cx: number;
  cy: number;
  diameter: number;
  opacity: number;
}

/** IslandRootView.botPosition — cy is measured from the island's top edge. */
export function botPosition(
  mode: IslandMode,
  view: IslandViewName,
  islandH: number,
  uploadProgress = 0,
  pill: PillGeometry = pillGeometry("small"),
): BotPlacement {
  switch (mode) {
    case "hidden":
      return { cx: 46, cy: 16, diameter: 6, opacity: 0 };
    case "compact":
      return { cx: pill.botCx, cy: pill.botCy, diameter: pill.diameter, opacity: 1 };
    case "expanded": {
      const layout = VIEW_LAYOUTS[view];
      if (view === "uploading") {
        return {
          cx: 36 + uploadProgress * 526,
          cy: layout.botY ?? 103,
          diameter: layout.botDiameter,
          opacity: 1,
        };
      }
      if (layout.botY != null) {
        return { cx: layout.botX, cy: layout.botY, diameter: layout.botDiameter, opacity: 1 };
      }
      // Centre of the fixed 84 pt card (8 pt top inset + 34 pt header → content at y = 42)
      const headerBottom = 42;
      const cardH = 84;
      const cy = headerBottom + (islandH - headerBottom - cardH) / 2 + cardH / 2;
      return { cx: layout.botX, cy, diameter: layout.botDiameter, opacity: 1 };
    }
  }
}

export function botGlowColor(s: BotStateName): string {
  switch (s) {
    case "working":
      return "#3B9EFF";
    // Thinking and searching glow in Glim's own amber (they were purple and
    // indigo, upstream's palette).
    case "thinking":
      return "#FFB347";
    case "searching":
      return "#FFB347";
    case "approval":
      return "#F5A524";
    case "error":
      return "#F4505E";
    case "finished":
      return "#34D399";
    case "ratelimit":
      return "#F59E0B";
    default:
      return "#FFFFFF";
  }
}

export function botGlowOpacity(s: BotStateName): number {
  switch (s) {
    case "idle":
    case "sleeping":
      return 0.15;
    case "dizzy":
      return 0;
    default:
      return 0.65;
  }
}

// Project colours (IslandConst.projectColors)
const PROJECT_COLORS: Record<string, string> = {
  korus: "#FF5A4E",
  "sbe hub": "#2EC4A0",
  "morning ai brief": "#F29B38",
  "publication ig": "#7C5CFF",
  "ig post": "#7C5CFF",
  "louisraille.fr": "#38BDF8",
  louisraille: "#38BDF8",
  "notch buddy": "#EC4899",
  "notch-buddy": "#EC4899",
  notchbuddy: "#EC4899",
};

const FALLBACK_COLORS = ["#22C55E", "#EAB308", "#60A5FA", "#E879F9"];

export function colorForProject(name: string): string {
  const key = name.toLowerCase().trim();
  const exact = PROJECT_COLORS[key];
  if (exact) return exact;
  for (const [k, c] of Object.entries(PROJECT_COLORS)) {
    if (key.startsWith(k) || key.includes(k)) return c;
  }
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0;
  return FALLBACK_COLORS[Math.abs(hash) % FALLBACK_COLORS.length];
}

// Card wash colours (CardBackground.washColor)
export type Wash = "red" | "green" | "pink" | "amber" | "cyan" | "ember" | "soft" | null;

export function washRGBA(wash: Wash): string {
  switch (wash) {
    case "red":
      return "rgba(244,80,94,0.55)";
    case "green":
      return "rgba(52,211,153,0.5)";
    case "pink":
      return "rgba(244,114,182,0.55)";
    case "amber":
      return "rgba(245,165,36,0.42)";
    case "cyan":
      return "rgba(34,211,238,0.38)";
    // Glim's warm amber (#FFB347) at low opacity; was upstream's indigo.
    case "ember":
      return "rgba(255,179,71,0.22)";
    case "soft":
      return "rgba(255,255,255,0.08)";
    default:
      return "rgba(0,0,0,0)";
  }
}

/** The overview's right card (the other pills, to switch to) only has a
 *  reason to be there when there is another pill. */
export function showsSwitcher(otherPills: number): boolean {
  return otherPills > 0;
}
