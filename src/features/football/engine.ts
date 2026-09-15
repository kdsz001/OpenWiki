import {
  BALL_HIT_SIZE,
  type FootballLayout,
  type FootballResult,
  type PointerInput,
  type SfxName,
} from "./types";
import { deform, drawBall, drawCountdownLine, drawGoalBack, drawGoalFrame, drawShadow, type Goal, type Net } from "./draw";

const GOAL = { w: 170, h: 104, margin: 28 };
const BALL_R = 18;
/**
 * Slingshot aiming: the ball follows the pull up to 100px (rubber band); releasing
 * under 14px puts it back. 35° off the goal direction reaches a post. Within 50°
 * the ball flies straight at the goal and always scores; beyond that it rolls for real.
 */
const AIM = {
  maxPull: 100,
  full: 92,
  dead: 14,
  dragStart: 10,
  spread: (35 * Math.PI) / 180,
  direct: (50 * Math.PI) / 180,
};
/**
 * Shots away from the goal roll for real: straight lines, mirror bounces off the work-area
 * edges (no nudging), slowing down all the way and losing speed at every bounce. After three
 * bounces the ball comes to rest at the next edge it reaches. Rolling into the goal scores;
 * stopping anywhere else is a miss and nothing is saved.
 */
const ROLL = {
  speed: 1500,
  speedPerPower: 1500,
  /** Rolling friction, px/s². */
  friction: 1400,
  /** Share of the speed kept after each bounce. */
  keep: 0.82,
  bounces: 3,
  /** Reaching two edges within this many px is one corner hit that flips both directions. */
  corner: 6,
  /** Rolling at this speed the ball spins and stretches fully; slower, less. */
  spin: 3000,
  /** Tuned on a 1440×813 work area. Other screens scale speed and friction together: same timing, same bounces. */
  screen: Math.hypot(1440, 813),
};

export interface Point {
  x: number;
  y: number;
}

export interface FootballTexts {
  cheers: string[];
  worldie: string;
  banana: string;
  rocket: string;
  tapIn: string;
  rebound: string;
  doubleRebound: string;
  tripleRebound: string;
  winner: string;
}

export interface FootballOptions {
  countdown: number;
  defaultAction: "save" | "dismiss";
  /** Goal size relative to the default 170x104 goal (the "large" setting is 1). */
  goalScale: number;
  /** The user has already pulled the ball once, so the hint is not needed. */
  learned: boolean;
  texts: FootballTexts;
  /** The ball is on screen: put the input window on it. */
  onBallReady: (x: number, y: number) => void;
  /** The ball was kicked into the goal: save right away (the animation is only decoration). */
  onShot: () => void;
  /** This ball will not be saved (kicked wide, or it expired): stop taking input. */
  onNoSave: () => void;
  onDone: (result: FootballResult) => void;
  onSfx: (name: SfxName, value?: number) => void;
  onLearned: () => void;
  onTip: (tip: Point | null) => void;
  onPop: (cheer: string, x: number, y: number) => void;
  onPopHide: () => void;
  /** The missed ball deflated here: show the "not saved" note above this point. */
  onMissNote: (x: number, y: number) => void;
  onMissNoteHide: () => void;
}

interface Path {
  p0: Point;
  c: Point;
  p2: Point;
  dur: number;
  /** From this share of the flight on, the ball is drawn behind the goal frame (default 0.86). */
  insideAt?: number;
}

/** A straight roll slowing down evenly from v0 to v1; n is the edge hit at its end (null: it stops, or reaches the goal). */
interface RollLeg {
  p0: Point;
  p2: Point;
  len: number;
  v0: number;
  v1: number;
  a: number;
  n: Point | null;
  dur: number;
}

/** A spot in the back of the net. */
interface Landing {
  P: Point;
  u: number;
  v: number;
}

/** The whole flight, decided at release: rolls off screen edges (if any), then into the net; no path when it misses. */
interface ShotPlan {
  k: number;
  u: number;
  v: number;
  legs: RollLeg[];
  path: Path | null;
  bounces: number;
}

/** Where the ball's center can go: the work area inset by the ball radius. */
interface Table {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
}

interface Ball {
  x: number;
  y: number;
  r: number;
  rot: number;
  sx: number;
  sy: number;
  a: number;
  vx: number;
  vy: number;
  dir: number;
}

interface Shot {
  u: number;
  v: number;
  k: number;
  power: number;
  aimed: boolean;
  /** Screen edges the ball bounced off on its way in (0 = straight at the goal). */
  bounces: number;
}

interface Drag {
  x: number;
  y: number;
  ux: number;
  uy: number;
  pull: number;
  power: number;
  valid: boolean;
  full: boolean;
  lastStep: number;
}

interface Press {
  sx: number;
  sy: number;
  offX: number;
  offY: number;
}

interface Particle {
  x: number;
  y: number;
  life: number;
  max: number;
  s: number;
  c: string;
  vx: number;
  vy: number;
  rot: number;
  vr: number;
}

interface Ring {
  x: number;
  y: number;
  r0: number;
  color: string;
  t: number;
}

/** A soft dust puff from a deflating ball: grows and fades, no gravity. */
interface Puff {
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
  life: number;
  max: number;
  c: string;
}

type Phase = "appear" | "wait" | "kick" | "fly" | "net" | "scored" | "leave" | "miss" | "expire";

interface Session {
  phase: Phase;
  t: number;
  remaining: number;
  hover: boolean;
  hoverA: number;
  trail: Array<Point & { r: number }>;
  fade: number;
  shake: number;
  rise: number;
  inside: boolean;
  lastSecond: boolean;
  drag: Drag | null;
  snap: { t: number; x: number; y: number } | null;
  legs: RollLeg[];
  /** The ball will come to rest without going in. */
  miss: boolean;
  /** The "not saved" note of a miss. */
  note: "none" | "shown" | "hidden";
  squash: { t: number; dir: number } | null;
  shot: Shot | null;
  autoShot: Shot;
  net: Net;
  goal: Goal;
  base: { x0: number; x1: number; y: number };
  rest: Point;
  r0: number;
  rEnd: number;
  path: Path;
  ball: Ball;
}

