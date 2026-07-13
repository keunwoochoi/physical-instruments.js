# Pianoteq-class piano — the piano north star

Date: 2026-07-11
Status: accepted as direction (Keunwoo: "I want our piano sound to be as good as the
Pianoteq sound"; familiarity-ladder principle makes piano the highest bar). Authorizes
the phased campaign below; does NOT authorize shipping any reference sample data.

## Motivation

Pianoteq (Modartt) is the world benchmark for physically-modeled piano — ~20 years of
expert calibration. Our v4 piano passes AB gates against toy baselines; the owner's
target is the real benchmark. The honest gap decomposes into enumerable engineering,
and the auto-research loop is structurally suited to the biggest item (per-key
calibration), which for Modartt was years of human labor.

## Thesis

Close the gap in phases, each gated by listening + metrics against a high-quality
reference corpus (Salamander Grand, CC-BY, 48 kHz/24-bit, 16 velocity layers —
reference-use only, attributed in the ledger). The LLM loop fits per-key parameters
the way Modartt's tools + humans did, at a fraction of the labor.

## Phased plan

- **P1 — Per-key calibration table** (the big one): 88-entry table {inharmonicity B →
  dispersion-cascade design, t60 ladder, hammer K/p/contact, strike point, unison
  detune, level} fitted key-by-key against Salamander (multi-velocity), by a
  calibration loop (agent or scripted optimizer per key + agent for structure).
  Exit: per-key partial frequencies within a few cents of Salamander's measured
  inharmonicity curve; per-partial decay rates within ~20%; velocity→brightness
  curves matching per key.
- **P2 — Soundboard/radiation**: replace the 6-mode ladder + knock with a proper
  soundboard response (init-time-synthesized dense modal IR or measured-IR-informed
  parametric); listener/mic perspective as a stereo body pair.
- **P3 — ff realism**: longitudinal modes / phantom partials (band-limited),
  hammer multi-contact ripple, per-key strike-point combs.
- **P4 — Pedal vocabulary**: half-pedal (partial damper), repedaling capture,
  una corda (hammer shift = fewer strings + softer contact), sostenuto.
- Each phase: match-reference loop discipline, held-out keys/velocities, human gate.

## Evidence base

Salamander corpus: freepats.zenvoid.org mirror, CC-BY 3.0 (Alexander Holm) — ledger
row required. Literature anchors: Bank & Chabassier piano modeling; Stulov felt;
Weinreich coupled strings; Conklin longitudinal modes/phantom partials (JASA);
Askenfelt & Jansson touch papers; Smith/Van Duyne commuted piano.

## Deferred until demanded

Mic models/binaural, historic temperaments, physical noise layers (key/action
mechanics beyond the thump), morphing between piano models.
