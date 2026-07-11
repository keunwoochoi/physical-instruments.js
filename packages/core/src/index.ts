/**
 * instruments.js public API — v0, implemented.
 *
 * Constraints (PRINCIPLES.md, architecture doc 2026-07-11):
 * - SSR-safe: importing this module never touches window/AudioContext/fetch.
 * - One shared AudioWorklet + WASM engine hosts ALL tracks; multi-track arrangements
 *   are first-class (PRINCIPLES #4).
 * - Loud on failure: engine init errors reject `ready`; nothing falls back silently.
 *
 * v0 honesty notes:
 * - Events are applied at quantum boundaries in the worklet (≤2.67 ms jitter @48k);
 *   sample-offset accuracy inside the quantum is issue #6 follow-up.
 * - Some GM families are placeholders until their physical models land (see
 *   GROUP_TO_INSTRUMENT): piano→electric piano, strings→vibraphone, etc.
 */

export interface NoteEvent {
  midiPitch: number;
  startSeconds: number;
  endSeconds: number;
  /** 0–127. Changes timbre, not just level. */
  velocity: number;
  isDrum?: boolean;
  instrumentGroup?: InstrumentGroup;
}

export type InstrumentGroup =
  | "piano"
  | "guitar"
  | "bass"
  | "strings"
  | "brass"
  | "woodwind"
  | "voice"
  | "percussion"
  | "synth"
  | "mallet"
  | "marimba"
  | "vibraphone"
  | "glockenspiel"
  | "musicbox"
  | "epiano"
  | "drums"
  | "synthpad"
  | "unknown";

/** Engine-side instrument ids (crates/dsp kernels::Instrument). */
const INST = {
  marimba: 0,
  vibraphone: 1,
  glockenspiel: 2,
  musicbox: 3,
  guitar: 4,
  bass: 5,
  epiano: 6,
  drums: 7,
  synthpad: 8,
} as const;

/**
 * GM-ish family → engine instrument. Entries marked (placeholder) are honest stand-ins
 * until their physical models land (roadmap Q2–Q4); they are the best-sounding current
 * mapping, not a claim of realism.
 */
const GROUP_TO_INSTRUMENT: Record<InstrumentGroup, number> = {
  marimba: INST.marimba,
  mallet: INST.marimba,
  vibraphone: INST.vibraphone,
  glockenspiel: INST.glockenspiel,
  musicbox: INST.musicbox,
  guitar: INST.guitar,
  bass: INST.bass,
  epiano: INST.epiano,
  drums: INST.drums,
  synthpad: INST.synthpad,
  piano: INST.epiano, // (placeholder — acoustic piano is v1.x)
  strings: INST.synthpad, // (placeholder — bowed string is Q3)
  brass: INST.synthpad, // (placeholder — winds are Q3)
  woodwind: INST.glockenspiel, // (placeholder)
  voice: INST.synthpad, // (placeholder)
  synth: INST.synthpad,
  percussion: INST.drums,
  unknown: INST.marimba,
};

export interface TrackOptions {
  gain?: number;
  /** -1 (left) .. 1 (right) */
  pan?: number;
}

export interface Track {
  readonly instrument: InstrumentGroup;
  readonly index: number;
  noteOn(midiPitch: number, velocity?: number, timeSeconds?: number): void;
  noteOff(midiPitch: number, timeSeconds?: number): void;
  set(options: TrackOptions): void;
}

export interface EngineStats {
  activeVoices: number;
  pendingEvents: number;
}

export interface Engine {
  /** Resolves when the worklet + WASM are live and the first note can sound instantly. */
  readonly ready: Promise<void>;
  readonly context: AudioContext;
  /** The engine's output node — connect it anywhere in the Web Audio graph. */
  readonly output: AudioWorkletNode;
  createTrack(instrument: InstrumentGroup, options?: TrackOptions): Track;
  /** Play a full (possibly multi-track) timeline. Resolves when playback finishes. */
  play(notes: readonly NoteEvent[]): Promise<void>;
  stop(): void;
  /** Deterministic offline bounce → 16-bit stereo WAV bytes. */
  renderOffline(notes: readonly NoteEvent[]): Promise<Uint8Array>;
  onStats(cb: (stats: EngineStats) => void): void;
  dispose(): Promise<void>;
}

export interface EngineOptions {
  /** Bring your own context (e.g. to compose with Tone.js / the raw Web Audio graph). */
  context?: AudioContext;
  /** Skip the default connection to context.destination. */
  connect?: boolean;
  /** Override asset URLs (bundler escape hatch until the zero-config story lands). */
  workletUrl?: string | URL;
  wasmUrl?: string | URL;
}

const MAX_TRACKS = 16;
const SCHED_LEAD = 0.08; // seconds of lead-in when playing a timeline

interface WorkletEvent {
  type: "event";
  when: number;
  kind: "on" | "off" | "track";
  track: number;
  midi?: number;
  vel?: number;
  inst?: number;
  gain?: number;
  pan?: number;
}