const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const mix = (a: Point, b: Point, t: number): Point => ({ x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t) });
const easeOutBack = (t: number) => {
  const c = 1.9;
  return 1 + (c + 1) * Math.pow(t - 1, 3) + c * Math.pow(t - 1, 2);
};
const bez = (p0: Point, c: Point, p2: Point, u: number): Point => ({
  x: (1 - u) * (1 - u) * p0.x + 2 * (1 - u) * u * c.x + u * u * p2.x,
  y: (1 - u) * (1 - u) * p0.y + 2 * (1 - u) * u * c.y + u * u * p2.y,
});
/** Angle to turn from direction f to direction d; positive = clockwise on screen. */
const signedAngle = (fx: number, fy: number, dx: number, dy: number) =>
  Math.atan2(fx * dy - fy * dx, fx * dx + fy * dy);

/** Control point on the launch direction: the ball leaves that way, then curls into P. */
function pathFrom(p0: Point, P: Point, power: number, d: Point): Path {
  const dist = Math.hypot(P.x - p0.x, P.y - p0.y) || 1;
  const ang0 = Math.atan2(P.y - p0.y, P.x - p0.x);
  let dev = Math.atan2(d.y, d.x) - ang0;
  dev = Math.atan2(Math.sin(dev), Math.cos(dev));
  const a = ang0 + clamp(dev, -1, 1);
  const L = dist * 0.5;
  const lift = dist * (0.05 + 0.16 * (1 - power));
  return {
    p0,
    p2: P,
    c: { x: p0.x + Math.cos(a) * L, y: p0.y + Math.sin(a) * L - lift },
    dur: clamp(dist / (1000 + 1600 * power) + 0.2, 0.3, 0.9),
  };
}

/** From p along the unit direction d: how far to the table edge, where, and which edge (both on a corner hit). */
function nextEdge(p: Point, d: Point, B: Table) {
  const tx = d.x > 1e-9 ? (B.x1 - p.x) / d.x : d.x < -1e-9 ? (B.x0 - p.x) / d.x : Infinity;
  const ty = d.y > 1e-9 ? (B.y1 - p.y) / d.y : d.y < -1e-9 ? (B.y0 - p.y) / d.y : Infinity;
  const t = Math.max(0, Math.min(tx, ty));
  const n = { x: tx - t < ROLL.corner ? (d.x > 0 ? -1 : 1) : 0, y: ty - t < ROLL.corner ? (d.y > 0 ? -1 : 1) : 0 };
  return { t, at: { x: p.x + d.x * t, y: p.y + d.y * t }, n };
}

/** Where segment a-b enters the box from outside (0–1), or -1 when it never does or already starts inside. */
function entryAlong(a: Point, b: Point, box: Table) {
  let t0 = 0;
  let t1 = 1;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const sides: Array<[number, number]> = [
    [-dx, a.x - box.x0],
    [dx, box.x1 - a.x],
    [-dy, a.y - box.y0],
    [dy, box.y1 - a.y],
  ];
  for (const [p, q] of sides) {
    if (Math.abs(p) < 1e-9) {
      if (q < 0) return -1;
      continue;
    }
    const t = q / p;
    if (p < 0) {
      if (t > t1) return -1;
      t0 = Math.max(t0, t);
    } else {
      if (t < t0) return -1;
      t1 = Math.min(t1, t);
    }
  }
  return t0 > 0 && t0 <= t1 ? t0 : -1;
}

/** A straight roll from p0 to p2, starting at speed v0 and slowing by a (px/s²). */
function rollLeg(p0: Point, p2: Point, v0: number, a: number, n: Point | null): RollLeg {
  const len = Math.hypot(p2.x - p0.x, p2.y - p0.y);
  const v1 = Math.sqrt(Math.max(0, v0 * v0 - 2 * a * len));
  return { p0, p2, len, v0, v1, a, n, dur: len > 1e-6 ? (2 * len) / (v0 + v1) : 0 };
}

/**
 * The football game drawn in the full-screen field window. Input arrives from the
 * ball window; saving, sounds and windows are handled by the callbacks.
 */
export class FootballEngine {
  private readonly canvas: HTMLCanvasElement;
  private readonly g: CanvasRenderingContext2D;
  private readonly layout: FootballLayout;
  private readonly opts: FootballOptions;
  private W = 0;
  private H = 0;
  private dpr = 1;
  private s: Session | null = null;
  private press: Press | null = null;
  private parts: Particle[] = [];
  private rings: Ring[] = [];
  private puffs: Puff[] = [];
  private freeze = 0;
  private raf = 0;
  private last = 0;
  private stopped = false;
  private learned: boolean;
  private saveKnown = false;
  /** Speed and friction scale for this screen size (see ROLL.screen). */
  private readonly rollK: number;

  constructor(canvas: HTMLCanvasElement, layout: FootballLayout, opts: FootballOptions) {
    const g = canvas.getContext("2d");
    if (!g) throw new Error("Canvas 2D is not available");
    this.canvas = canvas;
    this.g = g;
    this.layout = layout;
    this.opts = opts;
    this.learned = opts.learned;
    this.rollK = clamp(Math.hypot(layout.work.w, layout.work.h) / ROLL.screen, 0.75, 2);
  }

  start() {
    this.resize();
    const s = this.createSession();
    this.s = s;
    this.opts.onBallReady(s.rest.x, s.rest.y);
    this.last = performance.now();
    const loop = (now: number) => {
      if (this.stopped) return;
      const dt = Math.min(0.05, (now - this.last) / 1000);
      this.last = now;
      this.update(dt);
      if (this.stopped) return;
      this.render();
      this.raf = requestAnimationFrame(loop);
    };
    this.raf = requestAnimationFrame(loop);
  }

