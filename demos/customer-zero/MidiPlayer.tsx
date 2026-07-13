import { useEffect, useMemo, useRef, useState } from "react";
import type { CanonicalNote } from "@music-to-score/contracts";
import { createEngine, type Engine, type NoteEvent } from "instruments.js";

export type PlaybackEvent = { midi: number; start: number; duration: number; velocity: number; family: "piano" | "bass" | "guitar" | "drums" };

export const PLAYBACK_POLYPHONY_LIMITS: Record<PlaybackEvent["family"], number> = { piano: 32, guitar: 12, bass: 6, drums: 24 };

/** Apply deterministic voice stealing before the engine sees a dense or malformed transcription. */
export function limitPlaybackPolyphony(events: readonly PlaybackEvent[]): PlaybackEvent[] {
  const output: PlaybackEvent[] = [];
  const active = new Map<PlaybackEvent["family"], number[]>();
  for (const source of events) {
    const event = { ...source };
    const familyActive = (active.get(event.family) ?? []).filter((index) => output[index].duration >= 0.01 && output[index].start + output[index].duration > event.start + 1e-6);
    const limit = PLAYBACK_POLYPHONY_LIMITS[event.family];
    if (familyActive.length >= limit) {
      const victim = familyActive.reduce((best, index) => output[index].start + output[index].duration < output[best].start + output[best].duration ? index : best);
      const replacementDuration = event.start - output[victim].start;
      if (replacementDuration >= 0.01) {
        output[victim].duration = replacementDuration;
        familyActive.splice(familyActive.indexOf(victim), 1);
      } else {
        const quietest = familyActive.reduce((best, index) => output[index].velocity < output[best].velocity ? index : best);
        if (event.velocity <= output[quietest].velocity) continue;
        output[quietest].duration = 0;
        familyActive.splice(familyActive.indexOf(quietest), 1);
      }
    }
    output.push(event);
    familyActive.push(output.length - 1);
    active.set(event.family, familyActive);
  }
  return output.filter((event) => event.duration >= 0.01);
}

/** Pure scheduling boundary: keeping this separate makes timing edits testable without an AudioContext. */
export function createPlaybackPlan(notes: readonly CanonicalNote[]): PlaybackEvent[] {
  const events = notes
    .filter((note) => Number.isFinite(note.startSeconds) && Number.isFinite(note.endSeconds) && note.endSeconds > note.startSeconds)
    .map<PlaybackEvent>((note) => ({
      midi: Math.max(0, Math.min(127, Math.round(note.midiPitch))),
      start: Math.max(0, note.startSeconds),
      duration: Math.min(12, Math.max(0.05, note.endSeconds - note.startSeconds)),
      velocity: Math.max(0, Math.min(1, note.velocity / 127)),
      family: note.isDrum ? "drums" : /bass/i.test(note.instrumentGroup) ? "bass" : /guitar/i.test(note.instrumentGroup) ? "guitar" : "piano",
    }))
    .sort((a, b) => a.start - b.start || a.family.localeCompare(b.family) || a.midi - b.midi);
  return limitPlaybackPolyphony(events);
}

/** Return a score-relative plan beginning at a requested playhead position. */
export function playbackPlanFrom(events: readonly PlaybackEvent[], offset: number): PlaybackEvent[] {
  const safeOffset = Math.max(0, offset);
  return events
    .filter((event) => event.start + event.duration > safeOffset)
    .map((event) => {
      const elapsed = Math.max(0, safeOffset - event.start);
      return { ...event, start: Math.max(0, event.start - safeOffset), duration: event.duration - elapsed };
    });
}

function clockLabel(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
}

/** Self-hosted engine assets (privacy contract: nothing leaves the device at play time). */
const ENGINE_ASSETS = { workletUrl: "/instruments/instruments-processor.js", wasmUrl: "/instruments/instruments_dsp.wasm" };

function toNoteEvents(events: readonly PlaybackEvent[]): NoteEvent[] {
  return events.map((event) => ({
    midiPitch: event.midi,
    startSeconds: event.start,
    endSeconds: event.start + event.duration,
    velocity: Math.max(1, Math.round(event.velocity * 127)),
    isDrum: event.family === "drums",
    instrumentGroup: event.family,
  }));
}

/** Synthesized browser preview (instruments.js) with one shared timeline for every score part. */
export function MidiPlayer({ notes }: { notes: readonly CanonicalNote[] }) {
  const plan = useMemo(() => createPlaybackPlan(notes), [notes]);
  const duration = plan.reduce((maximum, event) => Math.max(maximum, event.start + event.duration), 0);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const engineRef = useRef<Engine | undefined>(undefined);
  const progressTimer = useRef<number | undefined>(undefined);
  const runId = useRef(0);

  const cancelPlayback = (resetPosition: boolean) => {
    runId.current += 1;
    if (progressTimer.current !== undefined) window.clearInterval(progressTimer.current);
    progressTimer.current = undefined;
    engineRef.current?.stop();
    setPlaying(false);
    if (resetPosition) setPosition(0);
  };

  const play = async (from: number) => {
    cancelPlayback(false);
    const currentRun = runId.current;
    // One engine for the app's lifetime — created on the first user gesture.
    // No sample downloads, no CDN, no network at play time; instant on replays.
    if (!engineRef.current) {
      engineRef.current = await createEngine(ENGINE_ASSETS);
      await engineRef.current.ready;
    }
    if (currentRun !== runId.current) return;
    const engine = engineRef.current;
    const safeFrom = Math.min(Math.max(0, from), duration);
    setPosition(safeFrom);
    setPlaying(true);
    const startedAt = engine.context.currentTime;
    progressTimer.current = window.setInterval(() => {
      setPosition(Math.min(duration, safeFrom + engine.context.currentTime - startedAt));
    }, 50);
    await engine.play(toNoteEvents(playbackPlanFrom(plan, safeFrom)));
    if (currentRun === runId.current) cancelPlayback(true);
  };

  const startPlayback = (from: number) => {
    void play(from).catch((error: unknown) => {
      cancelPlayback(false);
      console.error("[music-to-score] note playback failed", error);
    });
  };

  const seek = (next: number) => {
    const wasPlaying = playing;
    cancelPlayback(false);
    setPosition(next);
    if (wasPlaying && next < duration) startPlayback(next);
  };

  useEffect(() => () => {
    runId.current += 1;
    if (progressTimer.current !== undefined) window.clearInterval(progressTimer.current);
    void engineRef.current?.dispose();
    engineRef.current = undefined;
  }, []);

  useEffect(() => {
    cancelPlayback(true);
  }, [plan]);

  return <div className="midi-transport">
    <button className="midi-play" disabled={!plan.length} onClick={() => playing ? cancelPlayback(true) : startPlayback(position)}>{playing ? "■ Stop" : "▶ Play"}</button>
    <input className="midi-progress" aria-label="Playback position" type="range" min={0} max={Math.max(duration, 0.01)} step={0.01} value={Math.min(position, duration)} onChange={(event) => seek(Number(event.target.value))} />
    <span className="midi-time">{clockLabel(position)} / {clockLabel(duration)}</span>
  </div>;
}
