#!/usr/bin/env node
/**
 * Piano quality proxies + audition renders. Since agents can't listen, iterate on:
 * - two-stage decay: early t60 (attack bloom) must be much shorter than late t60
 * - spectral centroid drop: bright attack mellowing into the tail
 * Renders an audition phrase WAV for the human listening pass.
 * Usage: node scripts/dev/piano-audition.mjs [audition.wav]
 */
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const SR = 48000;
const Q = 128;
const WASM = fileURLToPath(new URL("../../packages/core/wasm/instruments_dsp.wasm", import.meta.url));
const PIANO = 9;

async function engine() {
  const { instance } = await WebAssembly.instantiate(await readFile(WASM), {});
  const x = instance.exports;
  const p = x.ij_engine_new(SR);
  x.ij_set_track(p, 0, PIANO, 0.9, 0.0);
  return { x, p };
}

function renderNotes(x, p, events, seconds) {
  const total = Math.round(seconds * SR);
  const out = new Float32Array(total);
  const lPtr = x.ij_out_l(p);
  const evs = events.map((e) => ({ ...e, f: Math.round(e.t * SR) })).sort((a, b) => a.f - b.f);
  let ei = 0;
  for (let f = 0; f < total; f += Q) {
    while (ei < evs.length && evs[ei].f <= f) {
      const e = evs[ei++];
      if (e.kind === "on") x.ij_note_on(p, 0, e.m, e.v);
      else if (e.kind === "off") x.ij_note_off(p, 0, e.m);
      else if (e.kind === "pedal") x.ij_pedal(p, 0, e.on);
    }
    const n = Math.min(Q, total - f);
    x.ij_process(p, n);
    out.set(new Float32Array(x.memory.buffer, lPtr, n), f);
  }
  return out;
}

function rmsEnvelope(x, hop = Math.round(0.05 * SR)) {
  const env = [];
  for (let i = 0; i + hop <= x.length; i += hop) {
    let s = 0;
    for (let j = i; j < i + hop; j++) s += x[j] * x[j];
    env.push(Math.sqrt(s / hop));
  }
  return env;
}

/** decay t60 estimated from the env slope (dB/s) between two times */
function t60Between(env, t0, t1) {
  const hop = 0.05;
  const i0 = Math.round(t0 / hop), i1 = Math.round(t1 / hop);
  const d0 = 20 * Math.log10(env[i0] + 1e-9), d1 = 20 * Math.log10(env[i1] + 1e-9);
  const rate = (d0 - d1) / (t1 - t0); // dB per second
  return rate > 0.1 ? 60 / rate : Infinity;
}

function centroid(x, at, n = 4096) {
  // Hann-windowed, full-grid DFT, 0–6 kHz. (An earlier unwindowed coarse-grid
  // version smeared leakage across the band and mis-reported rising centroids —
  // measurement tools need the same rigor as the DSP they judge.)
  const i0 = Math.round(at * SR);
  const seg = x.slice(i0, i0 + n);
  const win = new Float32Array(seg.length);
  for (let i = 0; i < seg.length; i++)
    win[i] = seg[i] * 0.5 * (1 - Math.cos((2 * Math.PI * i) / (seg.length - 1)));
  const kMax = Math.min(Math.floor((6000 * n) / SR), n / 2 - 1);
  let num = 0, den = 0;
  for (let k = 1; k <= kMax; k++) {
    let re = 0, im = 0;
    const w = (2 * Math.PI * k) / n;
    for (let i = 0; i < win.length; i++) {
      re += win[i] * Math.cos(w * i);
      im -= win[i] * Math.sin(w * i);
    }
    const mag = re * re + im * im; // power weighting
    num += ((k * SR) / n) * mag;
    den += mag;
  }
  return den > 0 ? num / den : 0;
}

// ---- metrics on single notes ----
console.log("note  vel   peak    t60_early  t60_late  ratio   centroid@60ms  @600ms  drop");
for (const [name, midi] of [["C2", 36], ["C4", 60], ["C6", 84]]) {
  for (const vel of [0.35, 0.95]) {
    const { x, p } = await engine();
    const out = renderNotes(x, p, [{ kind: "on", m: midi, v: vel, t: 0 }], 2.2);
    let peak = 0;
    for (const s of out) peak = Math.max(peak, Math.abs(s));
    const env = rmsEnvelope(out);
    const early = t60Between(env, 0.1, 0.4);
    const late = t60Between(env, 0.9, 1.9);
    const c1 = centroid(out, 0.06), c2 = centroid(out, 0.6);
    x.ij_engine_free(p);
    console.log(
      `${name}   ${vel.toFixed(2)}  ${peak.toFixed(3)}  ${early.toFixed(2).padStart(8)}s ${late.toFixed(2).padStart(8)}s  ${(late / early).toFixed(1).padStart(5)}   ${c1.toFixed(0).padStart(8)} Hz ${c2.toFixed(0).padStart(6)} Hz  ${(c1 / (c2 + 1)).toFixed(2)}`,
    );
  }
}

// ---- audition phrase for the human ear ----
const outPath = process.argv[2];
if (outPath) {
  const { x, p } = await engine();
  const ev = [];
  // 1) velocity ladder on C4
  [0.25, 0.5, 0.75, 0.98].forEach((v, i) => {
    ev.push({ kind: "on", m: 60, v, t: i * 0.7 });
    ev.push({ kind: "off", m: 60, t: i * 0.7 + 0.6 });
  });
  // 2) arpeggio across registers
  [36, 43, 48, 55, 60, 64, 67, 72, 76, 79, 84].forEach((m, i) => {
    ev.push({ kind: "on", m, v: 0.8, t: 3.2 + i * 0.22 });
    ev.push({ kind: "off", m, t: 3.2 + i * 0.22 + 0.5 });
  });
  // 3) pedaled chord — listen to the tail sing
  ev.push({ kind: "pedal", on: 1, t: 6.2 });
  [48, 55, 60, 64, 67].forEach((m, i) => ev.push({ kind: "on", m, v: 0.85, t: 6.3 + i * 0.03 }));
  [48, 55, 60, 64, 67].forEach((m) => ev.push({ kind: "off", m, t: 6.8 }));
  ev.push({ kind: "pedal", on: 0, t: 10.5 });
  // 4) staccato repeats (hammer character)
  for (let i = 0; i < 6; i++) {
    ev.push({ kind: "on", m: 67, v: 0.9, t: 11.0 + i * 0.16 });
    ev.push({ kind: "off", m: 67, t: 11.0 + i * 0.16 + 0.09 });
  }
  const mono = renderNotes(x, p, ev, 13.5);
  const buf = Buffer.alloc(44 + mono.length * 2);
  buf.write("RIFF", 0); buf.writeUInt32LE(36 + mono.length * 2, 4); buf.write("WAVE", 8);
  buf.write("fmt ", 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(SR, 24); buf.writeUInt32LE(SR * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
  buf.write("data", 36); buf.writeUInt32LE(mono.length * 2, 40);
  for (let i = 0; i < mono.length; i++) buf.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(mono[i] * 32767))), 44 + i * 2);
  await writeFile(outPath, buf);
  console.log(`\naudition phrase → ${outPath}`);
}