/**
 * Standard MIDI File parsing for instruments.js.
 * Formats 0 and 1, metrical division, tempo maps, running status, GM program →
 * instrument-family mapping, channel-10 drums, CC64 sustain pedal.
 *
 * Structurally compatible with instruments.js core's NoteEvent — no import needed,
 * the two packages stay decoupled.
 */

export interface MidiNote {
  midiPitch: number;
  startSeconds: number;
  endSeconds: number;
  velocity: number; // 1–127
  isDrum?: boolean;
  instrumentGroup?: string;
}

export interface MidiPedal {
  instrumentGroup: string;
  isDrum?: boolean;
  on: boolean;
  timeSeconds: number;
}

export interface ParsedMidi {
  name: string;
  durationSeconds: number;
  notes: MidiNote[];
  pedals: MidiPedal[];
  trackCount: number;
  format: number;
}

/** GM program number (0-based) → instruments.js family. */
export function gmProgramToGroup(program: number): string {
  if (program < 8) return "piano";
  if (program < 16) return program === 8 || program === 11 ? "vibraphone" : "mallet"; // celesta/glock/musicbox/vibes/marimba/xylo/bells/dulcimer
  if (program < 24) return "epiano"; // organs — placeholder
  if (program === 24) return "guitar"; // nylon
  if (program === 25) return "guitar-steel";
  if (program < 29) return "guitar-electric"; // jazz/clean/muted
  if (program < 32) return "guitar-distorted"; // overdriven/distortion/harmonics
  if (program < 40) return "bass";
  if (program < 56) return "strings"; // strings + ensemble + choir
  if (program < 64) return "brass";
  if (program < 80) return "woodwind"; // reeds + pipes
  if (program < 104) return "synth"; // leads, pads, fx
  if (program < 112) return "guitar"; // "ethnic" plucked: sitar/banjo/koto…
  return "percussion";
}

class Reader {
  private v: DataView;
  pos = 0;
  constructor(public bytes: Uint8Array) {
    this.v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }
  u8(): number {
    return this.v.getUint8(this.pos++);
  }
  u16(): number {
    const x = this.v.getUint16(this.pos);
    this.pos += 2;
    return x;
  }
  u32(): number {
    const x = this.v.getUint32(this.pos);
    this.pos += 4;
    return x;
  }
  ascii(n: number): string {
    let s = "";
    for (let i = 0; i < n; i++) s += String.fromCharCode(this.u8());
    return s;
  }
  varlen(): number {
    let x = 0;
    for (let i = 0; i < 4; i++) {
      const b = this.u8();
      x = (x << 7) | (b & 0x7f);
      if ((b & 0x80) === 0) break;
    }
    return x;
  }
  skip(n: number): void {
    this.pos += n;
  }
  get eof(): boolean {
    return this.pos >= this.bytes.byteLength;
  }
}

interface RawEvent {
  tick: number;
  order: number; // stable tie-break across merged tracks
  kind: "on" | "off" | "prog" | "cc64" | "tempo" | "name";
  ch: number;
  a: number;
  b: number;
  text?: string;
}

