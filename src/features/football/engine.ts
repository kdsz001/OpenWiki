import {
  BALL_HIT_SIZE,
  type FootballLayout,
  type FootballResult,
  type PointerInput,
  type SfxName,
} from "./types";

const GOAL = { w: 170, h: 104, margin: 28 };
const BALL_R = 18;
/**
 * Slingshot aiming: the ball follows the pull up to 100px (rubber band); releasing
 * under 14px puts it back. 35° off the goal direction reaches a post. Within 50°
 * the ball flies straight at the goal; beyond that it rebounds off the screen edge.
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
 * Bank shots (facing away from the goal): the ball leaves (almost) the way it was aimed and
 * bounces off 1–2 screen edges into the net. Each bounce may be nudged a little so the route
 * ends in the goal, and the route with the smallest nudge wins. When every route needs too
 * much, the shot falls back to one bounce plus a curl.
 */
const DEG = Math.PI / 180;
const BANK = {
  /** Largest nudge per bounce. Turning the launch counts 2.5x, so it turns at most 6°. */
  bend: 16 * DEG,
  launchWeight: 2.5,
  /** Leave an edge at 12° or more instead of sliding along it. */
  graze: Math.sin(12 * DEG),
  /** At equal nudges one bounce wins. */
  twoBias: 1.5 * DEG,
  /** Hits this close to a corner look odd. */
  corner: 14,
  minLastLeg: 90,
  speed: 1500,
  speedPerPower: 1500,
  /** Share of the speed kept after each bounce. */
  keep: 0.85,
  maxDur: 1.5,
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
  /** The ball was kicked: save right away (the animation is only decoration). */
  onShot: () => void;
  onDone: (result: FootballResult) => void;
  onSfx: (name: SfxName, value?: number) => void;
  onLearned: () => void;
  onTip: (tip: Point | null) => void;
  onPop: (cheer: string, x: number, y: number) => void;
  onPopHide: () => void;
}

interface Path {
  p0: Point;
  c: Point;
  p2: Point;
  dur: number;
}

interface WallPath extends Path {
  n: Point;
}

/** A spot in the back of the net. */
interface Landing {
  P: Point;
  u: number;
  v: number;
}

/** The whole flight, decided at release: straight legs to screen edges (if any), then into the net. */
interface ShotPlan {
  k: number;
  u: number;
  v: number;
  pres: WallPath[];
  path: Path;
}

/** Where the ball's center can go: the work area inset by the ball radius. */
interface Table {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
}

interface EdgeHit {
  at: Point;
  n: Point;
  t: number;
  corner: boolean;
}

interface Goal {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  bx0: number;
  bx1: number;
  by0: number;
  by1: number;
}

