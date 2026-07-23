// Offline multi-track render of a beta demo (demos.mjs) through the current wasm,
// for headroom/sanity checks. Usage: node scripts/dev/render-arr.mjs <demoId> <out.wav>
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { DEMOS, processMidi } from "../../apps/playground/demos.mjs";
import { parseMidi } from "../../packages/midi/dist/index.js";

const WASM = fileURLToPath(new URL("../../packages/core/wasm/instruments_dsp.wasm", import.meta.url));
const IDX = {
  marimba: 0, vibraphone: 1, glockenspiel: 2, musicbox: 3, guitar: 4, bass: 5,
  epiano: 6, drums: 7, percussion: 7, synthpad: 8, strings: 8, piano: 9,
  "guitar-steel": 10, "guitar-electric": 11, "guitar-distorted": 12,
  "drums-rock": 13, "drums-jazz": 14, cello: 15, trombone: 16, violin: 17,
  viola: 18, contrabass: 19, trumpet: 20, organ: 23,
  synth: 8, woodwind: 23, brass: 20, strings8: 8, harp: 27, vibraphone: 1,
  xylophone: 24, celesta: 26, glockenspiel2: 2,
};
const DRUM = new Set(["drums", "percussion", "drums-rock", "drums-jazz"]);
const drumGroup = (g) => (["drums", "drums-rock", "drums-jazz"].includes(g) ? g : "drums");

const id = process.argv[2], out = process.argv[3];
const demo = DEMOS.find((d) => d.id === id);
if (!demo) { console.error(`no demo ${id}; have: ${DEMOS.map((d) => d.id).join(", ")}`); process.exit(2); }
const SR = 48000, Q = 128;
const midiPath = fileURLToPath(new URL("../../apps/playground/" + demo.midi.replace(/^\.\//, ""), import.meta.url));
const notes = processMidi(parseMidi((await readFile(midiPath)).buffer), demo);
const total = Math.round((Math.max(...notes.map((n) => n.endSeconds)) + 2.5) * SR);

const { instance } = await WebAssembly.instantiate(await readFile(WASM), {});
const x = instance.exports;
const p = x.ij_engine_new(SR);

// one track per resolved group; apply the demo mix (gain/pan)
const trackOf = new Map();
let nextTrack = 0;
const SCALE = Number(process.env.SCALE ?? 1);
const resolve = (rawGroup, isDrum) => {
  const g = isDrum ? drumGroup(rawGroup) : rawGroup;
  if (!trackOf.has(g)) {
    const t = nextTrack++;
    const m = demo.mix?.[g] ?? demo.mix?.[rawGroup] ?? {};
    x.ij_set_track(p, t, IDX[g] ?? 0, (m.gain ?? 0.8) * SCALE, m.pan ?? 0);
    trackOf.set(g, t);
  }
  return trackOf.get(g);
};

// build a sorted event list (frame, fn)
const ev = [];
for (const nt of notes) {
  const isDrum = !!nt.isDrum || DRUM.has(nt.instrumentGroup);
  const tr = resolve(nt.instrumentGroup, isDrum);
  const on = Math.round(nt.startSeconds * SR);
  ev.push({ f: on, go: () => x.ij_note_on(p, tr, Math.round(nt.midiPitch), Math.max(1, Math.min(127, nt.velocity)) / 127) });
  if (!isDrum) ev.push({ f: Math.round(nt.endSeconds * SR), go: () => x.ij_note_off(p, tr, Math.round(nt.midiPitch)) });
}
ev.sort((a, b) => a.f - b.f);

const lPtr = x.ij_out_l(p), rPtr = x.ij_out_r(p);
const L = new Float32Array(total), R = new Float32Array(total);
let f = 0, ei = 0;
while (f < total) {
  while (ei < ev.length && ev[ei].f <= f) ev[ei++].go();
  const n = Math.min(Q, total - f);
  x.ij_process(p, n);
  L.set(new Float32Array(x.memory.buffer, lPtr, n), f);
  R.set(new Float32Array(x.memory.buffer, rPtr, n), f);
  f += n;
}
// interleave stereo, 16-bit
const b = Buffer.alloc(44 + total * 4);
b.write("RIFF", 0); b.writeUInt32LE(36 + total * 4, 4); b.write("WAVE", 8);
b.write("fmt ", 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(2, 22);
b.writeUInt32LE(SR, 24); b.writeUInt32LE(SR * 4, 28); b.writeUInt16LE(4, 32); b.writeUInt16LE(16, 34);
b.write("data", 36); b.writeUInt32LE(total * 4, 40);
let peak = 0, sum = 0;
for (let i = 0; i < total; i++) {
  peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
  sum += L[i] * L[i] + R[i] * R[i];
  b.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(L[i] * 32767))), 44 + i * 4);
  b.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(R[i] * 32767))), 44 + i * 4 + 2);
}
if (out) await writeFile(out, b);
const rms = Math.sqrt(sum / (2 * total));
console.log(JSON.stringify({ id, notes: notes.length, tracks: nextTrack, seconds: +(total / SR).toFixed(1), peak: +peak.toFixed(3), rms_db: +(20 * Math.log10(rms)).toFixed(1) }));
