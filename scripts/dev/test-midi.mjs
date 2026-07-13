#!/usr/bin/env node
/** Deterministic test for @instrumentsjs/midi: synthesizes an SMF with a tempo
 *  change, running status, program change, and CC64, then asserts the parse. */
import { strict as assert } from "node:assert";
import { parseMidi, gmProgramToGroup } from "../../packages/midi/dist/index.js";

const bytes = [];
const push = (...xs) => bytes.push(...xs);
const str = (s) => push(...[...s].map((c) => c.charCodeAt(0)));
const u32 = (x) => push((x >>> 24) & 255, (x >>> 16) & 255, (x >>> 8) & 255, x & 255);
const u16 = (x) => push((x >>> 8) & 255, x & 255);

// header: format 1, 2 tracks, 96 ticks/quarter
str("MThd"); u32(6); u16(1); u16(2); u16(96);

// track 0: tempo 120 BPM at tick 0, tempo 60 BPM at tick 96
{
  const t = [];
  const tp = (...xs) => t.push(...xs);
  tp(0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20); // 500000 µs/q = 120 BPM
  tp(0x60, 0xff, 0x51, 0x03, 0x0f, 0x42, 0x40); // at tick 96 → 1000000 µs/q = 60 BPM
  tp(0x00, 0xff, 0x2f, 0x00);
  str("MTrk"); u32(t.length); push(...t);
}

// track 1: program 25 (guitar family), A4 quarter note, second note via RUNNING
// STATUS, then pedal down/up
{
  const t = [];
  const tp = (...xs) => t.push(...xs);
  tp(0x00, 0xc0, 0x19); // program change ch0 → 25 (0-based: guitar)
  tp(0x00, 0x90, 0x45, 0x64); // note on A4(69) vel 100 @ tick 0
  tp(0x60, 0x45, 0x00); // running status: vel-0 note-off @ tick 96
  tp(0x00, 0x40, 0x50); // running status: note on E4(64) vel 80 @ tick 96
  tp(0x60, 0x80, 0x40, 0x40); // explicit note-off @ tick 192
  tp(0x00, 0xb0, 0x40, 0x7f); // CC64 pedal down @ tick 192
  tp(0x60, 0x40, 0x00); // running status CC: pedal up @ tick 288
  tp(0x00, 0xff, 0x2f, 0x00);
  str("MTrk"); u32(t.length); push(...t);
}

const parsed = parseMidi(new Uint8Array(bytes));
const eq = (a, b, m) => assert.ok(Math.abs(a - b) < 1e-6, `${m}: ${a} vs ${b}`);

assert.equal(parsed.format, 1);
assert.equal(parsed.trackCount, 2);
assert.equal(parsed.notes.length, 2, "two notes");
const [n1, n2] = parsed.notes;

// tick 0–96 at 120 BPM = 0.0–0.5 s
eq(n1.startSeconds, 0.0, "n1 start");
eq(n1.endSeconds, 0.5, "n1 end (quarter at 120 BPM)");
assert.equal(n1.midiPitch, 69);
assert.equal(n1.velocity, 100);
assert.equal(n1.instrumentGroup, "guitar-steel", "program 25 → steel-string acoustic");

// tick 96–192: tempo changed to 60 BPM at tick 96, so this quarter lasts 1.0 s
eq(n2.startSeconds, 0.5, "n2 start");
eq(n2.endSeconds, 1.5, "n2 end (quarter at 60 BPM)");
assert.equal(n2.velocity, 80, "running-status note-on");

// pedal: down at tick 192 (=1.5 s), up at tick 288 (=2.5 s)
assert.equal(parsed.pedals.length, 2);
assert.equal(parsed.pedals[0].on, true);
eq(parsed.pedals[0].timeSeconds, 1.5, "pedal down");
assert.equal(parsed.pedals[1].on, false);
eq(parsed.pedals[1].timeSeconds, 2.5, "pedal up");

// GM mapping spot checks
assert.equal(gmProgramToGroup(0), "piano");
assert.equal(gmProgramToGroup(24), "guitar");
assert.equal(gmProgramToGroup(27), "guitar-electric");
assert.equal(gmProgramToGroup(30), "guitar-distorted");
assert.equal(gmProgramToGroup(33), "bass");
assert.equal(gmProgramToGroup(41), "strings");
assert.equal(gmProgramToGroup(57), "brass");
assert.equal(gmProgramToGroup(74), "woodwind");

console.log("midi parser: all assertions passed");