interface Net {
  cx: number;
  cy: number;
  amp: number;
  v: number;
  flash: number;
  hold: number;
  sigma: number;
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

type Phase = "appear" | "wait" | "kick" | "fly" | "net" | "scored" | "leave" | "expire";

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
  pres: WallPath[];
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
const dot = (a: Point, b: Point) => a.x * b.x + a.y * b.y;
const rotate = (d: Point, a: number): Point => ({
  x: d.x * Math.cos(a) - d.y * Math.sin(a),
  y: d.x * Math.sin(a) + d.y * Math.cos(a),
});
/** Direction d after bouncing off an edge with normal n (angle in = angle out). */
const reflect = (d: Point, n: Point): Point => {
  const k = dot(d, n);
  return { x: d.x - 2 * k * n.x, y: d.y - 2 * k * n.y };
};
/** Unit direction from a to b, plus the distance. */
const toward = (a: Point, b: Point) => {
  const l = Math.hypot(b.x - a.x, b.y - a.y) || 1;
  return { x: (b.x - a.x) / l, y: (b.y - a.y) / l, l };
};

let ballTexture: HTMLCanvasElement | null = null;

/** Classic black-and-white ball, drawn once and then scaled (no emoji, per DESIGN.md). */
function getBallTexture(): HTMLCanvasElement {
  if (ballTexture) return ballTexture;
  const R = 64;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = R * 2 + 4;
  ballTexture = canvas;
  const x = canvas.getContext("2d");
  if (!x) return canvas;
  x.translate(R + 2, R + 2);
  const vtx = (cx: number, cy: number, rr: number, rot: number, i: number): [number, number] => {
    const a = rot + (i * Math.PI * 2) / 5;
    return [cx + rr * Math.cos(a), cy + rr * Math.sin(a)];
  };
  const pent = (cx: number, cy: number, rr: number, rot: number) => {
    x.beginPath();
    for (let i = 0; i < 5; i++) x.lineTo(...vtx(cx, cy, rr, rot, i));
    x.closePath();
    x.fill();
  };
  x.beginPath();
  x.arc(0, 0, R, 0, Math.PI * 2);
  x.fillStyle = "#FAFAF8";
  x.fill();
  x.save();
  x.clip();
  x.fillStyle = "#1C1917";
  x.strokeStyle = "#1C1917";
  x.lineWidth = R * 0.05;
  x.lineCap = "round";
  const pr = R * 0.3;
  const outer: Array<{ cx: number; cy: number; rot: number }> = [];
  pent(0, 0, pr, -Math.PI / 2);
  for (let i = 0; i < 5; i++) {
    const a = -Math.PI / 2 + (i * Math.PI * 2) / 5;
    const o = { cx: Math.cos(a) * R * 0.95, cy: Math.sin(a) * R * 0.95, rot: a + Math.PI };
    pent(o.cx, o.cy, pr, o.rot);
    outer.push(o);
    const [sx, sy] = vtx(0, 0, pr, -Math.PI / 2, i);
    const [ex, ey] = vtx(o.cx, o.cy, pr, o.rot, 0);
    x.beginPath();
    x.moveTo(sx, sy);
    x.lineTo(ex, ey);
    x.stroke();
  }
  const nearest = (p: { cx: number; cy: number; rot: number }, q: { cx: number; cy: number }) =>
    [1, 4]
      .map((k) => vtx(p.cx, p.cy, pr, p.rot, k))
      .sort((m, n) => Math.hypot(m[0] - q.cx, m[1] - q.cy) - Math.hypot(n[0] - q.cx, n[1] - q.cy))[0];
  for (let i = 0; i < 5; i++) {
    const A = outer[i];
    const B = outer[(i + 1) % 5];
    const [ax, ay] = nearest(A, B);
    const [bx, by] = nearest(B, A);
    x.beginPath();
    x.moveTo(ax, ay);
    x.lineTo(bx, by);
    x.stroke();
  }
  const shade = x.createRadialGradient(-R * 0.35, -R * 0.4, R * 0.08, 0, 0, R * 1.05);
  shade.addColorStop(0, "rgba(255,255,255,0.35)");
  shade.addColorStop(0.5, "rgba(255,255,255,0)");
  shade.addColorStop(1, "rgba(28,25,23,0.38)");
  x.fillStyle = shade;
  x.fillRect(-R, -R, R * 2, R * 2);
  x.restore();
  x.beginPath();
  x.arc(0, 0, R - 1, 0, Math.PI * 2);
  x.strokeStyle = "rgba(28,25,23,0.5)";
  x.lineWidth = 2;
  x.stroke();
  return canvas;
}

/** Squash/stretch follows the flight direction; rotation only spins the pattern. */
function drawBall(g: CanvasRenderingContext2D, b: Ball) {
  if (b.a <= 0.01 || b.r <= 0.3) return;
  const k = b.r * 1.03125;
  g.save();
  g.globalAlpha *= b.a;
  g.translate(b.x, b.y);
  g.rotate(b.dir);
  g.scale(b.sx, b.sy);
  g.rotate(-b.dir + b.rot);
  g.drawImage(getBallTexture(), -k, -k, k * 2, k * 2);
  g.restore();
}

function drawShadow(g: CanvasRenderingContext2D, x: number, groundY: number, r: number, alpha: number) {
  g.save();
  g.globalAlpha *= alpha * 0.45;
  g.fillStyle = "#000";
  g.beginPath();
  g.ellipse(x, groundY, r * 0.9, r * 0.22, 0, 0, Math.PI * 2);
  g.fill();
  g.restore();
}

/** Where the ball hits, the net gets pulled into a pocket, then springs back. */
function deform(p: Point, N: Net): Point {
  if (Math.abs(N.amp) < 0.002) return p;
  const dx = p.x - N.cx;
  const dy = p.y - N.cy;
  const f = N.amp * Math.exp(-(dx * dx + dy * dy) / (2 * N.sigma * N.sigma));
  return { x: p.x - dx * 0.7 * f, y: p.y - dy * 0.7 * f + 8 * f };
}

/** Net, back supports and impact flash (drawn behind the ball). */
function drawGoalBack(g: CanvasRenderingContext2D, F: Goal, N: Net, ox: number, oy: number) {
  const TL = { x: F.x0, y: F.y0 };
  const TR = { x: F.x1, y: F.y0 };
  const BL = { x: F.x0, y: F.y1 };
  const BR = { x: F.x1, y: F.y1 };
  const bTL = { x: F.bx0, y: F.by0 };
  const bTR = { x: F.bx1, y: F.by0 };
  const bBL = { x: F.bx0, y: F.by1 };
  const bBR = { x: F.bx1, y: F.by1 };
  g.save();
  g.translate(ox, oy);
  g.lineCap = "round";
  g.lineJoin = "round";
  g.fillStyle = "rgba(28,25,23,0.14)";
  g.beginPath();
  g.ellipse((F.x0 + F.x1) / 2, F.y1 + 1, (F.x1 - F.x0) * 0.56, 6, 0, 0, Math.PI * 2);
  g.fill();
  // All net lines in one path: dark outline first, then white — readable on light and dark desktops.
  g.beginPath();
  const line = (a: Point, b: Point) => {
    for (let i = 0; i <= 14; i++) {
      const p = deform(mix(a, b, i / 14), N);
      if (i) g.lineTo(p.x, p.y);
      else g.moveTo(p.x, p.y);
    }
  };
  for (let i = 0; i <= 10; i++) line(mix(bTL, bTR, i / 10), mix(bBL, bBR, i / 10));
  for (let j = 0; j <= 6; j++) line(mix(bTL, bBL, j / 6), mix(bTR, bBR, j / 6));
  for (let i = 1; i < 10; i++) line(mix(TL, TR, i / 10), mix(bTL, bTR, i / 10));
  for (const k of [1 / 3, 2 / 3]) line(mix(TL, bTL, k), mix(TR, bTR, k));
  for (let j = 1; j < 6; j++) {
    line(mix(TL, BL, j / 6), mix(bTL, bBL, j / 6));
    line(mix(TR, BR, j / 6), mix(bTR, bBR, j / 6));
  }
  for (const k of [1 / 3, 2 / 3]) {
    line(mix(TL, bTL, k), mix(BL, bBL, k));
    line(mix(TR, bTR, k), mix(BR, bBR, k));
  }
  g.strokeStyle = "rgba(28,25,23,0.32)";
  g.lineWidth = 2.4;
  g.stroke();
  g.strokeStyle = "rgba(255,255,255,0.95)";
  g.lineWidth = 1.1;
  g.stroke();
  g.beginPath();
  g.moveTo(F.x0, F.y0);
  g.lineTo(F.bx0, F.by0);
  g.lineTo(F.bx0, F.by1);
  g.lineTo(F.x0, F.y1);
  g.moveTo(F.x1, F.y0);
  g.lineTo(F.bx1, F.by0);
  g.lineTo(F.bx1, F.by1);
  g.lineTo(F.x1, F.y1);
  g.strokeStyle = "rgba(28,25,23,0.35)";
  g.lineWidth = 3.8;
  g.stroke();
  g.strokeStyle = "#F5F5F0";
  g.lineWidth = 2.2;
  g.stroke();
  if (N.flash > 0) {
    const glow = g.createRadialGradient(N.cx, N.cy, 0, N.cx, N.cy, 50);
    glow.addColorStop(0, `rgba(255,255,255,${(0.8 * N.flash).toFixed(3)})`);
    glow.addColorStop(1, "rgba(255,255,255,0)");
    g.fillStyle = glow;
    g.beginPath();
    g.arc(N.cx, N.cy, 50, 0, Math.PI * 2);
    g.fill();
  }
  g.restore();
}

/** Front posts and crossbar (drawn over the ball once it is inside). */
function drawGoalFrame(g: CanvasRenderingContext2D, F: Goal, ox: number, oy: number) {
  g.save();
  g.translate(ox, oy);
  g.lineCap = "round";
  g.lineJoin = "round";
  g.beginPath();
  g.moveTo(F.x0, F.y1);
  g.lineTo(F.x0, F.y0);
  g.lineTo(F.x1, F.y0);
  g.lineTo(F.x1, F.y1);
  g.shadowColor = "rgba(0,0,0,0.28)";
  g.shadowBlur = 10;
  g.shadowOffsetY = 3;
  g.strokeStyle = "rgba(28,25,23,0.55)";
  g.lineWidth = 9;
  g.stroke();
  g.shadowColor = "transparent";
  g.shadowBlur = 0;
  g.shadowOffsetY = 0;
  g.strokeStyle = "#FAFAF8";
  g.lineWidth = 6;
  g.stroke();
  g.restore();
}

/** Countdown: an orange line under the goal that shrinks. */
function drawCountdownLine(
  g: CanvasRenderingContext2D,
  base: { x0: number; x1: number; y: number },
  progress: number,
  ox: number,
  oy: number,
) {
  g.save();
  g.translate(ox, oy);
  g.lineCap = "round";
  g.lineWidth = 3;
  g.strokeStyle = "rgba(28,25,23,0.14)";
  g.beginPath();
  g.moveTo(base.x0, base.y);
  g.lineTo(base.x1, base.y);
  g.stroke();
  if (progress > 0.002) {
    g.strokeStyle = "#F97316";
    g.beginPath();
    g.moveTo(base.x0, base.y);
    g.lineTo(lerp(base.x0, base.x1, progress), base.y);
    g.stroke();
  }
  g.restore();
}

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

/** From p along d: where the ball's center meets the table edge, and that edge's normal. */
function rayToEdge(p: Point, d: Point, B: Table): EdgeHit {
  const tx = d.x > 1e-9 ? (B.x1 - p.x) / d.x : d.x < -1e-9 ? (B.x0 - p.x) / d.x : Infinity;
  const ty = d.y > 1e-9 ? (B.y1 - p.y) / d.y : d.y < -1e-9 ? (B.y0 - p.y) / d.y : Infinity;
  const t = Math.max(0, Math.min(tx, ty));
  const at = { x: p.x + d.x * t, y: p.y + d.y * t };
  const corner =
    Math.min(Math.abs(at.x - B.x0), Math.abs(at.x - B.x1)) < BANK.corner &&
    Math.min(Math.abs(at.y - B.y0), Math.abs(at.y - B.y1)) < BANK.corner;
  return { at, t, corner, n: tx < ty ? { x: d.x > 0 ? -1 : 1, y: 0 } : { x: 0, y: d.y > 0 ? -1 : 1 } };
}

/** Whether segment a-b passes through the box. */
function segmentHitsBox(a: Point, b: Point, box: Table) {
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
      if (q < 0) return false;
      continue;
    }
    const t = q / p;
    if (p < 0) {
      if (t > t1) return false;
      t0 = Math.max(t0, t);
    } else {
      if (t < t0) return false;
      t1 = Math.min(t1, t);
    }
  }
  return t0 <= t1;
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
  private freeze = 0;
  private raf = 0;
  private last = 0;
  private stopped = false;
  private learned: boolean;
  private saveKnown = false;

  constructor(canvas: HTMLCanvasElement, layout: FootballLayout, opts: FootballOptions) {
    const g = canvas.getContext("2d");
    if (!g) throw new Error("Canvas 2D is not available");
    this.canvas = canvas;
    this.g = g;
    this.layout = layout;
    this.opts = opts;
    this.learned = opts.learned;
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
      pres: [],
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
    s.pres = [];
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

  /** Release: shoot opposite to the pull; facing away from the goal rebounds off the edge. */
  private launch(s: Session, D: Drag) {
    const d = { x: -D.ux, y: -D.uy };
    const plan = this.planShot(s, { x: D.x, y: D.y }, d, D.power);
    s.shot = { k: plan.k, u: plan.u, v: plan.v, power: D.power, aimed: true, bounces: plan.pres.length };
    s.path = plan.path;
    s.pres = plan.pres;
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
    this.opts.onShot();
  }

  /** Left/right follows the aim (k), height follows the power — like a FIFA power bar. */
  private landing(s: Session, k: number, power: number): Landing {
    return this.landingAt(s, 0.5 - k / 2, power, 0);
  }

  /** A spot in the net: across by u (0 = left post), height by power, moved up or down by dv. */
  private landingAt(s: Session, u: number, power: number, dv: number): Landing {
    const G = s.goal;
    const hh = (G.by1 - G.by0) / 2;
    const x = clamp(lerp(G.bx0, G.bx1, u), G.bx0 + s.rEnd + 2, G.bx1 - s.rEnd - 2);
    const y = clamp((G.by0 + G.by1) / 2 + (lerp(0.55, -0.75, power) + dv) * hh, G.by0 + s.rEnd + 1, G.by1 - s.rEnd - 1);
    return { P: { x, y }, u: (x - G.bx0) / (G.bx1 - G.bx0), v: (y - G.by0) / (G.by1 - G.by0) };
  }

  private table(s: Session): Table {
    const w = this.layout.work;
    const m = s.r0;
    return { x0: w.x + m, x1: w.x + w.w - m, y0: w.y + m, y1: w.y + w.h - m };
  }

  /** From p, flying along d: which work-area edge is hit first, and where. */
  private wallHit(s: Session, p: Point, d: Point) {
    const { x0, x1, y0, y1 } = this.table(s);
    const from = { x: clamp(p.x, x0, x1), y: clamp(p.y, y0, y1) };
    const tx = d.x > 1e-6 ? (x1 - from.x) / d.x : d.x < -1e-6 ? (x0 - from.x) / d.x : Infinity;
    const ty = d.y > 1e-6 ? (y1 - from.y) / d.y : d.y < -1e-6 ? (y0 - from.y) / d.y : Infinity;
    const t = Math.max(0, Math.min(tx, ty));
    const n = tx < ty ? { x: d.x > 0 ? -1 : 1, y: 0 } : { x: 0, y: d.y > 0 ? -1 : 1 };
    return { from, at: { x: from.x + d.x * t, y: from.y + d.y * t }, n, dist: t };
  }

  /** The whole flight is decided at release; the ball always ends up in the goal. */
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
      return { k, u: L.u, v: L.v, pres: [], path: pathFrom(p0, L.P, power, d) };
    }
    // Facing away: a straight bank shot off 1–2 screen edges, when one looks natural enough
    const bank = this.bankShot(s, p0, d, power);
    if (bank) return bank;
    // Otherwise straight to the screen edge, bounce (angle in = angle out), curl into the goal
    const wall = this.wallHit(s, p0, d);
    const dn = d.x * wall.n.x + d.y * wall.n.y;
    const r = { x: d.x - 2 * dn * wall.n.x, y: d.y - 2 * dn * wall.n.y };
    let ux = T.x - wall.at.x;
    let uy = T.y - wall.at.y;
    const ul = Math.hypot(ux, uy) || 1;
    ux /= ul;
    uy /= ul;
    const k = clamp(signedAngle(ux, uy, r.x, r.y) / AIM.spread, -1, 1);
    const L = this.landing(s, k, power);
    const pre: WallPath = {
      p0: wall.from,
      c: mix(wall.from, wall.at, 0.5),
      p2: wall.at,
      n: wall.n,
      dur: clamp(wall.dist / (1500 + 1500 * power), 0.06, 0.6),
    };
    return { k, u: L.u, v: L.v, pres: [pre], path: pathFrom(wall.at, L.P, power * 0.8, r) };
  }

  /** Best straight route off 1–2 edges into the net, or null when every route needs too big a nudge. */
  private bankShot(s: Session, p0: Point, d: Point, power: number): ShotPlan | null {
    const B = this.table(s);
    const G = s.goal;
    const start = { x: clamp(p0.x, B.x0, B.x1), y: clamp(p0.y, B.y0, B.y1) };
    const goalBox: Table = { x0: G.x0 - 12, x1: G.x1 + 12, y0: G.y0 - 12, y1: G.y1 + 12 };
    const spots: Landing[] = [];
    for (let i = 0; i < 8; i++) {
      for (const dv of [-0.3, 0, 0.3]) spots.push(this.landingAt(s, 0.15 + 0.1 * i, power, dv));
    }
    // The exact launch direction first, then turned one degree at a time either way.
    const turns = [0];
    for (let i = 1; i <= Math.floor(BANK.bend / BANK.launchWeight / DEG); i++) turns.push(i * DEG, -i * DEG);
    let best: { score: number; spot: Landing; hits: EdgeHit[] } | null = null;
    for (const turn of turns) {
      const bend0 = Math.abs(turn) * BANK.launchWeight;
      if (best && bend0 >= best.score) break;
      const dl = rotate(d, turn);
      const h1 = rayToEdge(start, dl, B);
      if (h1.corner || h1.t < 1) continue;
      const r1 = reflect(dl, h1.n);
      for (const spot of spots) {
        // One bounce: from the first edge straight into the net
        const o = toward(h1.at, spot.P);
        const bend1 = Math.abs(signedAngle(r1.x, r1.y, o.x, o.y));
        const one = Math.max(bend0, bend1);
        if (bend1 <= BANK.bend && (!best || one < best.score) && dot(o, h1.n) >= BANK.graze && this.lastLegOk(h1.at, spot.P)) {
          best = { score: one, spot, hits: [h1] };
        }
        // Two bounces: nudge the first bounce by th; the second nudge follows from it
        for (let th = -BANK.bend; th <= BANK.bend + 1e-9; th += DEG) {
          if (best && Math.max(bend0, Math.abs(th)) + BANK.twoBias >= best.score) continue;
          const o1 = rotate(r1, th);
          if (dot(o1, h1.n) < BANK.graze) continue;
          const h2 = rayToEdge(h1.at, o1, B);
          if (h2.corner || h2.t < 1 || segmentHitsBox(h1.at, h2.at, goalBox)) continue; // never through the goal on the way
          const o2 = toward(h2.at, spot.P);
          const r2 = reflect(o1, h2.n);
          const bend2 = Math.abs(signedAngle(r2.x, r2.y, o2.x, o2.y));
          const two = Math.max(bend0, Math.abs(th), bend2) + BANK.twoBias;
          if (bend2 > BANK.bend || (best && two >= best.score)) continue;
          if (dot(o2, h2.n) < BANK.graze || !this.lastLegOk(h2.at, spot.P)) continue;
          best = { score: two, spot, hits: [h1, h2] };
        }
      }
    }
    if (!best) return null;
    const speed = BANK.speed + BANK.speedPerPower * power;
    const pts = [start, ...best.hits.map((h) => h.at), best.spot.P];
    const pres: WallPath[] = best.hits.map((h, i) => ({
      p0: pts[i],
      c: mix(pts[i], pts[i + 1], 0.5),
      p2: pts[i + 1],
      n: h.n,
      dur: Math.max(0.05, toward(pts[i], pts[i + 1]).l / (speed * Math.pow(BANK.keep, i))),
    }));
    const from = pts[pts.length - 2];
    const P = best.spot.P;
    const path: Path = {
      p0: from,
      c: mix(from, P, 0.5),
      p2: P,
      // The last leg eases out from 1.5x its average speed, which matches the speed after the bounce.
      dur: Math.max(0.28, (1.5 * toward(from, P).l) / (speed * Math.pow(BANK.keep, pres.length))),
    };
    const total = path.dur + pres.reduce((sum, q) => sum + q.dur, 0);
    if (total > BANK.maxDur) {
      // Long routes speed up as a whole
      const f = BANK.maxDur / total;
      for (const q of pres) q.dur *= f;
      path.dur *= f;
    }
    return { k: clamp((0.5 - best.spot.u) * 2, -1, 1), u: best.spot.u, v: best.spot.v, pres, path };
  }

  /** The last leg must be long enough and come in from the front or above, not from behind the goal or under it. */
  private lastLegOk(a: Point, P: Point) {
    const o = toward(a, P);
    const across = this.layout.side === "left" ? -o.x : o.x;
    return o.l >= BANK.minLastLeg && across >= -0.25 && o.y >= -0.35;
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
      if (sh.bounces >= 2) return T.doubleRebound;
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
    b.rot += dt * 22;

    if (s.pres.length) {
      // Rebound shot: straight to the next screen edge first
      const q = s.pres[0];
      const t = clamp(s.t / q.dur, 0, 1);
      const p = bez(q.p0, q.c, q.p2, t);
      if (Math.hypot(p.x - b.x, p.y - b.y) > 0.01) b.dir = Math.atan2(p.y - b.y, p.x - b.x);
      b.x = p.x;
      b.y = p.y;
      b.r = s.r0;
      b.sx = 1.14;
      b.sy = 1 / 1.14;
      this.stepSquash(s, dt); // still squashed from the previous edge
      if (t >= 1) {
        // Edge hit: short freeze, squash, thud, spray, then bounce away
        s.pres.shift();
        s.t = 0;
        this.freeze = 0.035;
        s.squash = { t: 0, dir: Math.atan2(q.n.y, q.n.x) };
        this.opts.onSfx("wall", s.shot ? s.shot.power : 0.6);
        this.ring(b.x, b.y, s.r0 * 0.9, "#FAFAF8");
        this.burstToward(b.x, b.y, q.n.x, q.n.y, 10, ["#F97316", "#FDBA74", "#FAFAF8"], 0.8);
      }
      return;
    }

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
    // Near the end the ball is drawn behind the frame, so it looks like it went in.
    s.inside = u > 0.86;
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
          this.opts.onSfx("whoosh", s.path.dur + s.pres.reduce((sum, q) => sum + q.dur, 0));
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
        s.phase === "appear" || s.phase === "wait" || s.phase === "kick" || s.phase === "fly" || s.phase === "expire";
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
