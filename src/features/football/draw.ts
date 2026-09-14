/** Canvas drawing of the football bubble, shared with the demo in the "what's new" card. */

interface Pt {
  x: number;
  y: number;
}

/** The goal seen from the front: the mouth (x0..x1, y0..y1) and the back of the net (bx/by). */
export interface Goal {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  bx0: number;
  bx1: number;
  by0: number;
  by1: number;
}

/** Where the ball hit the net, and how deep the pocket is pulled in right now. */
export interface Net {
  cx: number;
  cy: number;
  amp: number;
  v: number;
  flash: number;
  hold: number;
  sigma: number;
}

/** What drawing needs to know about the ball. */
export interface BallPose {
  x: number;
  y: number;
  r: number;
  rot: number;
  sx: number;
  sy: number;
  a: number;
  dir: number;
}

const mix = (a: Pt, b: Pt, t: number): Pt => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });

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
export function drawBall(g: CanvasRenderingContext2D, b: BallPose) {
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

export function drawShadow(g: CanvasRenderingContext2D, x: number, groundY: number, r: number, alpha: number) {
  g.save();
  g.globalAlpha *= alpha * 0.45;
  g.fillStyle = "#000";
  g.beginPath();
  g.ellipse(x, groundY, r * 0.9, r * 0.22, 0, 0, Math.PI * 2);
  g.fill();
  g.restore();
}

/**
 * Where the ball hits, the net gets pulled into a pocket, then springs back.
 * `scale` is the goal's size relative to the full-size goal.
 */
export function deform(p: Pt, N: Net, scale = 1): Pt {
  if (Math.abs(N.amp) < 0.002) return p;
  const dx = p.x - N.cx;
  const dy = p.y - N.cy;
  const f = N.amp * Math.exp(-(dx * dx + dy * dy) / (2 * N.sigma * N.sigma));
  return { x: p.x - dx * 0.7 * f, y: p.y - dy * 0.7 * f + 8 * scale * f };
}

/** Net, back supports and impact flash (drawn behind the ball). A smaller goal gets a coarser mesh and thinner lines. */
export function drawGoalBack(g: CanvasRenderingContext2D, F: Goal, N: Net, ox: number, oy: number, scale = 1) {
  const k = Math.sqrt(scale);
  const cols = Math.ceil(10 * k);
  const rows = Math.ceil(6 * k);
  const steps = scale < 1 ? 10 : 14;
  const depth = k < 0.8 ? [1 / 2] : [1 / 3, 2 / 3];
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
  g.ellipse((F.x0 + F.x1) / 2, F.y1 + 1, (F.x1 - F.x0) * 0.56, 6 * k, 0, 0, Math.PI * 2);
  g.fill();
  // All net lines in one path: dark outline first, then white — readable on light and dark desktops.
  g.beginPath();
  const line = (a: Pt, b: Pt) => {
    for (let i = 0; i <= steps; i++) {
      const p = deform(mix(a, b, i / steps), N, scale);
      if (i) g.lineTo(p.x, p.y);
      else g.moveTo(p.x, p.y);
    }
  };
  for (let i = 0; i <= cols; i++) line(mix(bTL, bTR, i / cols), mix(bBL, bBR, i / cols));
  for (let j = 0; j <= rows; j++) line(mix(bTL, bBL, j / rows), mix(bTR, bBR, j / rows));
  for (let i = 1; i < cols; i++) line(mix(TL, TR, i / cols), mix(bTL, bTR, i / cols));
  for (const d of depth) line(mix(TL, bTL, d), mix(TR, bTR, d));
  for (let j = 1; j < rows; j++) {
    line(mix(TL, BL, j / rows), mix(bTL, bBL, j / rows));
    line(mix(TR, BR, j / rows), mix(bTR, bBR, j / rows));
  }
  for (const d of depth) {
    line(mix(TL, bTL, d), mix(BL, bBL, d));
    line(mix(TR, bTR, d), mix(BR, bBR, d));
  }
  g.strokeStyle = "rgba(28,25,23,0.32)";
  g.lineWidth = 2.4 * k;
  g.stroke();
  g.strokeStyle = "rgba(255,255,255,0.95)";
  g.lineWidth = 1.1 * k;
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
  g.lineWidth = 3.8 * k;
  g.stroke();
  g.strokeStyle = "#F5F5F0";
  g.lineWidth = 2.2 * k;
  g.stroke();
  if (N.flash > 0) {
    const radius = 50 * scale;
    const glow = g.createRadialGradient(N.cx, N.cy, 0, N.cx, N.cy, radius);
    glow.addColorStop(0, `rgba(255,255,255,${(0.8 * N.flash).toFixed(3)})`);
    glow.addColorStop(1, "rgba(255,255,255,0)");
    g.fillStyle = glow;
    g.beginPath();
    g.arc(N.cx, N.cy, radius, 0, Math.PI * 2);
    g.fill();
  }
  g.restore();
}

/** Front posts and crossbar (drawn over the ball once it is inside). */
export function drawGoalFrame(g: CanvasRenderingContext2D, F: Goal, ox: number, oy: number, scale = 1) {
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
  g.shadowBlur = 10 * scale;
  g.shadowOffsetY = 3 * scale;
  g.strokeStyle = "rgba(28,25,23,0.55)";
  g.lineWidth = 9 * scale;
  g.stroke();
  g.shadowColor = "transparent";
  g.shadowBlur = 0;
  g.shadowOffsetY = 0;
  g.strokeStyle = "#FAFAF8";
  g.lineWidth = 6 * scale;
  g.stroke();
  g.restore();
}

/** Countdown: an orange line under the goal that shrinks. */
export function drawCountdownLine(
  g: CanvasRenderingContext2D,
  base: { x0: number; x1: number; y: number },
  progress: number,
  ox: number,
  oy: number,
  scale = 1,
) {
  g.save();
  g.translate(ox, oy);
  g.lineCap = "round";
  g.lineWidth = 3 * Math.sqrt(scale);
  g.strokeStyle = "rgba(28,25,23,0.14)";
  g.beginPath();
  g.moveTo(base.x0, base.y);
  g.lineTo(base.x1, base.y);
  g.stroke();
  if (progress > 0.002) {
    g.strokeStyle = "#F97316";
    g.beginPath();
    g.moveTo(base.x0, base.y);
    g.lineTo(base.x0 + (base.x1 - base.x0) * progress, base.y);
    g.stroke();
  }
  g.restore();
}
