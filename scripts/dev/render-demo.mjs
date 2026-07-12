#!/usr/bin/env node
/**
 * Render the demo arrangement through the WASM engine in Node — the numeric
 * verification path (and the seed of evals issue #9). Usage:
 *   node scripts/dev/render-demo.mjs [outfile.wav]
 *   node scripts/dev/render-demo.mjs --bench
 */
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { demoSong, DEMO_SONG_SECONDS } from "../../apps/playground/demo-song.mjs";

const SR = 48000;
const Q = 128;
const WASM = fileURLToPath(new URL("../../packages/core/wasm/instruments_dsp.wasm", import.meta.url));

// FULL mirror of packages/core GROUP_TO_INSTRUMENT (panel finding: a partial map
// silently rendered unknown families as marimba, diverging from the shipped path)
const GROUP = {
  marimba: 0, mallet: 0, unknown: 0, vibraphone: 1, strings_placeholder: 1,
  glockenspiel: 2, woodwind: 2, musicbox: 3, guitar: 4, bass: 5,
  epiano: 6, drums: 7, percussion: 7,
  synthpad: 8, strings: 8, brass: 8, voice: 8, synth: 8,
  piano: 9,
  "guitar-steel": 10, "guitar-electric": 11, "guitar-distorted": 12,
};

async function makeEngine() {
  const { instance } = await WebAssembly.instantiate(await readFile(WASM), {});
  const x = instance.exports;
  const p = x.ij_engine_new(SR);
  return { x, p };
}

function scheduleSong(notes) {
  const tracks = new Map();
  const events = [];
  let next = 0;
  for (const n of notes) {
    const key = n.isDrum ? "drums" : n.instrumentGroup;
    if (!tracks.has(key)) {
      tracks.set(key, next);
      events.push({ f: 0, kind: "track", track: next, inst: GROUP[key] ?? 0 });
      next++;
    }
    const t = tracks.get(key);
    events.push({ f: Math.round((0.05 + n.startSeconds) * SR), kind: "on", track: t, midi: n.midiPitch, vel: n.velocity / 127 });
    if (!n.isDrum) events.push({ f: Math.round((0.05 + n.endSeconds) * SR), kind: "off", track: t, midi: n.midiPitch });
  }
  events.sort((a, b) => a.f - b.f);
  return events;
}

// per-track pans/gains matching a reasonable mix
// single source of truth for the arrangement mix (shared with the playground)
import { DEMO_MIX } from "../../apps/playground/demo-song.mjs";
const TRACK_STYLE = Object.fromEntries(
  Object.entries(DEMO_MIX).map(([k, v]) => [k, [v.gain, v.pan]]),
);

function applyEvent(x, p, e, styleByTrack) {
  if (e.kind === "track") {
    const [gain, pan] = styleByTrack.get(e.track) ?? [0.8, 0];
    x.ij_set_track(p, e.track, e.inst, gain, pan);
  } else if (e.kind === "on") x.ij_note_on(p, e.track, e.midi, e.vel);
  else x.ij_note_off(p, e.track, e.midi);
}

function wavBytes(l, r, sr) {
  const frames = l.length;
  const data = Buffer.alloc(44 + frames * 4);
  data.write("RIFF", 0); data.writeUInt32LE(36 + frames * 4, 4); data.write("WAVE", 8);
  data.write("fmt ", 12); data.writeUInt32LE(16, 16); data.writeUInt16LE(1, 20); data.writeUInt16LE(2, 22);
  data.writeUInt32LE(sr, 24); data.writeUInt32LE(sr * 4, 28); data.writeUInt16LE(4, 32); data.writeUInt16LE(16, 34);
  data.write("data", 36); data.writeUInt32LE(frames * 4, 40);
  let o = 44;
  for (let i = 0; i < frames; i++) {
    data.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(l[i] * 32767))), o); o += 2;
    data.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(r[i] * 32767))), o); o += 2;
  }
  return data;
}

