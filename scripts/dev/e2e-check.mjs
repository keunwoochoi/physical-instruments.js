#!/usr/bin/env node
/**
 * Headless end-to-end check of the playground: loads the real page, clicks Start,
 * captures every console/page error, and verifies the browser pipeline produces
 * audio — both the offline render path and the live AudioContext path.
 * Usage: node scripts/dev/e2e-check.mjs [url]
 */
import { chromium } from "playwright";

const URL_ = process.argv[2] ?? "http://localhost:8173/apps/playground/";
const errors = [];
const messages = [];

const browser = await chromium.launch({
  args: ["--autoplay-policy=no-user-gesture-required"],
});
const page = await browser.newPage();
page.on("console", (m) => {
  messages.push(`[${m.type()}] ${m.text()}`);
  if (m.type() === "error") errors.push(m.text());
});
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

await page.goto(URL_, { waitUntil: "networkidle" });
const overlayVisible = await page.locator("#start").isVisible().catch(() => false);
if (overlayVisible) await page.click("#start");

// wait for the engine to report readiness via the status line
let status = "";
try {
  await page.waitForFunction(
    () => document.getElementById("status")?.textContent?.includes("engine live"),
    { timeout: 10000 },
  );
  status = await page.locator("#status").textContent();
} catch {
  status = (await page.locator("#status").textContent().catch(() => "")) ?? "";
}

// 1) offline render path: render one bar and measure RMS in-page
const offline = await page.evaluate(async () => {
  const { createEngine } = await import("/packages/core/dist/index.js");
  const engine = await createEngine({ connect: false });
  await engine.ready;
  const notes = [
    { instrumentGroup: "marimba", midiPitch: 69, startSeconds: 0.0, endSeconds: 0.4, velocity: 110 },
    { instrumentGroup: "bass", midiPitch: 33, startSeconds: 0.0, endSeconds: 0.8, velocity: 100 },
    { instrumentGroup: "percussion", midiPitch: 36, startSeconds: 0.0, endSeconds: 0.2, velocity: 110, isDrum: true },
    { instrumentGroup: "strings", midiPitch: 57, startSeconds: 0.0, endSeconds: 1.0, velocity: 80 },
  ];
  const wav = await engine.renderOffline(notes);
  // skip 44-byte header; 16-bit stereo PCM
  const pcm = new Int16Array(wav.buffer, 44);
  let sumSq = 0, peak = 0;
  for (let i = 0; i < pcm.length; i++) {
    const s = pcm[i] / 32768;
    sumSq += s * s;
    peak = Math.max(peak, Math.abs(s));
  }
  await engine.dispose();
  return { rms: Math.sqrt(sumSq / pcm.length), peak, bytes: wav.length };
});

// 2) live path: analyser tap on the engine output while holding a note
const live = await page.evaluate(async () => {
  const { createEngine } = await import("/packages/core/dist/index.js");
  const engine = await createEngine();
  await engine.ready;
  const ctx = engine.context;
  const analyser = ctx.createAnalyser();
  analyser.fftSize = 2048;
  engine.output.connect(analyser);
  const t = engine.createTrack("marimba");
  t.noteOn(69, 110);
  const t0 = ctx.currentTime;
  await new Promise((r) => setTimeout(r, 400));
  const buf = new Float32Array(analyser.fftSize);
  analyser.getFloatTimeDomainData(buf);
  let peak = 0;
  for (const s of buf) peak = Math.max(peak, Math.abs(s));
  const advanced = ctx.currentTime - t0;
  await engine.dispose();
  return { peak, clockAdvancedSeconds: +advanced.toFixed(3), state: ctx.state };
});

await browser.close();

const report = {
  url: URL_,
  statusLine: status,
  offlineRender: { ...offline, rms: +offline.rms.toFixed(4), peak: +offline.peak.toFixed(3) },
  livePath: live,
  consoleErrors: errors,
  // no escape hatches: BOTH paths must produce real audio (panel finding)
  verdict: errors.length === 0 && offline.rms > 0.005 && live.peak > 0.005 ? "PASS" : "FAIL",
};
console.log(JSON.stringify(report, null, 2));
if (report.verdict !== "PASS") {
  console.error("--- console messages ---");
  for (const m of messages.slice(-30)) console.error(m);
  process.exit(1);
}
