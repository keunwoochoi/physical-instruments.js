#!/usr/bin/env node
/**
 * Same-position A/B between two commits, for one instrument.
 *
 *   node scripts/dev/ab-compare.mjs --before <ref> [--after <ref>] [--instrument piano]
 *   npm run ab:piano                       # HEAD vs its first parent
 *
 * This exists because PR #41 sat in draft for three days, moved every metric it set out
 * to move, and turned out to be *audibly nothing* the first time a human heard it:
 * "they were very very very similar." The difference measured 20 dB below the signal.
 *
 * A metric delta is not a sound. This is the cheapest possible way to find that out, and
 * it should be run BEFORE a quality claim, not after a merge.
 *
 * What it does:
 *   1. builds the WASM at each ref in a throwaway worktree
 *   2. renders an identical stimulus set through both
 *   3. reports how big the difference actually is, per section, in dB below signal
 *   4. emits a page where switching KEEPS ITS PLACE in the take (restarting hides
 *      exactly the differences you are listening for) and can hide the labels
 *
 * Both renders share ONE fixed gain. Per-file normalisation is not offered, because it
 * would hide level changes -- and a level change is often the whole edit.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const SR = 48000;
const Q = 128;

const argv = process.argv.slice(2);
const arg = (k, d) => {
  const i = argv.indexOf(k);
  return i >= 0 ? argv[i + 1] : d;
};

const INSTRUMENTS = {
  piano: 9,
  epiano: 6,
  guitar: 4,
  bass: 5,
  cello: 15,
  trombone: 16,
  marimba: 0,
};

const instName = arg("--instrument", "piano");
const inst = INSTRUMENTS[instName];
if (inst === undefined) {
  console.error(`unknown instrument "${instName}". known: ${Object.keys(INSTRUMENTS).join(", ")}`);
  process.exit(1);
}

const sh = (cmd, args, cwd = ROOT) =>
  execFileSync(cmd, args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();

const after = arg("--after", sh("git", ["rev-parse", "HEAD"]));
const before = arg("--before", sh("git", ["rev-parse", `${after}^`]));
const shortRef = (r) => sh("git", ["rev-parse", "--short", r]);

const OUT = join(ROOT, "scratch-ab");
rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

/** Build the shipped WASM at a ref, in a worktree we throw away. */
function wasmAt(ref, tag) {
  const wt = join(OUT, `wt-${tag}`);
  sh("git", ["worktree", "add", "--detach", wt, ref]);
  try {
    execFileSync("cargo", ["build", "-q", "-p", "instruments-dsp", "--target", "wasm32-unknown-unknown", "--release"], {
      cwd: wt,
      stdio: ["ignore", "ignore", "inherit"],
    });
    return readFileSync(join(wt, "target/wasm32-unknown-unknown/release/instruments_dsp.wasm"));
  } finally {
    // leave the worktree; caller prunes at the end
  }
}

/**
 * The stimuli that actually decide a keyboard/sustained instrument, in the order a
 * listener notices them. Isolated notes alone are how you arrive at something beautiful
 * in a spectrogram and lifeless in a phrase.
 */
function stimuli() {
  const ev = [];
  const on = (t, m, v) => ev.push({ f: Math.round(t * SR), k: "on", m, v });
  const off = (t, m) => ev.push({ f: Math.round(t * SR), k: "off", m });
  const ped = (t, d) => ev.push({ f: Math.round(t * SR), k: "ped", d });
  const secs = [];
  let t = 0.3;

  secs.push({ name: "velocity ladder (pp→ff)", t });
  for (const v of [0.15, 0.4, 0.7, 1.0]) { on(t, 60, v); off(t + 0.7, 60); t += 0.9; }
  t += 0.4;

  secs.push({ name: "register anchors", t });
  for (const m of [33, 45, 60, 76, 88]) { on(t, m, 0.6); off(t + 0.7, m); t += 0.85; }
  t += 0.4;

  secs.push({ name: "chord (accumulation)", t });
  for (const m of [48, 55, 60, 64, 67]) on(t, m, 0.85);
  for (const m of [48, 55, 60, 64, 67]) off(t + 2.2, m);
  t += 3.0;

  secs.push({ name: "repeated note", t });
  for (let i = 0; i < 6; i++) { on(t + i * 0.22, 64, 0.75); off(t + i * 0.22 + 0.18, 64); }
  t += 2.0;

  secs.push({ name: "phrase + pedal", t });
  ped(t, 1);
  const mel = [60, 64, 67, 72, 71, 67, 64, 60];
  mel.forEach((m, i) => { on(t + 0.05 + i * 0.42, m, 0.65); off(t + 0.05 + i * 0.42 + 0.35, m); });
  t += mel.length * 0.42 + 1.2;
  ped(t, 0);

  ev.sort((a, b) => a.f - b.f);
  return { ev, secs, dur: t + 2.5 };
}

