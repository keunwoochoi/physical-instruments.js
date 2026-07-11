# Yotam Mann — full profile (researched 2026-07-11)

Operational lens: `skills/review-as/references/yotam.md`

## Verified background
Creator of Tone.js (2014–), the dominant JS framework for interactive browser music (~216k npm downloads/week as of 2026-07). Music + CS at UC Berkeley CNMAT. Adjunct professor, NYU Tisch ITP. Google Creative Lab / Magenta collaborations (NSynth "Sound Maker"; Chrome Music Lab was built on Tone.js). Co-founder (2020) of Never Before Heard Sounds (browser AI audio studio). WAC 2015 talk "Interactive Music with Tone.js". Creative Capital grantee, NEW INC.

- https://yotammann.info/ · https://github.com/Tonejs/Tone.js · https://wac.ircam.fr/pdf/wac15_submission_40.pdf · https://changelog.com/practicalai/143

## Taste profile
Making music *usable* in the browser: transport/scheduling, look-ahead timing vs Web Audio jitter, musician-friendly API design (note names, tempo-relative time). Lived through ScriptProcessor→AudioWorklet migration. NBHS shows he cares about AI/synthesis in the hands of actual musicians, not tech demos.

## Would praise / would attack
Praise: clean AudioWorklet architecture; sample-accurate scheduling; API a musician can read; small core; composability with the Web Audio graph (and Tone.js); mobile Safari support; no sample downloads.
Attack: badly reinvented scheduling; audio-thread blocking; glitches under load; DSP-PhD-only API; walled gardens; ignoring iOS unlock/latency realities.

## Notes
Tone.js deliberately declined to become a sound library (issue #290 closed unimplemented) — instruments.js is complementary to Tone, and Tone interop is an adoption channel.
