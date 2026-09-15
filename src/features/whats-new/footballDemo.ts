import {
  deform,
  drawBall,
  drawCountdownLine,
  drawGoalBack,
  drawGoalFrame,
  type BallPose,
  type Goal,
  type Net,
} from "../football/draw";

interface Pt {
  x: number;
  y: number;
}

interface Bit {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  max: number;
  s: number;
  c: string;
  rot: number;
  vr: number;
}

interface RingFx {
  x: number;
  y: number;
  r0: number;
  color: string;
  t: number;
}

interface Palette {
  stage: string;
  surface: string;
  border: string;
  line: string;
  head: string;
  dot: string;
  select: string;
  shadow: string;
}

interface DemoBall extends BallPose {
  inside: boolean;
  resting: boolean;
}

const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const mix = (a: Pt, b: Pt, t: number): Pt => ({ x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t) });
const seg = (t: number, a: number, b: number) => clamp((t - a) / (b - a), 0, 1);
const easeInOut = (t: number) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);
const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);
const easeOutBack = (t: number) => {
  const c = 1.9;
  return 1 + (c + 1) * Math.pow(t - 1, 3) + c * Math.pow(t - 1, 2);
};
const unit = (x: number, y: number): Pt => {
  const l = Math.hypot(x, y) || 1;
  return { x: x / l, y: y / l };
};
const bez = (p0: Pt, c: Pt, p2: Pt, u: number): Pt => ({
  x: (1 - u) * (1 - u) * p0.x + 2 * (1 - u) * u * c.x + u * u * p2.x,
  y: (1 - u) * (1 - u) * p0.y + 2 * (1 - u) * u * c.y + u * u * p2.y,
});

// A 432x270 mini screen: a document window on the left and a small goal in the bottom-right corner.
const W = 432;
const H = 270;
const DOC = { x: 18, y: 18, w: 254, h: 208 };
const LINES: Array<{ y: number; w: number; head?: boolean }> = [
  { y: 40, w: 132, head: true },
  { y: 66, w: 196 },
  { y: 84, w: 214 },
  { y: 102, w: 172 },
  { y: 120, w: 206 },
  { y: 138, w: 150 },
  { y: 156, w: 192 },
  { y: 174, w: 118 },
  { y: 192, w: 170 },
];
const PICK = LINES[3];
const SEL_START = { x: 36, y: PICK.y + 3.5 };
const SEL_END = { x: 38 + PICK.w + 2, y: PICK.y + 3.5 };
const IDLE = { x: 150, y: 246 };
/** The demo goal is the app's default goal at this size. */
const GS = 0.56;
const GOAL: Goal = (() => {
  const w = 170 * GS;
  const h = 104 * GS;
  const x1 = W - 14;
  const x0 = x1 - w;
  const y1 = H - 20;
  const y0 = y1 - h;
  return { x0, y0, x1, y1, bx0: x0 + w * 0.12, bx1: x1 - w * 0.12, by0: y0 + h * 0.26, by1: y1 - h * 0.08 };
})();
const BASE = { x0: GOAL.x0 + 2, x1: GOAL.x1 - 2, y: GOAL.y1 + 7 };
const R0 = 10;
const REND = R0 * 0.56;
const PULL = 30;
const REST = { x: SEL_END.x + 20, y: SEL_END.y + 20 };
const LAND = { x: lerp(GOAL.bx0, GOAL.bx1, 0.6), y: lerp(GOAL.by0, GOAL.by1, 0.45) };
// Aimed straight at the goal: pulled back directly away from the landing spot.
const SHOT = unit(LAND.x - REST.x, LAND.y - REST.y);
const PULLED = { x: REST.x - SHOT.x * PULL, y: REST.y - SHOT.y * PULL };
/** Control point of the flight: halfway along the shot and lifted a little, like the app's aimed shot. */
const ARC = (() => {
  const d = Math.hypot(LAND.x - PULLED.x, LAND.y - PULLED.y);
  return { x: PULLED.x + SHOT.x * d * 0.5, y: PULLED.y + SHOT.y * d * 0.5 - d * 0.12 };
})();