  destroy() {
    this.stopped = true;
    cancelAnimationFrame(this.raf);
  }

  /** The save finished (either way), so the celebration may end. */
  setSaveResult() {
    this.saveKnown = true;
  }

  /** A newer copy replaced the pending content: give the user the full countdown again. */
  restartCountdown() {
    const s = this.s;
    if (s && (s.phase === "appear" || s.phase === "wait")) s.remaining = this.opts.countdown;
  }

  /** Pointer input from the ball window (its coordinates are relative to that window). */
  input(ev: PointerInput) {
    const s = this.s;
    if (!s) return;
    const half = BALL_HIT_SIZE / 2;
    const x = ev.x + s.rest.x - half;
    const y = ev.y + s.rest.y - half;
    switch (ev.type) {
      case "down":
        if (s.phase !== "wait" && s.phase !== "appear") return;
        if (s.phase === "appear") {
          s.phase = "wait";
          s.t = 0;
          s.rise = 0;
        }
        this.press = { sx: x, sy: y, offX: s.rest.x - x, offY: s.rest.y - y };
        s.snap = null;
        return;
      case "move": {
        const p = this.press;
        if (!p || s.phase !== "wait") return;
        if (ev.buttons === 0) {
          this.endPress(false); // the button was released outside the window
          return;
        }
        if (!s.drag) {
          if (Math.hypot(x - p.sx, y - p.sy) < AIM.dragStart) return;
          s.drag = { x: s.rest.x, y: s.rest.y, ux: 0, uy: 1, pull: 0, power: 0, valid: false, full: false, lastStep: 0 };
          s.hover = false;
          this.opts.onTip(null);
          this.opts.onSfx("grab");
        }
        this.updateDrag(s, s.drag, x + p.offX, y + p.offY);
        return;
      }
      case "up":
        this.endPress(false);
        return;
      case "cancel":
        this.endPress(true);
        return;
      case "enter":
        s.hover = true;
        this.showTip(s);
        return;
      case "leave":
        s.hover = false;
        this.opts.onTip(null);
        return;
    }
  }

  key(key: string) {
    const s = this.s;
    if (!s || (s.phase !== "wait" && s.phase !== "appear")) return;
    if (key === "Escape") {
      if (s.drag) {
        this.press = null;
        this.cancelDrag(s); // the first Esc only cancels the aim
      } else {
        this.expire();
      }
    } else if (key === "Enter" && !s.drag) {
      this.kick();
    }
  }

  private resize() {
    this.dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.W = window.innerWidth;
    this.H = window.innerHeight;
    this.canvas.width = Math.round(this.W * this.dpr);
    this.canvas.height = Math.round(this.H * this.dpr);
    this.canvas.style.width = `${this.W}px`;
    this.canvas.style.height = `${this.H}px`;
  }

  private createSession(): Session {
    const L = this.layout;
    const w = L.work;
    const rand = Math.random();
    const rand2 = Math.random();
    const rand3 = Math.random();
    const scale = clamp(this.opts.goalScale || 1, 0.5, 2);
    const gw = GOAL.w * scale;
    const gh = GOAL.h * scale;
    const x0 = L.side === "left" ? w.x + GOAL.margin : w.x + w.w - GOAL.margin - gw;
    const x1 = x0 + gw;
    const y1 = w.y + w.h - GOAL.margin;
    const y0 = y1 - gh;
    const goal: Goal = {
      x0,
      y0,
      x1,
      y1,
      bx0: x0 + gw * 0.12,
      bx1: x1 - gw * 0.12,
      by0: y0 + gh * 0.26,
      by1: y1 - gh * 0.08,
    };
    // The ball appears just below-right of the cursor, inside the work area and clear of the goal.
    const cursor = L.cursor ?? { x: w.x + w.w * 0.4, y: w.y + w.h * 0.45 };
    let rx = clamp(cursor.x + 36, w.x + 40, w.x + w.w - 40);
    let ry = clamp(cursor.y + 34, w.y + 40, w.y + w.h - 40);
    if (ry > y0 - 100 && rx > x0 - 80 && rx < x1 + 80) {
      rx = L.side === "left" ? x1 + 110 : x0 - 110;
      ry = Math.min(ry, y0 - 60);
    }
    const rest = { x: rx, y: ry };
    // Automatic shot (click, Enter, or countdown with "save" as default): 40% go top corner.
    const top = rand2 < 0.4;
    const u = top ? (rand3 < 0.5 ? 0.18 : 0.82) + (rand - 0.5) * 0.08 : 0.3 + rand * 0.4;
    const v = top ? 0.2 + rand * 0.1 : 0.35 + rand3 * 0.35;
    const p2 = { x: lerp(goal.bx0, goal.bx1, u), y: lerp(goal.by0, goal.by1, v) };
    const dist = Math.hypot(p2.x - rest.x, p2.y - rest.y);
    const path: Path = {
      p0: { ...rest },
      p2,
      c: {
        x: lerp(rest.x, p2.x, 0.5),
        y: Math.min(rest.y, p2.y) - clamp(dist * 0.15, 30, 120) * (0.8 + rand * 0.4),
      },
      dur: clamp(dist / 1700 + 0.24, 0.4, 0.72),
    };
    return {
      phase: "appear",
      t: 0,
      remaining: this.opts.countdown,
      hover: false,
      hoverA: 0,
      trail: [],
      fade: 1,
      shake: 0,
      rise: 0,
      inside: false,
      lastSecond: false,
      drag: null,
      snap: null,
      legs: [],
      miss: false,
      note: "none",
      squash: null,
      shot: null,
      autoShot: { u, v, k: 0, power: 0.6, aimed: false, bounces: 0 },
      net: { cx: 0, cy: 0, amp: 0, v: 0, flash: 0, hold: 0, sigma: 30 * scale },
      goal,
      base: { x0: x0 + 3, x1: x1 - 3, y: y1 + 11 },
      rest,
      r0: BALL_R,
      rEnd: BALL_R * 0.56,
      path,
      ball: { x: rest.x, y: rest.y, r: BALL_R, rot: 0, sx: 1, sy: 1, a: 1, vx: 0, vy: 0, dir: 0 },
    };
  }