export function parseMidi(input: ArrayBuffer | Uint8Array): ParsedMidi {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const r = new Reader(bytes);
  if (r.ascii(4) !== "MThd") throw new Error("not a Standard MIDI File (missing MThd)");
  const hlen = r.u32();
  const format = r.u16();
  const ntrks = r.u16();
  const division = r.u16();
  r.skip(hlen - 6);
  if (division & 0x8000) throw new Error("SMPTE-division MIDI files are not supported yet");
  if (format > 1) throw new Error(`MIDI format ${format} not supported (formats 0/1 only)`);

  // ---- pass 1: merge every track's events into one tick-ordered list ----
  const events: RawEvent[] = [];
  let order = 0;
  for (let t = 0; t < ntrks && !r.eof; t++) {
    if (r.ascii(4) !== "MTrk") throw new Error(`track ${t}: missing MTrk chunk`);
    const len = r.u32();
    const end = r.pos + len;
    let tick = 0;
    let running = 0;
    while (r.pos < end) {
      tick += r.varlen();
      let status = r.u8();
      if (status < 0x80) {
        // running status: this byte is data
        r.pos--;
        status = running;
      } else if (status < 0xf0) {
        running = status;
      }
      const type = status & 0xf0;
      const ch = status & 0x0f;
      if (status === 0xff) {
        const meta = r.u8();
        const mlen = r.varlen();
        if (meta === 0x51 && mlen === 3) {
          const us = (r.u8() << 16) | (r.u8() << 8) | r.u8();
          events.push({ tick, order: order++, kind: "tempo", ch: 0, a: us, b: 0 });
        } else if ((meta === 0x03 || meta === 0x01) && events.every((e) => e.kind !== "name")) {
          const text = new TextDecoder().decode(bytes.subarray(r.pos, r.pos + mlen));
          r.skip(mlen);
          events.push({ tick, order: order++, kind: "name", ch: 0, a: 0, b: 0, text });
        } else {
          r.skip(mlen);
        }
      } else if (status === 0xf0 || status === 0xf7) {
        r.skip(r.varlen());
      } else if (type === 0x90) {
        const p = r.u8(), v = r.u8();
        events.push({ tick, order: order++, kind: v === 0 ? "off" : "on", ch, a: p, b: v });
      } else if (type === 0x80) {
        const p = r.u8();
        r.u8();
        events.push({ tick, order: order++, kind: "off", ch, a: p, b: 0 });
      } else if (type === 0xb0) {
        const cc = r.u8(), v = r.u8();
        if (cc === 64) events.push({ tick, order: order++, kind: "cc64", ch, a: v, b: 0 });
      } else if (type === 0xc0) {
        events.push({ tick, order: order++, kind: "prog", ch, a: r.u8(), b: 0 });
      } else if (type === 0xd0) {
        r.skip(1);
      } else if (type === 0xa0 || type === 0xe0) {
        r.skip(2);
      } else {
        throw new Error(`track ${t}: unexpected status byte 0x${status.toString(16)} at ${r.pos}`);
      }
    }
    r.pos = end;
  }
  events.sort((x, y) => x.tick - y.tick || x.order - y.order);

  // ---- pass 2: tempo map (piecewise tick → seconds) ----
  let sec = 0;
  let lastTick = 0;
  let usPerQuarter = 500_000; // MIDI default: 120 BPM
  const toSeconds = (tick: number) => sec + ((tick - lastTick) * usPerQuarter) / division / 1e6;

  // ---- pass 3: walk merged events, pair notes, track programs & pedal ----
  const notes: MidiNote[] = [];
  const pedals: MidiPedal[] = [];
  const open = new Map<string, { start: number; vel: number; group: string; isDrum: boolean }[]>();
  const program: number[] = new Array(16).fill(0);
  let name = "";

  const groupOf = (ch: number) => (ch === 9 ? "percussion" : gmProgramToGroup(program[ch]!));

  for (const e of events) {
    const t = toSeconds(e.tick);
    switch (e.kind) {
      case "tempo":
        sec = t;
        lastTick = e.tick;
        usPerQuarter = e.a;
        break;
      case "name":
        name = e.text ?? "";
        break;
      case "prog":
        program[e.ch] = e.a;
        break;
      case "on": {
        const key = `${e.ch}:${e.a}`;
        const stack = open.get(key) ?? [];
        stack.push({ start: t, vel: e.b, group: groupOf(e.ch), isDrum: e.ch === 9 });
        open.set(key, stack);
        break;
      }
      case "off": {
        const stack = open.get(`${e.ch}:${e.a}`);
        const o = stack?.shift();
        if (o) {
          notes.push({
            midiPitch: e.a,
            startSeconds: +o.start.toFixed(5),
            endSeconds: +Math.max(t, o.start + 0.02).toFixed(5),
            velocity: o.vel,
            isDrum: o.isDrum,
            instrumentGroup: o.group,
          });
        }
        break;
      }
      case "cc64":
        pedals.push({ instrumentGroup: groupOf(e.ch), isDrum: e.ch === 9, on: e.a >= 64, timeSeconds: +t.toFixed(5) });
        break;
    }
  }
  // close any hanging notes at the end of the file
  let duration = 0;
  for (const n of notes) duration = Math.max(duration, n.endSeconds);
  for (const [key, stack] of open) {
    const pitch = Number(key.split(":")[1]);
    for (const o of stack) {
      const end = Math.max(duration, o.start + 0.5);
      notes.push({
        midiPitch: pitch,
        startSeconds: +o.start.toFixed(5),
        endSeconds: +end.toFixed(5),
        velocity: o.vel,
        isDrum: o.isDrum,
        instrumentGroup: o.group,
      });
      duration = Math.max(duration, end);
    }
  }
  notes.sort((a, b) => a.startSeconds - b.startSeconds);
  return { name, durationSeconds: +duration.toFixed(3), notes, pedals, trackCount: ntrks, format };
}
