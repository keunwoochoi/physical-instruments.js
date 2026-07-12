#!/usr/bin/env node
/**
 * Standardized per-family audition sets (loop audit 2026-07-12): fixed notes,
 * velocities, phrases, and FILENAMES every round, so the owner can A/B rounds
 * by ear (pair two output dirs with scripts/dev/ab-page.mjs).
 *
 *   node scripts/dev/render-auditions.mjs <family|all> <outdir> [sampleRate]
 *
 * Float32 stereo 48 kHz. One engine per family; drums map GM notes.
 */
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const SR = Number(process.argv[4] ?? 48000), Q = 128;
if (!Number.isFinite(SR) || SR < 8000 || SR > 192000) { console.error("sampleRate must be 8000..192000"); process.exit(2); }
const WASM = fileURLToPath(new URL("../../packages/core/wasm/instruments_dsp.wasm", import.meta.url));

const GROUP = {
  marimba: 0, vibraphone: 1, glockenspiel: 2, musicbox: 3, guitar: 4, bass: 5,
  epiano: 6, drums: 7, synthpad: 8, piano: 9,
  "guitar-steel": 10, "guitar-electric": 11, "guitar-distorted": 12,
  "drums-rock": 13, "drums-jazz": 14, "drums-808": 15,
};

/** n = single note {m, v, t, d}; phrases = arrays of them. Times in beats of 0.5 s. */
const single = (m, v, d = 3.0) => [{ m, v, t: 0, d }];
const arp = (root, vels = 84) => [0, 4, 7, 12, 7, 4, 0].map((iv, i) => ({ m: root + iv, v: vels, t: i * 0.55, d: 0.5 }));
const KIT = (ids) => Object.entries(ids).map(([name, m]) => ({ name, notes: single(m, 100, 0.3), drum: true, tail: 4 }));

const SETS = {
  piano: [
    { name: "single-A1-mf", notes: single(33, 90, 6) }, { name: "single-C4-mf", notes: single(60, 90, 5) },
    { name: "single-Cs5-mf", notes: single(73, 90, 4) },
    { name: "single-C4-pp", notes: single(60, 30, 5) }, { name: "single-C4-ff", notes: single(60, 122, 5) },
    { name: "arpeggio", notes: arp(57) },
  ],
  guitar: [
    { name: "single-E2-mf", notes: single(40, 84, 4) }, { name: "single-A3-mf", notes: single(57, 84, 3.5) },
    { name: "single-C5-mf", notes: single(72, 84, 3) }, { name: "single-A3-pp", notes: single(57, 36, 3.5) },
    { name: "single-A3-ff", notes: single(57, 120, 3.5) }, { name: "arpeggio", notes: arp(45) },
  ],
  bass: [
    { name: "single-E1-mf", notes: single(28, 88, 4) }, { name: "single-A1-mf", notes: single(33, 88, 4) },
    { name: "single-D2-mf", notes: single(38, 88, 3.5) }, { name: "single-A1-pp", notes: single(33, 38, 4) },
    { name: "single-A1-ff", notes: single(33, 122, 4) },
    { name: "groove", notes: [0, 0.5, 1, 1.75, 2, 2.5, 3, 3.5].map((t, i) => ({ m: [28, 28, 35, 33, 28, 40, 38, 35][i], v: 84 + (i % 2) * 20, t: t * 0.6, d: 0.5 })) },
  ],
  epiano: [
    { name: "single-C3-mf", notes: single(48, 84, 4) }, { name: "single-C4-mf", notes: single(60, 84, 4) },
    { name: "single-C4-ff", notes: single(60, 120, 4) }, { name: "arpeggio", notes: arp(60) },
  ],
  marimba: [
    { name: "single-C3-mf", notes: single(48, 90, 3) }, { name: "single-C5-mf", notes: single(72, 90, 2) },
    { name: "arpeggio", notes: arp(60, 92) },
  ],
  vibraphone: [{ name: "single-C4-mf", notes: single(60, 90, 5) }, { name: "arpeggio", notes: arp(60, 84) }],
  glockenspiel: [{ name: "single-C6-mf", notes: single(84, 90, 3) }, { name: "arpeggio", notes: arp(84, 84) }],
  musicbox: [{ name: "single-C5-mf", notes: single(72, 90, 4) }, { name: "arpeggio", notes: arp(72, 84) }],
  synthpad: [{ name: "chord", notes: [57, 60, 64].map((m) => ({ m, v: 70, t: 0, d: 4 })), tail: 3 }],
  drums: KIT({ kick: 36, snare: 38, "hat-closed": 42, "hat-open": 46, ride: 51, crash: 49, "tom-mid": 47 }),
  "drums-rock": KIT({ kick: 36, snare: 38, "hat-closed": 42, "hat-open": 46, ride: 51, crash: 49, "tom-mid": 47 }),
  "drums-jazz": KIT({ kick: 36, snare: 38, "hat-closed": 42, "hat-open": 46, ride: 51, crash: 49, "tom-mid": 47 }),
  "drums-808": KIT({ kick: 36, snare: 38, clap: 39, "hat-closed": 42, "hat-open": 46, cymbal: 49, cowbell: 56, "tom-mid": 47, "conga-mid": 63 }),
};
// velocity ladders for drum kicks/snares (the voices under active listening)
for (const kit of ["drums", "drums-rock", "drums-jazz", "drums-808"]) {
  for (const v of [30, 75, 120]) SETS[kit].push({ name: `kick-v${v}`, notes: single(36, v, 0.3), drum: true, tail: 4 });
  SETS[kit].push({ name: "groove", drum: true, tail: 2, notes: [
    { m: 36, v: 104, t: 0 }, { m: 42, v: 60, t: 0 }, { m: 42, v: 48, t: 0.3 }, { m: 38, v: 96, t: 0.6 },
    { m: 42, v: 60, t: 0.6 }, { m: 42, v: 48, t: 0.9 }, { m: 36, v: 92, t: 1.2 }, { m: 42, v: 60, t: 1.2 },
    { m: 36, v: 84, t: 1.5 }, { m: 42, v: 48, t: 1.5 }, { m: 38, v: 100, t: 1.8 }, { m: 46, v: 72, t: 2.1 },
  ].map((n) => ({ ...n, d: 0.2 })) });
}
for (const [name, midi] of [["snare", 38], ["clap", 39], ["hat-closed", 42]]) {
  for (const v of [30, 75, 120]) SETS["drums-808"].push({ name: `${name}-v${v}`, notes: single(midi, v, 0.2), drum: true, tail: 2 });
}
SETS["drums-808"].push({ name: "repeated-hats", drum: true, tail: 1, notes: Array.from({ length: 16 }, (_, i) => ({ m: 42, v: 76, t: i * 0.12, d: 0.08 })) });
// electrics share the guitar set shape but hold longer (amp sustain is under review)
SETS["guitar-steel"] = SETS.guitar;
SETS["guitar-electric"] = SETS.guitar.map((c) => ({ ...c, notes: c.notes.map((n) => ({ ...n, d: n.d + 1 })) }));
SETS["guitar-distorted"] = [
  { name: "single-E2-ff", notes: single(40, 118, 5) }, { name: "single-A3-ff", notes: single(57, 118, 5) },
  { name: "power-chord", notes: [40, 47, 52].map((m) => ({ m, v: 112, t: 0, d: 3.5 })), tail: 2 },
];

