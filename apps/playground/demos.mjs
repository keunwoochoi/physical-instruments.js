/**
 * Owner-curated demo set / recognisable music across every instrument family
 * in the library. Owner direction 2026-07-22: the demo page's job is to show
 * *this physical-model engine sounding good on music a listener actually knows* —
 * "Sad But True" through the distorted-guitar model, "Wave" through the nylon
 * guitar, "Take Five" through the piano. License-hygiene is recorded honestly
 * in agentic-docs/licensing.md (not prefixed by a gate) and the user's
 * local-only "＋ Your MIDI" button stays for any file the repo does not carry.
 *
 *   • Owner-named tracks: MuScriptor medium transcripts (MPS, 2026-07-22) of
 *     the owner's own recordings or third-party works / see licensing.md for
 *     per-file provenance and licence status.
 *   • Public-Domain classical (Mutopia Project): Beethoven Symphony №5 Mvt I,
 *     Chopin Nocturne Op.9 №2, plus the two Bach organ works (re-voiced to
 *     organ). Mutopia PD sequences are clean-permissive.
 *   • Web MIDI from a public archive (github.com/Possibly93/possibly93.github.io,
 *     /c/programs/midi/songs/, verified 2026-07-22): recognised pop/jazz/rock/
 *     latin/folk instrumentals. These are third-party works, demo-only.
 *
 * A demo entry: { id, name, genre, combo, midi, instrument (null = keep the
 * MIDI's own per-part instruments), excerpt (seconds), mix, stretch? }. The
 * browser fetches `midi`, the Node harness reads it; both call
 * parseMidi()+processMidi(). Provenance + sha + licence are in
 * agentic-docs/licensing.md ("Demo music").
 */

export const DEMOS = [
  { id: "you-and-i", name: "you and i", genre: "keunwoo.OOO - the best music in the world",
    combo: "piano / drums / strings / bass / guitar / woodwind",
    midi: "./midi/you-and-i.mid", instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=Am-8_vyIJf0",
    mix: {
      piano: { gain: 0.36, pan: 0.12 }, drums: { gain: 0.18, pan: 0 },
      strings: { gain: 0.22, pan: -0.2 }, bass: { gain: 0.42, pan: 0 },
      guitar: { gain: 0.30, pan: -0.18 }, woodwind: { gain: 0.26, pan: 0.22 },
    } },
  { id: "windup", name: "Windup", genre: "Hayoung Lyou - jazz piano trio",
    combo: "piano / bass / drums", midi: "./midi/hayoung.mid",
    instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=vdV1UEqv5CA",
    mix: {
      piano: { gain: 0.40, pan: 0.10 }, bass: { gain: 0.44, pan: 0 },
      drums: { gain: 0.18, pan: 0 },
    } },
  { id: "yyz", name: "YYZ", genre: "Rush - instrumental prog rock",
    combo: "synth / bass / drums", midi: "./midi/yyz-rush.mid",
    youtube: "https://www.youtube.com/watch?v=LxI6kO2gxpM",
    instrument: null, excerpt: 60,
    mix: {
      synth: { gain: 0.40, pan: 0 }, bass: { gain: 0.44, pan: 0 },
      drums: { gain: 0.22, pan: 0 },
    } },
  { id: "take-five", name: "Take Five", genre: "Dave Brubeck - jazz",
    combo: "piano / sax (woodwind)", midi: "./midi/take-five.mid",
    youtube: "https://www.youtube.com/watch?v=vmDDOFWBn1U",
    instrument: null, excerpt: 60,
    mix: {
      piano: { gain: 0.44, pan: 0.10 }, woodwind: { gain: 0.36, pan: -0.20 },
    } },
  { id: "wave", name: "Wave", genre: "Antônio Jobim - bossa nova",
    combo: "nylon guitar / bass / drums / piano / strings",
    midi: "./midi/wave-jobim.mid", instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=Af0Cdw7v3aI",
    mix: {
      guitar: { gain: 0.42, pan: -0.15 }, piano: { gain: 0.32, pan: 0.10 },
      bass: { gain: 0.46, pan: 0 }, drums: { gain: 0.20, pan: 0 },
      strings: { gain: 0.22, pan: 0.20 }, vibraphone: { gain: 0.28, pan: -0.10 },
    } },
  { id: "chopin", name: "Nocturne Op.9 №2", genre: "Frédéric Chopin - solo piano",
    combo: "piano", midi: "./midi/chopin-nocturne-op9-no2.mid",
    youtube: "https://www.youtube.com/watch?v=otkUpr7B5Eo",
    instrument: null, excerpt: 60,
    mix: { piano: { gain: 0.44, pan: 0 } } },
  { id: "beethoven-5", name: "Symphony №5 - Mvt I", genre: "L.v. Beethoven - orchestral",
    combo: "strings / woodwind / timpani",
    midi: "./midi/beethoven-symphony-5-mvt1.mid", instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=J26C7z1M1EU",
    mix: {
      woodwind: { gain: 0.26, pan: 0.20 }, violin: { gain: 0.34, pan: -0.10 },
      viola: { gain: 0.30, pan: 0.10 }, cello: { gain: 0.32, pan: -0.15 },
      contrabass: { gain: 0.36, pan: 0 }, percussion: { gain: 0.22, pan: 0 },
    } },
  { id: "axel-f", name: "Axel F", genre: "Harold Faltermeyer - 80s synth theme",
    combo: "synth / bass / drums / strings / mallets / woodwind",
    midi: "./midi/axel-f.mid", instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=NgIYqLrTqDI",
    mix: {
      synth: { gain: 0.40, pan: 0 }, drums: { gain: 0.22, pan: 0 },
      strings: { gain: 0.26, pan: 0.20 }, bass: { gain: 0.42, pan: -0.15 },
      vibraphone: { gain: 0.26, pan: -0.25 }, xylophone: { gain: 0.26, pan: 0.25 },
      woodwind: { gain: 0.24, pan: 0.10 },
    } },
  { id: "tico-tico", name: "Tico Tico no Fubá", genre: "Zequinha de Abreu - latin choro",
    combo: "piano / bass / drums",
    midi: "./midi/tico-tico.mid", instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=JzGS7OSDzW4",
    mix: {
      piano: { gain: 0.44, pan: 0.10 }, bass: { gain: 0.40, pan: 0 },
      drums: { gain: 0.20, pan: 0 },
    } },
  { id: "orange-blossom", name: "Orange Blossom Special",
    genre: "Ervin T. Rouse - bluegrass fiddle instrumental",
    combo: "steel guitar / fiddle (viola) / electric guitar / bass / woodwind / celesta",
    midi: "./midi/orange-blossom-special.mid", instrument: null, excerpt: 60,
    youtube: "https://www.youtube.com/watch?v=Bd2J7OtxkKk",
    mix: {
      "guitar-steel": { gain: 0.42, pan: -0.20 }, viola: { gain: 0.40, pan: 0.20 },
      guitar: { gain: 0.34, pan: -0.10 }, "guitar-electric": { gain: 0.30, pan: 0.10 },
      bass: { gain: 0.42, pan: 0 }, woodwind: { gain: 0.26, pan: 0.25 },
      celesta: { gain: 0.22, pan: 0.30 }, pizzicato: { gain: 0.26, pan: -0.10 },
    } },
  { id: "toccata", name: "Toccata & Fugue in D minor", genre: "Baroque - J.S. Bach (BWV 565)",
    combo: "organ", midi: "./midi/bach-toccata-fugue-dm.mid",
    instrument: "organ", excerpt: 52,
    // Slow the famous opening flourish (0–12s) by 1.5× / it raced past at the
    // MIDI's 120 BPM placeholder tempo. A real organist lingers here. Excerpt
    // stays at 52s source material; the stretched intro adds ~6s, so the clip
    // plays for ~58s.
    stretch: [{ from: 0, to: 12, factor: 1.5 }],
    mix: { organ: { gain: 0.44 } } },
  { id: "invention", name: "Invention No. 2", genre: "Baroque - J.S. Bach (BWV 773)",
    combo: "organ", midi: "./midi/bach-invention-2.mid",
    instrument: "organ", excerpt: 49, mix: { organ: { gain: 0.5 } } },
];