  private showTip(s: Session) {
    if (this.learned || s.drag || (s.phase !== "wait" && s.phase !== "appear")) return;
    this.opts.onTip({ x: s.rest.x, y: s.rest.y - s.r0 * 1.2 - 12 });
  }

  private endPress(cancelled: boolean) {
    if (!this.press) return;
    this.press = null;
    const s = this.s;
    if (!s || s.phase !== "wait") return;
    const D = s.drag;
    if (!D) {
      if (!cancelled) this.kick(); // no drag = a click = automatic shot
      return;
    }
    if (!cancelled && D.valid) this.launch(s, D);
    else this.cancelDrag(s);
  }

  private updateDrag(s: Session, D: Drag, px: number, py: number) {
    const R = s.rest;
    const vx = px - R.x;
    const vy = py - R.y;
    const len = Math.hypot(vx, vy);
    const pull = AIM.maxPull * Math.tanh(len / AIM.maxPull); // the further, the harder to pull
    const ux = len > 0.001 ? vx / len : -0.7;
    const uy = len > 0.001 ? vy / len : -0.7;
    D.x = R.x + ux * pull;
    D.y = R.y + uy * pull;
    D.ux = ux;
    D.uy = uy;
    D.pull = pull;
    D.power = clamp((pull - AIM.dead) / (AIM.full - AIM.dead), 0, 1);
    D.valid = pull >= AIM.dead; // every direction shoots; only "put back" cancels
    const step = D.valid ? Math.floor(D.power * 8 + 1e-6) : 0;
    if (step > D.lastStep) this.opts.onSfx(step >= 8 ? "full" : "tick", step);
    D.full = step >= 8;
    D.lastStep = step;
  }

  private cancelDrag(s: Session) {
    const D = s.drag;
    if (!D) return;
    s.drag = null;
    s.snap = { t: 0, x: D.x - s.rest.x, y: D.y - s.rest.y };
    this.opts.onSfx("boing");
  }

  /** Automatic shot from the resting ball into a random spot of the goal. */
  private kick(fromCountdown = false) {
    const s = this.s;
    if (!s || (s.phase !== "wait" && s.phase !== "appear")) return;
    s.lastSecond = !fromCountdown && s.remaining <= 1.05;
    s.shot = s.autoShot;
    s.drag = null;
    s.snap = null;
    s.legs = [];
    s.miss = false;
    s.squash = null;
    this.press = null;
    s.phase = "kick";
    s.t = 0;
    s.rise = 0;
    this.opts.onTip(null);
    const b = s.ball;
    b.x = s.rest.x;
    b.y = s.rest.y;
    b.r = s.r0;
    b.a = 1;
    b.dir = Math.atan2(s.path.c.y - s.rest.y, s.path.c.x - s.rest.x);
    this.opts.onSfx("kick", 1);
    this.ring(b.x, b.y, s.r0, "#F97316");
    this.burst(b.x, b.y + s.r0 * 0.7, 7, -1, ["#A8A29E", "#D6D3D1", "#FAFAF8"], 0.45);
    this.opts.onShot();
  }

  /** Release: shoot opposite to the pull. Whether it goes in is decided right here (and not shown). */
  private launch(s: Session, D: Drag) {
    const d = { x: -D.ux, y: -D.uy };
    const plan = this.planShot(s, { x: D.x, y: D.y }, d, D.power);
    s.shot = { k: plan.k, u: plan.u, v: plan.v, power: D.power, aimed: true, bounces: plan.bounces };
    if (plan.path) s.path = plan.path;
    s.legs = plan.legs;
    s.miss = !plan.path;
    s.squash = null;
    s.drag = null;
    s.snap = null;
    if (!this.learned) {
      this.learned = true;
      this.opts.onLearned();
    }
    s.lastSecond = s.remaining <= 1.05;
    s.phase = "kick";
    s.t = 0;
    s.rise = 0;
    this.opts.onTip(null);
    const b = s.ball;
    b.x = D.x;
    b.y = D.y;
    b.r = s.r0;
    b.a = 1;
    b.sx = 1;
    b.sy = 1;
    b.dir = Math.atan2(d.y, d.x);
    this.opts.onSfx("twang", D.power);
    this.opts.onSfx("kick", 0.75 + 0.35 * D.power);
    this.ring(b.x, b.y, s.r0 * (1 + 0.4 * D.power), "#F97316");
    this.burst(b.x, b.y + s.r0 * 0.7, 6 + Math.round(6 * D.power), -1, ["#A8A29E", "#D6D3D1", "#FAFAF8"], 0.45 + 0.3 * D.power);
    if (s.miss) this.opts.onNoSave();
    else this.opts.onShot();
  }

  /** Left/right follows the aim (k), height follows the power — like a FIFA power bar. */
  private landing(s: Session, k: number, power: number): Landing {
    const G = s.goal;
    const hh = (G.by1 - G.by0) / 2;
    const x = clamp(lerp(G.bx0, G.bx1, 0.5 - k / 2), G.bx0 + s.rEnd + 2, G.bx1 - s.rEnd - 2);
    const y = clamp((G.by0 + G.by1) / 2 + lerp(0.55, -0.75, power) * hh, G.by0 + s.rEnd + 1, G.by1 - s.rEnd - 1);
    return { P: { x, y }, u: (x - G.bx0) / (G.bx1 - G.bx0), v: (y - G.by0) / (G.by1 - G.by0) };
  }

  private table(s: Session): Table {
    const w = this.layout.work;
    const m = s.r0;
    return { x0: w.x + m, x1: w.x + w.w - m, y0: w.y + m, y1: w.y + w.h - m };
  }

