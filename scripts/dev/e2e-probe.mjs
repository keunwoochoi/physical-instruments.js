#!/usr/bin/env node
/** Step-by-step pipeline probe: reports exactly which stage of the worklet+WASM
 *  handshake stalls. Every step has a timeout — this script always terminates. */
import { chromium } from "playwright";

const base = process.argv[2] ?? "http://localhost:8173";
const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const page = await browser.newPage();
const logs = [];
page.on("console", (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
await page.goto(`${base}/apps/playground/`, { waitUntil: "load" });

const report = await page.evaluate(async () => {
  const step = async (name, promise, ms = 4000) => {
    const t0 = performance.now();
    try {
      const v = await Promise.race([
        promise,
        new Promise((_, rej) => setTimeout(() => rej(new Error("TIMEOUT")), ms)),
      ]);
      return { name, ok: true, ms: Math.round(performance.now() - t0), value: v ?? null };
    } catch (e) {
      return { name, ok: false, ms: Math.round(performance.now() - t0), error: String(e) };
    }
  };
  const out = [];
  const ctx = new AudioContext();
  out.push({ name: "context", ok: true, value: { state: ctx.state, sr: ctx.sampleRate } });

  out.push(await step("resume", ctx.resume().then(() => ctx.state)));
  out.push(await step("addModule", ctx.audioWorklet.addModule("/packages/core/worklet/instruments-processor.js").then(() => "ok")));

  const wasmResp = await fetch("/packages/core/wasm/instruments_dsp.wasm");
  const bytes = await wasmResp.arrayBuffer();
  out.push({ name: "fetchWasm", ok: true, value: { bytes: bytes.byteLength } });
  out.push(await step("compile", WebAssembly.compile(bytes.slice(0)).then(() => "ok")));
  const module = await WebAssembly.compile(bytes.slice(0));

  let node;
  try {
    node = new AudioWorkletNode(ctx, "instruments-processor", {
      numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2],
    });
    node.connect(ctx.destination);
    out.push({ name: "createNode", ok: true });
  } catch (e) {
    out.push({ name: "createNode", ok: false, error: String(e) });
    return out;
  }

  const messages = [];
  let expect = null;
  node.port.onmessage = (ev) => {
    messages.push(ev.data.type);
    if (expect && ev.data.type === expect.type) expect.res(JSON.stringify(ev.data));
  };
  node.port.onmessageerror = () => messages.push("MESSAGEERROR");
  node.onprocessorerror = () => messages.push("PROCESSORERROR");
  const waitFor = (type, ms) =>
    step(`msg:${type}`, new Promise((res) => (expect = { type, res })), ms);

  node.port.postMessage({ type: "ping" });
  out.push(await waitFor("pong", 3000));

  const copy = bytes.slice(0);
  node.port.postMessage({ type: "init", bytes: copy }, [copy]);
  out.push(await waitFor("ready", 5000));
  out.push({ name: "allMessages", ok: true, value: messages.join(",") });

  const t0 = ctx.currentTime;
  await new Promise((r) => setTimeout(r, 500));
  out.push({ name: "clockAdvance", ok: ctx.currentTime > t0, value: +(ctx.currentTime - t0).toFixed(3) });
  return out;
});

await browser.close();
console.log(JSON.stringify(report, null, 2));
if (logs.length) console.log("console:", logs.slice(-15).join("\n"));
