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
    /** time-sorted pending events + monotonic read cursor (never shift() — O(N²)) */
    this.queue = [];
    this.qHead = 0;
    this.framesBehind = 0;
    this.lastFrame = -1;
    this.droppedQuanta = 0;
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
        while (i > this.qHead && q[i - 1].when > msg.when) i--;
        q.splice(Math.max(i, this.qHead), 0, msg);
        break;
      }
      case "events": {
        // batch schedule (MIDI files): main thread pre-sorts; merge two sorted
        // lists in O(n). Runs on the message task, not inside process().
        const pending = this.queue.slice(this.qHead);
        const inc = msg.list;
        const merged = new Array(pending.length + inc.length);
        let a = 0, b = 0, k = 0;
        while (a < pending.length && b < inc.length)
          merged[k++] = pending[a].when <= inc[b].when ? pending[a++] : inc[b++];
        while (a < pending.length) merged[k++] = pending[a++];
        while (b < inc.length) merged[k++] = inc[b++];
        this.queue = merged;
        this.qHead = 0;
        break;
      }
      case "allOff":
        this.queue.length = 0;
        this.qHead = 0;
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
    if ((e.kind === "reverb" && !x.ij_set_reverb) || (e.kind === "room" && !x.ij_set_room)) {
      // loud on failure: a missing export means the cached WASM predates this
      // worklet — say so instead of crashing the processor with a TypeError
      this.port.postMessage({ type: "error",
        message: `engine binary is older than the page (no ij_set_${e.kind === "reverb" ? "reverb" : "room"} export) — hard refresh to reload the WASM` });
      return;
    }
    if (e.kind === "on") x.ij_note_on(p, e.track, e.midi, e.vel);
    else if (e.kind === "off") x.ij_note_off(p, e.track, e.midi);
    else if (e.kind === "pedal") x.ij_pedal(p, e.track, e.on);
    else if (e.kind === "track") x.ij_set_track(p, e.track, e.inst, e.gain, e.pan);
    else if (e.kind === "reverb") x.ij_set_reverb(p, e.reverb);
    else if (e.kind === "room") x.ij_set_room(p, e.track, e.send);
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    if (!this.engine || !out || out.length === 0) return true;

    // xrun detection: a gap in currentFrame between calls means the render thread
    // missed its deadline and the browser dropped quanta — never hide that
    if (this.lastFrame >= 0) {
      const gap = currentFrame - this.lastFrame - out[0].length;
      if (gap > 0) this.droppedQuanta += Math.round(gap / out[0].length);
    }
    this.lastFrame = currentFrame;

    const mem = this.exports.memory.buffer;
    if (!this.viewL || this.viewL.buffer !== mem) {
      this.viewL = new Float32Array(mem, this.exports.ij_out_l(this.engine), 128);
      this.viewR = new Float32Array(mem, this.exports.ij_out_r(this.engine), 128);
    }

    // Sample-accurate scheduling: render in segments between event boundaries,
    // applying each event at its exact frame offset within the quantum.
    // Allocation-free: cursor into the queue (no shift), manual copy (no subarray).
    const q = this.queue;
    const outL = out[0];
    const outR = out[1];
    const frames = outL.length;
    let done = 0;
    while (done < frames) {
      const tNow = currentTime + done / sampleRate;
      while (this.qHead < q.length && q[this.qHead].when <= tNow) this.apply(q[this.qHead++]);
      let next = frames;
      if (this.qHead < q.length) {
        const f = Math.ceil((q[this.qHead].when - currentTime) * sampleRate);
        if (f < frames) next = Math.max(done + 1, f);
      }
      const n = next - done;
      this.exports.ij_process(this.engine, n);
      const vL = this.viewL;
      const vR = this.viewR;
      for (let i = 0; i < n; i++) outL[done + i] = vL[i];
      if (outR) for (let i = 0; i < n; i++) outR[done + i] = vR[i];
      done = next;
    }
    // reclaim consumed prefix outside the hot loop, occasionally
    if (this.qHead > 512 && this.qHead * 2 > q.length) {
      this.queue = q.slice(this.qHead);
      this.qHead = 0;
    }

    // ~1 Hz diagnostics for UIs ("processor fell behind" style honesty)
    this.framesBehind += frames;
    if (this.framesBehind >= sampleRate) {
      this.framesBehind = 0;
      this.port.postMessage({
        type: "stats",
        activeVoices: this.exports.ij_active_voices(this.engine),
        pendingEvents: q.length - this.qHead,
        droppedQuanta: this.droppedQuanta,
      });
    }
    return true;
  }
}

registerProcessor("instruments-processor", InstrumentsProcessor);
