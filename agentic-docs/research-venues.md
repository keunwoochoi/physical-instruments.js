# Research venues — where the sound-synthesis literature lives

Owner doc for literature searches (port-audit paper hunts, algorithm design, eval
methodology). When implementing or improving a physical model, search these first.

## Core venues for physical modeling / audio DSP

| Venue | What to find there |
|---|---|
| **DAFx** (Int'l Conf. on Digital Audio Effects) | THE venue for real-time synthesis & effects algorithms: waveguides, WDF, virtual analog, anti-aliasing (polyBLEP/ADAA), hammer/bow/reed models. Open-access proceedings, dafx.de. |
| **ICMC** (Int'l Computer Music Conference) | The classic CCRMA-era venue — Smith's waveguide papers, Smith & Van Duyne commuted piano (1995), Cook's PhISEM. Older gold is here. |
| **SMC** (Sound and Music Computing) | European counterpart; physical modeling, mass-interaction (CORDIS-ANIMA lineage), sound design. Open access. |
| **NIME** (New Interfaces for Musical Expression) | Instruments-as-interfaces: MPE-style expressivity, playability studies, controller→synthesis mapping. Less DSP, more interaction. |
| **WAC** (Web Audio Conference) | Our platform's home turf: AudioWorklet/WASM engineering, browser audio benchmarks, Tone.js-era papers. ~Biennial. |
| **AES** (conventions + JAES) | Pro-audio engineering: loudness (BS.1770 lineage), perceptual evaluation, room/reverb, transducers. |
| **ICASSP / WASPAA** (IEEE) | Signal-processing rigor: source-filter models, DDSP-adjacent neural synthesis, fast filter structures. WASPAA = audio-focused workshop. |
| **ISMIR** | MIR — evaluation methodology, MAESTRO-style datasets, transcription (customer zero's world). Keunwoo's home community. |

## Journals

- **JASA** (J. Acoust. Soc. Am.) — instrument acoustics ground truth: Chabassier grand-piano model, Weinreich coupled piano strings (1977), bar/plate modal data.
- **IEEE/ACM TASLP** — longer-form synthesis papers (Bank & Välimäki piano/string modeling).
- **Computer Music Journal** (MIT Press) — Jaffe-Smith extended Karplus-Strong (1983), Rabenstein/Trautmann FTM.
- **EURASIP J. Audio/Speech/Music Proc.** — Bank 2003 physically-informed piano.

## People whose citation trails pay off (the "music geeks")

Julius O. Smith III (CCRMA — waveguides, PASP online book), Vesa Välimäki (Aalto —
fractional delay, string models, VA anti-aliasing), Balázs Bank (piano), Stefan Bilbao
(Edinburgh — FDTD, *Numerical Sound Synthesis*), Perry Cook (STK, PhISEM), Gary Scavone
(McGill — STK, winds), Kurt Werner (WDF), Émilie Gillet (Mutable — open code, not papers),
Romain Michon (Faust physical models), Antoine Chaigne (instrument acoustics).

## Preprints / search entry points

- arXiv **eess.AS** + **cs.SD** — neural/DDSP synthesis lands here first.
- Zenodo/DAFx + ICMC (quod.lib.umich.edu) archives — most proceedings are open.
- CCRMA STANM reports + Smith's PASP/Filters online books — ccrma.stanford.edu/~jos/.