// Timeline in seconds: copy -> ball appears -> pull back -> release at the goal -> goal -> fade.
const TL = (() => {
  const release = 3.8;
  const kickEnd = release + 0.07;
  const goal = kickEnd + 0.62;
  const leave = goal + 2.2;
  const gone = leave + 0.35;
  return {
    cursorIn: [0.15, 0.75] as const,
    select: [0.85, 1.55] as const,
    keyOn: 1.6,
    ballIn: [1.75, 2.1] as const,
    keyOff: 2.2,
    toBall: [2.3, 2.8] as const,
    pull: [2.95, 3.7] as const,
    release,
    kickEnd,
    goal,
    leave,
    gone,
    loop: gone + 1.2,
    poster: goal + 0.9,
  };
})();

let arrow: Path2D | null = null;

function readPalette(): Palette {
  const dark = document.documentElement.classList.contains("dark") || document.body.classList.contains("dark");
  const css = getComputedStyle(document.body);
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    stage: v("--color-surface-raised", dark ? "#292524" : "#F5F5F0"),
    surface: v("--color-surface", dark ? "#1C1917" : "#FFFFFF"),
    border: v("--color-border", dark ? "#3D3935" : "#E7E5E4"),
    line: dark ? "#3D3935" : "#E7E5E4",
    head: dark ? "#57534E" : "#D6D3D1",
    dot: dark ? "#57534E" : "#D6D3D1",
    select: dark ? "rgba(251, 146, 60, 0.3)" : "rgba(249, 115, 22, 0.24)",
    shadow: dark ? "rgba(0, 0, 0, 0.35)" : "rgba(28, 25, 23, 0.08)",
  };
}

export interface FootballDemoParts {
  canvas: HTMLCanvasElement;
  stage: HTMLElement;
  keycap: HTMLElement;
  cheer: HTMLElement;
  /** Which of the three steps the demo is on (0 = none). */
  onStep: (step: number) => void;
  /** True while the demo is a still picture (reduced motion) waiting for "play". */
  onPoster: (poster: boolean) => void;
}

/**
 * Scripted, looping demo of the football bubble for the "what's new" card. It draws with the
 * same code as the real bubble; a timeline drives the story, simple physics the net and bits.
 */
export class FootballDemo {
  private readonly ui: FootballDemoParts;
  private readonly g: CanvasRenderingContext2D;
  private readonly motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  private readonly resizeObserver: ResizeObserver;
  private readonly themeObserver: MutationObserver;
  private pal = readPalette();
  private scale = 1;
  private dpr = 1;
  private clock = 0;
  private freeze = 0;
  private raf = 0;
  private last = 0;
  private running = false;
  private once = false;
  private poster = false;
  private step = -1;
  private bits: Bit[] = [];
  private rings: RingFx[] = [];
  private trail: Array<Pt & { r: number }> = [];
  private sim: { x: number; y: number; vx: number; vy: number; rot: number } | null = null;
  private shake = 0;
  private readonly net: Net = { cx: 0, cy: 0, amp: 0, v: 0, flash: 0, hold: 0, sigma: 30 * GS };

  constructor(ui: FootballDemoParts) {
    const g = ui.canvas.getContext("2d");
    if (!g) throw new Error("Canvas 2D is not available");
    this.ui = ui;
    this.g = g;
    this.place(ui.keycap, REST.x - 2, SEL_END.y - 10);
    this.place(ui.cheer, (GOAL.x0 + GOAL.x1) / 2, GOAL.y0 - 8);
    this.resizeObserver = new ResizeObserver(this.resize);
    this.themeObserver = new MutationObserver(() => {
      this.pal = readPalette();
      this.render();
    });
  }

  start() {
    this.resizeObserver.observe(this.ui.stage);
    this.themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    this.themeObserver.observe(document.body, { attributes: true, attributeFilter: ["class"] });
    this.motion.addEventListener("change", this.onMotion);
    document.addEventListener("visibilitychange", this.onVisibility);
    this.resize();
    this.onMotion();
  }

  destroy() {
    this.stop();
    this.resizeObserver.disconnect();
    this.themeObserver.disconnect();
    this.motion.removeEventListener("change", this.onMotion);
    document.removeEventListener("visibilitychange", this.onVisibility);
  }

  /** Plays the demo once from a still picture, then returns to it. */
  playOnce() {
    this.play(true);
  }

  private readonly onMotion = () => {
    if (this.motion.matches) this.showPoster();
    else this.play(false);
  };

