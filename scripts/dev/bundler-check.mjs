// Build the library into a real Vite, webpack and Next app, then make each BUILT bundle
// produce audio in a browser.
//
// PRINCIPLES #6 is "zero-config or it doesn't ship", and until this existed that was an
// assertion in a README. install-check.mjs proves the tarball is well-formed, but it
// imports by path from node_modules, which bypasses module resolution and bypasses the
// bundler entirely. Everything that actually breaks for users lives in the gap between
// those two things:
//
//   - a bare specifier that does not resolve through the "exports" map
//   - `new URL("./wasm/...", import.meta.url)` that the bundler rewrites wrongly, or
//     inlines, or fails to emit as an asset at all
//   - a worklet the bundler renames, transforms, or fails to emit — this package hands
//     `audioWorklet.addModule()` a real file URL, which is the fragile shape
//   - a package that evaluates browser-only globals during a server-side build (Next)
//
// None of those show up in the source tree. All of them show up here.
//
//     node scripts/dev/bundler-check.mjs            # all three
//     node scripts/dev/bundler-check.mjs vite       # one
import { execSync, spawn } from "node:child_process";
import { cpSync, mkdtempSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as pw from "playwright";

const ROOT = process.cwd();
const FIXTURES = join(ROOT, "scripts/dev/fixtures/bundlers");

/** Each fixture: how to build it, and where the built site lands. */
const BUNDLERS = {
  vite: { serve: "dist", port: 8411 },
  webpack: { serve: "dist", port: 8412 },
  // Next's static export writes to out/, not dist/.
  next: { serve: "out", port: 8413 },
};

const want = process.argv.slice(2).filter((a) => !a.startsWith("-"));
const names = want.length ? want : Object.keys(BUNDLERS);
for (const n of names) if (!BUNDLERS[n]) { console.error(`unknown bundler ${n}`); process.exit(2); }

// fail() calls process.exit, which SKIPS `finally` -- so every failure used to leak the
// http.server, and a leaked server holds the port and serves the PREVIOUS run's build to
// the next one. check_quickstart.mjs was caught reporting a stale result that way.
const cleanup = [];
process.on("exit", () => { for (const fn of cleanup) { try { fn(); } catch {} } });
const fail = (m) => { console.error("BUNDLER FAIL: " + m); process.exit(1); };

console.log("packing the library…");
execSync("npm run build", { cwd: ROOT, stdio: "ignore" });
execSync("node scripts/dev/stage-package-files.mjs", { cwd: ROOT, stdio: "ignore" });
const tgz = execSync("npm pack --silent", { cwd: join(ROOT, "packages/core"), encoding: "utf8" }).trim();
const tarball = join(ROOT, "packages/core", tgz);

const results = [];
try {
  for (const name of names) {
    const cfg = BUNDLERS[name];
    const work = mkdtempSync(join(tmpdir(), `subsynth-${name}-`));
    console.log(`\n── ${name} ─────────────────────────────`);

    cpSync(join(FIXTURES, name), work, { recursive: true });
    // The shared app body lives one level up so all three fixtures use the identical
    // code; copy it in rather than duplicating it three times in the tree.
    cpSync(join(FIXTURES, "app.js"), join(work, "app.js"));

    console.log("  installing (this pulls the real bundler)…");
    execSync(`npm install --no-audit --no-fund --silent "${tarball}"`,
             { cwd: work, stdio: "inherit", timeout: 600_000 });

    console.log("  building…");
    try {
      execSync("npm run build", { cwd: work, stdio: "pipe", timeout: 600_000 });
    } catch (e) {
      const out = `${e.stdout ?? ""}${e.stderr ?? ""}`.slice(-2000);
      fail(`${name} build failed:\n${out}`);
    }

    const site = join(work, cfg.serve);
    if (!existsSync(site)) fail(`${name} built nothing at ${cfg.serve}/`);

    // Serve the BUILT output. Serving the dev server instead would skip exactly the
    // asset-emission step that breaks WASM URLs.
    // A nonce only this run can serve, so a stale server squatting on the port is
    // detected rather than silently measured as if it were our build.
    const nonce = `${process.pid}-${name}-${process.hrtime.bigint()}`;
    writeFileSync(join(site, "nonce.txt"), nonce);

    const server = spawn("python3", ["-m", "http.server", String(cfg.port), "--bind", "127.0.0.1"],
                         { cwd: site, stdio: "ignore" });
    cleanup.push(() => server.kill());
    await new Promise((r) => setTimeout(r, 800));
    {
      const res = await fetch(`http://127.0.0.1:${cfg.port}/nonce.txt`).catch(() => null);
      const got = res && res.ok ? (await res.text()).trim() : null;
      if (got !== nonce) {
        fail(`something else is already serving port ${cfg.port} — its files, not the ` +
             `${name} build, would have been measured. Kill it:  lsof -ti :${cfg.port} | xargs kill`);
      }
    }
    const browser = await pw.chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
    cleanup.push(() => browser.close());
    try {
      const page = await browser.newPage();
      const errs = [];
      page.on("pageerror", (e) => errs.push(String(e)));
      page.on("console", (m) => { if (m.type() === "error") errs.push(m.text()); });
      await page.goto(`http://127.0.0.1:${cfg.port}/`, { timeout: 20000 });
      // Next mounts on an effect, so __result appears a tick after load.
      await page.waitForFunction(() => window.__result, null, { timeout: 20000 })
        .catch(() => fail(`${name}: the page never started a render` +
                          (errs.length ? ` — ${errs.slice(0, 2).join(" | ")}` : "")));
      const r = await page.evaluate(() => window.__result);
      if (errs.length) fail(`${name} page errors: ${errs.slice(0, 2).join(" | ")}`);
      if (r.bad) fail(`${name}: ${r.bad} non-finite samples`);
      if (r.rms < 0.001) fail(`${name}: built bundle produced silence — rms ${r.rms}`);
      if (r.peak > 1.0) fail(`${name}: clipped — peak ${r.peak}`);
      console.log(`  ok — rms ${r.rms.toFixed(4)}, peak ${r.peak.toFixed(3)}`);
      results.push([name, r]);
    } finally {
      await browser.close();
      server.kill();
    }
  }
} finally {
  execSync(`rm -f "${tarball}"`);
}

console.log(`\nBUNDLER OK — ${results.map(([n]) => n).join(", ")} each built the ` +
            `library with no library-specific configuration and made a sound`);