  /**
   * The ball's center rolling in here scores. Where the goal stands too close to the screen edge
   * for the ball to pass, the area reaches that edge, so the ball never squeezes past a post.
   */
  private mouth(s: Session): Table {
    const G = s.goal;
    const B = this.table(s);
    const w = this.layout.work;
    const tight = 2 * s.r0;
    return {
      x0: G.x0 - w.x < tight ? B.x0 : G.x0 + 8,
      x1: w.x + w.w - G.x1 < tight ? B.x1 : G.x1 - 8,
      y0: G.y0 + 8,
      y1: w.y + w.h - G.y1 < tight ? B.y1 : G.y1 - 2,
    };
  }

  /** The whole flight is decided at release: roughly at the goal always scores, anything else rolls for real. */
  private planShot(s: Session, p0: Point, d: Point, power: number): ShotPlan {
    const G = s.goal;
    const T = { x: (G.bx0 + G.bx1) / 2, y: (G.by0 + G.by1) / 2 };
    let tx = T.x - p0.x;
    let ty = T.y - p0.y;
    const tl = Math.hypot(tx, ty) || 1;
    tx /= tl;
    ty /= tl;
    const delta = signedAngle(tx, ty, d.x, d.y);
    if (Math.abs(delta) <= AIM.direct) {
      // Roughly at the goal: straight in (a big angle curls into a banana kick)
      const k = clamp(delta / AIM.spread, -1, 1);
      const L = this.landing(s, k, power);
      return { k, u: L.u, v: L.v, legs: [], path: pathFrom(p0, L.P, power, d), bounces: 0 };
    }
    return this.rollShot(s, p0, d, power);
  }

  /** Rolls off at most three edges. Into the goal mouth it scores; coming to rest anywhere else misses (no path). */
  private rollShot(s: Session, p0: Point, d: Point, power: number): ShotPlan {
    const B = this.table(s);
    const M = this.mouth(s);
    const a = ROLL.friction * this.rollK;
    const legs: RollLeg[] = [];
    let p = { x: clamp(p0.x, B.x0, B.x1), y: clamp(p0.y, B.y0, B.y1) };
    let dir = d;
    let speed = (ROLL.speed + ROLL.speedPerPower * power) * this.rollK;
    for (let hits = 0; ; hits++) {
      const edge = nextEdge(p, dir, B);
      const stop = (speed * speed) / (2 * a);
      const len = Math.min(edge.t, stop);
      const end = { x: p.x + dir.x * len, y: p.y + dir.y * len };
      const enter = entryAlong(p, end, M);
      if (enter >= 0) {
        // Rolled into the goal
        const E = mix(p, end, enter);
        const leg = rollLeg(p, E, speed, a, null);
        legs.push(leg);
        return { ...this.diveInto(s, E, dir, leg.v1, a), legs, bounces: hits };
      }
      if (stop <= edge.t) {
        // Comes to rest before the next edge
        legs.push(rollLeg(p, end, speed, a, null));
        return { k: 0, u: 0.5, v: 0.5, legs, path: null, bounces: hits };
      }
      if (hits >= ROLL.bounces) {
        // Three bounces already: slows to a stop right at this edge instead of bouncing a fourth time
        legs.push(rollLeg(p, edge.at, speed, (speed * speed) / (2 * Math.max(1, edge.t)), null));
        return { k: 0, u: 0.5, v: 0.5, legs, path: null, bounces: hits };
      }
      const leg = rollLeg(p, edge.at, speed, a, edge.n);
      legs.push(leg);
      speed = leg.v1 * ROLL.keep;
      dir = { x: edge.n.x ? -dir.x : dir.x, y: edge.n.y ? -dir.y : dir.y }; // angle in = angle out
      p = edge.at;
    }
  }

  /** Rolled in: carries on the same way into the net, shrinking as it goes deeper (faster = deeper). */
  private diveInto(s: Session, E: Point, dir: Point, speed: number, friction: number) {
    const G = s.goal;
    const reach = clamp((speed * speed) / (2 * friction), 24, 70);
    const P = {
      x: clamp(E.x + dir.x * reach, G.bx0 + s.rEnd + 2, G.bx1 - s.rEnd - 2),
      y: clamp(E.y + dir.y * reach, G.by0 + s.rEnd + 1, G.by1 - s.rEnd - 1),
    };
    const len = Math.hypot(P.x - E.x, P.y - E.y);
    const dur = clamp((1.5 * len) / Math.max(1, speed), 0.2, 0.45);
    const path: Path = { p0: E, c: mix(E, P, 0.5), p2: P, dur, insideAt: 0 };
    return { k: 0, u: (P.x - G.bx0) / (G.bx1 - G.bx0), v: (P.y - G.by0) / (G.by1 - G.by0), path };
  }

  /** Came to rest outside the goal: the miss animation plays, then the capture is dismissed. */
  private startMiss(s: Session) {
    s.phase = "miss";
    s.t = 0;
    s.squash = null;
    s.inside = false;
  }

  private expire() {
    const s = this.s;
    if (!s || (s.phase !== "wait" && s.phase !== "appear")) return;
    s.phase = "expire";
    s.t = 0;
    s.drag = null;
    s.snap = null;
    this.press = null;
    this.opts.onTip(null);
    this.opts.onNoSave();
  }

  private finish(result: FootballResult) {
    this.s = null;
    this.stopped = true;
    cancelAnimationFrame(this.raf);
    this.g.setTransform(1, 0, 0, 1, 0, 0);
    this.g.clearRect(0, 0, this.canvas.width, this.canvas.height);
    this.opts.onDone(result);
  }