  private readonly onVisibility = () => {
    if (!this.running) return;
    cancelAnimationFrame(this.raf);
    if (!document.hidden) {
      this.last = performance.now();
      this.raf = requestAnimationFrame(this.frame);
    }
  };

  private place(el: HTMLElement, x: number, y: number) {
    el.style.left = `${(x / W) * 100}%`;
    el.style.top = `${(y / H) * 100}%`;
  }

  private readonly resize = () => {
    const rect = this.ui.stage.getBoundingClientRect();
    if (!rect.width) return;
    this.dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.scale = rect.width / W;
    this.ui.canvas.width = Math.round(rect.width * this.dpr);
    this.ui.canvas.height = Math.round(rect.height * this.dpr);
    this.ui.stage.style.setProperty("--s", this.scale.toFixed(3));
    this.render();
  };

  private play(once: boolean) {
    cancelAnimationFrame(this.raf);
    this.setPoster(false);
    this.reset();
    this.once = once;
    this.running = true;
    this.last = performance.now();
    this.raf = requestAnimationFrame(this.frame);
  }

  private stop() {
    this.running = false;
    cancelAnimationFrame(this.raf);
  }

  private showPoster() {
    this.stop();
    this.reset();
    while (this.clock < TL.poster) this.advance(1 / 60);
    this.freeze = 0;
    this.ui.cheer.classList.remove("is-showing", "is-leaving");
    this.ui.cheer.classList.add("is-static");
    this.setPoster(true);
    this.render();
  }

  private setPoster(poster: boolean) {
    if (this.poster === poster) return;
    this.poster = poster;
    this.ui.onPoster(poster);
  }

  private readonly frame = (now: number) => {
    if (!this.running) return;
    const dt = Math.min(0.05, (now - this.last) / 1000);
    this.last = now;
    if (this.freeze > 0) {
      this.freeze -= dt; // hit-stop on the goal
    } else {
      this.advance(dt);
      if (this.clock >= TL.loop) {
        if (this.once) {
          this.showPoster();
          return;
        }
        this.reset();
      }
    }
    this.render();
    this.raf = requestAnimationFrame(this.frame);
  };

  private reset() {
    this.clock = 0;
    this.freeze = 0;
    this.bits = [];
    this.rings = [];
    this.trail = [];
    this.sim = null;
    this.shake = 0;
    Object.assign(this.net, { cx: 0, cy: 0, amp: 0, v: 0, flash: 0, hold: 0 });
    this.ui.keycap.classList.remove("is-on");
    this.ui.cheer.classList.remove("is-showing", "is-leaving", "is-static");
  }

  private advance(dt: number) {
    const prev = this.clock;
    this.clock += dt;
    this.fire(prev, this.clock);
    const N = this.net;
    N.v += (-210 * N.amp - 9 * N.v) * dt;
    N.amp += N.v * dt;
    N.flash = Math.max(0, N.flash - dt / 0.2);
    this.shake = Math.max(0, this.shake - dt);
    const sim = this.sim;
    if (sim) {
      if (N.hold > 0) {
        N.hold -= dt;
        const p = deform(LAND, N, GS);
        sim.x = p.x;
        sim.y = p.y;
        sim.rot += 4 * dt;
      } else {
        sim.vy += 1400 * GS * dt;
        sim.x += sim.vx * dt;
        sim.y += sim.vy * dt;
        sim.rot += (sim.vx / REND) * dt;
        const floor = GOAL.by1 - REND * 0.85;
        if (sim.y > floor) {
          sim.y = floor;
          sim.vy = sim.vy > 30 ? -sim.vy * 0.35 : 0;
          sim.vx *= Math.pow(0.1, dt);
        }
        sim.x = clamp(sim.x, GOAL.bx0 + REND, GOAL.bx1 - REND);
      }
    }
    for (let i = this.bits.length - 1; i >= 0; i--) {
      const p = this.bits[i];
      p.life += dt;
      if (p.life >= p.max) {
        this.bits.splice(i, 1);
        continue;
      }
      p.vy += 500 * dt;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      p.rot += p.vr * dt;
    }
    for (let i = this.rings.length - 1; i >= 0; i--) {
      this.rings[i].t += dt;
      if (this.rings[i].t > 0.22) this.rings.splice(i, 1);
    }
    const b = this.ballAt(this.clock);
    if (b && !b.resting && this.clock < TL.goal) {
      this.trail.push({ x: b.x, y: b.y, r: b.r });
      if (this.trail.length > 7) this.trail.shift();
    } else if (this.trail.length) {
      this.trail.shift();
    }
  }

