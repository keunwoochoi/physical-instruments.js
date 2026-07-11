/**
 * instruments.js AudioWorklet processor (plain JS — served verbatim, never bundled).
 *
 * Contract with packages/core:
 * - main thread compiles the WASM module (fetch is unavailable in this scope) and
 *   posts it here; we instantiate synchronously in the audio thread.
 * - all engine calls happen on this thread; events arrive time-tagged in context
 *   seconds and are applied at quantum boundaries (2.67 ms granularity @48k).
 * - the render path allocates nothing after init (views are reused; they are only
 *   recreated if WASM memory growth replaces the backing ArrayBuffer).
 */
class InstrumentsProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.exports = null;
    this.engine = 0;
    this.viewL = null;
    this.viewR = null;
    /** time-sorted pending events */
    this.queue = [];
    this.framesBehind = 0;
    this.port.onmessage = (ev) => this.onMessage(ev.data);
    // Offline rendering path: an OfflineAudioContext may not service port messages
    // before its render loop finishes, so init bytes + the full event schedule can be
    // delivered via processorOptions — cloned synchronously at construction.
    const po = options && options.processorOptions;
    if (po && po.bytes) {
      this.onMessage({ type: "init", bytes: po.bytes });
      if (Array.isArray(po.events)) {
        this.queue = po.events.slice().sort((a, b) => a.when - b.when);
      }
    }
  }

  onMessage(msg) {
    switch (msg.type) {
      case "ping":
        this.port.postMessage({ type: "pong" });
        break;
      case "init": {
        try {
          // Raw bytes, always: structured-cloning a WebAssembly.Module into an
          // AudioWorklet silently fails on Safari and Chromium headless (messageerror,
          // not error). Sync compile is legal off the main thread and costs ~1 ms here.
          const module = new WebAssembly.Module(msg.bytes);
          const instance = new WebAssembly.Instance(module, {});
          this.exports = instance.exports;
          this.engine = this.exports.ij_engine_new(sampleRate);
          this.port.postMessage({ type: "ready", sampleRate });
        } catch (err) {
          // loud on failure, never silent (PRINCIPLES)
          this.port.postMessage({ type: "error", message: String(err) });
        }
        break;
      }
      case "event": {
        const q = this.queue;
        let i = q.length;
        while (i > 0 && q[i - 1].when > msg.when) i--;
        q.splice(i, 0, msg);
        break;
      }
      case "allOff":
        this.queue.length = 0;
        if (this.engine) this.exports.ij_all_off(this.engine);
        break;
      case "dispose":
        if (this.engine) {
          this.exports.ij_engine_free(this.engine);
          this.engine = 0;
        }
        break;
    }
  }

  apply(e) {
    const x = this.exports;
    const p = this.engine;
    if (e.kind === "on") x.ij_note_on(p, e.track, e.midi, e.vel);
    else if (e.kind === "off") x.ij_note_off(p, e.track, e.midi);
    else if (e.kind === "track") x.ij_set_track(p, e.track, e.inst, e.gain, e.pan);
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    if (!this.engine || !out || out.length === 0) return true;

    const q = this.queue;
    while (q.length > 0 && q[0].when <= currentTime) this.apply(q.shift());

    const frames = out[0].length;
    this.exports.ij_process(this.engine, frames);

    const mem = this.exports.memory.buffer;
    if (!this.viewL || this.viewL.buffer !== mem) {
      this.viewL = new Float32Array(mem, this.exports.ij_out_l(this.engine), 128);
      this.viewR = new Float32Array(mem, this.exports.ij_out_r(this.engine), 128);
    }
    out[0].set(this.viewL.subarray(0, frames));
    if (out[1]) out[1].set(this.viewR.subarray(0, frames));

    // ~1 Hz diagnostics for UIs ("processor fell behind" style honesty)
    this.framesBehind += frames;
    if (this.framesBehind >= sampleRate) {
      this.framesBehind = 0;
      this.port.postMessage({
        type: "stats",
        activeVoices: this.exports.ij_active_voices(this.engine),
        pendingEvents: q.length,
      });
    }
    return true;
  }
}

registerProcessor("instruments-processor", InstrumentsProcessor);
