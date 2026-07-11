/**
 * instruments.js public API — v0 contract.
 *
 * Constraints (PRINCIPLES.md, architecture doc 2026-07-11):
 * - SSR-safe: importing this module must never touch window/AudioContext/fetch.
 * - One shared AudioWorklet + WASM engine hosts ALL tracks; multi-track arrangements
 *   are first-class (PRINCIPLES #4).
 * - The customer-zero test: replacing music-transcription-app's 36-line MidiPlayer.tsx
 *   with `createEngine()` + `play(notes)` must be a one-for-one swap.
 *
 * Implementation lands with issues #5/#6/#10; until then factories throw loudly
 * (no silent fallbacks — PRINCIPLES engineering rules).
 */

/** The customer-zero note shape. `instrumentGroup` follows the GM-ish family taxonomy. */
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
  | "unknown";

export interface TrackOptions {
  gain?: number;
  /** -1 (left) .. 1 (right) */
  pan?: number;
}

export interface Track {
  readonly instrument: InstrumentGroup;
  noteOn(midiPitch: number, velocity?: number, timeSeconds?: number): void;
  noteOff(midiPitch: number, timeSeconds?: number): void;
  set(options: TrackOptions): void;
}

export interface Engine {
  /** Resolves when the worklet + WASM are live and the first note can sound instantly. */
  readonly ready: Promise<void>;
  readonly context: AudioContext;
  /** Independent instrument channel over the shared engine. N tracks, one worklet, one voice pool. */
  createTrack(instrument: InstrumentGroup, options?: TrackOptions): Track;
  /** Play a full (possibly multi-track) timeline. Sample-accurate, pre-scheduled. */
  play(notes: readonly NoteEvent[]): Promise<void>;
  stop(): void;
  /** Deterministic offline bounce (OfflineAudioContext) → WAV bytes. */
  renderOffline(notes: readonly NoteEvent[]): Promise<Uint8Array>;
  dispose(): Promise<void>;
}

export interface EngineOptions {
  /** Bring your own context (e.g. to compose with Tone.js / the raw Web Audio graph). */
  context?: AudioContext;
  /** Total voice budget across all tracks. Default 64; degrade by voice-stealing, never by glitching. */
  maxVoices?: number;
}

/**
 * Create the shared engine. Lazy: no AudioContext is created until this call,
 * and callers should invoke it from (or after) a user gesture on iOS.
 */
export async function createEngine(_options?: EngineOptions): Promise<Engine> {
  throw new Error(
    "instruments.js: not implemented yet — worklet+WASM pipeline lands with issue #5. " +
      "This package is pre-alpha; nothing is published.",
  );
}