  private fire(prev: number, now: number) {
    const crossed = (at: number) => prev < at && now >= at;
    const { keycap, cheer } = this.ui;
    if (crossed(TL.keyOn)) keycap.classList.add("is-on");
    if (crossed(TL.keyOff)) keycap.classList.remove("is-on");
    if (crossed(TL.release)) {
      this.ring(PULLED.x, PULLED.y, R0 * 1.3, "#F97316");
      this.burst(PULLED.x, PULLED.y + R0 * 0.7, 7, 0, -1, 2.2, ["#A8A29E", "#D6D3D1", "#FAFAF8"], 0.45);
    }
    if (crossed(TL.goal)) {
      Object.assign(this.net, { cx: LAND.x, cy: LAND.y, flash: 1, hold: 0.14 });
      this.net.v += 13.6;
      this.freeze = 0.08;
      this.shake = 0.3;
      this.sim = { x: LAND.x, y: LAND.y, vx: (Math.random() - 0.5) * 40, vy: 0, rot: (TL.goal - TL.kickEnd) * 22 };
      this.ring(LAND.x, LAND.y, 9, "#F97316");
      this.burst(LAND.x, LAND.y, 16, -1, -1, 2.4, ["#F97316", "#FDBA74", "#FAFAF8", "#FACC15"], 0.9);
      this.burst(LAND.x, LAND.y, 8, 1, -1, 2.0, ["#F97316", "#FAFAF8"], 0.6);
      cheer.classList.remove("is-leaving", "is-static");
      void cheer.offsetWidth; // restart the pop-in animation
      cheer.classList.add("is-showing");
    }
    if (crossed(TL.leave)) {
      cheer.classList.remove("is-showing");
      cheer.classList.add("is-leaving");
    }
  }

  private burst(x: number, y: number, n: number, dirX: number, dirY: number, spread: number, colors: string[], speed: number) {
    const base = Math.atan2(dirY, dirX);
    for (let i = 0; i < n; i++) {
      const a = base + (Math.random() - 0.5) * spread;
      const v = (60 + Math.random() * 150) * speed;
      this.bits.push({
        x,
        y,
        vx: Math.cos(a) * v,
        vy: Math.sin(a) * v,
        life: 0,
        max: 0.35 + Math.random() * 0.35,
        s: 1 + Math.random() * 1.3,
        c: colors[i % colors.length],
        rot: Math.random() * 6,
        vr: (Math.random() - 0.5) * 20,
      });
    }
  }

  private ring(x: number, y: number, r0: number, color: string) {
    this.rings.push({ x, y, r0, color, t: 0 });
  }

  private cursorAt(t: number): { p: Pt; pressed: boolean } {
    if (t < TL.cursorIn[0]) return { p: IDLE, pressed: false };
    if (t < TL.cursorIn[1]) return { p: mix(IDLE, SEL_START, easeInOut(seg(t, ...TL.cursorIn))), pressed: false };
    if (t < TL.select[0]) return { p: SEL_START, pressed: false };
    if (t < TL.select[1]) return { p: mix(SEL_START, SEL_END, easeInOut(seg(t, ...TL.select))), pressed: true };
    if (t < TL.toBall[0]) return { p: SEL_END, pressed: false };
    if (t < TL.toBall[1]) return { p: mix(SEL_END, REST, easeInOut(seg(t, ...TL.toBall))), pressed: false };
    if (t < TL.pull[0]) return { p: REST, pressed: false };
    if (t < TL.release) return { p: mix(REST, PULLED, easeOut(seg(t, ...TL.pull))), pressed: true };
    if (t < TL.leave) return { p: PULLED, pressed: false };
    return { p: mix(PULLED, IDLE, easeInOut(seg(t, TL.leave, TL.loop - 0.2))), pressed: false };
  }

