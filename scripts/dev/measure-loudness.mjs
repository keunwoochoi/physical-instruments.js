#!/usr/bin/env node
/**
 * Measure per-instrument loudness at reference velocity and print the makeup-gain
 * table for crates/dsp/src/kernels.rs::makeup_gain. Loudness proxy: RMS over the
 * first 1.2 s of a family-representative phrase at velocity 0.8, track gain 1.0.
 * Reference family: marimba (gain 1.0 by definition).
 */
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const SR = 48000;
const Q = 128;
const WASM = fileURLToPath(new URL("../../packages/core/wasm/instruments_dsp.wasm", import.meta.url));

// representative material per family: [instId, [[midi, atSec], ...]]
const CASES = [
  ["marimba", 0, [[60, 0], [67, 0.3], [72, 0.6]]],
  ["vibraphone", 1, [[60, 0], [67, 0.3], [72, 0.6]]],
  ["glockenspiel", 2, [[84, 0], [91, 0.3], [96, 0.6]]],
  ["musicbox", 3, [[72, 0], [79, 0.3], [84, 0.6]]],
  ["guitar", 4, [[52, 0], [59, 0.3], [64, 0.6]]],
  ["bass", 5, [[33, 0], [40, 0.3], [36, 0.6]]],
  ["epiano", 6, [[60, 0], [64, 0.3], [67, 0.6]]],
  ["drums", 7, [[36, 0], [38, 0.3], [42, 0.45], [36, 0.6], [38, 0.9]]],
  ["synthpad", 8, [[57, 0], [60, 0], [64, 0]]],
  ["piano", 9, [[60, 0], [64, 0.3], [67, 0.6]]],
  ["guitar-steel", 10, [[52, 0], [59, 0.3], [64, 0.6]]],
  ["guitar-electric", 11, [[52, 0], [59, 0.3], [64, 0.6]]],
  ["guitar-distorted", 12, [[40, 0], [47, 0], [52, 0.5]]],
  ["drums-rock", 13, [[36, 0], [38, 0.3], [42, 0.45], [36, 0.6], [38, 0.9]]],
  ["drums-jazz", 14, [[36, 0], [38, 0.3], [42, 0.45], [36, 0.6], [38, 0.9]]],
];

// --wav-dir <dir>: also write per-family mono WAVs for the pyloudnorm (BS.1770
// LUFS) measurement in measure-loudness.py — the authoritative perceptual number.
const wavDirIdx = process.argv.indexOf("--wav-dir");
const wavDir = wavDirIdx > 0 ? process.argv[wavDirIdx + 1] : null;
const { writeFile: wf } = await import("node:fs/promises");

const { instance } = await WebAssembly.instantiate(await readFile(WASM), {});
const x = instance.exports;
const results = [];
for (const [name, inst, notes] of CASES) {
  const p = x.ij_engine_new(SR);
  x.ij_set_track(p, 0, inst, 1.0, 0.0);
  // loop the phrase 3× so short-decay families fill the LUFS gating blocks
  const events = [];
  for (let rep = 0; rep < 3; rep++)
    for (const [m, t] of notes) events.push({ f: Math.round((t + rep * 1.2) * SR), m });
  events.sort((a, b) => a.f - b.f);
  const total = Math.round(3.6 * SR);
  const mono = new Float32Array(total);
  const lPtr = x.ij_out_l(p);
  let ei = 0, sumSq = 0, peak = 0;
  for (let f = 0; f < total; f += Q) {
    while (ei < events.length && events[ei].f <= f) x.ij_note_on(p, 0, events[ei++].m, 0.8);
    const n = Math.min(Q, total - f);
    x.ij_process(p, n);
    const v = new Float32Array(x.memory.buffer, lPtr, n);
    mono.set(v, f);
    for (let i = 0; i < n; i++) {
      sumSq += v[i] * v[i];
      peak = Math.max(peak, Math.abs(v[i]));
    }
  }
  x.ij_engine_free(p);
  if (wavDir) {
    const b = Buffer.alloc(44 + total * 2);
    b.write("RIFF", 0); b.writeUInt32LE(36 + total * 2, 4); b.write("WAVE", 8);
    b.write("fmt ", 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
    b.writeUInt32LE(SR, 24); b.writeUInt32LE(SR * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
    b.write("data", 36); b.writeUInt32LE(total * 2, 40);
    for (let i = 0; i < total; i++) b.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(mono[i] * 32767))), 44 + i * 2);
    await wf(`${wavDir}/family-${name}.wav`, b);
  }
  results.push({ name, inst, rms: Math.sqrt(sumSq / total), peak });
}

const ref = results.find((r) => r.name === "marimba").rms;
console.log("family        rms(dB)   peak    suggested makeup (marimba = 1.0)");
for (const r of results) {
  const db = 20 * Math.log10(r.rms + 1e-12);
  const makeup = ref / r.rms;
  console.log(
    `${r.name.padEnd(13)} ${db.toFixed(1).padStart(6)}  ${r.peak.toFixed(3)}   ${makeup.toFixed(2)}`,
  );
}
console.log("\nPaste clamped values (≤2.5 suggested) into kernels.rs::makeup_gain.");
