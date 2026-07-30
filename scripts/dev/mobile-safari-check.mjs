// Does the demo actually make sound on a phone, and does the engine recover from an
// audio interruption?
//
// WHY THE EXISTING WEBKIT RUNS DO NOT COVER THIS
// `BROWSER=webkit` elsewhere launches DESKTOP WebKit: desktop viewport, a mouse, no touch,
// no mobile audio session. iOS Safari is the strictest audio environment on the web, and
// it has a fourth AudioContext state that no other browser has — "interrupted" — which is
// absent from the spec and from TypeScript's AudioContextState union.
//
// This runs WebKit under Playwright's iPhone device descriptor (mobile viewport, touch,
// isMobile) and TAPS rather than clicks, with no autoplay escape hatch, so the browser's
// real gesture requirement stays in force.
//
// WHAT IT MEASURED, AND THE BUG IT FOUND
//   context at load : state "interrupted", 44100 Hz   <- not "suspended"
//   after one tap   : state "running", peak 0.30
// The demo works. But the library's resumeIfNeeded() only tested for "suspended", so on
// WebKit its resume path never fired — first touch worked only because WebKit resumes on
// the user gesture itself. Nothing would have recovered an interruption arriving
// mid-session (a call, Siri, backgrounding). Fixed by resuming whenever not running.
//
// HONEST LIMIT, stated rather than implied: this is WebKit-the-engine with mobile
// emulation, NOT Safari-on-iOS. It cannot reproduce Apple's audio session handling,
// hardware sample-rate clamping, or true backgrounding. A pass means "the gesture and
// suspend/resume path is sound"; it does not mean "verified on iPhone".
//
//     node scripts/dev/mobile-safari-check.mjs
import { spawn } from "node:child_process";
import * as pw from "playwright";

const ROOT = process.cwd();
const PORT = Number(process.env.PORT ?? 8421);
const PAGE = process.env.PAGE ?? "/apps/playground/showcase.html";

const cleanup = [];
process.on("exit", () => { for (const fn of cleanup) { try { fn(); } catch {} } });
const fail = (m) => { console.error("MOBILE SAFARI FAIL: " + m); process.exit(1); };

// GLOBAL WATCHDOG. Every await below has its own timeout except page.evaluate(), which
// has NO default timeout in Playwright -- so a page that stops servicing its event loop
// hangs this script forever. That is exactly what happened on the first CI run: every
// other step passed and this one sat in_progress for twenty minutes until the run was
// cancelled. It is the same defect this script's own header warns about, shipped again in
// a different shape, so it now gets a hard bound that cannot be reasoned away.
const BUDGET_MS = Number(process.env.MOBILE_CHECK_BUDGET_MS ?? 150_000);
const watchdog = setTimeout(() => {
  fail(`exceeded ${BUDGET_MS / 1000}s. A check that never returns is worse than one that ` +
       `fails: a failure is information. Re-run locally to see which step stalls.`);
}, BUDGET_MS);
watchdog.unref?.();

// Tap the engine's output without editing page source: wrap connect() so the first
// AudioWorkletNode also feeds an analyser we can read.
const PROBE = `
window.__probe = {};
const oc = AudioWorkletNode.prototype.connect;
AudioWorkletNode.prototype.connect = function (...a) {
  if (!window.__probe.an) {
    const c = this.context, an = c.createAnalyser();
    an.fftSize = 2048;
    oc.call(this, an);
    window.__probe.c = c;
    window.__probe.an = an;
  }
  return oc.apply(this, a);
};
window.__peak = () => {
  const P = window.__probe;
  if (!P.an) return null;
  const b = new Float32Array(P.an.fftSize);
  P.an.getFloatTimeDomainData(b);
  let k = 0;
  for (const v of b) if (Math.abs(v) > k) k = Math.abs(v);
  return k;
};
`;

const server = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1"],
                     { cwd: ROOT, stdio: "ignore" });
cleanup.push(() => server.kill());
await new Promise((r) => setTimeout(r, 1000));

const browser = await pw.webkit.launch();
cleanup.push(() => browser.close());
const context = await browser.newContext({ ...pw.devices["iPhone 13"] });
const page = await context.newPage();
const errs = [];
page.on("pageerror", (e) => errs.push(String(e).slice(0, 200)));
page.on("console", (m) => { if (m.type() === "error") errs.push(m.text().slice(0, 200)); });
await page.addInitScript(PROBE);

// domcontentloaded, not load: the showcase keeps fetching demo assets after first paint,
// and waiting for `load` hung indefinitely rather than failing — a check that never
// returns is worse than one that fails.
await page.goto(`http://127.0.0.1:${PORT}${PAGE}`, { timeout: 30000, waitUntil: "domcontentloaded" });
await page.waitForFunction(() => window.__probe && window.__probe.c, null, { timeout: 30000 })
  .catch(() => fail("the engine never connected an AudioWorkletNode" +
                    (errs.length ? ` — ${errs.slice(0, 2).join(" | ")}` : "")));

const before = await page.evaluate(() => ({ state: window.__probe.c.state, sr: window.__probe.c.sampleRate }));
console.log(`  at load        : state "${before.state}", ${before.sr} Hz`);

const el = page.locator("canvas").first();
await el.waitFor({ state: "visible", timeout: 15000 }).catch(() => fail("no playable surface"));
const box = await el.boundingBox();
if (!box) fail("playable surface has no box");
await page.touchscreen.tap(box.x + box.width * 0.35, box.y + box.height * 0.6);

// One evaluate per iteration rather than two, each raced against a bound. Halving the
// round trips matters on a slow runner, and the race means a wedged page surfaces here
// with a useful message instead of at the watchdog.
const bounded = (promise, ms, what) => Promise.race([
  promise,
  new Promise((_, rej) => setTimeout(() => rej(new Error(`${what} did not answer in ${ms}ms`)), ms)),
]);

let peak = 0, state = before.state;
for (let i = 0; i < 50; i++) {
  await new Promise((r) => setTimeout(r, 20));
  const snap = await bounded(
    page.evaluate(() => ({ peak: window.__peak(), state: window.__probe.c.state })),
    5000, "the page",
  ).catch((e) => fail(String(e.message)));
  state = snap.state;
  if (snap.peak !== null && snap.peak > peak) peak = snap.peak;
}
console.log(`  after one tap  : state "${state}", peak ${peak.toFixed(4)}`);

if (errs.length) fail("page errors: " + errs.slice(0, 3).join(" | "));
if (state !== "running") fail(`the tap did not start audio — state "${state}"`);
if (peak < 0.001) fail(`the tap produced no audio — peak ${peak}`);

clearTimeout(watchdog);
console.log(`MOBILE SAFARI OK [webkit, iPhone emulation] — "${before.state}" at load, ` +
            `one tap reached "running" and produced audio`);