/**
 * Turn a parsed MIDI (from packages/midi parseMidi) into the showcase note list:
 * take the first `excerpt` seconds from the first onset, re-zero, and either keep
 * each note's own instrument (instrument == null → multi-instrument, routed by
 * gmProgramToGroup) or re-voice every note onto one instrument. Pure / no I/O —
 * so the browser and the Node render harness share it exactly.
 */
export function processMidi(parsed, demo) {
  const src = parsed.notes;
  if (!src.length) return [];
  const first = src[0].startSeconds;
  // Time-stretch regions (demo.stretch: [{from,to,factor}]). Stretch is applied
  // in source-seconds, before excerpt windowing. A note entirely inside a stretch
  // region shifts and lengthens by `factor`; a note crossing a boundary is split
  // at that boundary into two segments, each stretched by its own region's factor
  // (a 12s note wholly past the intro keeps its original duration). Pure / no
  // I/O / so the browser and Node harness share it exactly.
  const stretches = (demo.stretch ?? []).slice().sort((a, b) => a.from - b.from);
  function stretchPoint(t) {
    let shifted = t;
    for (const s of stretches) {
      if (t <= s.from) continue;         // before this region / no change
      const span = Math.min(t, s.to) - s.from;
      shifted += span * (s.factor - 1);
    }
    return shifted;
  }
  const end = first + (demo.excerpt ?? 50);
  const out = [];
  for (const nRaw of src) {
    if (nRaw.startSeconds >= end) continue;
    const rawStart = nRaw.startSeconds - first;
    const rawEnd = Math.min(nRaw.endSeconds, end) - first;
    if (rawEnd <= rawStart) continue;
    const start = stretchPoint(rawStart);
    const finish = stretchPoint(rawEnd);
    if (finish <= start) continue;
    out.push({
      instrumentGroup: demo.instrument ?? nRaw.instrumentGroup,
      midiPitch: nRaw.midiPitch,
      startSeconds: +start.toFixed(5),
      endSeconds: +finish.toFixed(5),
      velocity: nRaw.velocity,
      isDrum: demo.instrument ? false : !!nRaw.isDrum,
    });
  }
  return out;
}