async function fetchWasmBytes(url: string | URL): Promise<ArrayBuffer> {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`instruments.js: failed to fetch WASM (${resp.status}) from ${url}`);
  return resp.arrayBuffer();
}

/**
 * Send the engine binary to a processor as raw bytes (compiled inside the worklet).
 * Never post a WebAssembly.Module: Safari and Chromium-headless silently drop the
 * clone into `messageerror`, which presents as an engine that never becomes ready.
 */
function postInit(port: MessagePort, bytes: ArrayBuffer): void {
  const copy = bytes.slice(0);
  port.postMessage({ type: "init", bytes: copy }, [copy]);
}

function defaultUrls(): { worklet: URL; wasm: URL } {
  return {
    worklet: new URL("../worklet/instruments-processor.js", import.meta.url),
    wasm: new URL("../wasm/instruments_dsp.wasm", import.meta.url),
  };
}

/**
 * Create the shared engine. Call from (or after) a user gesture on iOS —
 * a suspended context is resumed on the first noteOn/play attempt.
 */
export async function createEngine(options: EngineOptions = {}): Promise<Engine> {
  const urls = defaultUrls();
  const workletUrl = options.workletUrl ?? urls.worklet;
  const wasmUrl = options.wasmUrl ?? urls.wasm;
  const context = options.context ?? new AudioContext({ latencyHint: "interactive" });

  const [wasm] = await Promise.all([
    fetchWasmBytes(wasmUrl),
    context.audioWorklet.addModule(workletUrl),
  ]);

  const node = new AudioWorkletNode(context, "instruments-processor", {
    numberOfInputs: 0,
    numberOfOutputs: 1,
    outputChannelCount: [2],
  });
  if (options.connect !== false) node.connect(context.destination);

  let statsCb: ((s: EngineStats) => void) | null = null;
  const ready = new Promise<void>((resolve, reject) => {
    node.port.onmessage = (ev: MessageEvent) => {
      const msg = ev.data;
      if (msg.type === "ready") resolve();
      else if (msg.type === "error") reject(new Error(`instruments.js worklet: ${msg.message}`));
      else if (msg.type === "stats" && statsCb) statsCb(msg);
    };
    // never-silent guards: clone failures and processor crashes must reject loudly
    node.port.onmessageerror = () =>
      reject(new Error("instruments.js: worklet message failed to deserialize (structured-clone unsupported)"));
    node.onprocessorerror = () =>
      reject(new Error("instruments.js: AudioWorklet processor crashed during construction/render"));
  });
  postInit(node.port, wasm);

  let nextTrack = 0;
  const groupTracks = new Map<string, number>();

  const post = (e: WorkletEvent) => node.port.postMessage(e);
  const resumeIfNeeded = () => {
    if (context.state === "suspended") void context.resume();
  };

  function allocTrack(instrument: InstrumentGroup, opts: TrackOptions, at = 0): number {
    if (nextTrack >= MAX_TRACKS) throw new Error(`instruments.js: track limit (${MAX_TRACKS}) reached`);
    const idx = nextTrack++;
    post({
      type: "event",
      when: at,
      kind: "track",
      track: idx,
      inst: GROUP_TO_INSTRUMENT[instrument] ?? 0,
      gain: opts.gain ?? 0.8,
      pan: opts.pan ?? 0,
    });
    return idx;
  }

  function makeTrack(instrument: InstrumentGroup, idx: number): Track {
    return {
      instrument,
      index: idx,
      noteOn(midiPitch, velocity = 96, timeSeconds = 0) {
        resumeIfNeeded();
        post({
          type: "event",
          when: timeSeconds,
          kind: "on",
          track: idx,
          midi: Math.round(midiPitch),
          vel: Math.min(127, Math.max(1, velocity)) / 127,
        });
      },
      noteOff(midiPitch, timeSeconds = 0) {
        post({ type: "event", when: timeSeconds, kind: "off", track: idx, midi: Math.round(midiPitch) });
      },
      set(o: TrackOptions) {
        post({
          type: "event",
          when: 0,
          kind: "track",
          track: idx,
          inst: GROUP_TO_INSTRUMENT[instrument] ?? 0,
          gain: o.gain ?? 0.8,
          pan: o.pan ?? 0,
        });
      },
    };
  }

  /** Schedule a note list onto auto-managed per-family tracks. Returns end time (ctx seconds). */
  function scheduleNotes(notes: readonly NoteEvent[], t0: number): number {
    let end = t0;
    for (const n of notes) {
      const key = n.isDrum ? "drums" : (n.instrumentGroup ?? "unknown");
      let idx = groupTracks.get(key);
      if (idx === undefined) {
        idx = allocTrack(n.isDrum ? "drums" : (n.instrumentGroup ?? "unknown"), {}, 0);
        groupTracks.set(key, idx);
      }
      const vel = Math.min(127, Math.max(1, n.velocity)) / 127;
      post({ type: "event", when: t0 + n.startSeconds, kind: "on", track: idx, midi: Math.round(n.midiPitch), vel });
      if (!n.isDrum) {
        post({ type: "event", when: t0 + n.endSeconds, kind: "off", track: idx, midi: Math.round(n.midiPitch) });
      }
      end = Math.max(end, t0 + n.endSeconds);
    }
    return end;
  }

  const engine: Engine = {
    ready,
    context,
    output: node,
    createTrack(instrument, opts = {}) {
      const idx = allocTrack(instrument, opts);
      return makeTrack(instrument, idx);
    },
    async play(notes) {
      await ready;
      resumeIfNeeded();
      const t0 = context.currentTime + SCHED_LEAD;
      const end = scheduleNotes(notes, t0) + 2.0; // let tails ring
      await new Promise<void>((resolve) => {
        const tick = () => {
          if (context.currentTime >= end) resolve();
          else setTimeout(tick, 120);
        };
        tick();
      });
    },
    stop() {
      node.port.postMessage({ type: "allOff" });
    },
    async renderOffline(notes) {
      const duration = Math.max(...notes.map((n) => n.endSeconds), 0) + 2.5;
      const sr = context.sampleRate;
      const off = new OfflineAudioContext(2, Math.ceil(duration * sr), sr);
      await off.audioWorklet.addModule(workletUrl);
      // An OfflineAudioContext may not service port messages before its render loop
      // finishes — deliver init bytes AND the full schedule via processorOptions,
      // which is cloned synchronously at construction.
      const events: WorkletEvent[] = [];
      const local = new Map<string, number>();
      let localNext = 0;
      for (const n of notes) {
        const key = n.isDrum ? "drums" : (n.instrumentGroup ?? "unknown");
        let idx = local.get(key);
        if (idx === undefined) {
          idx = localNext++;
          local.set(key, idx);
          events.push({
            type: "event", when: 0, kind: "track", track: idx,
            inst: GROUP_TO_INSTRUMENT[n.isDrum ? "drums" : (n.instrumentGroup ?? "unknown")] ?? 0,
            gain: 0.8, pan: 0,
          });
        }
        const vel = Math.min(127, Math.max(1, n.velocity)) / 127;
        events.push({
          type: "event", when: 0.05 + n.startSeconds, kind: "on", track: idx,
          midi: Math.round(n.midiPitch), vel,
        });
        if (!n.isDrum) {
          events.push({
            type: "event", when: 0.05 + n.endSeconds, kind: "off", track: idx, midi: Math.round(n.midiPitch),
          });
        }
      }
      const offNode = new AudioWorkletNode(off, "instruments-processor", {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
        processorOptions: { bytes: wasm.slice(0), events },
      });
      let offError: Error | null = null;
      offNode.port.onmessage = (ev: MessageEvent) => {
        if (ev.data.type === "error") offError = new Error(`instruments.js worklet: ${ev.data.message}`);
      };
      offNode.connect(off.destination);
      const rendered = await off.startRendering();
      if (offError) throw offError;
      return encodeWav(rendered);
    },
    onStats(cb) {
      statsCb = cb;
    },
    async dispose() {
      node.port.postMessage({ type: "allOff" });
      node.port.postMessage({ type: "dispose" });
      node.disconnect();
      if (!options.context) await context.close();
    },
  };
  return engine;
}

