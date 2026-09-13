/** Screen layout from Rust (`get_football_layout`), in the field window's logical coordinates. */
export interface FootballLayout {
  width: number;
  height: number;
  /** Monitor origin in global logical coordinates (used to place native windows). */
  originX: number;
  originY: number;
  /** Usable area without menu bar / Dock / taskbar. */
  work: { x: number; y: number; w: number; h: number };
  cursor: { x: number; y: number } | null;
  side: "left" | "right";
}

export type PointerKind = "down" | "move" | "up" | "cancel" | "enter" | "leave";

/** Pointer event forwarded from the ball window, in the ball window's own coordinates. */
export interface PointerInput {
  type: PointerKind;
  x: number;
  y: number;
  buttons: number;
}

export type SfxName = "grab" | "tick" | "full" | "twang" | "kick" | "whoosh" | "wall" | "goal" | "boing";

export interface SfxCue {
  name: SfxName;
  value?: number;
}

export type FootballResult = "scored" | "dismissed";

export const FIELD_LABEL = "football-field";
export const BALL_LABEL = "bubble";
/** The ball window is this big and centered on the resting ball. */
export const BALL_HIT_SIZE = 56;

/** Events between the ball window (input) and the field window (drawing). */
export const FOOTBALL_EVENTS = {
  /** ball → field: pointer input */
  input: "football:input",
  /** ball → field: Enter / Escape */
  key: "football:key",
  /** ball → field: a newer copy replaced the pending capture */
  restart: "football:restart",
  /** ball → field: saved (payload: today's goal count) */
  saved: "football:saved",
  /** ball → field: saving failed */
  failed: "football:failed",
  /** field → ball: the ball was kicked, save now */
  shot: "football:shot",
  /** field → ball: animation finished (payload: FootballResult) */
  done: "football:done",
  /** field → ball: play a sound (only the ball window gets the user gesture audio needs) */
  sfx: "football:sfx",
} as const;
