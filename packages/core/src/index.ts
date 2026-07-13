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
 * - Scheduled events are sample-accurate (the worklet renders in segments between
 *   event frame offsets). LIVE input (noteOn with no timestamp) lands at the next
 *   quantum boundary — up to 2.67 ms of timing jitter on live playing.
 * - Some GM families are placeholders until their physical models land (see
 *   GROUP_TO_INSTRUMENT): piano→electric piano, strings→synth pad, etc.
 */

export interface NoteEvent {
  midiPitch: number;
  startSeconds: number;
  endSeconds: number;
  /** 0–127. Changes timbre, not just level. */
  velocity: number;
  isDrum?: boolean;
  /** Family name; unknown strings fall back to marimba. Loose type so decoupled
   *  producers (e.g. @instrumentsjs/midi) interoperate without a cast. */
  instrumentGroup?: InstrumentGroup | (string & {});
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
  | "drums-rock"
  | "drums-jazz"
  | "synthpad"
  | "guitar-steel"
  | "guitar-electric"
  | "guitar-distorted"
  // naming aliases (Keunwoo 2026-07-12): resolve to the same engines
  | "guitar-acoustic"
  | "guitar-acoustic-nylon"
  | "guitar-acoustic-steel"
  | "bass-electric"
  | "electric-bass"
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
  drumsRock: 13,
  drumsJazz: 14,
  synthpad: 8,
  piano: 9,
  guitarSteel: 10,
  guitarElectric: 11,
  guitarDistorted: 12,
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
  piano: INST.piano, // multi-string waveguide acoustic piano
  "drums-rock": INST.drumsRock,
  "drums-jazz": INST.drumsJazz,
  "guitar-steel": INST.guitarSteel,
  "guitar-electric": INST.guitarElectric,
  "guitar-distorted": INST.guitarDistorted,
  // "acoustic"/"electric bass" naming aliases (Keunwoo 2026-07-12) — canonical names above stay
  "guitar-acoustic": INST.guitar,
  "guitar-acoustic-nylon": INST.guitar,
  "guitar-acoustic-steel": INST.guitarSteel,
  "bass-electric": INST.bass,
  "electric-bass": INST.bass,
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
  /** Sustain pedal (CC64): while down, note-offs are deferred until pedal-up. */
  pedal(on: boolean, timeSeconds?: number): void;
  set(options: TrackOptions): void;
}

export interface EngineStats {
  activeVoices: number;
  pendingEvents: number;
  /** Render quanta the browser dropped because the processor missed its deadline
   *  (cumulative). Nonzero = audible glitches happened; never hidden. */
  droppedQuanta: number;
}

/** Shared spatial stage voicings ("several reverb choices" — Keunwoo 2026-07-13). */
export type ReverbType = "off" | "room" | "hall" | "plate" | "spring";
const REVERB_ID: Record<ReverbType, number> = { off: 0, room: 1, hall: 2, plate: 3, spring: 4 };

export interface Engine {
  /** Resolves when the worklet + WASM are live and the first note can sound instantly. */
  readonly ready: Promise<void>;
  readonly context: AudioContext;
  /** The engine's output node — connect it anywhere in the Web Audio graph. */
  readonly output: AudioWorkletNode;
  createTrack(instrument: InstrumentGroup, options?: TrackOptions): Track;
  /** Play a full (possibly multi-track) timeline. Resolves when playback finishes. */
  play(notes: readonly NoteEvent[], options?: PlayOptions): Promise<void>;
  stop(): void;
  /** Select the shared reverb voicing (default "room" — subtle glue, not an effect). */
  setReverb(type: ReverbType): void;
  /** Deterministic offline bounce → stereo WAV bytes (16-bit PCM, or float32 via options). */
  renderOffline(notes: readonly NoteEvent[], options?: RenderOptions): Promise<Uint8Array>;
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
  kind: "on" | "off" | "track" | "pedal" | "reverb" | "room";
  track?: number;
  reverb?: number;
  send?: number;
  midi?: number;
  vel?: number;
  inst?: number;
  gain?: number;
  pan?: number;
  on?: number;
}