  private ballAt(t: number): DemoBall | null {
    if (t < TL.ballIn[0] || t >= TL.gone) return null;
    const b: DemoBall = { x: REST.x, y: REST.y, r: R0, sx: 1, sy: 1, dir: 0, rot: 0, a: 1, inside: false, resting: true };
    if (t < TL.ballIn[1]) {
      b.r = R0 * Math.max(0.001, easeOutBack(seg(t, ...TL.ballIn)));
      return b;
    }
    if (t < TL.pull[0]) {
      b.r = R0 * (1 + 0.07 * seg(t, TL.toBall[1] - 0.15, TL.toBall[1]));
      return b;
    }
    if (t < TL.release) {
      const k = easeOut(seg(t, ...TL.pull));
      const p = mix(REST, PULLED, k);
      const jitter = k > 0.92 ? 0.8 : 0; // trembles at full draw
      b.x = p.x + (Math.random() - 0.5) * jitter;
      b.y = p.y + (Math.random() - 0.5) * jitter;
      b.dir = Math.atan2(PULLED.y - REST.y, PULLED.x - REST.x);
      b.sx = 1 + 0.14 * k;
      b.sy = 1 / b.sx;
      return b;
    }
    const shotDir = Math.atan2(SHOT.y, SHOT.x);
    if (t < TL.kickEnd) {
      const q = Math.sin(Math.PI * seg(t, TL.release, TL.kickEnd));
      return { ...b, x: PULLED.x, y: PULLED.y, dir: shotDir, sx: 1 - 0.2 * q, sy: 1 + 0.24 * q };
    }
    b.resting = false;
    b.rot = (Math.min(t, TL.goal) - TL.kickEnd) * 22;
    if (t < TL.goal) {
      // Straight at the goal on a low arc, easing out and shrinking into the net like the app's shot
      const k = seg(t, TL.kickEnd, TL.goal);
      const u = 0.5 * k + 0.5 * (1 - (1 - k) * (1 - k));
      const p = bez(PULLED, ARC, LAND, u);
      const ahead = bez(PULLED, ARC, LAND, Math.min(1, u + 0.02));
      const dir = u < 0.98 ? Math.atan2(ahead.y - p.y, ahead.x - p.x) : shotDir;
      const stretch = 1 + 0.14 * Math.sin(Math.PI * Math.min(1, k * 2.5));
      return { ...b, x: p.x, y: p.y, r: lerp(R0, REND, u), dir, sx: stretch, sy: 1 / stretch, inside: u > 0.86 };
    }
    if (!this.sim) return null;
    return { ...b, x: this.sim.x, y: this.sim.y, r: REND, inside: true, rot: this.sim.rot, a: 1 - seg(t, TL.leave, TL.gone) };
  }

  private roundRect(x: number, y: number, w: number, h: number, r: number) {
    const g = this.g;
    g.beginPath();
    if (typeof g.roundRect === "function") g.roundRect(x, y, w, h, r);
    else g.rect(x, y, w, h);
  }

  private drawCursor(p: Pt, pressed: boolean) {
    const g = this.g;
    arrow ??= new Path2D("M0 0 L0 16 L4.3 12.1 L7.2 18.6 L9.9 17.4 L7.1 11 L12.8 11 Z");
    g.save();
    g.translate(p.x, p.y);
    const s = pressed ? 0.88 : 1;
    g.scale(s, s);
    g.shadowColor = "rgba(0,0,0,0.25)";
    g.shadowBlur = 2;
    g.shadowOffsetY = 1;
    g.lineJoin = "round";
    g.lineWidth = 1.8;
    g.strokeStyle = "#FFFFFF";
    g.stroke(arrow);
    g.shadowColor = "transparent";
    g.fillStyle = "#111111";
    g.fill(arrow);
    g.restore();
  }

  private syncStep() {
    const t = this.clock;
    const step = this.poster ? 0 : t < TL.toBall[0] ? 1 : t < TL.goal ? 2 : 3;
    if (step === this.step) return;
    this.step = step;
    this.ui.onStep(step);
  }

