#!/usr/bin/env node
/**
 * Generic single-instrument render tool for the match-reference loop.
 * Usage: node scripts/dev/render-note.mjs <family> <midi> <vel 1-127> <noteSec> <out.wav> [totalSec] [sr]
 * Renders through the CURRENT wasm build (rebuild first: cargo build ... && cp).
 */
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const WASM = fileURLToPath(new URL("../../packages/core/wasm/instruments_dsp.wasm", import.meta.url));
const GROUP = {
  marimba: 0, mallet: 0, unknown: 0, vibraphone: 1, glockenspiel: 2, woodwind: 2,
  musicbox: 3, guitar: 4, bass: 5, epiano: 6, piano: 9, drums: 7, percussion: 7,
  synthpad: 8, strings: 8, brass: 8, voice: 8, synth: 8,
  "guitar-steel": 10, "guitar-electric": 11, "guitar-distorted": 12,
};

const [family, midiS, velS, durS, out, totalS, srS] = process.argv.slice(2);
if (!out) {
  console.error("usage: render-note.mjs <family> <midi> <vel1-127> <noteSec> <out.wav> [totalSec] [sr]");
  process.exit(2);
}
const SR = Number(srS ?? 48000);
const Q = 128;
const midi = Number(midiS), vel = Number(velS) / 127, dur = Number(durS);
const total = Math.round(Number(totalS ?? dur + 2.0) * SR);
const inst = GROUP[family];
if (inst === undefined) { console.error(`unknown family ${family}`); process.exit(2); }

const { instance } = await WebAssembly.instantiate(await readFile(WASM), {});
const x = instance.exports;
const p = x.ij_engine_new(SR);
x.ij_set_track(p, 0, inst, 0.9, 0.0);
const isDrum = family === "drums" || family === "percussion";
const offAt = Math.round(dur * SR);
const mono = new Float32Array(total);
const lPtr = x.ij_out_l(p);
let started = false, stopped = false;
for (let f = 0; f < total; f += Q) {
  if (!started) { x.ij_note_on(p, 0, midi, vel); started = true; }
  if (!stopped && !isDrum && f >= offAt) { x.ij_note_off(p, 0, midi); stopped = true; }
  const n = Math.min(Q, total - f);
  x.ij_process(p, n);
  mono.set(new Float32Array(x.memory.buffer, lPtr, n), f);
}
const b = Buffer.alloc(44 + total * 2);
b.write("RIFF", 0); b.writeUInt32LE(36 + total * 2, 4); b.write("WAVE", 8);
b.write("fmt ", 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
b.writeUInt32LE(SR, 24); b.writeUInt32LE(SR * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
b.write("data", 36); b.writeUInt32LE(total * 2, 40);
for (let i = 0; i < total; i++) b.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(mono[i] * 32767))), 44 + i * 2);
await writeFile(out, b);
let peak = 0;
for (const s of mono) peak = Math.max(peak, Math.abs(s));
console.log(JSON.stringify({ out, family, midi, vel: Number(velS), peak: +peak.toFixed(3), seconds: total / SR }));