/** Sustain-pedal event (CC64) accompanying a note timeline. */
export interface PedalEvent {
  instrumentGroup?: InstrumentGroup | string;
  isDrum?: boolean;
  on: boolean;
  timeSeconds: number;
}

export interface PlayOptions {
  pedals?: readonly PedalEvent[];
  /** Per-family mix (gain/pan) for auto-created tracks, e.g. {"drums": {gain: 0.6}}. */
  tracks?: Readonly<Record<string, TrackOptions>>;
}

export interface RenderOptions extends PlayOptions {
  /** Encode 32-bit float WAV instead of 16-bit PCM. */
  float32?: boolean;
  /** Progress callback (0..1), roughly once per rendered second. */
  onProgress?: (fraction: number) => void;
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
      else if (msg.type === "error") {
        // before ready: fail init; after ready (a settled promise ignores
        // reject): still loud — runtime errors must never disappear
        console.error(`instruments.js worklet: ${msg.message}`);
        reject(new Error(`instruments.js worklet: ${msg.message}`));
      }
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
      pedal(on, timeSeconds = 0) {
        post({ type: "event", when: timeSeconds, kind: "pedal", track: idx, on: on ? 1 : 0 });
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

  /**
   * Build a batched event schedule for a note timeline (+ optional pedal lane).
   * `resolve` maps a family key to a track index, appending the track-config
   * event to `events` on first use.
   */
  function buildSchedule(
    notes: readonly NoteEvent[],
    options: PlayOptions,
    t0: number,
    resolve: (group: InstrumentGroup, events: WorkletEvent[]) => number,
  ): { events: WorkletEvent[]; end: number } {
    const pedals = options.pedals;
    const events: WorkletEvent[] = [];
    let end = t0;
    for (const n of notes) {
      const group = (n.isDrum ? "drums" : (n.instrumentGroup ?? "unknown")) as InstrumentGroup;
      const idx = resolve(group, events);
      const vel = Math.min(127, Math.max(1, n.velocity)) / 127;
      events.push({ type: "event", when: t0 + n.startSeconds, kind: "on", track: idx, midi: Math.round(n.midiPitch), vel });
      if (!n.isDrum) {
        events.push({ type: "event", when: t0 + n.endSeconds, kind: "off", track: idx, midi: Math.round(n.midiPitch) });
      }
      end = Math.max(end, t0 + n.endSeconds);
    }
    for (const p of pedals ?? []) {
      const group = (p.isDrum ? "drums" : (p.instrumentGroup ?? "unknown")) as InstrumentGroup;
      const idx = resolve(group, events);
      events.push({ type: "event", when: t0 + p.timeSeconds, kind: "pedal", track: idx, on: p.on ? 1 : 0 });
      end = Math.max(end, t0 + p.timeSeconds);
    }
    return { events, end };
  }

  /** Track resolver for the live engine: per-family tracks persist across play() calls. */
  function makeLiveResolver(mix: PlayOptions["tracks"]) {
    return (group: InstrumentGroup, events: WorkletEvent[]): number => {
      let idx = groupTracks.get(group);
      if (idx === undefined) {
        if (nextTrack >= MAX_TRACKS) throw new Error(`instruments.js: track limit (${MAX_TRACKS}) reached`);
        idx = nextTrack++;
        groupTracks.set(group, idx);
        const m = mix?.[group] ?? {};
        events.push({
          type: "event", when: 0, kind: "track", track: idx,
          inst: GROUP_TO_INSTRUMENT[group] ?? 0, gain: m.gain ?? 0.8, pan: m.pan ?? 0,
        });
      }
      return idx;
    };
  }

  const engine: Engine = {
    ready,
    context,
    output: node,
    createTrack(instrument, opts = {}) {
      const idx = allocTrack(instrument, opts);
      return makeTrack(instrument, idx);
    },
    async play(notes, options = {}) {
      await ready;
      resumeIfNeeded();
      const t0 = context.currentTime + SCHED_LEAD;
      const { events, end } = buildSchedule(notes, options, t0, makeLiveResolver(options.tracks));
      node.port.postMessage({ type: "events", list: events });
      const finish = end + 2.0; // let tails ring
      await new Promise<void>((resolve) => {
        const tick = () => {
          if (context.currentTime >= finish) resolve();
          else setTimeout(tick, 120);
        };
        tick();
      });
    },
    stop() {
      node.port.postMessage({ type: "allOff" });
    },
    setReverb(type) {
      const reverb = REVERB_ID[type];
      if (reverb === undefined) throw new Error(`unknown reverb type: ${type}`);
      node.port.postMessage({ type: "events", list: [{ type: "event", when: 0, kind: "reverb", reverb }] });
    },
    async renderOffline(notes, options = {}) {
      const duration =
        Math.max(...notes.map((n) => n.endSeconds), ...(options.pedals ?? []).map((p) => p.timeSeconds), 0) + 2.5;
      const sr = context.sampleRate;
      const off = new OfflineAudioContext(2, Math.ceil(duration * sr), sr);
      await off.audioWorklet.addModule(workletUrl);
      // An OfflineAudioContext may not service port messages before its render loop
      // finishes — deliver init bytes AND the full schedule via processorOptions,
      // which is cloned synchronously at construction.
      const local = new Map<InstrumentGroup, number>();
      let localNext = 0;
      const { events } = buildSchedule(notes, options, 0.05, (group, evts) => {
        let idx = local.get(group);
        if (idx === undefined) {
          idx = localNext++;
          local.set(group, idx);
          const m = options.tracks?.[group] ?? {};
          evts.push({
            type: "event", when: 0, kind: "track", track: idx,
            inst: GROUP_TO_INSTRUMENT[group] ?? 0, gain: m.gain ?? 0.8, pan: m.pan ?? 0,
          });
        }
        return idx;
      });
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
      // progress via suspend/resume checkpoints (~1 s apart)
      const onProgress = (options as RenderOptions).onProgress;
      if (onProgress) {
        for (let t = 1; t < duration; t += 1) {
          void off.suspend(t).then(() => {
            onProgress(Math.min(1, t / duration));
            void off.resume();
          });
        }
      }
      const rendered = await off.startRendering();
      if (offError) throw offError;
      onProgress?.(1);
      return encodeWav(rendered, (options as RenderOptions).float32 === true);
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

/** Encode an AudioBuffer as WAV — 16-bit PCM (default) or 32-bit float. */
export function encodeWav(buf: AudioBuffer, float32 = false): Uint8Array {
  const ch = Math.min(2, buf.numberOfChannels);
  const frames = buf.length;
  const bytes = float32 ? 4 : 2;
  const dataLen = frames * ch * bytes;
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
  v.setUint16(20, float32 ? 3 : 1, true); // 3 = IEEE float
  v.setUint16(22, ch, true);
  v.setUint32(24, buf.sampleRate, true);
  v.setUint32(28, buf.sampleRate * ch * bytes, true);
  v.setUint16(32, ch * bytes, true);
  v.setUint16(34, bytes * 8, true);
  str(36, "data");
  v.setUint32(40, dataLen, true);
  const chans = [] as Float32Array[];
  for (let c = 0; c < ch; c++) chans.push(buf.getChannelData(c));
  let o = 44;
  for (let i = 0; i < frames; i++) {
    for (let c = 0; c < ch; c++) {
      const s = chans[c]![i]!;
      if (float32) {
        v.setFloat32(o, s, true);
      } else {
        const q = Math.max(-1, Math.min(1, s));
        v.setInt16(o, q < 0 ? q * 0x8000 : q * 0x7fff, true);
      }
      o += bytes;
    }
  }
  return new Uint8Array(out);
}