  private shotLabel(s: Session) {
    const T = this.opts.texts;
    if (s.lastSecond) return T.winner;
    const sh = s.shot;
    if (sh) {
      if (sh.bounces >= 3) return T.tripleRebound;
      if (sh.bounces === 2) return T.doubleRebound;
      if (sh.bounces === 1) return T.rebound;
      if (sh.v < 0.32 && Math.abs(sh.u - 0.5) > 0.28) return T.worldie;
      if (sh.aimed && Math.abs(sh.k) > 0.65) return T.banana;
      if (sh.aimed && sh.power > 0.9) return T.rocket;
      if (sh.aimed && sh.power < 0.25) return T.tapIn;
    }
    return T.cheers[Math.floor(Math.random() * T.cheers.length)] ?? T.cheers[0];
  }

  /** The ball reaches the back of the net: hit-stop, pocket, flash, confetti, shake, cheer. */
  private hitNet(s: Session) {
    const N = s.net;
    const b = s.ball;
    const power = s.shot ? s.shot.power : 0.6;
    const inward = this.layout.side === "left" ? 1 : -1;
    N.cx = s.path.p2.x;
    N.cy = s.path.p2.y;
    N.v += 10 + 6 * power;
    N.flash = 1;
    N.hold = 0.14;
    b.vx = (Math.random() - 0.5) * 70;
    b.vy = 0;
    this.freeze = 0.06 + 0.04 * power;
    s.shake = 0.3;
    this.ring(N.cx, N.cy, 12 + 8 * power, "#F97316");
    this.burst(N.cx, N.cy, 16 + Math.round(12 * power), inward, ["#F97316", "#FDBA74", "#FAFAF8", "#FACC15"], 0.9 + 0.4 * power);
    this.burst(N.cx, N.cy, 10, -inward, ["#F97316", "#FAFAF8"], 0.7);
    s.phase = "scored";
    s.t = 0;
    this.opts.onSfx("goal", power);
    const G = s.goal;
    this.opts.onPop(this.shotLabel(s), clamp((G.x0 + G.x1) / 2, 100, this.W - 100), G.y0 - 16);
  }

  /** In the net: held in the pocket for a moment, then drops to the bottom and bounces. */
  private stepBallInNet(s: Session, dt: number) {
    const b = s.ball;
    const G = s.goal;
    const N = s.net;
    if (N.hold > 0) {
      N.hold -= dt;
      const p = deform({ x: N.cx, y: N.cy }, N);
      b.x = p.x;
      b.y = p.y;
      b.rot += 4 * dt;
      return;
    }
    b.vy += 1400 * dt;
    b.x += b.vx * dt;
    b.y += b.vy * dt;
    b.rot += (b.vx / Math.max(4, b.r)) * dt;
    const floor = G.by1 - b.r * 0.85;
    if (b.y > floor) {
      b.y = floor;
      b.vy = b.vy > 60 ? -b.vy * 0.35 : 0;
      b.vx *= Math.pow(0.1, dt);
    }
    b.x = clamp(b.x, G.bx0 + b.r, G.bx1 - b.r);
  }

  /** Right after an edge hit the ball is squashed against it for a moment. */
  private stepSquash(s: Session, dt: number) {
    const sq = s.squash;
    if (!sq) return;
    const b = s.ball;
    sq.t += dt;
    const k = clamp(sq.t / 0.09, 0, 1);
    const q = Math.sin(Math.PI * k);
    b.dir = sq.dir;
    b.sx = 1 - 0.28 * q;
    b.sy = 1 + 0.2 * q;
    if (k >= 1) s.squash = null;
  }

  private stepFlight(s: Session, dt: number) {
    const b = s.ball;
    s.trail.push({ x: b.x, y: b.y, r: b.r });
    if (s.trail.length > 9) s.trail.shift();

    if (s.legs.length) {
      // Rolling: straight along the leg and slowing down, to the next edge, into the goal, or to a stop
      const q = s.legs[0];
      const t = Math.min(s.t, q.dur);
      const speed = Math.max(0, q.v0 - q.a * t);
      const p = q.len > 1e-6 ? mix(q.p0, q.p2, Math.min(1, (q.v0 * t - 0.5 * q.a * t * t) / q.len)) : q.p2;
      if (Math.hypot(p.x - b.x, p.y - b.y) > 0.01) b.dir = Math.atan2(p.y - b.y, p.x - b.x);
      const kv = clamp(speed / (ROLL.spin * this.rollK), 0, 1);
      b.x = p.x;
      b.y = p.y;
      b.r = s.r0;
      b.sx = 1 + 0.14 * kv;
      b.sy = 1 / b.sx;
      b.rot += dt * 22 * kv;
      this.stepSquash(s, dt); // still squashed from the previous edge
      if (s.t >= q.dur) {
        s.legs.shift();
        s.t = 0;
        if (q.n) {
          // Edge hit: squash, thud, spray, then bounce away; harder hits are louder and bigger
          const hard = clamp(q.v1 / (2500 * this.rollK), 0.12, 1);
          if (hard > 0.4) this.freeze = 0.035;
          s.squash = { t: 0, dir: Math.atan2(q.n.y, q.n.x) };
          this.opts.onSfx("wall", hard);
          this.ring(b.x, b.y, s.r0 * (0.6 + 0.3 * hard), "#FAFAF8");
          this.burstToward(b.x, b.y, q.n.x, q.n.y, Math.round(3 + 7 * hard), ["#F97316", "#FDBA74", "#FAFAF8"], 0.35 + 0.45 * hard);
        }
        if (!s.legs.length && s.miss) this.startMiss(s);
      }
      return;
    }

    b.rot += dt * 22;
    const { p0, c, p2, dur } = s.path;
    const t = clamp(s.t / dur, 0, 1);
    const u = 0.5 * t + 0.5 * (1 - (1 - t) * (1 - t));
    const p = bez(p0, c, p2, u);
    if (Math.hypot(p.x - b.x, p.y - b.y) > 0.01) b.dir = Math.atan2(p.y - b.y, p.x - b.x);
    b.x = p.x;
    b.y = p.y;
    b.r = lerp(s.r0, s.rEnd, u);
    const stretch = 1 + 0.14 * Math.sin(Math.PI * Math.min(1, t * 2.5));
    b.sx = stretch;
    b.sy = 1 / stretch;
    this.stepSquash(s, dt);
    // Near the end the ball is drawn behind the frame, so it looks like it went in (a ball rolling in: right away).
    s.inside = u >= (s.path.insideAt ?? 0.86);
    if (t >= 1) {
      b.sx = b.sy = 1;
      s.phase = "net";
      s.t = 0;
      this.hitNet(s);
    }
  }

