#!/usr/bin/env node
/**
 * Generic single-instrument render tool for the match-reference loop.
 * Usage: node scripts/dev/render-note.mjs <family> <midi> <vel 1-127> <noteSec> <out.wav> [totalSec] [sr] [--float32] [--lead-seconds N]
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
  "drums-rock": 13, "drums-jazz": 14, "drums-808": 15,
};

const raw = process.argv.slice(2);
const args = [];
let FLOAT = false, leadSeconds = 0;
for (let i = 0; i < raw.length; i++) {
  if (raw[i] === "--float32") FLOAT = true;
  else if (raw[i] === "--lead-seconds") leadSeconds = Number(raw[++i]);
  else args.push(raw[i]);
}
const [family, midiS, velS, durS, out, totalS, srS] = args;
if (!out) {
  console.error("usage: render-note.mjs <family> <midi> <vel1-127> <noteSec> <out.wav> [totalSec] [sr]");
  process.exit(2);
}
const SR = Number(srS ?? 48000);
const Q = 128;
const midi = Number(midiS), vel = Number(velS) / 127, dur = Number(durS);
const total = Math.round(Number(totalS ?? leadSeconds + dur + 2.0) * SR);
const inst = GROUP[family];
if (inst === undefined) { console.error(`unknown family ${family}`); process.exit(2); }

const { instance } = await WebAssembly.instantiate(await readFile(WASM), {});
const x = instance.exports;
const p = x.ij_engine_new(SR);
x.ij_set_track(p, 0, inst, 0.9, 0.0);
if (!Number.isFinite(leadSeconds) || leadSeconds < 0) { console.error("--lead-seconds must be >= 0"); process.exit(2); }
const isDrum = family.startsWith("drums") || family === "percussion";
const onAt = Math.round(leadSeconds * SR);
const offAt = Math.round((leadSeconds + dur) * SR);
const mono = new Float32Array(total);
const lPtr = x.ij_out_l(p);
const events = [{ frame: onAt, on: true }];
if (!isDrum) events.push({ frame: offAt, on: false });
let f = 0, ei = 0;
while (f < total) {
  while (ei < events.length && events[ei].frame <= f) {
    if (events[ei].on) x.ij_note_on(p, 0, midi, vel); else x.ij_note_off(p, 0, midi);
    ei++;
  }
  const boundary = ei < events.length ? events[ei].frame : total;
  const n = Math.min(Q, total - f, Math.max(1, boundary - f));
  x.ij_process(p, n);
  mono.set(new Float32Array(x.memory.buffer, lPtr, n), f);
  f += n;
}
const bytes = FLOAT ? 4 : 2;
const b = Buffer.alloc(44 + total * bytes);
b.write("RIFF", 0); b.writeUInt32LE(36 + total * bytes, 4); b.write("WAVE", 8);
b.write("fmt ", 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(FLOAT ? 3 : 1, 20); b.writeUInt16LE(1, 22);
b.writeUInt32LE(SR, 24); b.writeUInt32LE(SR * bytes, 28); b.writeUInt16LE(bytes, 32); b.writeUInt16LE(bytes * 8, 34);
b.write("data", 36); b.writeUInt32LE(total * bytes, 40);
for (let i = 0; i < total; i++) {
  if (FLOAT) b.writeFloatLE(mono[i], 44 + i * 4);
  else b.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(mono[i] * 32767))), 44 + i * 2);
}
await writeFile(out, b);
let peak = 0;
for (const s of mono) peak = Math.max(peak, Math.abs(s));
console.log(JSON.stringify({ out, family, midi, vel: Number(velS), peak: +peak.toFixed(3), seconds: total / SR, onsetSeconds: onAt / SR, noteOffSeconds: isDrum ? null : offAt / SR, sampleRate: SR, float32: FLOAT }));