/** Encode an AudioBuffer as 16-bit PCM WAV. */
export function encodeWav(buf: AudioBuffer): Uint8Array {
  const ch = Math.min(2, buf.numberOfChannels);
  const frames = buf.length;
  const dataLen = frames * ch * 2;
  const out = new ArrayBuffer(44 + dataLen);
  const v = new DataView(out);
  const str = (o: number, s: string) => {
    for (let i = 0; i < s.length; i++) v.setUint8(o + i, s.charCodeAt(i));
  };
  str(0, "RIFF");
  v.setUint32(4, 36 + dataLen, true);
  str(8, "WAVE");
  str(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, ch, true);
  v.setUint32(24, buf.sampleRate, true);
  v.setUint32(28, buf.sampleRate * ch * 2, true);
  v.setUint16(32, ch * 2, true);
  v.setUint16(34, 16, true);
  str(36, "data");
  v.setUint32(40, dataLen, true);
  const chans = [] as Float32Array[];
  for (let c = 0; c < ch; c++) chans.push(buf.getChannelData(c));
  let o = 44;
  for (let i = 0; i < frames; i++) {
    for (let c = 0; c < ch; c++) {
      const s = Math.max(-1, Math.min(1, chans[c]![i]!));
      v.setInt16(o, s < 0 ? s * 0x8000 : s * 0x7fff, true);
      o += 2;
    }
  }
  return new Uint8Array(out);
}