  private update(dt: number) {
    if (this.freeze > 0) {
      this.freeze -= dt; // hit-stop
      return;
    }
    this.stepFx(dt);
    const s = this.s;
    if (!s) return;
    s.t += dt;
    const N = s.net;
    N.v += (-210 * N.amp - 9 * N.v) * dt;
    N.amp += N.v * dt;
    N.flash = Math.max(0, N.flash - dt / 0.2);
    s.shake = Math.max(0, s.shake - dt);
    s.hoverA = lerp(s.hoverA, s.hover && s.phase === "wait" ? 1 : 0, 1 - Math.exp(-dt * 14));
    const b = s.ball;

    if ((s.phase === "appear" || s.phase === "wait") && !s.drag) {
      s.remaining -= dt; // paused while aiming
      if (s.remaining <= 0) {
        if (this.opts.defaultAction === "save") this.kick(true);
        else this.expire();
        return;
      }
    }

    switch (s.phase) {
      case "appear": {
        const k = clamp(s.t / 0.32, 0, 1);
        b.x = s.rest.x;
        b.y = s.rest.y;
        b.r = s.r0 * Math.max(0.001, easeOutBack(k));
        s.rise = (1 - easeOutBack(k)) * 24;
        if (k >= 1) {
          s.phase = "wait";
          s.t = 0;
          s.rise = 0;
        }
        break;
      }
      case "wait": {
        b.a = 1;
        if (s.drag) {
          const D = s.drag;
          const jitter = D.full ? 1.6 : 0; // trembles at full draw
          b.x = D.x + (Math.random() - 0.5) * jitter;
          b.y = D.y + (Math.random() - 0.5) * jitter;
          b.r = s.r0;
          b.dir = Math.atan2(D.uy, D.ux);
          b.sx = 1 + 0.14 * D.power;
          b.sy = 1 / b.sx;
        } else if (s.snap) {
          const q = s.snap;
          q.t += dt;
          const e = Math.exp(-7 * q.t) * Math.cos(20 * q.t);
          b.x = s.rest.x + q.x * e;
          b.y = s.rest.y + q.y * e;
          b.r = s.r0;
          b.sx = b.sy = 1;
          if (q.t > 0.6) s.snap = null;
        } else {
          b.r = s.r0 * (1 + 0.07 * s.hoverA);
          b.x = s.rest.x;
          b.y = s.rest.y;
          b.sx = b.sy = 1;
        }
        break;
      }
      case "kick": {
        const k = clamp(s.t / 0.07, 0, 1);
        const q = Math.sin(Math.PI * k);
        b.sx = 1 - 0.2 * q;
        b.sy = 1 + 0.24 * q;
        if (k >= 1) {
          s.phase = "fly";
          s.t = 0;
          b.sx = b.sy = 1;
          this.opts.onSfx("whoosh", Math.min(0.8, (s.miss ? 0 : s.path.dur) + s.legs.reduce((sum, q) => sum + q.dur, 0)));
        }
        break;
      }
      case "fly":
        this.stepFlight(s, dt);
        break;
      case "net":
      case "scored":
      case "leave": {
        if (s.trail.length) s.trail.shift();
        this.stepBallInNet(s, dt);
        // Keep celebrating until the save result is in (at most 4s).
        if (s.phase === "scored" && ((s.t > 1.6 && this.saveKnown) || s.t > 4)) {
          s.phase = "leave";
          s.t = 0;
          this.opts.onPopHide();
        }
        if (s.phase === "leave") {
          s.fade = 1 - clamp(s.t / 0.25, 0, 1);
          if (s.t > 0.28) this.finish("scored");
        }
        break;
      }
      case "miss": {
        // Deflates, shrinks away in a small puff with a soft "poof", shows "not saved", fades out
        if (s.trail.length) s.trail.shift();
        if (s.t < 0.16) {
          const k = s.t / 0.16;
          b.dir = 0;
          b.sx = 1 + 0.18 * k;
          b.sy = 1 - 0.24 * k;
        } else {
          const k = clamp((s.t - 0.16) / 0.22, 0, 1);
          b.r = s.r0 * (1 - k * k);
          b.rot += dt * 4;
        }
        if (s.note === "none" && s.t >= 0.22) {
          s.note = "shown";
          this.opts.onSfx("poof");
          this.puff(b.x, b.y, s.r0);
          const w = this.layout.work;
          this.opts.onMissNote(clamp(b.x, w.x + 70, w.x + w.w - 70), Math.max(w.y + 40, b.y - s.r0 - 6));
        }
        if (s.note === "shown" && s.t > 1.35) {
          s.note = "hidden";
          this.opts.onMissNoteHide();
        }
        s.fade = 1 - clamp((s.t - 1.15) / 0.35, 0, 1);
        if (s.t > 1.65) this.finish("dismissed");
        break;
      }
      case "expire": {
        b.r = s.r0 * (1 - clamp(s.t / 0.25, 0, 1));
        s.rise = clamp(s.t / 0.35, 0, 1) * 12;
        s.fade = 1 - clamp((s.t - 0.1) / 0.3, 0, 1);
        if (s.t > 0.45) this.finish("dismissed");
        break;
      }
    }
  }

