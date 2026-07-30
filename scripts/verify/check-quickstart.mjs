// Run the README's quickstart, verbatim, against the packed and installed package.
//
// Release-standard area F: "a quickstart that is EXECUTED rather than written — a
// copy-pasteable snippet nobody has run is a bug report waiting." Everything that makes
// a quickstart wrong is invisible to every other gate in this repo:
//
//   - an import path that does not exist in the "exports" map
//   - a function renamed since the README was written
//   - a preset id that is no longer in the bank
//   - a snippet that only works because the author had a bundler or a dev server
//
// The README block is GENERATED from examples/quickstart.js (see scripts/gen_docs.py),
// so the file executed here and the text a reader copies cannot drift apart.
//
// This deliberately uses a LIVE AudioContext, not an OfflineAudioContext. The quickstart
// calls `createEngine()` with no arguments, which is the path a real user takes and the
// only path that exercises the default context, the default WASM URL and the Blob-URL
// worklet together. install-check.mjs already covers the offline path.
//
//     node scripts/verify/check_quickstart.mjs
import { execSync, spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, cpSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pw from "playwright";

const ROOT = process.cwd();
const BROWSER = process.env.BROWSER ?? "chromium";
const launchArgs = BROWSER === "chromium"
  ? ["--autoplay-policy=no-user-gesture-required"] : [];

// Cleanup registry. fail() calls process.exit, which SKIPS `finally` blocks -- so the
// first version of this script leaked its http.server on every failure. The leaked
// server kept the port, the next run's server silently failed to bind, and Playwright
// then loaded the PREVIOUS run's temp directory: the check reported a stale result and
// went on reporting it after the source was fixed. Caught while deliberately breaking
// the quickstart to prove this gate can fail. Same class as the six-day-old dev-server
// squatter this repo hit before, and the same fix -- clean up, then PROVE the server
// answering is ours.
const cleanup = [];
process.on("exit", () => { for (const fn of cleanup) { try { fn(); } catch {} } });
const fail = (m) => { console.error("QUICKSTART FAIL: " + m); process.exit(1); };

// The snippet must be the same text the README shows. Assert the marker is still there
// rather than trusting it, because losing it would silently publish the harness lines.
const src = readFileSync(join(ROOT, "examples/quickstart.js"), "utf8");
if (!src.includes("// --8<--")) fail("examples/quickstart.js lost its --8<-- marker");
const snippet = src.split("// --8<--")[0].trimEnd();
// BOTH READMEs. packages/core/README.md is the one npm renders on the package page, and
// it used to be an internal owner doc -- no install command, no snippet, no demo link --
// while this check happily validated the repo-root README and reported green. Found by
// the panel review before v0.1.0 shipped. A check that verifies the surface nobody sees
// is worse than no check, because it reports confidence about the wrong artefact.
for (const rel of ["README.md", "packages/core/README.md"]) {
  const readme = readFileSync(join(ROOT, rel), "utf8");
  if (!readme.includes(snippet)) {
    fail(`${rel} does not contain the quickstart from examples/quickstart.js verbatim`);
  }
  if (!/npm install physical-instruments\.js/.test(readme)) {
    fail(`${rel} has no "npm install physical-instruments.js" line above its quickstart`);
  }
}

console.log("packing…");
execSync("npm run build", { cwd: ROOT, stdio: "ignore" });
execSync("node scripts/dev/stage-package-files.mjs", { cwd: ROOT, stdio: "ignore" });
const tgz = execSync("npm pack --silent", { cwd: join(ROOT, "packages/core"), encoding: "utf8" }).trim();
const tarball = join(ROOT, "packages/core", tgz);

const work = mkdtempSync(join(tmpdir(), "subsynth-quickstart-"));
writeFileSync(join(work, "package.json"),
              JSON.stringify({ name: "consumer", private: true, type: "module" }));
execSync(`npm install --no-audit --no-fund --silent "${tarball}"`, { cwd: work, stdio: "ignore" });
cpSync(join(ROOT, "examples/quickstart.js"), join(work, "quickstart.js"));

// An import map, so the file can use the BARE specifier exactly as printed. Without one
// a browser cannot resolve "physical-instruments.js" at all — and rewriting the
// import to a path would mean testing a snippet the README never shows.
writeFileSync(join(work, "index.html"), `<!doctype html><meta charset="utf-8"><body>
<script type="importmap">
{"imports": {
  "physical-instruments.js": "/node_modules/physical-instruments.js/dist/index.js"
}}
</script>
<script type="module">
  window.__ready = (async () => {
    const { engine } = await import("/quickstart.js");
    // Tap the engine's own output node. This is documented public surface (Engine.node),
    // so the quickstart above runs completely unmodified — nothing is injected into it.
    const ctx = engine.context;
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    engine.output.connect(analyser);
    if (ctx.state === "suspended") await ctx.resume();
    await engine.ready;

    // Watch for ~1 s and keep the loudest frame. A single sample of a live context can
    // land in the attack or between callbacks and read as silence for reasons that have
    // nothing to do with the library.
    const buf = new Float32Array(analyser.fftSize);
    let peak = 0, bad = 0;
    for (let i = 0; i < 60; i++) {
      await new Promise((r) => setTimeout(r, 16));
      analyser.getFloatTimeDomainData(buf);
      for (const v of buf) {
        if (!Number.isFinite(v)) { bad++; continue; }
        if (Math.abs(v) > peak) peak = Math.abs(v);
      }
    }
    return { peak, bad, state: ctx.state };
  })();
</script></body>`);

const PORT = 8403;
// A nonce this run alone can serve, so a squatter on the port is detected rather than
// silently measured. Cheaper than picking a free port, and it catches a stale server
// left by ANY process, not just a previous run of this script.
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
  // The snippet's own rejection arrives HERE, as a rejected __ready, not as a pageerror.
  // Without this catch the check dies with a raw Playwright stack and CI shows a crash
  // rather than a finding -- verified by pointing the quickstart at a preset that does
  // not exist, which is exactly the drift this gate is for.
  let r;
  try {
    r = await page.evaluate(() => window.__ready, { timeout: 30000 });
  } catch (e) {
    fail("the quickstart threw: " + String(e).split("\n")[0].replace(/^page\.evaluate: /, ""));
  }
  if (errs.length) fail("the quickstart threw: " + errs.slice(0, 2).join(" | "));
  if (r.bad) fail(`${r.bad} non-finite samples`);
  if (r.state !== "running") fail(`context is "${r.state}", not running`);
  if (r.peak < 0.01) fail(`the quickstart made no sound — peak ${r.peak}`);
  console.log(`quickstart ran on a live context: peak ${r.peak.toFixed(3)}, context ${r.state}`);
  console.log(`QUICKSTART OK [${BROWSER}] — the snippet in the README is the snippet that ran`);
} finally {
  await browser.close();
  server.kill();
  execSync(`rm -f "${tarball}"`);
}
