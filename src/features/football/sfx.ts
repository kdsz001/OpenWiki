import type { SfxName } from "./types";

// All football sounds are synthesized on the fly — no audio files, no licensing.

interface Audio {
  c: AudioContext;
  out: GainNode;
  noise: AudioBuffer;
}

let audioState: Audio | null = null;

function getAudio(): Audio | null {
  try {
    if (!audioState) {
      const Ctor =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctor) return null;
      const c = new Ctor();
      const compressor = c.createDynamicsCompressor();
      const out = c.createGain();
      out.gain.value = 0.9;
      out.connect(compressor);
      compressor.connect(c.destination);
      const noise = c.createBuffer(1, c.sampleRate, c.sampleRate);
      const data = noise.getChannelData(0);
      for (let i = 0; i < data.length; i++) data[i] = Math.random() * 2 - 1;
      audioState = { c, out, noise };
    }
    if (audioState.c.state === "suspended") void audioState.c.resume();
    return audioState;
  } catch {
    return null;
  }
}

/** Call from a real user gesture (pointerdown / keydown) so the webview allows sound. */
export function unlockAudio() {
  getAudio();
}

function envelope(gain: GainNode, t: number, peak: number, attack: number, decay: number) {
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(Math.max(0.0002, peak), t + attack);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + attack + decay);
}

function tone(
  a: Audio,
  type: OscillatorType,
  f0: number,
  f1: number,
  t: number,
  peak: number,
  attack: number,
  decay: number,
) {
  const osc = a.c.createOscillator();
  const gain = a.c.createGain();
  osc.type = type;
  osc.frequency.setValueAtTime(f0, t);
  if (f1 !== f0) osc.frequency.exponentialRampToValueAtTime(f1, t + attack + decay);
  envelope(gain, t, peak, attack, decay);
  osc.connect(gain).connect(a.out);
  osc.start(t);
  osc.stop(t + attack + decay + 0.05);
}

function noise(
  a: Audio,
  t: number,
  type: BiquadFilterType,
  freq: number,
  q: number,
  peak: number,
  attack: number,
  decay: number,
  freqTo?: number,
) {
  const src = a.c.createBufferSource();
  src.buffer = a.noise;
  const filter = a.c.createBiquadFilter();
  filter.type = type;
  filter.Q.value = q;
  filter.frequency.setValueAtTime(freq, t);
  if (freqTo) filter.frequency.exponentialRampToValueAtTime(freqTo, t + attack + decay);
  const gain = a.c.createGain();
  envelope(gain, t, peak, attack, decay);
  src.connect(filter).connect(gain).connect(a.out);
  src.start(t, Math.random() * 0.5);
  src.stop(t + attack + decay + 0.05);
}

/** `value` means power (0–1) for shots, impact (0–1) for wall hits, duration for the whoosh, step (1–7) for ticks. */
export function playSfx(name: SfxName, value = 0.6) {
  const a = getAudio();
  if (!a) return;
  const t = a.c.currentTime;
  switch (name) {
    case "kick": // instep strike: low thump + leather slap
      tone(a, "sine", 170, 46, t, 0.95 * value, 0.004, 0.16);
      noise(a, t, "bandpass", 1400, 0.8, 0.5 * value, 0.002, 0.05);
      break;
    case "whoosh": // air rushing past while the ball flies
      noise(a, t + 0.02, "bandpass", 380, 1.4, 0.16, value * 0.5, value * 0.5, 1900);
      break;
    case "goal": {
      // net swish + deep boom + "saved" chime
      const k = 0.8 + 0.35 * value;
      noise(a, t, "highpass", 1700, 0.7, 0.55 * k, 0.01, 0.45);
      noise(a, t, "bandpass", 600, 0.9, 0.28 * k, 0.004, 0.18);
      tone(a, "sine", 110, 38, t, Math.min(0.95, 0.8 * k), 0.004, 0.3);
      tone(a, "sine", 1046.5, 1046.5, t + 0.12, 0.13, 0.008, 0.32);
      tone(a, "sine", 1568, 1568, t + 0.21, 0.13, 0.008, 0.32);
      break;
    }
    case "wall": // bouncing off the screen edge
      tone(a, "sine", 190, 85, t, 0.45 + 0.25 * value, 0.003, 0.11);
      noise(a, t, "bandpass", 950, 1.1, 0.3 + 0.15 * value, 0.002, 0.06);
      break;
    case "poof": // a missed ball deflating
      noise(a, t, "lowpass", 1100, 0.7, 0.24, 0.008, 0.24, 160);
      tone(a, "sine", 260, 90, t, 0.12, 0.005, 0.2);
      break;
    case "grab":
      tone(a, "sine", 480, 360, t, 0.12, 0.003, 0.06);
      break;
    case "tick": {
      // ratchet clicks while pulling, rising with the step
      const f = 900 + value * 140;
      noise(a, t, "bandpass", f * 2, 5, 0.12 + value * 0.012, 0.001, 0.03);
      tone(a, "triangle", f, f * 0.9, t, 0.05, 0.001, 0.03);
      break;
    }
    case "full": // fully drawn
      tone(a, "sine", 1975, 1975, t, 0.09, 0.002, 0.12);
      noise(a, t, "bandpass", 4200, 4, 0.2, 0.001, 0.035);
      break;
    case "twang": {
      // releasing the band
      const osc = a.c.createOscillator();
      const filter = a.c.createBiquadFilter();
      const gain = a.c.createGain();
      osc.type = "sawtooth";
      osc.frequency.setValueAtTime(220 + 180 * value, t);
      osc.frequency.exponentialRampToValueAtTime(80, t + 0.2);
      filter.type = "lowpass";
      filter.frequency.setValueAtTime(2400, t);
      filter.frequency.exponentialRampToValueAtTime(400, t + 0.2);
      envelope(gain, t, 0.22 + 0.18 * value, 0.002, 0.2);
      osc.connect(filter).connect(gain).connect(a.out);
      osc.start(t);
      osc.stop(t + 0.26);
      break;
    }
    case "boing": {
      // cancelled: the ball springs back
      const osc = a.c.createOscillator();
      const gain = a.c.createGain();
      const lfo = a.c.createOscillator();
      const lfoGain = a.c.createGain();
      osc.type = "sine";
      osc.frequency.setValueAtTime(320, t);
      osc.frequency.exponentialRampToValueAtTime(170, t + 0.3);
      lfo.frequency.value = 16;
      lfoGain.gain.value = 30;
      lfo.connect(lfoGain);
      lfoGain.connect(osc.frequency);
      envelope(gain, t, 0.16, 0.005, 0.3);
      osc.connect(gain).connect(a.out);
      osc.start(t);
      lfo.start(t);
      osc.stop(t + 0.36);
      lfo.stop(t + 0.36);
      break;
    }
  }
}