  private render() {
    const g = this.g;
    if (!this.ui.canvas.width) return;
    const t = this.clock;
    const pal = this.pal;
    g.setTransform(this.scale * this.dpr, 0, 0, this.scale * this.dpr, 0, 0);
    g.clearRect(0, 0, W, H);
    g.fillStyle = pal.stage;
    g.fillRect(0, 0, W, H);
    const fade = 1 - seg(t, TL.leave, TL.gone);

    // Document window with the line being copied
    g.save();
    g.shadowColor = pal.shadow;
    g.shadowBlur = 14;
    g.shadowOffsetY = 3;
    this.roundRect(DOC.x, DOC.y, DOC.w, DOC.h, 10);
    g.fillStyle = pal.surface;
    g.fill();
    g.restore();
    this.roundRect(DOC.x + 0.5, DOC.y + 0.5, DOC.w - 1, DOC.h - 1, 10);
    g.strokeStyle = pal.border;
    g.lineWidth = 1;
    g.stroke();
    for (let i = 0; i < 3; i++) {
      g.beginPath();
      g.arc(DOC.x + 14 + i * 10, DOC.y + 12, 3, 0, Math.PI * 2);
      g.fillStyle = pal.dot;
      g.fill();
    }
    const selected = easeInOut(seg(t, ...TL.select));
    if (selected > 0 && fade > 0) {
      g.save();
      g.globalAlpha = fade;
      this.roundRect(SEL_START.x - 2, PICK.y - 4.5, (SEL_END.x - SEL_START.x + 4) * selected, 16, 3);
      g.fillStyle = pal.select;
      g.fill();
      g.restore();
    }
    for (const l of LINES) {
      this.roundRect(38, l.y + (l.head ? -1 : 0), l.w, l.head ? 9 : 7, 3.5);
      g.fillStyle = l.head ? pal.head : pal.line;
      g.fill();
    }

    // Goal, ball, countdown
    const b = this.ballAt(t);
    const goalAlpha = t < TL.ballIn[0] ? 0 : Math.min(seg(t, TL.ballIn[0], TL.ballIn[0] + 0.25), fade);
    const k = this.shake > 0 ? this.shake / 0.3 : 0;
    const ox = k ? Math.sin(this.shake * 90) * 4.5 * GS * k : 0;
    const oy = k ? Math.cos(this.shake * 70) * 1.6 * GS * k : 0;
    if (goalAlpha > 0) {
      g.save();
      g.globalAlpha = goalAlpha;
      drawGoalBack(g, GOAL, this.net, ox, oy, GS);
      if (b?.resting) {
        g.save();
        g.globalAlpha *= 0.45 * b.a;
        g.fillStyle = "#000";
        g.beginPath();
        g.ellipse(b.x, b.y + R0 + 2, b.r * 0.9, b.r * 0.22, 0, 0, Math.PI * 2);
        g.fill();
        g.restore();
      }
      this.trail.forEach((p, i) => {
        const kk = (i + 1) / this.trail.length;
        g.save();
        g.globalAlpha *= 0.22 * kk;
        g.fillStyle = "#F97316";
        g.beginPath();
        g.arc(p.x, p.y, p.r * (0.5 + 0.5 * kk), 0, Math.PI * 2);
        g.fill();
        g.restore();
      });
      if (b?.inside) drawBall(g, b);
      drawGoalFrame(g, GOAL, ox, oy, GS);
      if (b && !b.inside) drawBall(g, b);
      if (t < TL.goal) {
        const shrink = 0.2 * (Math.min(t, TL.release) - TL.ballIn[0]);
        drawCountdownLine(g, BASE, 1 - Math.max(0, shrink), ox, oy, GS);
      }
      g.restore();
    } else if (b) {
      drawBall(g, b);
    }
    for (const q of this.rings) {
      const kk = q.t / 0.22;
      g.save();
      g.globalAlpha = (1 - kk) * 0.85;
      g.strokeStyle = q.color;
      g.lineWidth = 2 * (1 - kk) + 0.5;
      g.beginPath();
      g.arc(q.x, q.y, q.r0 * (1 + 1.3 * kk), 0, Math.PI * 2);
      g.stroke();
      g.restore();
    }
    for (const p of this.bits) {
      g.save();
      g.globalAlpha = 1 - p.life / p.max;
      g.translate(p.x, p.y);
      g.rotate(p.rot);
      g.fillStyle = p.c;
      g.fillRect(-p.s, -p.s * 0.6, p.s * 2, p.s * 1.2);
      g.restore();
    }
    const cursor = this.cursorAt(t);
    this.drawCursor(cursor.p, cursor.pressed);
    this.syncStep();
  }
}
