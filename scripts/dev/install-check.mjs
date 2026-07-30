// Pack the package, install it into a throwaway project, and make a sound from it.
//
// This is the release gate for "npm install → a few lines → an orchestra". Every other
// check in this repo runs against the SOURCE TREE, where the paths happen to line up and
// nothing is missing from `files`. None of them can tell you whether what we PUBLISH
// works — a wrong `exports` map, a file left out of `files`, or an asset URL that only
// resolves relative to src/ all produce a package that fails on someone else's machine
// and passes on ours.
//
// It already earned its place before it was written: packing by hand showed the tarball
// contained NO LICENCE TEXT for a package whose manifest claims "MIT OR Apache-2.0",
// because `files` listed paths that live at the repo root and npm silently omits a listed
// path that does not exist.
//
//     node scripts/dev/install-check.mjs
//     BROWSER=webkit node scripts/dev/install-check.mjs
import { execSync, spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pw from "playwright";

const ROOT = process.cwd();
const BROWSER = process.env.BROWSER ?? "chromium";
// --autoplay-policy is Chromium-only; WebKit refuses to start on an unknown option.
const launchArgs = BROWSER === "chromium"
  ? ["--autoplay-policy=no-user-gesture-required"] : [];

// fail() calls process.exit, which SKIPS `finally`. Without a cleanup registry a failing
// run leaks its http.server, the leaked server keeps the port, and the NEXT run silently
// measures the PREVIOUS run's files. That exact bug bit the sibling project.
const cleanup = [];
process.on("exit", () => { for (const fn of cleanup) { try { fn(); } catch {} } });
const fail = (m) => { console.error("INSTALL FAIL: " + m); process.exit(1); };

console.log("packing…");
execSync("npm run build", { cwd: ROOT, stdio: "ignore" });
execSync("node scripts/dev/stage-package-files.mjs", { cwd: ROOT, stdio: "ignore" });
const tgz = execSync("npm pack --silent", { cwd: join(ROOT, "packages/core"), encoding: "utf8" }).trim();
const tarball = join(ROOT, "packages/core", tgz);
cleanup.push(() => execSync(`rm -f "${tarball}"`));

const work = mkdtempSync(join(tmpdir(), "physinst-install-"));
console.log(`installing ${tgz} into a clean project…`);
writeFileSync(join(work, "package.json"),
              JSON.stringify({ name: "consumer", private: true, type: "module" }));
execSync(`npm install --no-audit --no-fund --silent "${tarball}"`, { cwd: work, stdio: "ignore" });

const installed = join(work, "node_modules/physical-instruments.js");
const rootFiles = readdirSync(installed);
console.log("published root:", rootFiles.join(", "));
// The package ROOT, not just dist/. The licences were missing here and nothing noticed.
for (const need of ["dist", "worklet", "wasm", "README.md", "LICENSE-MIT", "LICENSE-APACHE"]) {
  if (!rootFiles.includes(need)) fail(`${need} was not published (check "files")`);
}
for (const need of ["index.js", "index.d.ts"]) {
  if (!readdirSync(join(installed, "dist")).includes(need)) fail(`dist/${need} was not published`);
}

// renderOffline gives a deterministic bounce, so this asserts real audio rather than
// "no exception was thrown" — a library can import cleanly and still be silent.
writeFileSync(join(work, "index.html"), `<!doctype html><meta charset="utf-8"><body>
<script type="module">
  import { createEngine } from "/node_modules/physical-instruments.js/dist/index.js";
  window.__result = (async () => {
    const engine = await createEngine();
    await engine.ready;
    const wav = await engine.renderOffline([
      { midiPitch: 60, startSeconds: 0.0, endSeconds: 1.2, velocity: 100, instrumentGroup: "piano" },
      { midiPitch: 64, startSeconds: 0.3, endSeconds: 1.5, velocity: 100, instrumentGroup: "piano" },
      { midiPitch: 67, startSeconds: 0.6, endSeconds: 1.8, velocity: 100, instrumentGroup: "piano" },
    ], { float32: true });

    // Decode the float32 WAV we just asked for: find "data", read the samples.
    const dv = new DataView(wav.buffer, wav.byteOffset, wav.byteLength);
    let off = 12, dataOff = -1, dataLen = 0;
    while (off + 8 <= wav.byteLength) {
      const id = String.fromCharCode(wav[off], wav[off+1], wav[off+2], wav[off+3]);
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
    return { bytes: wav.byteLength, samples: n, peak, rms: Math.sqrt(sum / n), bad };
  })();
</script></body>`);

const PORT = 8401;
// A nonce only this run can serve, so a stale server on the port is detected rather than
// silently measured as if it were ours.
const NONCE = `${process.pid}-${process.hrtime.bigint()}`;
writeFileSync(join(work, "nonce.txt"), NONCE);

const server = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1"],
                     { cwd: work, stdio: "ignore" });
cleanup.push(() => server.kill());
await new Promise((r) => setTimeout(r, 800));
{
  const res = await fetch(`http://127.0.0.1:${PORT}/nonce.txt`).catch(() => null);
  const got = res && res.ok ? (await res.text()).trim() : null;
  if (got !== NONCE) {
    fail(`something else is already serving port ${PORT} — its files, not ours, would ` +
         `have been measured. Kill it:  lsof -ti :${PORT} | xargs kill`);
  }
}

const browser = await pw[BROWSER].launch({ args: launchArgs });
cleanup.push(() => browser.close());
try {
  const page = await browser.newPage();
  const errs = [];
  page.on("pageerror", (e) => errs.push(String(e)));
  page.on("console", (m) => { if (m.type() === "error") errs.push(m.text()); });
  await page.goto(`http://127.0.0.1:${PORT}/`, { timeout: 20000 });

  let r;
  try {
    r = await page.evaluate(() => window.__result, { timeout: 60000 });
  } catch (e) {
    fail("the installed package threw: " + String(e).split("\n")[0]);
  }
  if (r.error) fail(r.error);
  console.log("installed package rendered:",
              JSON.stringify({ ...r, peak: +r.peak.toFixed(4), rms: +r.rms.toFixed(5) }));
  if (errs.length) fail("page errors: " + errs.slice(0, 2).join(" | "));
  if (r.bad) fail(`${r.bad} non-finite samples`);
  if (r.rms < 0.001) fail(`installed package produced silence — rms ${r.rms}`);
  if (r.peak > 1.0) fail(`clipped — peak ${r.peak}`);
  console.log(`INSTALL OK [${BROWSER}] — packed, installed clean, resolved its own ` +
              `WASM and worklet, and rendered audio`);
} finally {
  await browser.close();
  server.kill();
}
