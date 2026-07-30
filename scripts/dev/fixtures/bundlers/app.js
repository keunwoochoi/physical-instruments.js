// The body of every bundler fixture. Shared on purpose: if each bundler got its own
// snippet, a fixture could pass because its snippet was easier, not because its bundler
// works. One file, three build systems, one claim.
//
// The BARE SPECIFIER is the whole point. install-check.mjs imports by path, which proves
// the tarball is well-formed but bypasses module resolution entirely. Only a bundler
// resolving "physical-instruments.js" through node_modules — and then deciding what to do
// with the `new URL("../worklet/...", import.meta.url)` and `new URL("../wasm/...")`
// inside it — exercises what a real user hits.
import { createEngine } from "physical-instruments.js";

export async function run() {
  const engine = await createEngine();
  await engine.ready;
  const wav = await engine.renderOffline([
    { midiPitch: 60, startSeconds: 0.0, endSeconds: 1.2, velocity: 100, instrumentGroup: "piano" },
    { midiPitch: 64, startSeconds: 0.3, endSeconds: 1.5, velocity: 100, instrumentGroup: "piano" },
    { midiPitch: 67, startSeconds: 0.6, endSeconds: 1.8, velocity: 100, instrumentGroup: "piano" },
  ], { float32: true });

  const dv = new DataView(wav.buffer, wav.byteOffset, wav.byteLength);
  let off = 12, dataOff = -1, dataLen = 0;
  while (off + 8 <= wav.byteLength) {
    const id = String.fromCharCode(wav[off], wav[off + 1], wav[off + 2], wav[off + 3]);
    const size = dv.getUint32(off + 4, true);
    if (id === "data") { dataOff = off + 8; dataLen = size; break; }
    off += 8 + size + (size & 1);
  }
  if (dataOff < 0) return { error: "no data chunk in the WAV" };

  let peak = 0, sum = 0, bad = 0;
  const n = Math.floor(dataLen / 4);
  for (let i = 0; i < n; i++) {
    const v = dv.getFloat32(dataOff + i * 4, true);
    if (!Number.isFinite(v)) { bad++; continue; }
    if (Math.abs(v) > peak) peak = Math.abs(v);
    sum += v * v;
  }
  return { samples: n, peak, rms: Math.sqrt(sum / n), bad };
}