async function render(wasm, { ev, dur }) {
  const { instance } = await WebAssembly.instantiate(wasm, {});
  const x = instance.exports;
  const p = x.ij_engine_new(SR);
  x.ij_set_track(p, 0, inst, 1.0, 0.0);
  const total = Math.ceil((dur * SR) / Q);
  const L = new Float32Array(total * Q);
  const R = new Float32Array(total * Q);
  const lp = x.ij_out_l(p), rp = x.ij_out_r(p);
  let ei = 0, peak = 0, nan = 0;
  for (let q = 0; q < total; q++) {
    const f0 = q * Q;
    while (ei < ev.length && ev[ei].f < f0 + Q) {
      const e = ev[ei++];
      if (e.k === "on") x.ij_note_on(p, 0, e.m, e.v);
      else if (e.k === "off") x.ij_note_off(p, 0, e.m);
      else x.ij_pedal(p, 0, e.d);
    }
    x.ij_process(p, Q);
    const l = new Float32Array(x.memory.buffer, lp, Q);
    const r = new Float32Array(x.memory.buffer, rp, Q);
    L.set(l, f0); R.set(r, f0);
    for (let i = 0; i < Q; i++) {
      if (!Number.isFinite(l[i])) nan++;
      peak = Math.max(peak, Math.abs(l[i]), Math.abs(r[i]));
    }
  }
  return { L, R, peak, nan };
}

function wav(L, R, gain) {
  const n = L.length;
  const b = Buffer.alloc(44 + n * 4);
  b.write("RIFF", 0); b.writeUInt32LE(36 + n * 4, 4); b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(2, 22);
  b.writeUInt32LE(SR, 24); b.writeUInt32LE(SR * 4, 28); b.writeUInt16LE(4, 32);
  b.writeUInt16LE(16, 34); b.write("data", 36); b.writeUInt32LE(n * 4, 40);
  const clamp = (v) => Math.max(-32768, Math.min(32767, (v * gain * 32767) | 0));
  for (let i = 0; i < n; i++) {
    b.writeInt16LE(clamp(L[i]), 44 + i * 4);
    b.writeInt16LE(clamp(R[i]), 46 + i * 4);
  }
  return b;
}

const db = (x) => 20 * Math.log10(Math.max(x, 1e-12));
const rms = (a, s, e) => { let x = 0; for (let i = s; i < e; i++) x += a[i] * a[i]; return Math.sqrt(x / (e - s)); };
const drms = (a, b, s, e) => { let x = 0; for (let i = s; i < e; i++) { const d = a[i] - b[i]; x += d * d; } return Math.sqrt(x / (e - s)); };

const st = stimuli();
console.log(`A/B  ${instName}   before=${shortRef(before)}  after=${shortRef(after)}\n`);

const wA = wasmAt(before, "before");
const wB = wasmAt(after, "after");
const A = await render(wA, st);
const B = await render(wB, st);

// ONE shared gain. Per-file normalisation would hide level changes, and a level change is
// often the entire edit (PR #41's headline was a -5 dB trim).
const gain = 0.85 / Math.max(A.peak, B.peak, 1e-9);
writeFileSync(join(OUT, "before.wav"), wav(A.L, A.R, gain));
writeFileSync(join(OUT, "after.wav"), wav(B.L, B.R, gain));

const n = Math.min(A.L.length, B.L.length);
const overall = db(rms(B.L, 0, n)) - db(rms(A.L, 0, n));
console.log(`  overall level change: ${overall >= 0 ? "+" : ""}${overall.toFixed(2)} dB`);
console.log(`  NaN: before ${A.nan}, after ${B.nan}`);
console.log(`\n  section                     difference, in dB BELOW that section's own signal`);
console.log(`  (a broadband difference much under -20 dB is at or below the threshold of audibility)\n`);
const rows = [];
for (let i = 0; i < st.secs.length; i++) {
  const s = Math.round(st.secs[i].t * SR);
  const e = Math.round((i + 1 < st.secs.length ? st.secs[i + 1].t : st.dur) * SR);
  const rel = db(drms(A.L, B.L, s, e)) - db(rms(A.L, s, e));
  rows.push({ ...st.secs[i], rel });
  const bar = "#".repeat(Math.max(0, Math.round((rel + 45) / 2)));
  console.log(`  ${st.secs[i].name.padEnd(26)} ${rel.toFixed(1).padStart(6)} dB  ${bar}`);
}
const loudest = rows.reduce((a, b) => (b.rel > a.rel ? b : a));
console.log(`\n  biggest change: "${loudest.name}" at ${loudest.rel.toFixed(1)} dB below signal`);
if (loudest.rel < -18) {
  console.log(`  ⚠️  NOTHING here exceeds -18 dB. Whatever the metrics say, this is likely INAUDIBLE.`);
}

const page = readFileSync(join(ROOT, "scripts/dev/ab-page.html"), "utf8")
  .replace("__TITLE__", `${instName}: ${shortRef(before)} → ${shortRef(after)}`)
  .replace("__SECTIONS__", JSON.stringify(rows.map((r) => ({ name: r.name, t: r.t }))))
  .replace("__BEFORE__", shortRef(before))
  .replace("__AFTER__", shortRef(after));
writeFileSync(join(OUT, "index.html"), page);

sh("git", ["worktree", "prune"]);
console.log(`\n  page: ${join(OUT, "index.html")}`);
console.log(`  serve: (cd ${OUT} && python3 -m http.server 8899) then open http://localhost:8899/`);