function wav(L, R) {
  const n = L.length, data = 8 * n, b = Buffer.alloc(44 + data);
  b.write("RIFF", 0); b.writeUInt32LE(36 + data, 4); b.write("WAVE", 8); b.write("fmt ", 12);
  b.writeUInt32LE(16, 16); b.writeUInt16LE(3, 20); b.writeUInt16LE(2, 22); b.writeUInt32LE(SR, 24);
  b.writeUInt32LE(SR * 8, 28); b.writeUInt16LE(8, 32); b.writeUInt16LE(32, 34); b.write("data", 36);
  b.writeUInt32LE(data, 40);
  for (let i = 0; i < n; i++) { b.writeFloatLE(L[i], 44 + i * 8); b.writeFloatLE(R[i], 48 + i * 8); }
  return b;
}

const [fam, outdir] = process.argv.slice(2);
if (!fam || !outdir) { console.error("usage: render-auditions.mjs <family|all> <outdir> [sampleRate]"); process.exit(1); }
await mkdir(outdir, { recursive: true });
const bytes = await readFile(WASM);
const families = fam === "all" ? Object.keys(SETS) : [fam];

for (const f of families) {
  const set = SETS[f];
  if (!set) { console.error(`unknown family ${f}`); process.exit(1); }
  for (const c of set) {
    const { instance } = await WebAssembly.instantiate(bytes, {});
    const x = instance.exports, p = x.ij_engine_new(SR);
    x.ij_set_track(p, 0, GROUP[f], 0.85, 0);
    const evs = [];
    let tEnd = 0;
    for (const n of c.notes) {
      evs.push({ f: Math.round((0.03 + n.t) * SR), on: true, m: n.m, v: n.v / 127 });
      if (!c.drum) evs.push({ f: Math.round((0.03 + n.t + n.d) * SR), on: false, m: n.m });
      tEnd = Math.max(tEnd, n.t + (n.d ?? 0.3));
    }
    evs.sort((a, b) => a.f - b.f);
    const total = Math.ceil((tEnd + (c.tail ?? 3)) * SR / Q) * Q;
    const L = new Float32Array(total), R = new Float32Array(total);
    const lp = x.ij_out_l(p), rp = x.ij_out_r(p);
    let ei = 0;
    for (let fr = 0; fr < total; fr += Q) {
      while (ei < evs.length && evs[ei].f < fr + Q) {
        const e = evs[ei++];
        if (e.on) x.ij_note_on(p, 0, e.m, e.v); else x.ij_note_off(p, 0, e.m);
      }
      x.ij_process(p, Q);
      L.set(new Float32Array(x.memory.buffer, lp, Q), fr);
      R.set(new Float32Array(x.memory.buffer, rp, Q), fr);
    }
    let peak = 0, nan = 0;
    for (let i = 0; i < total; i++) {
      const m = Math.max(Math.abs(L[i]), Math.abs(R[i]));
      if (m > peak) peak = m;
      if (Number.isNaN(L[i]) || Number.isNaN(R[i])) nan++;
    }
    const path = `${outdir}/${f}--${c.name}.wav`;
    await writeFile(path, wav(L, R));
    console.log(`${path}  peak=${peak.toFixed(3)}${nan ? `  NaN=${nan} ⚠` : ""}`);
    if (nan) process.exit(1);
  }
}