async function render() {
  const { x, p } = await makeEngine();
  const notes = demoSong();
  const events = scheduleSong(notes);
  // resolve track styles by key order used in scheduleSong
  const styleByTrack = new Map();
  {
    const seen = new Map(); let next = 0;
    for (const n of notes) {
      const key = n.isDrum ? "drums" : n.instrumentGroup;
      if (!seen.has(key)) { seen.set(key, next); styleByTrack.set(next, TRACK_STYLE[key] ?? [0.8, 0]); next++; }
    }
  }
  const total = Math.ceil((DEMO_SONG_SECONDS + 2.5) * SR);
  const L = new Float32Array(total);
  const R = new Float32Array(total);
  const lPtr = x.ij_out_l(p), rPtr = x.ij_out_r(p);
  let ei = 0, maxVoices = 0;
  const t0 = process.hrtime.bigint();
  // sample-accurate segment scheduling — mirrors the shipped worklet exactly
  let f = 0;
  while (f < total) {
    while (ei < events.length && events[ei].f <= f) applyEvent(x, p, events[ei++], styleByTrack);
    let next = Math.min(f + Q, total);
    if (ei < events.length && events[ei].f < next) next = Math.max(f + 1, events[ei].f);
    const n = next - f;
    x.ij_process(p, n);
    L.set(new Float32Array(x.memory.buffer, lPtr, n), f);
    R.set(new Float32Array(x.memory.buffer, rPtr, n), f);
    maxVoices = Math.max(maxVoices, x.ij_active_voices(p));
    f = next;
  }
  const elapsedMs = Number(process.hrtime.bigint() - t0) / 1e6;

  // ---- stats over BOTH channels (the "did it render music, cleanly?" report) ----
  let peak = 0, sumSq = 0, nan = 0, maxJump = 0;
  for (const ch of [L, R]) {
    for (let i = 0; i < total; i++) {
      const s = ch[i];
      if (Number.isNaN(s)) nan++;
      const a = Math.abs(s);
      if (a > peak) peak = a;
      sumSq += (s * s) / 2;
      if (i > 0) maxJump = Math.max(maxJump, Math.abs(s - ch[i - 1]));
    }
  }
  const rms = Math.sqrt(sumSq / total);
  const quanta = total / Q;
  const usPerQuantum = (elapsedMs * 1000) / quanta;
  const stats = {
    seconds: +(total / SR).toFixed(2),
    peak: +peak.toFixed(3),
    rmsDb: +(20 * Math.log10(rms + 1e-12)).toFixed(1),
    nanSamples: nan,
    maxSampleJump: +maxJump.toFixed(3),
    maxVoices,
    renderXRealtime: +((total / SR) / (elapsedMs / 1000)).toFixed(1),
    usPerQuantumNode: +usPerQuantum.toFixed(1),
    budgetUs: 2666.7,
    budgetPct: +((usPerQuantum / 2666.7) * 100).toFixed(2),
  };
  return { L, R, stats };
}

if (process.argv.includes("--bench")) {
  // sustained worst-case-ish: 4 tracks, re-trigger to hold ~48 voices
  const { x, p } = await makeEngine();
  x.ij_set_track(p, 0, 0, 0.7, -0.3); x.ij_set_track(p, 1, 5, 0.8, 0);
  x.ij_set_track(p, 2, 7, 0.8, 0.1); x.ij_set_track(p, 3, 6, 0.6, 0.3);
  const t0 = process.hrtime.bigint();
  const quanta = 4000; // ~10.7 s
  for (let i = 0; i < quanta; i++) {
    if (i % 8 === 0) { // dense retriggering across tracks
      x.ij_note_on(p, 0, 48 + (i % 36), 0.9);
      x.ij_note_on(p, 1, 28 + (i % 24), 0.9);
      x.ij_note_on(p, 2, [36, 38, 42, 46][i % 4], 1.0);
      x.ij_note_on(p, 3, 52 + (i % 24), 0.8);
    }
    x.ij_process(p, Q);
  }
  const us = Number(process.hrtime.bigint() - t0) / 1e3 / quanta;
  console.log(JSON.stringify({
    bench: "sustained multi-track retrigger", quanta,
    activeVoicesAtEnd: x.ij_active_voices(p),
    usPerQuantum: +us.toFixed(1), budgetUs: 2666.7, budgetPct: +((us / 2666.7) * 100).toFixed(2),
  }, null, 2));
} else {
  const { L, R, stats } = await render();
  console.log(JSON.stringify(stats, null, 2));
  // hard gates (CI smoke): silent, clipping, NaN, or budget-blown renders fail loudly
  const fail = [];
  if (stats.nanSamples > 0) fail.push(`NaN samples: ${stats.nanSamples}`);
  if (stats.peak < 0.05) fail.push(`near-silent render: peak ${stats.peak}`);
  if (stats.peak > 0.99) fail.push(`clipping: peak ${stats.peak}`);
  if (stats.budgetPct > 50) fail.push(`over 50% of audio budget: ${stats.budgetPct}%`);
  if (fail.length) {
    console.error("RENDER GATES FAILED:\n - " + fail.join("\n - "));
    process.exit(1);
  }
  const out = process.argv[2];
  if (out) {
    await writeFile(out, wavBytes(L, R, SR));
    console.log(`wrote ${out}`);
  }
}