  private render() {
    const g = this.g;
    g.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    g.clearRect(0, 0, this.W, this.H);
    const s = this.s;
    if (s) {
      const k = s.shake > 0 ? s.shake / 0.3 : 0;
      const ox = k ? Math.sin(s.shake * 90) * 4.5 * k : 0;
      const oy = (k ? Math.cos(s.shake * 70) * 1.6 * k : 0) + s.rise;
      const b = s.ball;
      const inPlay =
        s.phase === "appear" ||
        s.phase === "wait" ||
        s.phase === "kick" ||
        s.phase === "fly" ||
        s.phase === "miss" ||
        s.phase === "expire";
      const ballBehind = s.inside || !inPlay;
      g.save();
      g.globalAlpha = s.fade * (s.phase === "appear" ? clamp(s.t / 0.25, 0, 1) : 1);
      drawGoalBack(g, s.goal, s.net, ox, oy);
      if (s.phase === "appear" || s.phase === "wait" || s.phase === "kick") {
        drawShadow(g, b.x, b.y + s.r0 + 3, b.r, b.a);
      }
      this.drawTrail(s);
      if (ballBehind) drawBall(g, b);
      drawGoalFrame(g, s.goal, ox, oy);
      if (!ballBehind) drawBall(g, b);
      if (s.phase === "appear" || s.phase === "wait" || s.phase === "kick" || s.phase === "fly") {
        drawCountdownLine(g, s.base, clamp(s.remaining / this.opts.countdown, 0, 1), ox, oy);
      }
      g.restore();
    }
    this.drawFx();
  }

  private drawTrail(s: Session) {
    const g = this.g;
    s.trail.forEach((p, i) => {
      const k = (i + 1) / s.trail.length;
      g.save();
      g.globalAlpha *= 0.22 * k;
      g.fillStyle = "#F97316";
      g.beginPath();
      g.arc(p.x, p.y, p.r * (0.5 + 0.5 * k), 0, Math.PI * 2);
      g.fill();
      g.restore();
    });
  }

  private burst(x: number, y: number, n: number, dir: number, colors: string[], spread = 1) {
    for (let i = 0; i < n; i++) {
      this.parts.push({
        x,
        y,
        life: 0,
        max: 0.45 + Math.random() * 0.35,
        s: 1.6 + Math.random() * 2,
        c: colors[i % colors.length],
        vx: (dir * (60 + Math.random() * 220) + (Math.random() - 0.5) * 80) * spread,
        vy: -(80 + Math.random() * 260) * spread,
        rot: Math.random() * 6,
        vr: (Math.random() - 0.5) * 20,
      });
    }
  }

  /** Spray pointing along (nx, ny), e.g. away from the edge that was hit. */
  private burstToward(x: number, y: number, nx: number, ny: number, n: number, colors: string[], speed = 1) {
    const base = Math.atan2(ny, nx);
    for (let i = 0; i < n; i++) {
      const a = base + (Math.random() - 0.5) * 1.8;
      const v = (120 + Math.random() * 260) * speed;
      this.parts.push({
        x,
        y,
        life: 0,
        max: 0.35 + Math.random() * 0.3,
        s: 1.4 + Math.random() * 1.8,
        c: colors[i % colors.length],
        vx: Math.cos(a) * v,
        vy: Math.sin(a) * v,
        rot: Math.random() * 6,
        vr: (Math.random() - 0.5) * 20,
      });
    }
  }

  private ring(x: number, y: number, r0: number, color: string) {
    this.rings.push({ x, y, r0, color, t: 0 });
  }

  /** A soft gray puff around a deflating ball. */
  private puff(x: number, y: number, r: number) {
    const colors = ["#A8A29E", "#C9C4BC", "#D6D3D1"];
    for (let i = 0; i < 10; i++) {
      const a = (i / 10) * Math.PI * 2 + Math.random() * 0.5;
      const v = 50 + Math.random() * 70;
      this.puffs.push({
        x: x + Math.cos(a) * r * 0.3,
        y: y + Math.sin(a) * r * 0.3,
        vx: Math.cos(a) * v,
        vy: Math.sin(a) * v - 25,
        r: r * (0.22 + Math.random() * 0.16),
        life: 0,
        max: 0.45 + Math.random() * 0.25,
        c: colors[i % colors.length],
      });
    }
  }

  private stepFx(dt: number) {
    for (let i = this.parts.length - 1; i >= 0; i--) {
      const p = this.parts[i];
      p.life += dt;
      if (p.life >= p.max) {
        this.parts.splice(i, 1);
        continue;
      }
      p.vy += 900 * dt;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.rot += p.vr * dt;
    }
    for (let i = this.rings.length - 1; i >= 0; i--) {
      this.rings[i].t += dt;
      if (this.rings[i].t > 0.22) this.rings.splice(i, 1);
    }
    for (let i = this.puffs.length - 1; i >= 0; i--) {
      const p = this.puffs[i];
      p.life += dt;
      if (p.life >= p.max) {
        this.puffs.splice(i, 1);
        continue;
      }
      const drag = Math.pow(0.03, dt);
      p.vx *= drag;
      p.vy *= drag;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.r += 16 * dt;
    }
  }

  private drawFx() {
    const g = this.g;
    for (const q of this.rings) {
      const k = q.t / 0.22;
      g.save();
      g.globalAlpha = (1 - k) * 0.85;
      g.strokeStyle = q.color;
      g.lineWidth = 2.5 * (1 - k) + 0.5;
      g.beginPath();
      g.arc(q.x, q.y, q.r0 * (1 + 1.3 * k), 0, Math.PI * 2);
      g.stroke();
      g.restore();
    }
    for (const p of this.puffs) {
      g.save();
      g.globalAlpha = 0.55 * (1 - p.life / p.max);
      g.fillStyle = p.c;
      g.beginPath();
      g.arc(p.x, p.y, p.r, 0, Math.PI * 2);
      g.fill();
      g.restore();
    }
    for (const p of this.parts) {
      g.save();
      g.globalAlpha = 1 - p.life / p.max;
      g.translate(p.x, p.y);
      g.rotate(p.rot);
      g.fillStyle = p.c;
      g.fillRect(-p.s, -p.s * 0.6, p.s * 2, p.s * 1.2);
      g.restore();
    }
  }
}
