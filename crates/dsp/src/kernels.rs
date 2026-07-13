//! Instrument kernels. All state is fixed-size and allocated inside the voice pool at
//! engine init; `render` never allocates. Every recursive state variable passes through
//! `flush_denormal` once per block.
//!
//! Synthesis references (papers/manuals only — see agentic-docs/licensing.md):
//! - Extended Karplus-Strong: Jaffe & Smith 1983; Smith, *Physical Audio Signal Processing*.
//! - Modal bar synthesis: Adrien 1991; tuned-bar partial ratios from standard acoustics
//!   literature (Rossing, *Science of Percussion Instruments*: marimba ~1:4:10,
//!   free bar 1:2.76:5.40:8.93).
//! - Two-pole resonator form: y[n] = 2r·cos(ω)·y[n-1] − r²·y[n-2] + g·x[n].

use crate::flush_denormal;

pub const MAX_BLOCK: usize = 128;
pub const MAX_MODES: usize = 8;
/// 2048 samples ≥ one period of 23.4 Hz at 48 kHz — covers 5-string bass low B (30.9 Hz).
pub const PLUCK_BUF: usize = 2048;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Instrument {
    Marimba = 0,
    Vibraphone = 1,
    Glockenspiel = 2,
    MusicBox = 3,
    Guitar = 4,
    Bass = 5,
    EPiano = 6,
    Drums = 7,
    /// Classic subtractive pad (PRINCIPLES #5: no paradigm purity — fast + beautiful wins).
    SynthPad = 8,
    /// Acoustic piano: multi-string waveguide with dispersion + hammer excitation.
    Piano = 9,
    /// Steel-string acoustic (brighter pluck than the nylon `Guitar`).
    GuitarSteel = 10,
    /// Electric guitar, clean — sustainy dark string through a lightly driven amp.
    GuitarElectric = 11,
    /// Electric guitar, distorted — same string, hot ADAA amp stage on the track bus.
    GuitarDistorted = 12,
    /// Rock kit: bigger looser snare, hard beater click, washy crash, loud ride ping.
    DrumsRock = 13,
    /// Jazz kit: dark ride-forward, small dry kick, high-tuned shell-toned snare.
    DrumsJazz = 14,
}

impl Instrument {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Vibraphone,
            2 => Self::Glockenspiel,
            3 => Self::MusicBox,
            4 => Self::Guitar,
            5 => Self::Bass,
            6 => Self::EPiano,
            7 => Self::Drums,
            8 => Self::SynthPad,
            9 => Self::Piano,
            10 => Self::GuitarSteel,
            11 => Self::GuitarElectric,
            12 => Self::GuitarDistorted,
            13 => Self::DrumsRock,
            14 => Self::DrumsJazz,
            _ => Self::Marimba,
        }
    }

    /// All GM drum-kit families (pop/rock/jazz share note semantics: one-shot
    /// percussion, hat-choke note interaction, kit stereo layout).
    pub fn is_drum_kit(self) -> bool {
        matches!(self, Self::Drums | Self::DrumsRock | Self::DrumsJazz)
    }
}

/// Kit voicing style: one GM note map, three parametrizations of the same
/// kick/snare/cymbal machinery (see `DrumVoice::start`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KitStyle {
    Pop,
    Rock,
    Jazz,
}

/// Default amp-stage settings per instrument: (drive pre-gain, tone lowpass Hz).
/// drive 0.0 = bypass. Electric guitars are DEFINED by their amp — the drive lives
/// on the track bus so simultaneous notes intermodulate like a real amplifier.
pub fn amp_defaults(inst: Instrument) -> (f32, f32) {
    match inst {
        Instrument::GuitarElectric => (1.6, 3200.0),
        // high gain: preamp gain must keep the tanh saturated for seconds so the
        // note SINGS while the string decays >20 dB (drive 11 fell linear after
        // ~1 s — a crunch, not a lead channel); tone (cab corner) at 5.5 kHz —
        // r3: 3.4 kHz choked the presence the refs keep (5–7.5 kHz at −13…−19
        // rel max); fizz stays gated by the 12 kHz collapse either way
        Instrument::GuitarDistorted => (90.0, 5500.0),
        _ => (0.0, 0.0),
    }
}

/// WDF circuit-sim amp voicing per instrument (design doc
/// 2026-07-13-wdf-amp-circuit-sim.md). `None` = no WDF voicing (behavioral only).
/// `Some(true)` = lead channel (high-gain 12AX7), `Some(false)` = clean.
/// This is only the *availability* table; the behavioral chain stays DEFAULT —
/// a per-track `wdf_on` flag (default matching `WDF_AMP_DEFAULT`) selects it, and
/// the listening gate flips the default. Kept here so the switch is kernels-side.
pub const WDF_AMP_DEFAULT: bool = false;

pub fn amp_wdf_voicing(inst: Instrument) -> Option<bool> {
    match inst {
        Instrument::GuitarElectric => Some(false), // clean channel
        Instrument::GuitarDistorted => Some(true),  // lead channel
        _ => None,
    }
}

/// Amp gain-ride per instrument: (threshold, exponent p = 1 − 1/R, max gain,
/// recovery seconds). 0.0 threshold = bypass. The "amplifier factor" (owner
/// verdict 2026-07-12: notes must remain longer): a tube amp's supply rail sags
/// under attack current and RECOVERS as the string decays, and bias shift in
/// the power stage acts the same way — the amp's gain effectively rises
/// relative to the signal, flattening the envelope so the note sings instead
/// of dying at the string's own rate (Pakarinen & Yeh, "A Review of Digital
/// Techniques for Modeling Vacuum-Tube Guitar Amplifiers", CMJ 33(2) 2009:
/// sag/bias-shift = program-dependent compression). Implemented on the track
/// bus PRE-drive as an upward-only slow gain ride: unity at attack (velocity
/// dynamics and pick transients pass untouched — the guard), rising toward
/// (thr/env)^p as the envelope falls below thr, capped. For the distorted
/// channel the rising gain re-feeds the ADAA-tanh limiter and extends its
/// hold the way a cascaded-triode preamp does (single tanh at drive 90 spans
/// ~35 dB of limiter range; the FreePats refs behave like 60+ dB — measured
/// t_-3dB 3.6–22.6 s vs our 1.1–3.9 s baseline).
pub fn amp_ride_defaults(inst: Instrument) -> (f32, f32, f32, f32) {
    match inst {
        // clean: ratio 2:1 above the ride knee (p = 0.5), +12 dB cap —
        // NSynth 022 (deep-sag rig) holds slope_sustain −3.0…−3.4 dB/s where
        // the raw string gives −8…−13; moderate ratio keeps the 028 rig's
        // velocity/attack character (owner kept the bright rig, r3).
        Instrument::GuitarElectric => (0.14, 0.67, 6.0, 0.5),
        // distorted: ratio 4:1 (p = 0.75), +20 dB cap — the drive-sustain of
        // a multi-stage preamp; refs hold output within −3 dB for 3.6–8.8 s
        // (wound strings) and 22 s soft-picked.
        Instrument::GuitarDistorted => (0.18, 0.75, 10.0, 0.4),
        _ => (0.0, 0.0, 1.0, 0.1),
    }
}

/// Post-drive cab/presence EQ per instrument: two RBJ peaking sections
/// ((freq Hz, Q, gain dB) × 2), applied on the track bus AFTER the ADAA drive
/// and tone lowpass. 0.0 freq = bypass. This is r3's filed recommendation #1:
/// at drive 90 the tanh is a limiter, so no pre-clip EQ survives into the
/// output spectrum — presence and the cab's LF bump must be POST-clip (r3
/// measured a pre-drive 60 Hz/Q2 bump NEUTRAL-to-worse). A guitar cab is the
/// speaker's own response: a strong low resonance bump and a presence edge
/// before the HF collapse (Zollner ch. 10 speaker curves). Linear stage — no
/// aliasing, no allocation.
pub fn amp_post_eq_defaults(inst: Instrument) -> ((f32, f32, f32), (f32, f32, f32)) {
    match inst {
        // FreePats dist2 refs, band balance re band max (measured 2026-07-12):
        // A2/E2 refs put 250 Hz–2.5 kHz at −12…−22 below the 60–250 Hz cab
        // bump (we sat −4.7…−7.5 — the chug's LF dominance is a cab feature);
        // the B3 ref keeps 2.5–7.5 kHz presence at −9…−13 (we sat −18…−27).
        Instrument::GuitarDistorted => ((105.0, 0.9, 9.0), (4800.0, 0.7, 11.0)),
        _ => ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)),
    }
}

/// Magnetic-pickup resonance per instrument: (resonant-lowpass Hz, Q). The RLC
/// resonance of a real pickup is the core "electric" tone; 0.0 = bypass.
pub fn pickup_defaults(inst: Instrument) -> (f32, f32) {
    match inst {
        // NSynth guitar_electronic refs cliff at a pitch-independent ~1.2-1.5 kHz:
        // a heavily loaded pickup + rolled tone pot pulls the RLC resonance down
        // and damps its Q (Zollner, Physics of the Electric Guitar, ch. 5).
        Instrument::GuitarElectric => (2400.0, 1.8),
        // distorted (r3 refit vs FreePats FSBS dist2 CC0 refs): PRESENCE
        // resonance into the clipper — at drive 90 the tanh is a limiter, so
        // only narrow strong pre-boosts survive into the output spectrum
        // (measured: broad 3 kHz/Q1.5 LOST 4 dB of top vs the old 1700/Q3.4
        // whose hump visibly survived). A real channel boosts 3–5 kHz into
        // the power stage; the refs keep 2.5–5 kHz within −6…−11 dB of the
        // band max where a bare clipped square would sit at −20.
        Instrument::GuitarDistorted => (1700.0, 3.4),
        _ => (0.0, 0.0),
    }
}

pub const MAX_BODY_MODES: usize = 16;

/// Instrument body as a parallel modal resonator bank on the track bus
/// (Karjalainen/Smith commuted-body lineage: string+body are ~linear, so the body
/// can be one shared filter per track). Returns (dry mix, modes[(freq Hz, t60 s, gain)]).
/// A plucked string without this is a string nailed to a plank — the single biggest
/// reason bare EKS guitars sound thin (listening note 2026-07-11).
pub fn body_defaults(inst: Instrument) -> (f32, &'static [(f32, f32, f32)]) {
    match inst {
        // Acoustic guitars: modes FIT to the NSynth reference clusters'
        // early-spectrum envelopes (2026-07-11 loop). The resonator peak gain is
        // |H|peak ≈ g/(2·sin(2πf/sr)), so g = P·2·sin(2πf/48k) for a desired
        // peak P relative to the dry path — the previous rows implied P≈55
        // (+35 dB) at ~100 Hz and drowned both guitars in lows.
        // Nylon (refs 010/014), 16-mode round-2 refit from pooled per-partial
        // deltas (bodydelta.py, 2026-07-11): renders ran +14…+20 dB hot at the
        // low fundamentals / T1 region and −5…−8 dB in the 300–380 / 610–770 /
        // 980–1250 valleys. Dry cut 0.45→0.22, T1 trimmed, denser mid ladder.
        // Body round 2026-07-12: low-mode t60s raised to measured guitar Qs
        // (A0 Q≈25, plate/back cluster Q≈40 — Woodhouse 2012; previous rows
        // sat at half that). Peak |H| is set by g alone (independent of r),
        // so the fitted spectral peaks stay put; the modes ring longer between
        // and after notes — the body speaking (Keunwoo listening verdict).
        Instrument::Guitar => (
            0.22,
            &[
                // A0/T1: g scaled with t60 (lib.rs applies g·(1−r)) so the
                // SKIRT and transient knock keep their fitted level and the
                // peak rises with Q — an underdamped resonator has a taller
                // peak, same skirt. Mid modes below keep peak-fit g instead
                // (their r3 anchors were on-peak partial maxima).
                (100.0, 0.55, 0.0433),  // P=1.65 A0 Helmholtz, Q 25
                (190.0, 0.46, 0.0975),  // P=2.0 T1, Q 40
                (285.0, 0.31, 0.1194),  // P=1.6 T2/back
                (340.0, 0.26, 0.1602),  // P=1.8
                (425.0, 0.21, 0.2113),  // P=1.9
                (520.0, 0.17, 0.2041),  // P=1.5
                (640.0, 0.10, 0.3012),  // P=1.8
                (730.0, 0.09, 0.4198),  // P=2.2
                (850.0, 0.08, 0.3553),  // P=1.6
                (1000.0, 0.07, 0.5743), // P=2.2
                (1120.0, 0.06, 0.7596), // P=2.6
                (1270.0, 0.055, 0.5626), // P=1.7
                (1450.0, 0.05, 0.5283), // P=1.4
                (1700.0, 0.045, 0.4855), // P=1.1
                (2100.0, 0.04, 0.4343), // P=0.8
                (2600.0, 0.035, 0.4006), // P=0.6
            ],
        ),
        // Steel: r3 refit to SOURCE 015 ONLY (r2 pooled 015+021 — two different
        // guitars — which smeared the peaks; body-mode frequency JND is ~1%,
        // Woodhouse et al. 2012). Cross-note triangulation of 015's partial
        // maxima (D2 h10 742 / F#2 h8 746=max / E2 h9 750) pins a strong body
        // peak at ~745 Hz that the old row split into 630/705; other peaks at
        // 258-295, 370-415, ~940, ~1080-1130; dips at 460-590 and ~660.
        Instrument::GuitarSteel => (
            0.28,
            &[
                // low-mode t60s at measured Qs; A0/T1 g scaled with t60 to
                // preserve skirt+knock (see nylon note) — the sustained
                // h1-h2 balance at ff (render -7 dB vs ref +5..+7) says the
                // old A0 peak was low even at its on-peak anchor (G2 h1)
                (100.0, 0.55, 0.0600),   // P=2.3 A0, Q 25 (it9: G2 h1 on-peak still 9 dB shy)
                (190.0, 0.46, 0.0411),   // P=0.83, Q 40
                (258.0, 0.34, 0.1216),   // P=1.8 T1' ref peak
                (295.0, 0.30, 0.1081),   // P=1.4
                (370.0, 0.24, 0.1452),   // P=1.5
                (415.0, 0.21, 0.1520),   // P=1.4
                (505.0, 0.17, 0.0793),   // P=0.6 (dip region 460-590)
                (745.0, 0.10, 0.4285),   // P=2.2 ref peak (was split 630/705)
                (820.0, 0.09, 0.2572),   // P=1.2
                (940.0, 0.085, 0.3437),  // P=1.4
                (1080.0, 0.075, 0.3383), // P=1.2
                (1180.0, 0.07, 0.3078),  // P=1.0
                (1350.0, 0.065, 0.3869), // P=1.1
                (1520.0, 0.06, 0.3561),  // P=0.9
                (2400.0, 0.05, 0.6798),  // P=1.1
                (3600.0, 0.045, 1.2712), // P=1.4
            ],
        ),
        // Bass (NSynth bass_electronic refs): gentle DI tone tilt — low modes
        // reinforce 40–500 Hz ~+6 dB over the dry path, so the sustained
        // 700–1500 Hz sits ~−6 dB relative (mel-error profile 2026-07-11:
        // only mild +2–3 excess patches; a steep cab rolloff over-corrected).
        Instrument::Bass => (
            0.5,
            &[
                (45.0, 0.090, 0.0059),
                (100.0, 0.080, 0.0144),
                (180.0, 0.070, 0.0236),
                (300.0, 0.060, 0.0298),
                (480.0, 0.050, 0.0276),
            ],
        ),
        // piano: soundboard low-mode ladder (broad, subtle — per-voice knock
        // stays). r2: extended to 8 modes — the sustained board coloration
        // reaches the mid band (gentle peaks, P≈0.4–0.6), where the refs keep
        // a near-partial mode cluster singing in the tail.
        Instrument::Piano => (
            0.75,
            &[
                (62.0, 0.30, 0.7),
                (110.0, 0.24, 0.9),
                (175.0, 0.18, 0.8),
                (255.0, 0.13, 0.6),
                (370.0, 0.10, 0.45),
                (520.0, 0.08, 0.3),
                (720.0, 0.07, 0.12),
                (1400.0, 0.05, 0.15),
            ],
        ),
        _ => (1.0, &[]),
    }
}

/// Per-instrument loudness makeup, applied at the track bus so equal velocity lands
/// at roughly equal perceived level across families. Values are MEASURED, not tuned
/// by eye: `scripts/dev/measure-loudness.mjs` renders a reference note per family
/// and derives these from RMS against the marimba reference. Re-run after any
/// preset change and paste the table it prints.
pub fn makeup_gain(inst: Instrument) -> f32 {
    // Measured 2026-07-11 with pyloudnorm (BS.1770 integrated LUFS, K-weighted) via
    // scripts/dev/measure-loudness.{mjs,py} — all families referenced to marimba at
    // vel 0.8 / gain 1.0. Perceptual, not RMS: K-weighting is why glock/music box
    // need far more gain than RMS suggested and why the piano needed −4.4 LU.
    // Re-run both scripts after any preset change and paste the corrected values.
    match inst {
        Instrument::Marimba => 2.1,       // reference
        Instrument::Vibraphone => 4.9,    // was -30.5 LUFS
        Instrument::Glockenspiel => 30.6, // reverb pre-delay re-bake 2026-07-13 (x1.11)
        Instrument::MusicBox => 14.8,     // was -35.6 LUFS
        Instrument::Guitar => 0.184,       // reverb pre-delay re-bake 2026-07-13 (x1.16)
        Instrument::Bass => 1.97,         // reverb pre-delay re-bake 2026-07-13 (x1.09)
        Instrument::EPiano => 1.17,       // EP r2 tine/pickup rebuild, reverb-flat re-bake 2026-07-13 (×1.05)
        Instrument::Drums => 0.58,        // drums r4 0.61 x room 0.95 (verify by sweep)
        Instrument::SynthPad => 0.51,     // reverb pre-delay re-bake 2026-07-13 (x1.08)
        Instrument::Piano => 0.066, // P1 per-key calibration re-bake (per-key LUFS trims raised the mid; was -14.9 LUFS at 0.130, x0.51 per measure-loudness)
        Instrument::GuitarSteel => 0.364,   // body-round 0.387 x room 0.94 (verify by sweep)
        Instrument::GuitarElectric => 0.416, // reverb pre-delay re-bake 2026-07-13 (x1.08)
        Instrument::GuitarDistorted => 0.135, // reverb pre-delay re-bake 2026-07-13 (x1.06)
        Instrument::DrumsRock => 0.38,      // drums r4 0.41 x room 0.93 (verify by sweep)
        Instrument::DrumsJazz => 0.56,      // reverb pre-delay re-bake 2026-07-13 (x1.17)
    }
}

/// Default room send per family (engine's shared early-reflection/room stage,
/// audit 2026-07-12 "amplifier + resonance"). Subtle by design — the room glues,
/// it must never read as "reverb on". Drums highest (a kit IS its room),
/// electrics lowest (the cab/amp chain already carries their space).
pub fn room_send(inst: Instrument) -> f32 {
    match inst {
        Instrument::Drums | Instrument::DrumsRock => 0.16,
        Instrument::DrumsJazz => 0.18, // brushes/jazz kits are recorded roomier
        Instrument::Piano => 0.09,
        Instrument::Guitar | Instrument::GuitarSteel => 0.11,
        Instrument::GuitarElectric => 0.05,
        Instrument::GuitarDistorted => 0.04,
        Instrument::Bass => 0.035,
        Instrument::EPiano => 0.06,
        Instrument::Marimba | Instrument::Vibraphone => 0.10,
        Instrument::Glockenspiel => 0.09,
        Instrument::MusicBox => 0.08,
        Instrument::SynthPad => 0.05,
    }
}

#[inline(always)]
pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * ((midi - 69.0) / 12.0).exp2()
}

/// Deterministic per-voice noise (no host RNG in the core — reproducible renders).
#[derive(Clone, Copy)]
pub struct Lcg(pub u32);
impl Lcg {
    #[inline(always)]
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        // top 23 bits → [-1, 1)
        (self.0 >> 9) as f32 * (2.0 / 8388608.0) - 1.0
    }
}

/// t60 seconds → per-sample amplitude ratio.
#[inline(always)]
fn t60_gain(t60: f32, sr: f32) -> f32 {
    if t60 <= 0.0 {
        0.0
    } else {
        (-6.907755 / (t60 * sr)).exp() // ln(10^-3)
    }
}

// ---------------------------------------------------------------------------
// Modal bank (mallets, bells, e-piano partials, drum modes)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct ModalVoice {
    n_modes: usize,
    a1: [f32; MAX_MODES],
    r2: [f32; MAX_MODES],
    y1: [f32; MAX_MODES],
    y2: [f32; MAX_MODES],
    g: [f32; MAX_MODES],
    /// raised-cosine strike pulse
    pulse_len: u32,
    pulse_pos: u32,
    pulse_amp: f32,
    click_amp: f32,
    click_decay: f32,
    click_env: f32,
    /// pickup nonlinearity drive (0 = linear) — e-piano
    drive: f32,
    /// rotary tremolo (vibraphone motor); rate in radians/sample, depth 0..1
    trem_rate: f32,
    trem_depth: f32,
    trem_phase: f32,
    // --- EP (tine/reed + magnetic pickup) extensions. ep_amp = 0.0 ⇒ every
    // field below is inert and the render path is the legacy one, bit-identical
    // for all non-EP families (marimba/vibes/glock/musicbox/drum shells). ---
    /// displacement drive into the pickup flux curve (key-tracked down the top
    /// octaves, same law as the old tanh drive — anti-aliasing guard)
    ep_amp: f32,
    /// tine rest offset from the pickup axis in flux-width units (the Rhodes
    /// "timbre screw"): asymmetry ⇒ even harmonics that grow with amplitude
    ep_off: f32,
    /// output normalization: small-signal gain comp × register level law
    ep_gain: f32,
    /// previous flux sample — the pickup output IS the flux first difference
    /// (dΦ/dt), which is first-order-ADAA band-limited by construction
    ep_flux1: f32,
    /// key-action thunk (note-on) / damper-felt bump (note-off): decaying
    /// noise through a one-pole lowpass, added post-pickup (frame vibration)
    th_env: f32,
    th_dec: f32,
    th_lc: f32,
    th_y1: f32,
    /// hammer flight: samples between key action and string contact — the
    /// refs' tone peaks 2–17 ms AFTER the action knock starts (slower at pp)
    ep_wait: u32,
    /// key-action knock (refs: coherent LF onset swell, centroid 90–300 Hz,
    /// near-zero zero-crossings pre-peak, peaking 10–17 ms in). Two paths:
    /// act_amp = raised-cosine displacement bump into the flux curve (gap
    /// modulation → the ff "spit"), act_gain = direct mechanical path to the
    /// output (case/frame), a squared-raised-cosine LF hump whose level is
    /// independent of pickup geometry (it9/10: gap-path level shifts with
    /// ep_off and kept flipping the crest gate)
    act_pos: u32,
    act_len: u32,
    act_amp: f32,
    act_gain: f32,
    rng: Lcg,
    life: u64,
    age: u64,
}

pub struct ModeDef {
    pub ratio: f32,
    pub amp: f32,
    pub t60: f32,
}

impl ModalVoice {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        f0: f32,
        vel: f32,
        sr: f32,
        modes: &[ModeDef],
        strike_ms: f32,
        click: f32,
        drive: f32,
        seed: u32,
    ) -> Self {
        let mut v = Self {
            n_modes: 0,
            a1: [0.0; MAX_MODES],
            r2: [0.0; MAX_MODES],
            y1: [0.0; MAX_MODES],
            y2: [0.0; MAX_MODES],
            g: [0.0; MAX_MODES],
            pulse_len: 1,
            pulse_pos: 0,
            pulse_amp: 0.0,
            click_amp: click * vel,
            click_decay: t60_gain(0.006, sr),
            click_env: 1.0,
            drive,
            trem_rate: 0.0,
            trem_depth: 0.0,
            trem_phase: 0.0,
            ep_amp: 0.0,
            ep_off: 0.0,
            ep_gain: 0.0,
            ep_flux1: 0.0,
            th_env: 0.0,
            th_dec: 0.0,
            th_lc: 0.0,
            th_y1: 0.0,
            ep_wait: 0,
            act_pos: 0,
            act_len: 0,
            act_amp: 0.0,
            act_gain: 0.0,
            rng: Lcg(seed | 1),
            life: 0,
            age: 0,
        };
        // Harder hits → shorter contact → brighter. Soft hits roll the top modes off.
        let contact_ms = strike_ms * (1.6 - vel).max(0.35);
        v.pulse_len = ((contact_ms * 1e-3 * sr) as u32).max(2);
        v.pulse_amp = vel;
        let nyq = 0.45 * sr;
        let mut max_t60 = 0.0f32;
        for m in modes.iter() {
            if v.n_modes == MAX_MODES {
                break;
            }
            let f = f0 * m.ratio;
            if f >= nyq {
                continue; // never synthesize above Nyquist — aliasing guard
            }
            // Higher partials decay faster on real bars; also fade them for soft hits.
            let t60 = m.t60 / (1.0 + 0.7 * (m.ratio - 1.0) * 0.3);
            let r = t60_gain(t60, sr);
            let w = core::f32::consts::TAU * f / sr;
            let i = v.n_modes;
            v.a1[i] = 2.0 * r * w.cos();
            v.r2[i] = r * r;
            // (1−r) normalizes resonator gain across decay times; vel^brightness on uppers.
            let bright = vel.powf(0.7 + 0.8 * (m.ratio - 1.0).min(4.0) * 0.25);
            v.g[i] = m.amp * (1.0 - r) * 2.5 * bright;
            v.n_modes += 1;
            max_t60 = max_t60.max(t60);
        }
        v.life = ((max_t60 * 1.2 + 0.05) * sr) as u64;
        v
    }

    /// EP round 2026-07-12 (owner verdict: "electric piano — too much like
    /// marimba"). A Rhodes/Wurli is not a struck bar; it is a tine/reed +
    /// tonebar pair read by an ASYMMETRIC magnetic pickup. This constructor
    /// kills the marimba tells (measured on the old preset: clean 1:3.97 bar
    /// ladder, odd-only tanh harmonics h2 ≈ −73…−140 dB, zero beating,
    /// single-exponential decay, no key noise):
    ///   - partials = tine f0 (prompt) + tonebar at f0+~0.9 Hz (aftersound:
    ///     slow beat + two-stage decay, Weinreich 1977 lineage) + one
    ///     inharmonic tine-mode-2 "bell" near 6.3·f0 (cantilever mode ratios
    ///     1:6.27, Fletcher & Rossing ch.4/§13 — NOT the 1:4 tuned-bar mark).
    ///   - velocity→timbre lives in the PICKUP, not mode brightness: the
    ///     Wurlitzer EP200 refs measure h2/h1 ≈ −20 dB at pp rising to
    ///     −3…+16 dB at ff (references/epiano-wurli, manifest caveats there).
    ///   - t60 law fits the refs' pp (linear-regime) ladder: ≈25 s at A1,
    ///     7.7 s at C#4, 3.7 s at C#5.
    ///   - key-action thunk at note-on; damper felt bump in damp().
    /// Mode gains are DISPLACEMENT-normalized (g = amp·sin ω / Σpulse) so the
    /// flux-curve drive u is calibrated in velocity units; the shared start()
    /// normalization targets summed OUTPUT instead and is left untouched.
    pub fn start_epiano(midi: u32, f0: f32, vel: f32, sr: f32, seed: u32) -> Self {
        let key = (((midi as f32) - 28.0) / 60.0).clamp(0.0, 1.0);
        let t60_tine = 28.0 * (1.0 - key) * (1.0 - key) + 2.0;
        // hammer contact ∝ the struck element's period (compliant striker:
        // contact ≈ 0.5–1 period, Fletcher & Rossing ch.19 hammer–string /
        // ch.4 struck-bar analysis), shorter when harder. Fits the refs'
        // measured onset blooms: a1ff peaks ~17 ms, ab3 ~10–12 ms, db5ff
        // ~2.3 ms after onset — a fixed-ms contact can't do all three.
        let contact_ms = (900.0 / f0) * (1.6 - 0.9 * vel).max(0.45);
        // it8: the returning element throws the hammer off — contact cannot
        // exceed ~a period (beyond that the drive decoheres and pp notes
        // lost both level and bark: measured h2 −31 dB vs the refs' −21)
        let contact_ms = contact_ms.min(1150.0 / f0).clamp(0.45, 25.0);
        let mut v = Self::start(f0, vel, sr, &[], 1.0, 0.0, 0.0, seed);
        v.pulse_len = ((contact_ms * 1e-3 * sr) as u32).max(2);
        v.pulse_amp = vel;
        v.click_amp = 0.0;
        let defs = [
            (f0, 1.0, t60_tine),
            // tonebar aftersound: sings past the tine, slightly mistuned →
            // the slow beat the refs keep at ~1–3 dB depth, 0.5–1 Hz.
            // it7: the coupling weakens up the map (short tonebars, heavier
            // relative damping) — a key-flat 0.22/2.0× read 20 dB beat depth
            // at C#5 where the refs read ~1.5 dB
            (
                f0 + 0.9,
                0.22 * (1.1 - 0.85 * key).max(0.2),
                t60_tine * (2.0 - 0.8 * key),
            ),
            // tine mode 2: fast bell ping on the attack
            (f0 * 6.3, 0.10 + 0.10 * vel, (0.9 - 0.6 * key).max(0.15)),
        ];
        let nyq = 0.45 * sr;
        let mut max_t60 = 0.0f32;
        let pulse_sum = 0.5 * v.pulse_len as f32; // Σ raised-cosine
        for &(f, amp, t60) in defs.iter() {
            if f >= nyq {
                continue; // aliasing guard, same as start()
            }
            let r = t60_gain(t60, sr);
            let w = core::f32::consts::TAU * f / sr;
            let i = v.n_modes;
            v.a1[i] = 2.0 * r * w.cos();
            v.r2[i] = r * r;
            v.g[i] = amp * w.sin() / pulse_sum;
            v.n_modes += 1;
            max_t60 = max_t60.max(t60);
        }
        v.life = ((max_t60 * 1.2 + 0.05) * sr) as u64;
        // pickup drive: THE velocity→timbre axis. Key-tracked down the top
        // octaves — same guard idea as the old tanh drive (tine harmonics of
        // a high f0 would fold past Nyquist; the refs' top octaves measure
        // near-sinusoidal anyway: g5 h2 ≈ −21 dB even at ff). it6: linear key
        // law refit to the refs' DEEP-FOLD ff spectra (ab3ff keeps h5 within
        // −5 dB of h1; a1ff puts the whole comb ABOVE h1) — the old clamp
        // capped the low keys at u≈1.7, which can't fold
        v.ep_amp = (3.6 - 2.8 * key).clamp(0.8, 3.6);
        // rest offset in flux-width units (the Rhodes "timbre screw"):
        // asymmetry ⇒ even harmonics grow with amplitude. it9: small-signal
        // h2/h1 ∝ |2a²−1|/2a — 0.55 sat near the Gaussian inflection (a=1/√2)
        // where the curvature term cancels and pp read 7 dB too clean; 0.42
        // restores the refs' pp bark, ff (deep fold) is insensitive to a
        v.ep_off = 0.42;
        let slope = 2.0 * v.ep_off * (-(v.ep_off * v.ep_off)).exp();
        let level = 0.22 * (0.3 + 0.7 * vel);
        v.ep_gain = level / (slope * v.ep_amp * (core::f32::consts::TAU * f0 / sr));
        v.ep_flux1 = (-(v.ep_off * v.ep_off)).exp(); // rest flux: no onset step
        // attack mechanics (it3): the refs' onsets are action-knock-first —
        // a coherent LF swell 6–10 ms BEFORE the tone peaks (crest gate:
        // render first-3ms must sit ≈20 dB under body like the refs').
        // Hammer flight delays the strike; the key-bottoming knock shifts
        // the frame (a displacement bump through the pickup). Noise thunk
        // at note-on removed — the refs' pre-peak has ~zero zero-crossings.
        let wait_ms = (95.0 / f0.sqrt()) * (1.35 - 0.5 * vel);
        v.ep_wait = (wait_ms.clamp(0.0, 25.0) * 1e-3 * sr) as u32;
        // heavier low-key action rises slower (a1ff ref knock peaks ~17 ms)
        v.act_len = (((wait_ms + 13.0 * (2.0 - key)) * 1e-3 * sr) as u32).max(8);
        // knock/tone ratio is ~velocity-flat in the refs (ab3 thunk −14.9 dB
        // at pp vs −12.6 at ff) → the knock scales LINEARLY with velocity.
        // Gap-mod bump sized to stay on the slope side of the flux peak
        // (past it the knock folds into a sharp double-humped onset, it9);
        // the audible knock level rides the direct path (act_gain) instead.
        v.act_amp = 0.16 * vel * (1.35 - 0.8 * key);
        v.act_gain = 0.05 * vel * (1.35 - 0.8 * key);
        v
    }

    /// EP render path (ep_amp > 0 only): modal displacement → flux pickup.
    /// The pickup output is the flux FIRST DIFFERENCE — physically dΦ/dt (a
    /// magnetic pickup senses velocity, Zollner Physics of the Electric
    /// Guitar ch.4), numerically the first-order-ADAA form of Φ′(u)·u̇
    /// (Bilbao/Esqueda/Parker/Välimäki, IEEE SPL 2017), so the asymmetric
    /// distortion stays band-limited without oversampling — this EXTENDS the
    /// pre-round key-tracked-drive guard instead of replacing it.
    fn render_ep(&mut self, out: &mut [f32]) -> bool {
        let inv_len = 1.0 / self.pulse_len as f32;
        let act_inv = 1.0 / self.act_len.max(1) as f32;
        for o in out.iter_mut() {
            let mut x = 0.0;
            if self.ep_wait > 0 {
                self.ep_wait -= 1;
            } else if self.pulse_pos < self.pulse_len {
                let ph = self.pulse_pos as f32 * inv_len;
                x = self.pulse_amp * 0.5 * (1.0 - (core::f32::consts::TAU * ph).cos());
                self.pulse_pos += 1;
            }
            let mut s = 0.0;
            for m in 0..self.n_modes {
                let y = self.a1[m] * self.y1[m] - self.r2[m] * self.y2[m] + self.g[m] * x;
                self.y2[m] = self.y1[m];
                self.y1[m] = y;
                s += y;
            }
            // key-bottoming knock: frame displacement bump under the tine —
            // shifts the flux operating point (the ff "spit") — plus the
            // direct mechanical path (LF hump, see act_gain)
            let mut knock = 0.0;
            if self.act_pos < self.act_len {
                let ph = self.act_pos as f32 * act_inv;
                let bump = 0.5 * (1.0 - (core::f32::consts::TAU * ph).cos());
                s += self.act_amp * bump;
                knock = self.act_gain * bump;
                self.act_pos += 1;
            }
            let d = s * self.ep_amp - self.ep_off;
            let flux = (-(d * d).min(25.0)).exp();
            let mut y = (flux - self.ep_flux1) * self.ep_gain + knock;
            self.ep_flux1 = flux;
            if self.th_env > 1e-6 {
                // thunk/felt: LP noise burst added post-pickup (frame → coil)
                self.th_y1 += self.th_lc * (self.rng.next() * self.th_env - self.th_y1);
                self.th_env *= self.th_dec;
                y += self.th_y1;
            }
            *o += y;
        }
        for m in 0..self.n_modes {
            self.y1[m] = flush_denormal(self.y1[m]);
            self.y2[m] = flush_denormal(self.y2[m]);
        }
        self.th_y1 = flush_denormal(self.th_y1);
        self.age += out.len() as u64;
        self.age < self.life
    }

    /// Render one block, ADD into `out`. Returns false when the voice is spent.
    pub fn render(&mut self, out: &mut [f32]) -> bool {
        if self.ep_amp > 0.0 {
            return self.render_ep(out);
        }
        let inv_len = 1.0 / self.pulse_len as f32;
        for o in out.iter_mut() {
            // excitation: raised-cosine contact pulse + mallet click noise
            let mut x = 0.0;
            if self.pulse_pos < self.pulse_len {
                let ph = self.pulse_pos as f32 * inv_len;
                x = self.pulse_amp * 0.5 * (1.0 - (core::f32::consts::TAU * ph).cos());
                self.pulse_pos += 1;
            }
            if self.click_amp > 1e-5 {
                x += self.click_amp * self.click_env * self.rng.next();
                self.click_env *= self.click_decay;
            }
            let mut s = 0.0;
            for m in 0..self.n_modes {
                let y = self.a1[m] * self.y1[m] - self.r2[m] * self.y2[m] + self.g[m] * x;
                self.y2[m] = self.y1[m];
                self.y1[m] = y;
                s += y;
            }
            if self.drive > 0.0 {
                let d = 1.0 + self.drive;
                s = (d * s).tanh() / d.tanh().max(1e-6) * 0.8;
            }
            if self.trem_depth > 0.0 {
                self.trem_phase += self.trem_rate;
                s *= 1.0 - self.trem_depth * (0.5 + 0.5 * self.trem_phase.sin());
            }
            *o += s;
        }
        for m in 0..self.n_modes {
            self.y1[m] = flush_denormal(self.y1[m]);
            self.y2[m] = flush_denormal(self.y2[m]);
        }
        self.age += out.len() as u64;
        self.age < self.life
    }

    pub fn damp(&mut self, sr: f32) {
        // pull every mode's decay down to ~90 ms (mallet grab / pedal up)
        for m in 0..self.n_modes {
            let r_new = t60_gain(0.09, sr);
            let r_old2 = self.r2[m].max(1e-12);
            let scale = (r_new * r_new) / r_old2;
            self.r2[m] *= scale;
            self.a1[m] *= scale.sqrt();
        }
        if self.ep_amp > 0.0 {
            // damper felt lands on the ringing tine (Askenfelt & Jansson
            // release-transient lineage, EP scale): soft LP noise breath
            self.th_env = 0.05;
            self.th_dec = t60_gain(0.045, sr);
            self.th_lc = 1.0 - (-core::f32::consts::TAU * 420.0 / sr).exp();
            self.life = self.age + (0.30 * sr) as u64;
        } else {
            self.life = self.age + (0.12 * sr) as u64;
        }
    }
}

// ---------------------------------------------------------------------------
// Extended Karplus-Strong plucked string
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct PluckVoice {
    buf: [f32; PLUCK_BUF],
    len: usize,
    pos: usize,
    /// loop one-pole lowpass state + coefficient (brightness)
    lp: f32,
    lp_c: f32,
    /// HF bypass fraction of the blend loss filter (0 = pure one-pole)
    lp_mix: f32,
    /// per-sample loop loss
    loss: f32,
    /// fractional-delay allpass
    ap_c: f32,
    ap_x1: f32,
    ap_y1: f32,
    level: f32,
    life: u64,
    age: u64,
    sr: f32,
    // --- acoustic-guitar extensions (inert for the legacy constructor) ---
    /// second polarization (vertical): its own loop, slightly detuned, faster
    /// decay — two-stage decay + natural unison chorus (Weinreich 1977).
    buf2: [f32; PLUCK_BUF],
    len2: usize,
    pos2: usize,
    lp2: f32,
    loss2: f32,
    ap2_c: f32,
    ap2_x1: f32,
    ap2_y1: f32,
    pol_mix: f32,
    /// stiffness dispersion: M cascaded first-order allpasses, all with
    /// coefficient `disp_a` = −p (pole at +p ⇒ phase delay falls with
    /// frequency ⇒ upper partials arrive early ⇒ stretched, f_n ≈ n·f0·√(1+Bn²)).
    /// (M, p) are solved per note in `design_dispersion`. Both polarizations
    /// disperse (same physical string), with separate filter states.
    disp_a: f32,
    disp_n: u8,
    dsx: [f32; MAX_DISP],
    dsy: [f32; MAX_DISP],
    ds2x: [f32; MAX_DISP],
    ds2y: [f32; MAX_DISP],
    /// bridge-force output tap: leaky first difference ≈ string slope at the
    /// bridge (force drives the body, displacement does not radiate). 0 = off.
    br_rho: f32,
    br_x1: f32,
    /// coupling shelf (see shelf_h): per-period LF loss, shared coefficients,
    /// per-polarization states
    sh_d: f32,
    sh_c: f32,
    sh_y: f32,
    sh2_y: f32,
    /// radiation high-pass (breathing-sphere monopole, Woodhouse 2012 §2:
    /// R(ω) = (iω/ω_c)/(1 + iω/ω_c), f_c ≈ 250 Hz): a guitar radiates volume
    /// acceleration — the body is a poor radiator below its lowest air/plate
    /// modes, which is why real low-E fundamentals sit ~12 dB under h3 while
    /// a raw string tap keeps them dominant. rad_k = 0 disables.
    rad_k: f32,
    rad_p: f32,
    rad_x1: f32,
    rad_y1: f32,
    /// acceleration differencer (see AcPluck::acc_rho)
    acc_rho: f32,
    acc_x1: f32,
    /// tension modulation: hard plucks start sharp and settle (band-limited —
    /// the fractional-delay allpass coefficient follows a smoothed env²).
    tm_dev: f32,
    tm_env: f32,
    tm_c: f32,
    tm_norm: f32,
    frac1: f32,
    frac2: f32,
    /// direct contact-click transient: decaying band-passed noise added to the
    /// output (attack splash / release pluck-off), not stored in the string.
    /// Differenced noise alone tilts +6 dB/oct to Nyquist — a hi-hat-like tick
    /// the 16 kHz refs can't penalize (real pick scrape lives ~1-6 kHz), so the
    /// difference is followed by a one-pole LP (tr_lc) at ~4.2 kHz.
    /// Pick round 2026-07-12: rebuilt as a 2-pole contact resonator (center
    /// click_hz, Q≈1.5 — smooth sample-to-sample, no Nyquist alternation; the
    /// one-pole chain still jumped ±0.8× peak between adjacent samples) with
    /// TWO envelopes: tr_env = snap (release corner, ~8 ms, ∝ vel²) and
    /// tr2_env = scrape (pick sliding, 55→25 ms as velocity rises, ~∝ vel —
    /// refs keep the scrape/body energy ratio flat across velocity while the
    /// CREST collapses at pp: soft strokes slide, hard strokes snap through).
    /// ri_env: ~1.5 ms contact ramp-in kills the t=0 cliff.
    tr_env: f32,
    tr_dec: f32,
    tr2_env: f32,
    tr2_dec: f32,
    bp_y1: f32,
    bp_y2: f32,
    bp_a1: f32,
    bp_r2: f32,
    bp_g: f32,
    ri_env: f32,
    ri_c: f32,
    tr_rng: Lcg,
    /// body-pump transient (see AcPluck::thump): one windowed bipolar cycle,
    /// active while th_ph < 1
    th_amp: f32,
    th_dph: f32,
    th_ph: f32,
    /// release voicing: post-note-off t60 and pluck-off click level
    rel_t60: f32,
    rel_click: f32,
    /// release loss-glide (FLAGGED ADDITION, bass agent 2026-07-12): damp()
    /// historically STEPPED `loss` to the release gain; the samples written
    /// after the step sit g_rel/g_sus below their neighbors, and one period
    /// later the read head crosses that seam — a step the output differencer
    /// renorms (~×3900 at E1 for the bass's double tap) blow up into a
    /// ±full-scale clipped doublet at every note-off (measured). rel_ramp
    /// > 0 glides `loss`/`loss2` to a stored target through a one-pole of
    /// time-constant rel_ramp seconds instead. 0 = legacy instant switch;
    /// the render-loop madd is exact-identity at loss_c = 0 (x + 0.0·y ≡ x),
    /// so nylon/steel output stays bit-identical.
    rel_ramp: f32,
    loss_tgt: f32,
    loss_c: f32,
    /// click/transient level scale (pre-acceleration-renorm; the transients
    /// bypass the output differencers — see level_tr in start_acoustic)
    tr_lvl: f32,
    /// f0 > 0 marks the acoustic path (per-period loss calibration, new damp)
    f0: f32,
}

/// Acoustic-guitar pluck parameters (nylon/steel diverge through these).
pub struct AcPluck {
    pub f0: f32,
    pub vel: f32,
    /// fundamental decay target (s) — per-period loss is calibrated to hit it
    pub t60_f0: f32,
    /// loop lowpass coefficient: sets HF decay relative to the fundamental
    pub lp_c: f32,
    /// HF loss floor: target t60 (s) of the highest partials at this note
    /// (0 = pure one-pole). Real strings' HF loss saturates on air drag.
    pub hf_floor_t60: f32,
    /// one-pole knee (Hz) when the floor is active (ladder→floor transition)
    pub hf_knee_hz: f32,
    /// pluck point as a fraction of string length (comb is inherent in the shape)
    pub pick_pos: f32,
    /// contact-patch width as a fraction of string length (finger flesh ≫ pick
    /// tip); band-limits the WHOLE initial condition to ~len/width harmonics
    pub contact: f32,
    /// localized release-snap bump (velocity component of the initial condition)
    pub snap: f32,
    /// pick/finger contact noise mixed into the initial shape
    pub scrape: f32,
    /// direct (non-looped) contact-click transient level (0 = none): most pick
    /// noise radiates immediately instead of persisting as string modes — with
    /// the HF loss floor, keeping it all in-loop ran +30…+47 dB hot mid-note.
    /// Pick round: `click` is the SNAP component (release corner, ∝ vel²);
    /// `click_slow` the SCRAPE component (slide friction, ~∝ vel, longer and
    /// softer at low velocity); `click_hz` centers the shared 2-pole contact
    /// resonator (pick on wound steel ~2.8 kHz; fingertip/nail lower).
    pub click: f32,
    pub click_slow: f32,
    pub click_hz: f32,
    /// contact ramp-in (s): a pick releases in ~1.5 ms, fingertip flesh takes
    /// ~8-15 ms (Penttinen & Välimäki 2001 pluck contact) — sets how fast the
    /// snap/scrape transient reaches full level (nylon ff refs measure onset
    /// crest −24 dB; a 1.5 ms ramp read as a click)
    pub click_ramp: f32,
    /// second-polarization output level (0 = single string)
    pub pol_mix: f32,
    pub pol_detune_cents: f32,
    /// polarization-2 fundamental t60 = t60_f0 × this ratio
    pub pol_t60_ratio: f32,
    /// stiff-string inharmonicity coefficient B (f_n = n·f0·√(1+Bn²), Fletcher
    /// 1964). 0 = perfectly flexible. Measured from the reference corpus with a
    /// weighted partial-frequency fit (steel source 015 ≈ 3e-4, nylon 014 ≈ 5e-5).
    pub stiff_b: f32,
    /// initial tension-mod sharpening at vel = 1 (cents; small — refs show ≤3c)
    pub tm_cents: f32,
    /// post-note-off decay t60 (s): finger/palm damping is not instantaneous —
    /// steel refs ring ~0.5 s after release, nylon chokes fast
    pub rel_t60: f32,
    /// fret/finger release-click level ("pluck-off"), scaled by the string's
    /// remaining energy at note-off
    pub rel_click: f32,
    /// note-off loss transition time (s): 0 = legacy instant loss step
    /// (see PluckVoice::rel_ramp for why differencer-tapped voices need > 0)
    pub rel_ramp: f32,
    /// bridge differencer leak (0 = raw displacement out)
    pub br_rho: f32,
    /// radiation monopole high-pass corner (Hz; 0 = off)
    pub rad_hz: f32,
    /// second (acceleration) differencer leak (0 = off): radiated pressure
    /// follows the BODY's volume acceleration = d/dt(Y·F) ≈ dF/dt for the
    /// smooth part of the admittance (Woodhouse 2012 radiates bridge
    /// ACCELERATION through the monopole; force alone is 6 dB/oct short —
    /// the r2 renders' attack centroid sat at 274 Hz vs the refs' 1102)
    pub acc_rho: f32,
    /// Valette/Woodhouse string damping constants (2004 II Table 1 + eq. 8):
    /// eta_f = frequency-independent internal friction, eta_b = bending loss
    /// factor (dominates HF), eta_a = air-drag coefficient (dominates LF,
    /// enters as eta_a/omega). eta_f > 0 switches the loop-loss design to the
    /// physical law (t60(f) = 6.91/(pi f eta(f))), superseding t60_f0 /
    /// hf_floor_t60; 0 keeps the legacy hand-fit path (bass).
    pub eta_f: f32,
    pub eta_b: f32,
    pub eta_a: f32,
    /// body-coupling loss scale: peak extra eta near the A0/top-plate modes
    /// (string-mode damping tracks Re{Y_bridge}, Woodhouse 2004 I Fig. 7)
    pub couple: f32,
    /// body-pump transient ("woof"): the pluck release is a STEP of bridge
    /// force whose sub-f0 content the radiation differencers remove by
    /// design — but on a real guitar that step pumps the Helmholtz/top pair
    /// and radiates a low thump (commuted body response to the force step;
    /// Christensen & Vistisen 1980 two-oscillator model). Injected as one
    /// windowed bipolar cycle at `thump_hz` AFTER the radiation chain so the
    /// track body bank rings from it. Level ∝ vel² (refs' attack A0 band
    /// collapses from −10 dB deficit at ff to +7 excess at pp, 2026-07-12).
    pub thump: f32,
    pub thump_hz: f32,
    pub level: f32,
}

impl Default for AcPluck {
    /// Neutral parameter set: every feature off, unity level. Lets preset
    /// arms opt into new fields with `..Default::default()` instead of
    /// enumerating them (keeps parallel-agent merges small).
    fn default() -> Self {
        AcPluck {
            f0: 110.0,
            vel: 0.7,
            t60_f0: 4.0,
            lp_c: 0.7,
            hf_floor_t60: 0.0,
            hf_knee_hz: 0.0,
            pick_pos: 0.2,
            contact: 0.02,
            snap: 0.3,
            scrape: 0.0,
            click: 0.0,
            click_slow: 0.0,
            click_hz: 2800.0,
            click_ramp: 0.0015,
            rel_ramp: 0.0,
            pol_mix: 0.0,
            pol_detune_cents: 0.0,
            pol_t60_ratio: 1.0,
            stiff_b: 0.0,
            tm_cents: 0.0,
            rel_t60: 0.1,
            rel_click: 0.0,
            br_rho: 0.0,
            rad_hz: 0.0,
            acc_rho: 0.0,
            eta_f: 0.0,
            eta_b: 0.0,
            eta_a: 0.0,
            couple: 0.0,
            thump: 0.0,
            thump_hz: 110.0,
            level: 0.5,
        }
    }
}

/// Normalized Re{Y_bridge} shape for the coupling loss: A0 Helmholtz hump
/// (~100 Hz), top-plate cluster (~210 Hz, broadened — it stands in for the
/// 150–250 Hz group), and a small mid plateau that dies above ~2.6 kHz.
/// Peak of the A0 term = 1; scaled by AcPluck::couple.
fn body_coupling_shape(f: f32) -> f32 {
    let d0 = f / 100.0 - 100.0 / f;
    let d1 = f / 210.0 - 210.0 / f;
    // near-physical widths (measured A0 Q≈25, plate cluster Q≈47-59, slightly
    // broadened for the fret lottery); a first, too-broad cut (Q≈6-8, plateau
    // 0.22) quadrupled MID-band loss and collapsed the render (it2 regression
    // 2026-07-12) — guitars conserve mid/HF string energy (heavy bridge, no
    // violin bridge hill; Woodhouse 2004 II §5), so the inter-peak floor must
    // stay far below the A0/T1 peaks.
    let a0 = 1.0 / (1.0 + 200.0 * d0 * d0);
    let t1 = 0.9 / (1.0 + 300.0 * d1 * d1);
    let x = f / 350.0;
    let x4 = x * x * x * x;
    let hi = f / 2600.0;
    let hi6 = hi * hi * hi * hi * hi * hi;
    let plateau = 0.03 * (x4 / (1.0 + x4)) / (1.0 + hi6);
    a0 + t1 + plateau
}

/// One-pole loop lowpass y += c(x−y): magnitude at ω (for loss calibration).
#[inline]
fn onepole_mag(c: f32, w: f32) -> f32 {
    onepole_mag_cw(c, w.cos())
}

/// cos(ω)-hoisted variant for bisection loops (ω fixed, wasm transcendentals
/// are software floats — the r3 law fit's nested bisections cost ~ms/note-on
/// before hoisting).
#[inline]
fn onepole_mag_cw(c: f32, cw: f32) -> f32 {
    let b = 1.0 - c;
    c / (1.0 - 2.0 * b * cw + b * b).sqrt()
}

/// One-pole loop lowpass phase delay in samples at ω (exact, not the DC approx).
#[inline]
fn onepole_delay(c: f32, w: f32) -> f32 {
    let b = 1.0 - c;
    let ph = (b * w.sin()).atan2(1.0 - b * w.cos());
    ph / w
}

/// Blend loss filter H = m + (1−m)·H_lp — a one-pole ladder over a flat HF
/// bypass floor. The bypass `m` sets per-period HF survival, the one-pole knee
/// shapes the low-harmonic decay ladder: real strings' HF loss SATURATES (air
/// drag, not the one-pole's collapse — the SDL loss-filter role, Välimäki et
/// al. 1996; steel ref 015 keeps 2–6 kHz partials ringing t60 ≈ 2 s while a
/// pure one-pole killed them in tens of ms). m = 0 reduces to the one-pole.
fn blend_h(c: f32, m: f32, w: f32) -> (f32, f32) {
    let (sw, cw) = w.sin_cos();
    blend_h_sc(c, m, sw, cw)
}

#[inline]
fn blend_h_sc(c: f32, m: f32, sw: f32, cw: f32) -> (f32, f32) {
    let b = 1.0 - c;
    let d = 1.0 + b * b - 2.0 * b * cw;
    (
        m + (1.0 - m) * c * (1.0 - b * cw) / d,
        -(1.0 - m) * c * b * sw / d,
    )
}

#[inline]
fn blend_mag(c: f32, m: f32, w: f32) -> f32 {
    let (re, im) = blend_h(c, m, w);
    (re * re + im * im).sqrt()
}

#[inline]
fn blend_mag_sc(c: f32, m: f32, sw: f32, cw: f32) -> f32 {
    let (re, im) = blend_h_sc(c, m, sw, cw);
    (re * re + im * im).sqrt()
}

/// Blend filter phase delay in samples at ω.
#[inline]
fn blend_delay(c: f32, m: f32, w: f32) -> f32 {
    let (re, im) = blend_h(c, m, w);
    -im.atan2(re) / w
}

/// First-order allpass (a + z⁻¹)/(1 + a z⁻¹) phase delay in samples at ω.
#[inline]
fn allpass_delay(a: f32, w: f32) -> f32 {
    let (sw, cw) = w.sin_cos();
    let th_n = (-sw).atan2(a + cw);
    let th_d = (-a * sw).atan2(1.0 + a * cw);
    -(th_n - th_d) / w
}

/// Coupling shelf H = 1 − d·L(z), L = c/(1 − (1−c)z⁻¹) (unity-DC one-pole):
/// per-period LOW-frequency loop loss. Real strings' lowest partials decay
/// FASTER than the mid-band — air drag η_A/ω (Valette form; Woodhouse 2004
/// "Plucked guitar transients" eq. 8) plus energy pumped into the body where
/// Re{Y_bridge} is large (A0 ≈ 100 Hz, top-plate cluster 150–250 Hz —
/// Woodhouse 2004 "On the synthesis of guitar plucks" Fig. 7: string-mode
/// damping tracks Re{Y} below ~1 kHz). A monotone-dark loop filter cannot
/// express this U-shape; the r2 renders rang +12…+24 dB hot below 300 Hz at
/// mid-note vs the NSynth refs (pooled envdelta, guitar r3 2026-07-12).
/// Stability: |H_sh|² = 1 − 2d·Re{L} + d²|L|² ≤ 1 ⇔ d·c ≤ 2(1 − b·cos ω),
/// which holds for d ≤ 2 since Re{L} > 0 — the shelf never amplifies.
fn shelf_h(d: f32, c: f32, w: f32) -> (f32, f32) {
    if d <= 0.0 {
        return (1.0, 0.0);
    }
    let (sw, cw) = w.sin_cos();
    shelf_h_sc(d, c, sw, cw)
}

#[inline]
fn shelf_h_sc(d: f32, c: f32, sw: f32, cw: f32) -> (f32, f32) {
    let b = 1.0 - c;
    let den = 1.0 + b * b - 2.0 * b * cw;
    let lre = c * (1.0 - b * cw) / den;
    let lim = -c * b * sw / den;
    (1.0 - d * lre, -d * lim)
}

#[inline]
fn shelf_mag_sc(d: f32, c: f32, sw: f32, cw: f32) -> f32 {
    if d <= 0.0 {
        return 1.0;
    }
    let (re, im) = shelf_h_sc(d, c, sw, cw);
    (re * re + im * im).sqrt()
}

#[inline]
fn shelf_mag(d: f32, c: f32, w: f32) -> f32 {
    let (re, im) = shelf_h(d, c, w);
    (re * re + im * im).sqrt()
}

/// Shelf phase delay in samples at ω.
#[inline]
fn shelf_delay(d: f32, c: f32, w: f32) -> f32 {
    if d <= 0.0 {
        return 0.0;
    }
    let (re, im) = shelf_h(d, c, w);
    -im.atan2(re) / w
}

/// Max first-order stages in the stiffness-dispersion cascade. 6 keeps the
/// 8-voice steel render inside the µs budget; the solver still anchors the
/// stretch exactly at n*, at the cost of a few cents of shape residual
/// between anchors (verified by the h5/h10 stretch tests).
pub const MAX_DISP: usize = 6;

/// Solve a stiffness-dispersion cascade for inharmonicity `b` at `f0`:
/// returns (stages M, pole p). M identical first-order allpasses with pole at
/// +p have phase delay falling ~quadratically below the knee ω_c=(1−p)/√p —
/// the same law as the stiff-string target P(f_n) = N₀/√(1+Bn²) (Van Duyne &
/// Smith 1994 dispersion-filter approach; single-coefficient cascade à la
/// Rauhala & Välimäki 2006, coefficient solved here by bisection instead of
/// their polynomial fit). The knee is pinned at/above the highest matched
/// partial so the quadratic regime covers the audible stretch; numerically
/// verified ≤ ~6 cents residual over 18 partials at B=3e-4 (design_check.py,
/// 2026-07-11 round 2). Runs at note-on only.
fn design_dispersion(
    b: f32,
    f0: f32,
    sr: f32,
    lp_c: f32,
    lp_mix: f32,
    sh_d: f32,
    sh_c: f32,
) -> (usize, f32) {
    if b < 1e-6 {
        return (0, 0.0);
    }
    let n0 = sr / f0;
    let w1 = core::f32::consts::TAU * f0 / sr;
    // match partial: highest of the stretch law we anchor exactly (≈5.5 kHz
    // ceiling — refs are 16 kHz; cap 16 keeps ω_n in the allpass's clean range)
    let n_star = ((5500.0 / f0) as usize).clamp(3, 16) as f32;
    let wn = (n_star * w1).min(2.8);
    // geometric phase-delay deficit between partial 1 and n*, minus what the
    // loop lowpass + coupling shelf already contribute
    let d_geom = n0 * (1.0 / (1.0 + b).sqrt() - 1.0 / (1.0 + b * n_star * n_star).sqrt());
    let d_lp = blend_delay(lp_c, lp_mix, w1) - blend_delay(lp_c, lp_mix, wn)
        + shelf_delay(sh_d, sh_c, w1)
        - shelf_delay(sh_d, sh_c, wn);
    let target = d_geom - d_lp;
    if target <= 0.05 {
        return (0, 0.0);
    }
    // knee constraint (1−p)/√p = ω_n ⇒ p from the quadratic; keeps the cascade
    // quadratic through n*
    let q = 2.0 + wn * wn;
    let p_knee = 0.5 * (q - (q * q - 4.0).sqrt());
    let contrast = allpass_delay(-p_knee, w1) - allpass_delay(-p_knee, wn);
    let m = ((target / contrast.max(1e-6)).ceil() as usize).clamp(1, MAX_DISP);
    // bisect p ∈ (0.01, p_knee]: cascade deficit is monotone in p here
    let (mut lo, mut hi) = (0.01f32, p_knee);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let c = m as f32 * (allpass_delay(-mid, w1) - allpass_delay(-mid, wn));
        if c < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (m, 0.5 * (lo + hi))
}

/// Warm-start filter states for one loaded loop mode (see `load_carrier`).
#[derive(Clone, Copy, Default)]
struct LoopInit {
    lp: f64,
    sh: f64,
    dsx: [f64; MAX_DISP],
    dsy: [f64; MAX_DISP],
    apx: f64,
    apy: f64,
}

/// Load the pick-release triangle carrier as a superposition of the loop's TRUE
/// eigenmodes: for each mode (fixed point of f·P(f) = n·sr over the full
/// phase-delay budget), add 2·Re[G_n·λ_nⁱ] to the buffer, where λ_n = e^{σ+jω}
/// includes the per-tick decay σ = ln(loss·|H_lp|)/P — and warm the filter
/// states with the modes' t=−1 phasors. Loading a flat time-domain shape into
/// a dispersive loop instead mis-projects it: the modes sit d_disp·n/P cycles
/// off the buffer's integer bins AND taper e^{σi} across it, and the resulting
/// leakage skirt of the fundamental (~−36 dB) buried the intended −55…−90 dB
/// upper harmonics of the dark nylon/bass excitations (measured +19 dB at bass
/// h7, 2026-07-11 round 2).
///
/// G_n = −L²(1−e^{−j2πν·pk/L})/(4π²ν²·pk(L−pk)) — the circular triangle's
/// Fourier coefficient (its 2nd derivative is two deltas) at continuous
/// ν = f·L/sr — times the two forward contact moving averages' Dirichlet
/// response and phase advance. Modes past n_syn sit ≥ ~80 dB down the
/// MA²·1/ν² rolloff. Runs at note-on only (~n_syn·len flops).
#[allow(clippy::too_many_arguments)]
fn load_carrier(
    buf: &mut [f32],
    len: usize,
    frac: f32,
    lp_c: f32,
    lp_mix: f32,
    loss: f32,
    disp_a: f32,
    disp_n: usize,
    sh_d: f32,
    sh_c: f32,
    ap_c: f32,
    f0: f32,
    stiff_b: f32,
    sr: f32,
    pick_pos: f32,
    cw: usize,
    n_syn: usize,
) -> LoopInit {
    let mut st = LoopInit::default();
    let pk = ((pick_pos * len as f32) as usize).clamp(2, len - 2);
    let (lf, pkf, cwf) = (len as f64, pk as f64, cw as f64);
    let srf = sr as f64;
    for n in 1..=n_syn {
        let nn = n as f32;
        // exact mode frequency and total loop phase delay at it. Two fixed-point
        // iterations with the trig SHARED between the blend/allpass delay math:
        // wasm transcendentals are software floats, and the original 4×helper
        // version cost ~2 ms per note-on voice (measured, 8-chord = 15.7 ms).
        let mut f = nn * f0 * (1.0 + 0.5 * stiff_b * nn * nn);
        let mut ptot = len as f32 + frac;
        let mut bl_mag = 1.0f32;
        for _ in 0..2 {
            let w = core::f32::consts::TAU * f / sr;
            let (sw, cw_) = w.sin_cos();
            let b = 1.0 - lp_c;
            let dd = 1.0 + b * b - 2.0 * b * cw_;
            let hre = lp_mix + (1.0 - lp_mix) * lp_c * (1.0 - b * cw_) / dd;
            let him = -(1.0 - lp_mix) * lp_c * b * sw / dd;
            bl_mag = (hre * hre + him * him).sqrt();
            let mut d_lp = -him.atan2(hre) / w;
            if sh_d > 0.0 {
                let bs = 1.0 - sh_c;
                let ds = 1.0 + bs * bs - 2.0 * bs * cw_;
                let shre = 1.0 - sh_d * sh_c * (1.0 - bs * cw_) / ds;
                let shim = sh_d * sh_c * bs * sw / ds;
                bl_mag *= (shre * shre + shim * shim).sqrt();
                d_lp += -shim.atan2(shre) / w;
            }
            let d_disp = if disp_n > 0 {
                let a = disp_a;
                let th_n = (-sw).atan2(a + cw_);
                let th_d = (-a * sw).atan2(1.0 + a * cw_);
                -(th_n - th_d) / w * disp_n as f32
            } else {
                0.0
            };
            ptot = (len as f32 + frac + d_lp + d_disp).max(3.0);
            f = 0.5 * (f + nn * sr / ptot);
        }
        if f > 0.45 * sr {
            break;
        }
        let w = core::f64::consts::TAU * f as f64 / srf;
        let nu = f as f64 * lf / srf;
        // per-tick decay: round-trip gain is loss·|H_blend(w)| (allpasses unity)
        let (sw, cwn) = w.sin_cos();
        let sig = (loss as f64 * bl_mag as f64).min(0.99999).ln() / ptot as f64;
        // G = triangle coefficient × MA² × MA phase advance
        let c0 = lf * lf
            / (4.0 * core::f64::consts::PI * core::f64::consts::PI * nu * nu * pkf * (lf - pkf));
        let th = w * pkf;
        let (mut gre, mut gim) = (c0 * (th.cos() - 1.0), -c0 * th.sin());
        if cw > 1 {
            let x = core::f64::consts::PI * nu / lf;
            let hh = (x * cwf).sin() / (cwf * x.sin());
            let h = hh * hh;
            let (pre, pim) = ((w * (cwf - 1.0)).cos(), (w * (cwf - 1.0)).sin());
            let (r, i) = (h * (gre * pre - gim * pim), h * (gre * pim + gim * pre));
            gre = r;
            gim = i;
        }
        // buffer: u_i = 2·Re(G·λⁱ) via the decaying-resonator recurrence
        let es = sig.exp();
        let a1 = 2.0 * es * cwn;
        let a2 = es * es;
        let mut u2 = 2.0 * gre;
        let mut u1 = 2.0 * es * (gre * cwn - gim * sw);
        buf[0] += u2 as f32;
        if len > 1 {
            buf[1] += u1 as f32;
        }
        for t in buf.iter_mut().take(len).skip(2) {
            let u = a1 * u1 - a2 * u2;
            u2 = u1;
            u1 = u;
            *t += u as f32;
        }
        // filter warm states: the loop input phasor at t is G·λᵗ; each state
        // holds its node's t = −1 value. λ⁻¹ ≈ e^{−σ}e^{−jω}.
        let (z1re, z1im) = ((-sig).exp() * cwn, -(-sig).exp() * sw);
        let cmul = |ar: f64, ai: f64, br: f64, bi: f64| (ar * br - ai * bi, ar * bi + ai * br);
        // H_lp = c/(1 − b·e^{−jw}); the blend adds the flat bypass on top
        let b = 1.0 - lp_c as f64;
        let (dre, dim) = (1.0 - b * cwn, b * sw);
        let dd = dre * dre + dim * dim;
        // true response at z = e^{jω}: 1/D with D = (dre + j·dim) conjugates
        let hlp = (lp_c as f64 * dre / dd, -(lp_c as f64) * dim / dd);
        let mixf = lp_mix as f64;
        let hbl = (mixf + (1.0 - mixf) * hlp.0, (1.0 - mixf) * hlp.1);
        // H_ap(a) = (a + e^{−jw})/(1 + a·e^{−jw})
        let hap = |a: f64| {
            let (nre, nim) = (a + cwn, -sw);
            let (dre2, dim2) = (1.0 + a * cwn, -a * sw);
            let dd2 = dre2 * dre2 + dim2 * dim2;
            (
                (nre * dre2 + nim * dim2) / dd2,
                (nim * dre2 - nre * dim2) / dd2,
            )
        };
        let hd = hap(disp_a as f64);
        let hf = hap(ap_c as f64);
        let at = |cr: f64, ci: f64| {
            let (r, _i) = cmul(cr, ci, z1re, z1im);
            2.0 * r
        };
        // the `lp` state variable is the ONE-POLE's output; the signal that
        // proceeds down the chain is the blend output
        let (lre, lim) = cmul(gre, gim, hlp.0, hlp.1);
        st.lp += at(lre, lim);
        let (mut cr, mut ci) = cmul(gre, gim, hbl.0, hbl.1);
        // coupling shelf: internal one-pole L = c/(1 − (1−c)e^{−jω}) fed by the
        // blend output; its state warms with G·H_bl·L, the signal continues
        // through H_sh = 1 − d·L
        if sh_d > 0.0 {
            let bs = 1.0 - sh_c as f64;
            let (dsr, dsi) = (1.0 - bs * cwn, bs * sw);
            let dds = dsr * dsr + dsi * dsi;
            let l = (sh_c as f64 * dsr / dds, -(sh_c as f64) * dsi / dds);
            let (lr, li) = cmul(cr, ci, l.0, l.1);
            st.sh += at(lr, li);
            let (r, i) = cmul(cr, ci, 1.0 - sh_d as f64 * l.0, -(sh_d as f64) * l.1);
            cr = r;
            ci = i;
        }
        for k in 0..disp_n {
            st.dsx[k] += at(cr, ci);
            let (r, i) = cmul(cr, ci, hd.0, hd.1);
            cr = r;
            ci = i;
            st.dsy[k] += at(cr, ci);
        }
        st.apx += at(cr, ci);
        let (r, i) = cmul(cr, ci, hf.0, hf.1);
        st.apy += at(r, i);
    }
    st
}

/// Per-period amplitude ratio hitting `t60` seconds at frequency `f0`:
/// G^(t60·f0) = 10^-3  ⇒  G = 10^(−3/(t60·f0)).
#[inline]
fn per_period_gain(t60: f32, f0: f32) -> f32 {
    if t60 <= 0.0 {
        0.0
    } else {
        (10.0f32).powf(-3.0 / (t60 * f0))
    }
}

impl PluckVoice {
    /// All-inert extension state (legacy constructor + base for the acoustic one).
    fn blank() -> Self {
        Self {
            buf: [0.0; PLUCK_BUF],
            len: 2,
            pos: 0,
            lp: 0.0,
            lp_c: 0.5,
            lp_mix: 0.0,
            loss: 0.0,
            ap_c: 0.0,
            ap_x1: 0.0,
            ap_y1: 0.0,
            level: 0.0,
            life: 0,
            age: 0,
            sr: 48_000.0,
            buf2: [0.0; PLUCK_BUF],
            len2: 2,
            pos2: 0,
            lp2: 0.0,
            loss2: 0.0,
            ap2_c: 0.0,
            ap2_x1: 0.0,
            ap2_y1: 0.0,
            pol_mix: 0.0,
            disp_a: 0.0,
            disp_n: 0,
            dsx: [0.0; MAX_DISP],
            dsy: [0.0; MAX_DISP],
            ds2x: [0.0; MAX_DISP],
            ds2y: [0.0; MAX_DISP],
            br_rho: 0.0,
            br_x1: 0.0,
            sh_d: 0.0,
            sh_c: 0.5,
            sh_y: 0.0,
            sh2_y: 0.0,
            rad_k: 0.0,
            rad_p: 0.0,
            rad_x1: 0.0,
            rad_y1: 0.0,
            acc_rho: 0.0,
            acc_x1: 0.0,
            tm_dev: 0.0,
            tm_env: 0.0,
            tm_c: 0.0,
            tm_norm: 0.0,
            frac1: 0.0,
            frac2: 0.0,
            tr_env: 0.0,
            tr_dec: 0.0,
            tr2_env: 0.0,
            tr2_dec: 0.0,
            bp_y1: 0.0,
            bp_y2: 0.0,
            bp_a1: 0.0,
            bp_r2: 0.0,
            bp_g: 0.0,
            ri_env: 0.0,
            ri_c: 1.0,
            tr_rng: Lcg(1),
            th_amp: 0.0,
            th_dph: 0.0,
            th_ph: 1.0,
            rel_t60: 0.0,
            rel_click: 0.0,
            rel_ramp: 0.0,
            loss_tgt: 0.0,
            loss_c: 0.0,
            tr_lvl: 0.0,
            f0: 0.0,
        }
    }
    pub fn start(f0: f32, vel: f32, sr: f32, t60: f32, bright: f32, pick_pos: f32, seed: u32) -> Self {
        // Loop-filter tuning compensation (Jaffe-Smith): the one-pole loop lowpass
        // delays the loop by ~(1-c)/c samples, and c depends on brightness/velocity —
        // a fixed compensation detunes the string with velocity. Subtract the actual
        // filter delay, and bias the allpass fraction into [0.5, 1.5) so its pole
        // stays well inside the unit circle (ill-conditioned as frac→0).
        let lp_c = (bright * (0.35 + 0.6 * vel)).clamp(0.05, 0.995);
        let lp_delay = (1.0 - lp_c) / lp_c;
        let period = sr / f0;
        let total = (period - lp_delay).max(3.0);
        let len = ((total - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac = (total - len as f32).clamp(0.1, 1.5);
        let ap_c = (1.0 - frac) / (1.0 + frac);
        let mut v = Self {
            len,
            lp_c,
            loss: t60_gain(t60, sr),
            ap_c,
            level: 0.5 * (0.35 + 0.65 * vel),
            life: ((t60 * 1.5) * sr) as u64,
            sr,
            ..Self::blank()
        };
        // Excitation pre-loaded into the delay line: velocity-lowpassed noise,
        // comb-filtered at the pick position (Jaffe-Smith), DC-removed.
        let mut rng = Lcg(seed | 1);
        let mut lp = 0.0f32;
        let exc_c = 0.25 + 0.72 * vel; // soft pluck = duller excitation
        let mut tmp = [0.0f32; PLUCK_BUF];
        for t in tmp.iter_mut().take(len) {
            lp += exc_c * (rng.next() - lp);
            *t = lp;
        }
        let p = ((pick_pos * len as f32) as usize).clamp(1, len - 1);
        let mut mean = 0.0;
        for i in 0..len {
            let comb = tmp[i] - 0.9 * tmp[(i + len - p) % len];
            v.buf[i] = comb;
            mean += comb;
        }
        mean /= len as f32;
        for b in v.buf.iter_mut().take(len) {
            *b -= mean;
        }
        v
    }

    /// Acoustic guitar string (nylon/steel): displacement-release excitation,
    /// per-period loss calibrated to a fundamental t60, two polarizations,
    /// optional stiffness dispersion, bridge-force output, light tension mod.
    ///
    /// Decay calibration: in a KS loop each circulating wave sample passes the
    /// loss/lowpass ONCE PER PERIOD, so the per-application gain for a target
    /// t60(f0) is 10^(−3/(t60·f0)) — the legacy per-sample t60_gain() would be
    /// ~len× too slow (root cause of the flat guitar envelopes, 2026-07-11).
    /// Harmonic n additionally sees |H_lp(f_n)| per period, which is what gives
    /// the measured frequency-dependent t60 ladder (Välimäki/Jaffe-Smith).
    pub fn start_acoustic(p: &AcPluck, sr: f32, seed: u32) -> Self {
        let period = sr / p.f0;
        let w0 = core::f32::consts::TAU * p.f0 / sr;
        // ------- loop-loss design -------
        // Law mode (eta_f > 0): the whole t60(f) ladder comes from the physical
        // damping model — Valette-form string damping (Woodhouse 2004 II eq. 8)
        //   eta(f) = (eta_f + eta_a/(2πf) + s·eta_b)/(1 + s),  s = stiff_b·n²
        // plus body-coupling loss couple·body_coupling_shape(f), and
        //   t60(f) = ln(1000)/(π·f·eta(f)).
        // The ladder is U-shaped: lowest partials decay FASTER than the
        // mid-band (air drag + body coupling), highs faster still (bending
        // loss). Fit: blend filter carries the HF side (knee bisection as
        // before, floor from the law), the coupling shelf carries the LF side
        // (depth bisected to hit t60(f0) relative to the least-damped anchor).
        let t60_law = |f: f32| -> f32 {
            let n = f / p.f0;
            let s = p.stiff_b * n * n;
            let eta_s =
                (p.eta_f + p.eta_a / (core::f32::consts::TAU * f) + s * p.eta_b) / (1.0 + s);
            let eta = eta_s + p.couple * body_coupling_shape(f);
            6.907755 / (core::f32::consts::PI * f * eta.max(1e-6))
        };
        let law = p.eta_f > 0.0;
        let t60_f0 = if law { t60_law(p.f0) } else { p.t60_f0 };
        let g0 = per_period_gain(t60_f0, p.f0);
        let g_t = |f: f32| per_period_gain(t60_law(f), p.f0);
        let (lp_c, lp_mix, sh_d, sh_c, loss);
        if law {
            // least-damped anchor (top of the U) in [f0, 1.2 kHz]
            let (mut f_v, mut g_v) = (p.f0, g0);
            for &fc in &[300.0f32, 480.0, 700.0, 1000.0, 1200.0] {
                if fc > p.f0 && fc < 0.4 * sr {
                    let g = g_t(fc);
                    if g > g_v {
                        g_v = g;
                        f_v = fc;
                    }
                }
            }
            let w_v = core::f32::consts::TAU * f_v / sr;
            let (sv, cv) = w_v.sin_cos();
            let (s0, c0) = w0.sin_cos();
            // Blend fit to LAW targets at two anchors that the metrics can see
            // (NSynth refs are 16 kHz): the floor from ~4.5 kHz, the knee
            // (lp_c) bisected so the MID ladder (~1 kHz) matches. A first cut
            // anchored the plateau at 2.4×knee ≈ 6.8 kHz where the law is very
            // dark — the first-order blend then smeared that darkness down
            // into the 0.8–3 kHz band and collapsed the mid decay (it2
            // regression 2026-07-12: t60(1 kHz) 4.6 s → ~1 s).
            let f_top = (4500.0f32).min(0.4 * sr).max(2.0 * p.f0);
            let f_mid = (3.5 * p.f0).clamp(900.0, 2200.0).min(0.6 * f_top);
            lp_mix = (g_t(f_top) / g_v).clamp(0.0, 0.98);
            let w_m = core::f32::consts::TAU * f_mid / sr;
            let (sm, cm) = w_m.sin_cos();
            let target_mid = (g_t(f_mid) / g_v).clamp(lp_mix, 1.0);
            // |H_bl(w_m)| rises monotonically with lp_c (brighter one-pole)
            let (mut lo, mut hi) = (0.01f32, 0.995f32);
            for _ in 0..28 {
                let mid = 0.5 * (lo + hi);
                if blend_mag_sc(mid, lp_mix, sm, cm) < target_mid {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lp_c = 0.5 * (lo + hi);
            // coupling shelf: knee ~320 Hz; depth from the f0-vs-valley ratio
            if f_v > p.f0 * 1.02 {
                let cwsh = (core::f32::consts::TAU * 320.0 / sr).cos();
                let (mut lo, mut hi) = (0.005f32, 0.95f32);
                for _ in 0..24 {
                    let mid = 0.5 * (lo + hi);
                    if onepole_mag_cw(mid, cwsh) < 0.5 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                sh_c = 0.5 * (lo + hi);
                let r = (g0 / g_v)
                    * (blend_mag_sc(lp_c, lp_mix, sv, cv) / blend_mag_sc(lp_c, lp_mix, s0, c0));
                if r < 0.999 {
                    let (mut lo_d, mut hi_d) = (0.0f32, 0.92f32);
                    for _ in 0..30 {
                        let mid = 0.5 * (lo_d + hi_d);
                        let ratio =
                            shelf_mag_sc(mid, sh_c, s0, c0) / shelf_mag_sc(mid, sh_c, sv, cv);
                        if ratio > r {
                            lo_d = mid;
                        } else {
                            hi_d = mid;
                        }
                    }
                    sh_d = 0.5 * (lo_d + hi_d);
                } else {
                    sh_d = 0.0;
                }
            } else {
                sh_d = 0.0;
                sh_c = 0.5;
            }
            loss = (g_v / (blend_mag_sc(lp_c, lp_mix, sv, cv) * shelf_mag_sc(sh_d, sh_c, sv, cv)))
                .min(0.99995);
        } else {
            // legacy hand-fit path (bass): pure one-pole or ladder-over-floor —
            // knee via |H_lp(knee)| = ½, bypass from the target HF-floor t60
            // relative to the fundamental's; brighten minimally if the filter
            // is too dark to sustain the fundamental (loop stability).
            let (mut c, m) = if p.hf_floor_t60 > 0.0 {
                let wk = core::f32::consts::TAU * p.hf_knee_hz.max(2.0 * p.f0) / sr;
                let (mut lo, mut hi) = (0.005f32, 0.95f32);
                for _ in 0..24 {
                    let mid = 0.5 * (lo + hi);
                    if onepole_mag(mid, wk.min(3.0)) < 0.5 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let g_floor = per_period_gain(p.hf_floor_t60, p.f0);
                (0.5 * (lo + hi), (g_floor / g0).clamp(0.0, 0.98))
            } else {
                (p.lp_c.clamp(0.05, 0.995), 0.0)
            };
            while blend_mag(c, m, w0) < g0 && c < 0.99 {
                c += 0.01;
            }
            lp_c = c;
            lp_mix = m;
            sh_d = 0.0;
            sh_c = 0.5;
            loss = (g0 / blend_mag(lp_c, lp_mix, w0)).min(0.99995);
        }

        // Stiffness dispersion cascade (solved per note), then tuning: subtract
        // the exact loop-filter + cascade phase delays at f0 so partial 1 stays
        // on pitch while uppers stretch.
        let (disp_n, disp_p) = design_dispersion(p.stiff_b, p.f0, sr, lp_c, lp_mix, sh_d, sh_c);
        let disp_a = -disp_p;
        let d_lp = blend_delay(lp_c, lp_mix, w0) + shelf_delay(sh_d, sh_c, w0);
        let d_disp = disp_n as f32 * allpass_delay(disp_a, w0);
        let total = (period - d_lp - d_disp).max(3.0);
        // Bias the fraction high when tension-mod wants sharpening headroom.
        let bias = if p.tm_cents > 0.0 { 1.45 } else { 0.5 };
        let len = ((total - bias).ceil() as usize).clamp(2, PLUCK_BUF - 1);
        let frac1 = (total - len as f32).clamp(0.1, 1.5);

        // Second polarization: detuned by a couple cents, faster decay
        // (vertical motion pumps the bridge harder — Weinreich 1977 two-stage).
        let f2 = p.f0 * (p.pol_detune_cents / 1200.0).exp2();
        let total2 = (sr / f2 - d_lp - d_disp).max(3.0);
        let len2 = ((total2 - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac2 = (total2 - len2 as f32).clamp(0.1, 1.5);
        let g2 = per_period_gain(t60_f0 * p.pol_t60_ratio.max(0.05), f2);
        let loss2 =
            (g2 / (blend_mag(lp_c, lp_mix, w0) * shelf_mag(sh_d, sh_c, w0))).min(0.99995);

        // Tension modulation depth: cents → samples of delay reduction, limited
        // by the allpass fraction headroom (band-limited by a ~6 Hz env² follower).
        let tm_dev = if p.tm_cents > 0.0 {
            (period * p.tm_cents * p.vel * p.vel * core::f32::consts::LN_2 / 1200.0)
                .min(frac1 - 0.12)
                .max(0.0)
        } else {
            0.0
        };

        // Bridge differencer kills the fundamental (−38 dB at E2): renormalize
        // output level to the |H_br| magnitude at 3·f0, so the force spectrum's
        // TILT is kept but overall loudness stays register-comparable.
        let mut level = if p.br_rho > 0.0 {
            let w3 = (3.0 * w0).min(core::f32::consts::FRAC_PI_2);
            let mag = (1.0 - 2.0 * p.br_rho * w3.cos() + p.br_rho * p.br_rho).sqrt();
            p.level / mag.max(1e-3)
        } else {
            p.level
        };
        // The direct click transient bypasses the differencers: scale it by
        // the level BEFORE the acceleration renorm (the renorm compensates
        // the string's in-chain loss, which the click never suffers — left on
        // the post-renorm level it ran ~12x hot at E2 and clipped at onset).
        let level_tr = level;
        // Acceleration differencer: renormalized at 3·f0 like the force tap.
        if p.acc_rho > 0.0 {
            let w3 = (3.0 * w0).min(core::f32::consts::FRAC_PI_2);
            let mag = (1.0 - 2.0 * p.acc_rho * w3.cos() + p.acc_rho * p.acc_rho).sqrt();
            level /= mag.max(1e-3);
        }
        // Radiation monopole HP (bilinear s/(s+ω_c)); renormalized at 3·f0 like
        // the differencer so the low-register CUT below f_c is a tilt, not a
        // register-level rebalance.
        let (rad_k, rad_p) = if p.rad_hz > 0.0 {
            let t = (core::f32::consts::PI * p.rad_hz / sr).tan();
            let (k, pl) = (1.0 / (1.0 + t), (1.0 - t) / (1.0 + t));
            let w3 = (3.0 * w0).min(core::f32::consts::FRAC_PI_2);
            let mag = k * 2.0 * (w3 * 0.5).sin()
                / (1.0 - 2.0 * pl * w3.cos() + pl * pl).sqrt();
            level /= mag.max(1e-3).min(1.0);
            (k, pl)
        } else {
            (0.0, 0.0)
        };
        let mut v = Self {
            len,
            lp_c,
            lp_mix,
            loss,
            ap_c: (1.0 - frac1) / (1.0 + frac1),
            level,
            // the aftersound polarization may outlive the plucked one (ratio>1);
            // cap like the piano does (pool pressure)
            life: (((t60_f0 * p.pol_t60_ratio.max(1.0) * 1.1 + 0.5).min(18.0)) * sr) as u64,
            sr,
            len2,
            loss2,
            ap2_c: (1.0 - frac2) / (1.0 + frac2),
            pol_mix: p.pol_mix,
            disp_a,
            disp_n: disp_n as u8,
            br_rho: p.br_rho,
            acc_rho: p.acc_rho,
            sh_d,
            sh_c,
            rad_k,
            rad_p,
            tm_dev,
            tm_c: 1.0 - (-core::f32::consts::TAU * 6.0 / sr).exp(),
            frac1,
            frac2,
            // pick transients (see struct docs): snap ∝ vel² (crest law from
            // the refs: ~0 dB at ff, −22 dB at pp), scrape keeps its energy
            // at low velocity but spreads it over a longer, softer slide
            tr_env: p.click * (0.05 + 0.95 * p.vel * p.vel) * level_tr,
            tr_dec: t60_gain(0.008, sr),
            tr2_env: p.click_slow * (0.25 + 0.75 * p.vel) * level_tr,
            tr2_dec: t60_gain(0.055 - 0.030 * p.vel, sr),
            // 2-pole contact resonator at click_hz, BW ≈ 0.65·f (Q ≈ 1.5),
            // peak-normalized to 1 so click levels keep their fitted scale
            bp_a1: {
                let w = core::f32::consts::TAU * p.click_hz / sr;
                let r = (-core::f32::consts::PI * 0.65 * p.click_hz / sr).exp();
                2.0 * r * w.cos()
            },
            bp_r2: {
                let r = (-core::f32::consts::PI * 0.65 * p.click_hz / sr).exp();
                r * r
            },
            bp_g: {
                let w = core::f32::consts::TAU * p.click_hz / sr;
                let r = (-core::f32::consts::PI * 0.65 * p.click_hz / sr).exp();
                (1.0 - r) * 2.0 * w.sin().max(0.1)
            },
            ri_env: 0.0,
            ri_c: 1.0 - (-1.0 / (p.click_ramp.max(1e-4) * sr)).exp(),
            tr_rng: Lcg(seed.rotate_left(13) | 1),
            // body-pump: one bipolar cycle at thump_hz (see AcPluck::thump);
            // amp on the pre-differencer level scale like the click
            th_amp: p.thump * p.vel * p.vel * level_tr,
            th_dph: p.thump_hz / sr,
            th_ph: 0.0,
            rel_t60: p.rel_t60,
            rel_click: p.rel_click,
            rel_ramp: p.rel_ramp,
            tr_lvl: level_tr,
            f0: p.f0,
            ..Self::blank()
        };

        // --- pick-release excitation (displacement initial condition) ---
        // A pluck is a RELEASE of a displaced string: triangle peaked at the pick
        // point (harmonic amps ∝ sin(nπβ)/n² — the pick-position comb is inherent),
        // corner rounded by pick/finger compliance, plus a localized release-snap
        // bump (velocity component) and a dash of contact noise.
        //
        // The dark carrier (triangle ⊗ contact-MA²) is synthesized ADDITIVELY on
        // the loop's true (stretched) mode grid below. Preloading a time-domain
        // shape into the buffer while the dispersion cascade starts empty
        // mis-projects it onto the stiff-string modes — the modes sit d_disp·n/P
        // cycles off the buffer's integer bins, and the non-integer-bin leakage
        // skirt of the fundamental (~−36 dB) buried the intended −55…−90 dB
        // upper harmonics of the dark nylon/bass excitations (measured +19 dB at
        // bass h7, 2026-07-11 round 2). Steep spectra therefore go exact-grid;
        // the spectrally-broad parts (snap bump, scrape noise) stay time-domain,
        // where leakage only trades energy between near-equal neighbors.
        let mut rng = Lcg(seed | 1);
        let pk = ((p.pick_pos * len as f32) as usize).clamp(2, len - 2);
        let cw = ((p.contact * (1.2 - 0.2 * p.vel) * len as f32) as usize).clamp(1, len / 4);
        let mut tmp = [0.0f32; PLUCK_BUF];
        // release snap: narrow raised-cosine bump at the pick point (the corner
        // the pick leaves as it lets go); NSynth refs show attack brightness
        // grows with velocity but far less than linearly
        if p.snap > 0.0 {
            let wdt = ((len as f32 * 0.016) as usize + 2).min(len / 4);
            let amp = p.snap * (0.35 + 0.65 * p.vel);
            for j in 0..(2 * wdt) {
                let i = (pk + len - wdt + j) % len;
                let ph = j as f32 / (2 * wdt) as f32;
                tmp[i] += amp * 0.5 * (1.0 - (core::f32::consts::TAU * ph).cos());
            }
        }
        // contact noise (pick scrape / fingertip friction) near the pick point
        if p.scrape > 0.0 {
            let wdt = (len / 8).max(4);
            for j in 0..wdt {
                let i = (pk + len - wdt / 2 + j) % len;
                tmp[i] += p.scrape * (0.35 + 0.65 * p.vel) * rng.next() * 0.5;
            }
        }
        // compliance: the contact patch (fingertip flesh ≫ pick tip) rounds the
        // WHOLE initial condition. Two circular moving-average passes of width
        // contact·len ≈ triangular kernel: first spectral null at n = len/width,
        // so the excitation bandwidth is a physical fraction of f0.
        if cw > 1 {
            let mut acc = [0.0f32; PLUCK_BUF];
            for _ in 0..2 {
                let mut sum = 0.0;
                for j in 0..cw {
                    sum += tmp[j];
                }
                let inv = 1.0 / cw as f32;
                for (i, a) in acc.iter_mut().enumerate().take(len) {
                    *a = sum * inv;
                    sum += tmp[(i + cw) % len] - tmp[i];
                }
                tmp[..len].copy_from_slice(&acc[..len]);
            }
        }
        // DC removal of the time-domain parts, then load both polarizations
        // (pol2 gets the same snap/scrape — it IS the same string)
        let mut mean = 0.0;
        for t in tmp.iter().take(len) {
            mean += *t;
        }
        mean /= len as f32;
        for i in 0..len {
            v.buf[i] = tmp[i] - mean;
        }
        if p.pol_mix > 0.0 {
            for i in 0..len2 {
                // resample the len-shape onto len2 (nearest is fine: ±2 cents)
                let src = (i * len) / len2;
                v.buf2[i] = tmp[src] - mean;
            }
        }
        // (differencer priming happens AFTER the mode-exact carrier loads
        // below — load_carrier rewrites buf/buf2, and priming from the stale
        // pre-carrier shape passed the mismatch through both renormalized
        // differencers as a ±full-scale 2-sample impulse at onset.)
        // Tension mod follows the STRING displacement power (excitation shape is
        // peak-normalized ~1); the string is maximally elongated at release, so
        // the glide starts sharp and settles as the note decays.
        v.tm_norm = 1.0;
        if v.tm_dev > 0.0 {
            v.tm_env = 1.0;
            let f1 = (frac1 - v.tm_dev).clamp(0.1, 1.5);
            v.ap_c = (1.0 - f1) / (1.0 + f1);
            let f2 = (frac2 - v.tm_dev).clamp(0.1, 1.5);
            v.ap2_c = (1.0 - f2) / (1.0 + f2);
        }
        // Mode-exact triangle carrier + warm filter states, per polarization
        // (after the tension-mod block: modes are solved at the ONSET fractional
        // delay, which is what the first rendered blocks use).
        let n_syn = ((2.5 * len as f32 / cw as f32) as usize + 6)
            .min((0.45 * sr / p.f0) as usize)
            .min(40)
            .min(len / 2);
        let st = load_carrier(
            &mut v.buf,
            len,
            (frac1 - tm_dev).clamp(0.1, 1.5),
            lp_c,
            lp_mix,
            loss,
            disp_a,
            disp_n,
            sh_d,
            sh_c,
            v.ap_c,
            p.f0,
            p.stiff_b,
            sr,
            p.pick_pos,
            cw,
            n_syn,
        );
        v.lp = st.lp as f32;
        v.sh_y = st.sh as f32;
        v.ap_x1 = st.apx as f32;
        v.ap_y1 = st.apy as f32;
        for k in 0..disp_n {
            v.dsx[k] = st.dsx[k] as f32;
            v.dsy[k] = st.dsy[k] as f32;
        }
        if p.pol_mix > 0.0 {
            let st2 = load_carrier(
                &mut v.buf2,
                len2,
                (frac2 - tm_dev).clamp(0.1, 1.5),
                lp_c,
                lp_mix,
                loss2,
                disp_a,
                disp_n,
                sh_d,
                sh_c,
                v.ap2_c,
                f2,
                p.stiff_b,
                sr,
                p.pick_pos,
                cw,
                n_syn,
            );
            v.lp2 = st2.lp as f32;
            v.sh2_y = st2.sh as f32;
            v.ap2_x1 = st2.apx as f32;
            v.ap2_y1 = st2.apy as f32;
            for k in 0..disp_n {
                v.ds2x[k] = st2.dsx[k] as f32;
                v.ds2y[k] = st2.dsy[k] as f32;
            }
        }
        // Prime the output differencers (and radiation HP) with the true t=0
        // HISTORY — the periodic extension of the final carrier (a
        // recirculating wave's previous samples are buf[len−1], buf[len−2],
        // …, modulo one 0.5% loss pass). Body round 2026-07-12: the earlier
        // value-only priming (br_x1 = m0, acc_x1 = leak·m0) pretended the
        // pre-onset history was CONSTANT, so the first differencer's output
        // stepped from leak-only to slope+leak between samples 0→1; the
        // second differencer turned that slope discontinuity into a
        // one-sample impulse amplified ~×950 by the renorms — a
        // velocity-independent onset tick on EVERY steel note (peak 0.93 at
        // pp E2, masked in render-note.mjs by track-gain smoothing at t=0).
        // Priming value + slope makes sample 0 differenced like every other.
        // (History = backward linear extrapolation of the first two carrier
        // samples: the buffer wrap is NOT time-adjacent — the fractional
        // delay lives in the tuning allpass — so periodic indexing would
        // manufacture its own step. The carrier is band-limited ⇒ locally
        // linear at 48 kHz; extrapolation keeps value AND slope consistent.)
        if p.br_rho > 0.0 {
            let m0 = v.buf[0] + p.pol_mix * v.buf2[0];
            let mf = v.buf[1.min(len - 1)] + p.pol_mix * v.buf2[1.min(len2 - 1)];
            let hist = |k: f32| -> f32 { (1.0 + k) * m0 - k * mf };
            let (m1, m2, m3) = (hist(1.0), hist(2.0), hist(3.0));
            let f1 = m1 - p.br_rho * m2;
            let f2 = m2 - p.br_rho * m3;
            v.br_x1 = m1;
            v.acc_x1 = f1;
            if v.rad_k > 0.0 {
                v.rad_x1 = (f1 - p.acc_rho * f2) * v.level;
            }
        }
        v
    }

    pub fn render(&mut self, out: &mut [f32]) -> bool {
        // Tension modulation, once per block: the smoothed env² pulls the tuning
        // allpass fraction down (string starts sharp, settles as it decays).
        // Coefficient steps are ≤0.5 cent per 128-frame block — inaudible zipper,
        // provably band-limited (env is a 6 Hz one-pole of the output power).
        if self.tm_dev > 0.0 {
            let dev = self.tm_dev * self.tm_env.min(1.0);
            let f1 = (self.frac1 - dev).clamp(0.1, 1.5);
            self.ap_c = (1.0 - f1) / (1.0 + f1);
            let f2 = (self.frac2 - dev).clamp(0.1, 1.5);
            self.ap2_c = (1.0 - f2) / (1.0 + f2);
        }
        // Hoist small filter states into locals so the optimizer keeps them in
        // registers across the sample loop (through &mut self they reload per
        // sample — measured 108 µs/quantum at 8 steel voices, ~2× the budget).
        let mut dsx = self.dsx;
        let mut dsy = self.dsy;
        let mut ds2x = self.ds2x;
        let mut ds2y = self.ds2y;
        let disp_n = self.disp_n as usize;
        let (mut lp, mut ap_x1, mut ap_y1) = (self.lp, self.ap_x1, self.ap_y1);
        let (mut lp2, mut ap2_x1, mut ap2_y1) = (self.lp2, self.ap2_x1, self.ap2_y1);
        let (mut sh_y, mut sh2_y) = (self.sh_y, self.sh2_y);
        let (sh_d, sh_c) = (self.sh_d, self.sh_c);
        let (mut loss, mut loss2) = (self.loss, self.loss2);
        let (loss_c, loss_tgt) = (self.loss_c, self.loss_tgt);
        for o in out.iter_mut() {
            // release loss-glide (no-op while loss_c = 0: x + 0.0·y ≡ x, so
            // rel_ramp-less voices stay bit-identical — see rel_ramp docs)
            loss += loss_c * (loss_tgt - loss);
            loss2 += loss_c * (loss_tgt - loss2);
            let y = self.buf[self.pos];
            // blend loss filter: one-pole ladder over a flat HF bypass floor
            // (m·y + (1−m)·lp ≡ lp + m(y−lp); m = 0 is the pure one-pole)
            lp += self.lp_c * (y - lp);
            // stiffness dispersion: M-stage allpass cascade delays lows vs highs
            // (pole at +p ⇒ stretched partials, see design_dispersion)
            let mut s = self.lp_mix.mul_add(y - lp, lp);
            // coupling shelf: extra per-period loss for the lowest partials
            sh_y += sh_c * (s - sh_y);
            s -= sh_d * sh_y;
            for k in 0..disp_n {
                let d = self.disp_a * (s - dsy[k]) + dsx[k];
                dsx[k] = s;
                dsy[k] = d;
                s = d;
            }
            // fractional-delay allpass keeps the string in tune
            let ap = self.ap_c * (s - ap_y1) + ap_x1;
            ap_x1 = s;
            ap_y1 = ap;
            self.buf[self.pos] = ap * loss;
            self.pos += 1;
            if self.pos >= self.len {
                self.pos = 0;
            }
            let mut mix = y;
            // second polarization (own loop; summed at the bridge). Same string,
            // same stiffness: it disperses through its own cascade states.
            if self.pol_mix > 0.0 {
                let y2 = self.buf2[self.pos2];
                lp2 += self.lp_c * (y2 - lp2);
                let mut s2 = self.lp_mix.mul_add(y2 - lp2, lp2);
                sh2_y += sh_c * (s2 - sh2_y);
                s2 -= sh_d * sh2_y;
                for k in 0..disp_n {
                    let d = self.disp_a * (s2 - ds2y[k]) + ds2x[k];
                    ds2x[k] = s2;
                    ds2y[k] = d;
                    s2 = d;
                }
                let ap2 = self.ap2_c * (s2 - ap2_y1) + ap2_x1;
                ap2_x1 = s2;
                ap2_y1 = ap2;
                self.buf2[self.pos2] = ap2 * loss2;
                self.pos2 += 1;
                if self.pos2 >= self.len2 {
                    self.pos2 = 0;
                }
                mix += self.pol_mix * y2;
            }
            if self.tm_dev > 0.0 {
                let p = mix * mix * self.tm_norm;
                self.tm_env += self.tm_c * (p - self.tm_env);
            }
            // bridge force ≈ leaky first difference of displacement (+6 dB/oct):
            // what actually drives the top plate (Fletcher & Rossing ch. 9)
            let mut outv = if self.br_rho > 0.0 {
                let f = mix - self.br_rho * self.br_x1;
                self.br_x1 = mix;
                f
            } else {
                mix
            };
            // acceleration differencer (radiated pressure ~ volume acceleration)
            if self.acc_rho > 0.0 {
                let a = outv - self.acc_rho * self.acc_x1;
                self.acc_x1 = outv;
                outv = a;
            }
            let mut sig = outv * self.level;
            // pick transients (bypass the string loop): snap + scrape share
            // the 2-pole contact resonator; ~1.5 ms ramp-in, no t=0 cliff
            if self.tr_env > 1e-7 || self.tr2_env > 1e-7 {
                let n = self.tr_rng.next();
                let y = self.bp_a1 * self.bp_y1 - self.bp_r2 * self.bp_y2 + self.bp_g * n;
                self.bp_y2 = self.bp_y1;
                self.bp_y1 = y;
                self.ri_env += self.ri_c * (1.0 - self.ri_env);
                sig += (self.tr_env + self.tr2_env) * self.ri_env * y;
                self.tr_env *= self.tr_dec;
                self.tr2_env *= self.tr2_dec;
            }
            // radiation monopole high-pass (see rad_k docs)
            if self.rad_k > 0.0 {
                let y = self.rad_p * self.rad_y1 + self.rad_k * (sig - self.rad_x1);
                self.rad_x1 = sig;
                self.rad_y1 = y;
                sig = y;
            }
            // body-pump transient: injected AFTER the radiation HP (the HP
            // models the string→plate path; the pump IS the plate's own
            // low-frequency volume flow) — one raised-cosine-windowed sine
            // cycle, zero-mean, band-centered at thump_hz. The track body
            // bank (A0/T1) rings from it.
            if self.th_amp != 0.0 && self.th_ph < 1.0 {
                let ph = core::f32::consts::TAU * self.th_ph;
                sig += self.th_amp * ph.sin() * 0.5 * (1.0 - ph.cos());
                self.th_ph += self.th_dph;
            }
            *o += sig;
        }
        self.dsx = dsx;
        self.dsy = dsy;
        self.ds2x = ds2x;
        self.ds2y = ds2y;
        self.loss = loss;
        self.loss2 = loss2;
        self.lp = flush_denormal(lp);
        self.sh_y = flush_denormal(sh_y);
        self.ap_x1 = ap_x1;
        self.ap_y1 = flush_denormal(ap_y1);
        if self.pol_mix > 0.0 {
            self.lp2 = flush_denormal(lp2);
            self.sh2_y = flush_denormal(sh2_y);
            self.ap2_x1 = ap2_x1;
            self.ap2_y1 = flush_denormal(ap2_y1);
        }
        for k in 0..self.disp_n as usize {
            self.dsy[k] = flush_denormal(self.dsy[k]);
            self.ds2y[k] = flush_denormal(self.ds2y[k]);
        }
        self.tm_env = flush_denormal(self.tm_env);
        self.bp_y1 = flush_denormal(self.bp_y1);
        self.bp_y2 = flush_denormal(self.bp_y2);
        self.tr2_env = flush_denormal(self.tr2_env);
        self.br_x1 = flush_denormal(self.br_x1);
        self.acc_x1 = flush_denormal(self.acc_x1);
        self.rad_y1 = flush_denormal(self.rad_y1);
        self.age += out.len() as u64;
        self.age < self.life
    }

    pub fn damp(&mut self) {
        if self.f0 > 0.0 {
            // acoustic path: per-period loss toward the instrument's release
            // t60 — finger/palm damping is not instantaneous (steel refs ring
            // ~0.5 s after note-off); voice retired once the tail is spent
            let rel = self.rel_t60.max(0.05);
            let g = per_period_gain(rel, self.f0);
            if self.rel_ramp > 0.0 {
                // glide the write gain instead of stepping it (see rel_ramp)
                self.loss_tgt = g;
                self.loss_c = 1.0 - (-1.0 / (self.rel_ramp * self.sr)).exp();
            } else {
                self.loss = g;
                self.loss2 = g;
            }
            self.life = self.age + ((3.0 * rel + 0.15) * self.sr) as u64;
            // pluck-off: the fret/finger release re-excites the top with a
            // short click scaled by the string's REMAINING energy (a decayed
            // note releases quietly) — audible at note-off in the refs
            if self.rel_click > 0.0 {
                let mut acc = 0.0f32;
                for &b in self.buf.iter().take(self.len) {
                    acc += b * b;
                }
                let rms = (acc / self.len.max(1) as f32).sqrt();
                let amp = self.rel_click * rms.min(1.0) * self.tr_lvl;
                if amp > self.tr_env {
                    self.tr_env = amp;
                }
            }
        } else {
            self.loss = t60_gain(0.07, self.sr);
            self.life = self.age + (0.1 * self.sr) as u64;
        }
    }
}

// ---------------------------------------------------------------------------
// Acoustic piano — physically-informed (Bank 2003 / Smith-Van Duyne lineage):
// 1–3 detuned string waveguides per note (unison beating), cascaded dispersion
// allpasses (inharmonicity / stretched partials), velocity-dependent hammer
// excitation with strike-position comb, register-scaled decay, damper on
// note-off (deferred by the sustain pedal at the engine level).
// Deferred until demanded: commuted soundboard IR, sympathetic resonance.
// ---------------------------------------------------------------------------

/// Max stages in the piano stiffness-dispersion cascade (P1 per-key
/// calibration): deep-bass keys need ~44 samples of phase-delay contrast
/// between p1 and p20 (A0: B = 2.2e-4 measured on Salamander, loop ~1750
/// samples), which takes ~14-18 first-order sections. Solved per note in
/// `design_piano_dispersion`; treble keys use 1-3.
const PIANO_MAX_DISP: usize = 20;

#[derive(Clone, Copy)]
struct StringLoop {
    buf: [f32; PLUCK_BUF],
    len: usize,
    pos: usize,
    lp: f32,
    lp_c: f32,
    loss: f32,
    // fractional tuning allpass
    ap_c: f32,
    ap_x1: f32,
    ap_y1: f32,
    // stiffness dispersion: M cascaded first-order allpasses, all with
    // coefficient `disp_a` = −p (pole at +p ⇒ phase delay FALLS with
    // frequency ⇒ upper partials arrive early ⇒ stretch sharp, Fletcher
    // 1964). (M, p) solved per note from the per-key measured B —
    // `design_piano_dispersion`, same Van Duyne & Smith / Rauhala &
    // Välimäki lineage as the guitar's `design_dispersion`.
    disp_a: f32,
    disp_n: u8,
    dx: [f32; PIANO_MAX_DISP],
    dy: [f32; PIANO_MAX_DISP],
    // in-loop DC blocker (~19 Hz): hammer force injection is unipolar and would
    // otherwise park a slowly-decaying DC pedestal in the loop
    dc_x1: f32,
    dc_y1: f32,
}

/// In-loop DC-blocker pole (~0.76 Hz at 48 k). Shared by `tick` and the delay budget:
/// the blocker's phase LEAD at f0 shortens the effective loop delay and must be
/// compensated or every string plays sharp (worst in the bass, where the old 19 Hz
/// blocker left A1 audibly sharp — found while fixing its fundamental damping).
/// Round 3 pushed 0.9996 → 0.9999: the lead FALLS with frequency (5.5 samples at
/// C2's f0 but 0.3 at p4 with the old pole), so the blocker was a hidden
/// negative-dispersion element dragging bass partials ~12 cents FLAT — the
/// opposite of stiffness stretch. At 0.9999 the differential lead is ~1 sample,
/// DC is still blocked (pedestal drains in ~0.2 s; the 2nd-order radiation
/// highpass owns the output-side pedestal), and bass fundamentals lose even
/// less energy to the blocker.
const DC_POLE: f32 = 0.9999;

/// One-pole loop-lowpass H(z) = c/(1 − (1−c)z⁻¹): exact phase delay at ω, in
/// samples (the old (1−c)/c DC approximation left the mid keyboard ~+12…+35
/// cents sharp once the dispersion cascade grew — measured on the P1 baseline).
#[inline]
fn piano_lp_delay(c: f32, w: f32) -> f32 {
    let a = 1.0 - c;
    let (sw, cw) = w.sin_cos();
    (a * sw).atan2(1.0 - a * cw) / w
}

#[inline]
fn piano_lp_mag(c: f32, w: f32) -> f32 {
    let a = 1.0 - c;
    c / (1.0 + a * a - 2.0 * a * w.cos()).sqrt()
}

/// DC blocker H(z) = (1−z⁻¹)/(1−Rz⁻¹): exact phase LEAD at ω, in samples.
#[inline]
fn piano_dc_lead(w: f32) -> f32 {
    ((core::f32::consts::PI - w) / 2.0 - (DC_POLE * w.sin()).atan2(1.0 - DC_POLE * w.cos())) / w
}

#[inline]
fn piano_dc_mag(w: f32) -> f32 {
    (2.0 - 2.0 * w.cos()).sqrt() / (1.0 + DC_POLE * DC_POLE - 2.0 * DC_POLE * w.cos()).sqrt()
}

impl StringLoop {
    /// `detune_cents` shifts this string against the nominal pitch (unison
    /// beating). `loss`/`lp_c` are the per-round-trip loop loss and lowpass
    /// coefficient SOLVED by the caller from the per-key t60 targets
    /// (`solve_piano_loss`); `(disp_n, disp_a)` is the per-note dispersion
    /// cascade (`design_piano_dispersion`).
    fn new(f0: f32, detune_cents: f32, sr: f32, loss: f32, lp_c: f32, disp_n: usize, disp_a: f32) -> Self {
        let f = f0 * (detune_cents / 1200.0).exp2();
        // Total loop delay budget: buffer + tuning-allpass fraction + loop-lowpass
        // phase delay + M× dispersion-allpass phase delay (all EXACT at ω, not DC
        // approximations — the cascade is long enough in the bass that the DC-vs-ω
        // delay difference is whole samples) − the DC blocker's phase lead.
        let w = core::f32::consts::TAU * f / sr;
        let lp_delay = piano_lp_delay(lp_c, w);
        let disp_delay = disp_n as f32 * allpass_delay(disp_a, w);
        let dc_lead = piano_dc_lead(w);
        let total = (sr / f - lp_delay - disp_delay + dc_lead).max(3.0);
        let len = ((total - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac = (total - len as f32).clamp(0.1, 1.5);
        Self {
            buf: [0.0; PLUCK_BUF],
            len,
            pos: 0,
            lp: 0.0,
            lp_c,
            loss,
            ap_c: (1.0 - frac) / (1.0 + frac),
            ap_x1: 0.0,
            ap_y1: 0.0,
            disp_a,
            disp_n: disp_n as u8,
            dx: [0.0; PIANO_MAX_DISP],
            dy: [0.0; PIANO_MAX_DISP],
            dc_x1: 0.0,
            dc_y1: 0.0,
        }
    }

    /// String displacement at an offset ahead of the read head (contact point).
    #[inline(always)]
    fn read_at(&self, off: usize) -> f32 {
        self.buf[(self.pos + off) % self.len]
    }

    /// Inject hammer force into the string at the contact point.
    #[inline(always)]
    fn inject(&mut self, off: usize, v: f32) {
        let i = (self.pos + off) % self.len;
        self.buf[i] += v;
    }

    /// Read the loop output and run the loop filters, RETURNING the value bound
    /// for the bridge instead of writing it back: the PianoVoice bridge mixes
    /// the concurrent per-string values (mutual coupling) and hands each string
    /// its reflected share via `commit`. Split from a self-contained tick for
    /// round 3's true bridge coupling.
    #[inline(always)]
    fn tick(&mut self) -> (f32, f32) {
        let y = self.buf[self.pos];
        // loop lowpass (frequency-dependent loss)
        self.lp += self.lp_c * (y - self.lp);
        // dispersion: M cascaded first-order allpasses delay lows vs highs
        let mut d = self.lp;
        for k in 0..self.disp_n as usize {
            let out = self.disp_a * (d - self.dy[k]) + self.dx[k];
            self.dx[k] = d;
            self.dy[k] = out;
            d = out;
        }
        // fractional tuning allpass
        let ap = self.ap_c * (d - self.ap_y1) + self.ap_x1;
        self.ap_x1 = d;
        self.ap_y1 = ap;
        // DC blocker (loop must not carry the hammer's unipolar injection).
        // Pole 0.9996 ≈ 3 Hz at 48 k: in-loop, a 19 Hz blocker (old 0.9975) costs
        // −0.6 dB PER ROUND TRIP at 49 Hz (−30 dB/s on A1's fundamental) and −12 dB/s
        // at C3 — it was silently eating every bass fundamental (measured 2026-07-11:
        // C3 partial 1 sat 13 dB below the NSynth reference). 3 Hz still drains the
        // hammer's DC pedestal (τ ≈ 70 ms) at negligible fundamental cost (<1 dB/s).
        let dc = ap - self.dc_x1 + DC_POLE * self.dc_y1;
        self.dc_x1 = ap;
        self.dc_y1 = dc;
        (y, dc)
    }

    /// Second half of `tick`: write the bridge-reflected wave back into the
    /// loop (with this string's own round-trip loss) and advance.
    #[inline(always)]
    fn commit(&mut self, w: f32) {
        self.buf[self.pos] = w * self.loss;
        self.pos = (self.pos + 1) % self.len;
    }

    fn flush(&mut self) {
        self.lp = flush_denormal(self.lp);
        self.ap_y1 = flush_denormal(self.ap_y1);
        for k in 0..self.disp_n as usize {
            self.dy[k] = flush_denormal(self.dy[k]);
        }
        self.dc_y1 = flush_denormal(self.dc_y1);
    }
}

// ---------------------------------------------------------------------------
// P1 per-key piano calibration (Pianoteq-class campaign, 2026-07-12).
//
// 24 anchor keys measured on the Salamander Grand references (CC-BY, 48 kHz
// FLAC, 16 velocity layers; ledgered, reference-use only): every-3rd-semitone
// grid A0…C8 minus the six held-out verification keys (D#1, D#2, F#3, D#4,
// F#5, D#6), which the tables must reach by interpolation. Parameters are
// PHYSICAL (inharmonicity B, Railsback cents, t60 seconds) — sample-rate
// specific coefficients are solved at note-start. Measurement pipeline +
// fitted curves: scratchpad piano-p1/{calib,fit_targets,gen_tables}.py.
//
//   LOG10_B   Fletcher 1964 stiff-string inharmonicity, log10 — the classic
//             V-curve with the wound→plain bass break at ~F#2
//   TUNE_C    measured p1 offset vs equal temperament (Railsback stretch)
//   T60L_*    aftersound (anti-phase) t60 at f0 / at min(8f0, 4 kHz), from
//             heterodyne per-partial late slopes (1–6 s, Theil–Sen)
//   T60E_F0   prompt (in-phase) composite t60, early slopes 0.08–0.5 s
//   DIVE_DBPS p1 early excess rate → bridge admittance-LP term g1 (the
//             "fundamental dive" is measured ONLY around C3–G3, not the
//             broad bass Gaussian r3 painted)
//   TRIM_DB   per-key radiated-level trim (LUFS-matched, iterated)
//   K_VEL     velocity→level exponent (refs span 14–33 dB pp→ff, iterated)
//   CONTACT_MS hammer-felt contact target at mf (iterated vs attack centroid)
//   DETUNE_C  unison spread from measured beat rates (gated to 0.4–3.2 c;
//             wound-string false beats excluded)
//   STRIKE_Q  strike-point fraction (comb fit, de-aliased, smoothed)
// ---- generated by piano-p1/gen_tables.py from Salamander refs ----
const PIANO_CAL_N: usize = 24;
const PIANO_CAL_MIDI: [f32; PIANO_CAL_N] = [21.0, 24.0, 27.0, 33.0, 36.0, 42.0, 45.0, 48.0, 51.0, 57.0, 60.0, 66.0, 69.0, 72.0, 75.0, 81.0, 84.0, 87.0, 93.0, 96.0, 99.0, 102.0, 105.0, 108.0];
const PIANO_CAL_LOG10_B: [f32; PIANO_CAL_N] = [-3.654, -3.788, -3.881, -4.047, -4.041, -4.147, -4.12, -3.957, -3.898, -3.666, -3.539, -3.298, -3.196, -3.086, -2.959, -2.74, -2.617, -2.527, -2.281, -2.125, -2.053, -1.948, -1.878, -1.808];
const PIANO_CAL_TUNE_C: [f32; PIANO_CAL_N] = [-15.75, -15.85, -7.822, -6.356, -7.784, -3.369, -0.6742, -5.316, -2.697, -0.1674, -0.187, -1.39, 1.503, 3.475, 3.84, 5.457, 7.942, 10.04, 10.46, 11.76, 12.46, 24.74, 41.91, 87.71];
const PIANO_CAL_T60L_F0: [f32; PIANO_CAL_N] = [39.52, 60.0, 60.0, 60.0, 60.0, 60.0, 60.0, 11.49, 8.984, 19.13, 38.77, 60.0, 28.45, 60.0, 42.46, 15.57, 47.02, 45.61, 5.51, 3.228, 2.58, 2.449, 2.097, 1.741];
const PIANO_CAL_T60L_HI: [f32; PIANO_CAL_N] = [28.5, 52.08, 32.81, 40.24, 60.0, 60.0, 39.4, 11.49, 8.984, 19.13, 34.68, 15.51, 12.28, 36.44, 12.34, 7.038, 6.74, 12.52, 5.51, 3.228, 2.58, 2.449, 2.097, 1.741];
const PIANO_CAL_T60E_F0: [f32; PIANO_CAL_N] = [19.35, 7.207, 5.585, 7.093, 3.821, 5.992, 6.7, 7.784, 6.329, 4.204, 2.889, 4.253, 13.14, 1.353, 2.884, 4.069, 0.7914, 0.3542, 0.2239, 0.3303, 0.5497, 0.3866, 0.8122, 0.4071];
const PIANO_CAL_T60E_HI: [f32; PIANO_CAL_N] = [9.049, 29.73, 30.07, 22.92, 21.01, 19.1, 9.818, 8.403, 8.984, 9.839, 1.646, 0.2288, 0.1599, 0.3538, 0.5149, 0.5277, 0.6741, 1.349, 0.6923, 0.6916, 0.7157, 0.5484, 0.7539, 0.4749];
const PIANO_CAL_DIVE_DBPS: [f32; PIANO_CAL_N] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.45, 0.0, 45.15, 5.0, 0.0, 0.0, 0.0, 23.2, 0.0, 0.0, 0.0, 5.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.95, 0.0];
const PIANO_CAL_TRIM_DB: [f32; PIANO_CAL_N] = [4.41, 4.802, 4.325, 2.59, 8.59, 12.09, -0.1158, 8.565, 11.67, 13.79, 14.57, 13.61, 18.0, 9.922, 18.0, 18.0, 18.0, 17.01, 18.0, 13.61, 6.0, 2.24, 7.43, 1.49];
const PIANO_CAL_K_VEL: [f32; PIANO_CAL_N] = [-0.3805, 0.2497, 0.1766, -0.3688, -0.5473, -0.2073, -0.3047, 0.2313, 0.03717, 0.2669, 0.05652, -0.7196, 0.8308, -0.5829, 2.0, 1.619, 1.509, -0.9309, 1.64, 1.282, 0.4137, -0.7881, -1.2, -1.034];
const PIANO_CAL_CONTACT_MS: [f32; PIANO_CAL_N] = [0.4374, 0.8388, 0.957, 0.619, 0.7792, 1.051, 0.6433, 2.017, 0.9766, 1.191, 0.9312, 0.9539, 1.544, 0.5713, 1.366, 1.43, 1.083, 1.08, 0.9067, 0.7797, 0.7335, 0.8551, 0.5243, 0.9492];
const PIANO_CAL_DETUNE_C: [f32; PIANO_CAL_N] = [0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.7935, 0.987, 1.18, 1.306, 1.306, 1.055, 1.005, 0.7511, 0.7511, 0.5617, 1.007, 1.696, 1.696, 1.198, 1.151, 1.151, 2.568];
const PIANO_CAL_PLATEAU_DB: [f32; PIANO_CAL_N] = [-10.35, -3.875, 0.0, -7.05, -22.93, -19.29, -19.08, -12.2, -4.975, -12.8, -19.0, -17.94, -18.42, -15.9, -18.02, -17.63, -17.43, 0.0, -13.0, -2.175, 0.0, 0.0, -4.9, -2.0];
const PIANO_CAL_STRIKE_Q: [f32; PIANO_CAL_N] = [0.12, 0.12, 0.12, 0.12, 0.124, 0.12, 0.124, 0.12, 0.12, 0.12, 0.12, 0.12, 0.12, 0.098, 0.098, 0.1033, 0.1033, 0.09917, 0.09083, 0.08667, 0.0825, 0.07833, 0.07417, 0.07];

// ---------------------------------------------------------------------------
// P3 string nonlinearity (2026-07-12, Keunwoo: "non-linearity of the string's
// harmonics. boundary effect").
//
// A struck string at playing amplitude is not a linear resonator: the
// nonlinear tension term couples transverse motion to the LONGITUDINAL
// direction with a force quadratic in the transverse slope, and the bridge
// (the boundary) transduces that longitudinal force into sound — phantom
// partials at sums of transverse partial pairs f_i+f_j (Conklin, JASA 1999;
// Bank & Sujbert, JASA 2005). Measured on the Salamander ff layers, the
// phantom energy is not a flat quadratic spray: it CONCENTRATES at a per-key
// formant — the longitudinal mode region — whose center rises ~1.5 kHz (deep
// bass) → ~5.8 kHz (A4) while falling from ~26·f0 to ~13·f0 in ratio terms
// (the c_L/c_T signature). The table below holds the measured dominant
// phantom-cluster frequency per anchor key (scratchpad piano-p3/phan.py,
// dominant f_i+f_j clusters on v16 layers); the voice runs one second-order
// resonator there, driven by the squared transverse bridge signal, plus a
// low-weight broadband forced-response floor.
//
// Measured NON-findings, kept out by evidence (piano-p3 baseline):
//  - tension-modulation glide: Salamander p2/p3 early-vs-late tracks read
//    ≤ ~2.5 cents at every fit key and B/f0 are velocity-independent
//    (ΔB ≤ 3%, Δf0 ≤ 1 cent pp→ff) — the guitar-style per-voice tension
//    glide is NOT warranted on the piano.
//  - Fletcher-1964 clamped-boundary inharmonicity correction: exact
//    clamped-clamped eigenvalues (numeric characteristic-equation solve)
//    refit against the measured A5–C7 partials shrink the hinged-law
//    residuals by only −2…+3% (noise): the correction is absorbed by the
//    (f0, B) refit at measurable n. No engine cost.
const PIANO_LONG_N: usize = 10;
const PIANO_LONG_MIDI: [f32; PIANO_LONG_N] =
    [24.0, 30.0, 33.0, 36.0, 42.0, 48.0, 54.0, 60.0, 63.0, 69.0];
const PIANO_LONG_HZ: [f32; PIANO_LONG_N] =
    [1522.0, 1204.0, 1435.0, 1640.0, 2227.0, 2491.0, 3300.0, 3422.0, 4077.0, 5799.0];
/// Per-key phantom-level trim (dB) on the WHOLE phantom path (formant
/// resonator + broadband floor), fitted against the refs' formant-projection
/// table (iterated; bounded — C3's + window compensates a LINEAR parent
/// deficit around 2.5 kHz that is P1/P2 territory, logged in the P3 report).
const PIANO_LONG_DB: [f32; PIANO_LONG_N] =
    [-4.0, -3.0, 5.0, 14.0, 14.0, 4.0, -2.0, 1.0, -6.0, -16.0];

/// Anchor interpolation over an arbitrary (midi, value) table (clamped ends).
fn piano_anchor_interp(midi: f32, xs: &[f32], ys: &[f32]) -> f32 {
    if midi <= xs[0] {
        return ys[0];
    }
    for i in 0..xs.len() - 1 {
        if midi <= xs[i + 1] {
            let t = (midi - xs[i]) / (xs[i + 1] - xs[i]);
            return ys[i] + t * (ys[i + 1] - ys[i]);
        }
    }
    ys[ys.len() - 1]
}

/// Linear interpolation over the calibration anchors (clamped at the ends).
fn piano_cal(midi: f32, table: &[f32; PIANO_CAL_N]) -> f32 {
    if midi <= PIANO_CAL_MIDI[0] {
        return table[0];
    }
    for i in 0..PIANO_CAL_N - 1 {
        if midi <= PIANO_CAL_MIDI[i + 1] {
            let t = (midi - PIANO_CAL_MIDI[i]) / (PIANO_CAL_MIDI[i + 1] - PIANO_CAL_MIDI[i]);
            return table[i] + t * (table[i + 1] - table[i]);
        }
    }
    table[PIANO_CAL_N - 1]
}

/// Solve the piano stiffness-dispersion cascade for per-key inharmonicity `b`
/// at `f0`: M identical first-order allpasses with pole at +p (coefficient
/// −p), knee pinned at/above the anchor partial n* so the phase delay falls
/// ~quadratically through the matched range — the stiff-string law
/// P(f_n) = N₀/√(1+Bn²) (Van Duyne & Smith 1994; Rauhala & Välimäki 2006;
/// same construction as the guitar's `design_dispersion`, with the piano
/// loop's own filters divided out). Runs at note-on only.
fn design_piano_dispersion(b: f32, f0: f32, sr: f32, lp_c: f32) -> (usize, f32) {
    if b < 1e-7 {
        return (0, 0.0);
    }
    let n0 = sr / f0;
    let w1 = core::f32::consts::TAU * f0 / sr;
    // anchor partial: highest of the matched stretch range — the 48 kHz
    // Salamander refs measure clean partials to ~20 kHz, and treble B is so
    // large that anchoring at a 4.8 kHz ceiling left A5's p10 −54 c and its
    // p18 −270 c flat (iteration-1 measurement). Cap by Nyquist headroom and
    // 20 partials (≈ the audible/measurable stretch range).
    let n_star = (0.40 * sr / f0).clamp(3.0, 20.0).floor();
    let wn = (n_star * w1).min(2.8);
    // geometric phase-delay deficit between p1 and n*, minus what the loop
    // lowpass already contributes, plus what the DC blocker's falling lead
    // takes away (it is a negative-dispersion element — r3 finding)
    let d_geom = n0 * (1.0 / (1.0 + b).sqrt() - 1.0 / (1.0 + b * n_star * n_star).sqrt());
    let d_lp = piano_lp_delay(lp_c, w1) - piano_lp_delay(lp_c, wn);
    let d_dc = -piano_dc_lead(w1) + piano_dc_lead(wn);
    let target = d_geom - d_lp - d_dc;
    if target <= 0.05 {
        return (0, 0.0);
    }
    // Anchor the fit at the MIDPOINT partial (n_mid ≈ 0.6 n*): a matched
    // midpoint keeps the audible n≤10 range within a few cents while the
    // endpoint carries whatever deficit the budget forces (measured: the
    // endpoint-anchored fit bowed C4's n8–n14 +4…+8 c sharp).
    let n_mid = (0.6 * n_star).ceil().clamp(2.0, n_star);
    let wm = (n_mid * w1).min(2.8);
    let d_geom_mid = n0 * (1.0 / (1.0 + b).sqrt() - 1.0 / (1.0 + b * n_mid * n_mid).sqrt());
    let d_lp_mid = piano_lp_delay(lp_c, w1) - piano_lp_delay(lp_c, wm);
    let d_dc_mid = -piano_dc_lead(w1) + piano_dc_lead(wm);
    let target_mid = (d_geom_mid - d_lp_mid - d_dc_mid).max(0.01);
    // Direct M-sweep with a hard loop-budget guard: the cascade's phase-delay
    // compensation at ω1 must leave at least 55% of the loop as real delay
    // line. (The earlier knee-pinned-p heuristic chose p = 0.124 at C7 whose
    // per-stage contrast is only 0.22 samples — M exploded to the cap and the
    // 25.6-sample compensation exceeded the 22.8-sample loop: the note played
    // 380 cents flat. Short treble loops need FEW stages with LARGE p.)
    let budget = 0.45 * n0;
    let mut best: (usize, f32, f32) = (0, 0.0, -1.0); // (m, p, endpoint delivery)
    for m in 1..=PIANO_MAX_DISP {
        let mf = m as f32;
        // bisect p for the midpoint target (contrast is monotone in p)
        let (mut lo, mut hi) = (0.003f32, 0.92f32);
        for _ in 0..32 {
            let p = 0.5 * (lo + hi);
            let c = mf * (allpass_delay(-p, w1) - allpass_delay(-p, wm));
            if c < target_mid {
                lo = p;
            } else {
                hi = p;
            }
        }
        let p = 0.5 * (lo + hi);
        if mf * allpass_delay(-p, w1) > budget {
            continue; // this M cannot match the midpoint within the loop budget
        }
        // among budget-feasible fits, prefer the one delivering the most of
        // the ENDPOINT stretch (more stages at smaller p = wider quadratic
        // range), stopping early once the endpoint is essentially met
        let e = mf * (allpass_delay(-p, w1) - allpass_delay(-p, wn));
        if e > best.2 {
            best = (m, p, e);
        }
        if e >= 0.98 * target {
            break;
        }
    }
    if best.0 == 0 {
        return (0, 0.0);
    }
    (best.0, best.1)
}

/// Solve (per-round-trip loss, loop lp_c) so the string loop realizes the
/// per-key t60 targets at f0 and at f_hi (the same two-point parameterization
/// the reference fitter uses — physical seconds in the tables, coefficients
/// solved here per sample rate). Loop gain g(ω) = loss·|LP(ω)|·|DC(ω)|;
/// t60(ω_n) = −6.9077·period/(sr·ln g(ω_n)).
fn solve_piano_loss(f: f32, sr: f32, t60_f0: f32, t60_hi: f32, f_hi: f32) -> (f32, f32) {
    let w1 = core::f32::consts::TAU * f / sr;
    let g1 = (-6.907_755 / (f * t60_f0.max(0.05))).exp();
    // A flat ladder needs a UNITY wire (lp_c = 1 ⇒ the one-pole passes
    // through), not a "nearly-1" coefficient: at treble f0 even |LP| = 0.995
    // per trip is ~40 dB/s of phantom damping (0.5% × f0 trips/s) — measured
    // on iteration 1 as late p2–6 dying at −30 dB/s where the refs say −9.
    if f_hi <= f * 1.05 || t60_hi >= t60_f0 * 0.995 {
        let m1 = piano_dc_mag(w1).max(1e-3);
        return ((g1 / m1).min(0.999_95), 1.0);
    }
    let wh = (core::f32::consts::TAU * f_hi / sr).min(2.9);
    let gh = (-6.907_755 / (f * t60_hi.max(0.05))).exp();
    let r_target = (gh / g1).min(1.0);
    // R(c) = [|LP(c,ωh)|·|DC(ωh)|] / [|LP(c,ω1)|·|DC(ω1)|] is monotone
    // increasing in c (brighter filter → flatter ladder); bisect up to the
    // unity wire
    let (mut lo, mut hi) = (0.05f32, 1.0f32);
    for _ in 0..30 {
        let c = 0.5 * (lo + hi);
        let r = (piano_lp_mag(c, wh) * piano_dc_mag(wh))
            / (piano_lp_mag(c, w1) * piano_dc_mag(w1)).max(1e-6);
        if r < r_target {
            lo = c;
        } else {
            hi = c;
        }
    }
    let lp_c = 0.5 * (lo + hi);
    let m1 = (piano_lp_mag(lp_c, w1) * piano_dc_mag(w1)).max(1e-3);
    ((g1 / m1).min(0.999_95), lp_c)
}

/// Per-voice soundboard-knock mode count (the init-synthesized "mode cloud").
const PIANO_BOARD_MODES: usize = 12;

#[derive(Clone, Copy)]
pub struct PianoVoice {
    strings: [StringLoop; 3],
    strike_off: [usize; 3],
    // radiated mix per string: prompt-dominant at onset, aftersound plateau
    // ~7 dB below peak once the prompt dumps (see start() Weinreich note)
    out_w: [f32; 3],
    // Bridge coupling (round 3, Weinreich 1977): every string's loop write
    // becomes wᵢ − g0·Σⱼwⱼ − g1·LP(Σⱼwⱼ), i.e. the reflection matrix
    // R(ω) = I − G(ω)·J with G = g0 + g1·H(ω). R is symmetric with
    // eigenvalues {1 − N·G(ω), 1, 1}:
    // the IN-PHASE string motion (exactly what the hammer excites) loses
    // energy through the bridge every reflection, anti-phase motion loses none
    // — so the prompt sound and the singing aftersound EMERGE from unison
    // detuning rotating the state out of the fast subspace, per partial,
    // instead of being painted with two hand-fit t60s.
    //
    // H(ω) is a one-pole lowpass: the bridge admittance is largest around the
    // board's low modes and falls ~1/f above (Giordano 1998 mobility), which
    // is what Salamander's per-partial rates demand — C2's fundamental dives
    // at −52 dB/s while its p2 does −20 (C3: −58 vs −6; C4: −24 vs −20; the
    // contrast collapses as f0 climbs past the mobility peak).
    //
    // Energy-passivity: the real term needs N·g0 ≤ 2; for the LP term,
    // Re(1/H(ω)) ≥ 1 at every ω, so |1 − N·G(ω)| ≤ 1 whenever N·(g0+g1) ≤ 2 —
    // per-loop losses then contract strictly. N·(g0+g1) ≤ 0.5 is enforced by
    // test, and g1 additionally carries a phase-pull (detune) budget — see
    // start().
    bridge_g0: f32,
    bridge_g1: f32,
    bridge_lp: f32,
    bridge_c: f32,
    n_strings: usize,
    level: f32,
    // felt hammer state (nonlinear collision — Bank/Välimäki lineage).
    // Units: string-displacement units; per-sample integration, hammer mass = 1.
    h_x: f32,
    h_v: f32,
    h_k: f32,
    h_p: f32,
    h_gain: f32,
    h_active: bool,
    // Stulov hysteretic felt: F = K·(cᵖ + τ0·d(cᵖ)/dt) — loading is stiffer than
    // unloading (Stulov, JASA 1995; Hall & Askenfelt), steepening the pulse front.
    // A symmetric half-sine pulse left the attack ~12 dB short around partial 5.
    h_tau: f32,
    h_cp1: f32,
    // soundboard knock: a dense modal CLOUD (init-time synthesized board
    // response), not 3 lonely case modes — see PIANO_BOARD_MODES
    body_a1: [f32; PIANO_BOARD_MODES],
    body_r2: [f32; PIANO_BOARD_MODES],
    body_y1: [f32; PIANO_BOARD_MODES],
    body_y2: [f32; PIANO_BOARD_MODES],
    body_g: [f32; PIANO_BOARD_MODES],
    body_pulse_pos: u32,
    body_pulse_len: u32,
    /// board cloud active until this age (longest mode T60 0.55 s → the bank
    /// is silence long before a 10+ s note dies; skip its 12 modes after)
    body_live: u64,
    thump_env: f32,
    thump_decay: f32,
    thump_amp: f32,
    // noise coloring: 1.0 = white (attack key thump); dropped at release so
    // the damper-felt landing reads dark (one-pole LP on the noise source)
    noise_lp: f32,
    noise_lp_c: f32,
    // soundboard radiation buildup: the board is a driven resonant radiator whose
    // low-frequency output rises over several string periods (Suzuki, JASA 1986
    // soundboard mobility; driven-resonator transient). NSynth refs peak 5–9
    // periods after onset (G1 ≈ 160 ms, C3 ≈ 40 ms, C5 ≈ 20 ms) — the strings'
    // radiated sum gets a 1−e^(−t/τ) rise, τ ≈ 2.5 periods; knock/thump bypass.
    bloom: f32,
    bloom_c: f32,
    // radiation highpass, 2nd order at max(0.35·f0, 88 Hz): the board cannot
    // radiate below its first modes REGARDLESS of the string's pitch — the
    // deep-bass refs radiate p2 ABOVE p1 (G1-ff: p1 −13.6 dB rel p2) where an
    // f0-tracked cutoff passed our p1 untouched. Also drains the hammer's
    // unipolar subsonic pedestal (round-1 fix, kept).
    rad_c: f32,
    rad_lp: f32,
    rad_lp2: f32,
    // phantom partials: tension modulation pumps the string's LONGITUDINAL
    // direction with force ∝ (∂y/∂x)² — quadratic in the transverse motion —
    // radiating "phantom" partials at SUM frequencies of transverse partial
    // pairs (Conklin, JASA 1999; Bank & Sujbert, JASA 2005). They are what makes
    // an ff bass note snarl (round-1 gap: G1-ff attack centroid 113 vs ref 261,
    // ref partials p10–p23 strong where the render had nothing). Model: square
    // the prompt string, highpass it 2nd-order at 6·f0 (the quadratic's DC +
    // difference terms land back ON the low partials — measured +6 dB at
    // 250–500 Hz with a 1.8·f0 one-pole; two poles at 6·f0 give −40 dB at 2·f0
    // vs −5 dB at 10·f0), key-tracked gain that dies above key≈0.55.
    // Band-limited by construction: bass string content ≤~3 kHz doubles to
    // ≤6 kHz ≪ Nyquist; the tap is OFF where that argument would weaken.
    // Amplitude² scaling gives the forte-prominence for free.
    ph_gain: f32,
    ph_c: f32,
    ph_lp1: f32,
    ph_lp2: f32,
    // P3 longitudinal-mode resonator (see PIANO_LONG_* block): second-order
    // resonator at the per-key measured phantom-formant frequency, driven by
    // the squared transverse bridge signal — the free/forced longitudinal
    // response the bridge transduces (Bank & Sujbert 2005). Exact-peak
    // normalized at design time so both deploy sample rates radiate the same
    // formant level. Band-limited by construction: the resonator is
    // narrowband and its center is capped ≤ 0.35·sr.
    lg_a1: f32,
    lg_r2: f32,
    lg_g: f32,
    lg_y1: f32,
    lg_y2: f32,
    // broadband forced-response floor: relative weight + a bounding one-pole
    // lowpass (~7 kHz) — squaring doubles bandwidth, the LP owns the top end
    ph_fw: f32,
    ph_c3: f32,
    ph_lp3: f32,
    /// inverse drive-saturation scale (1/D); 0 = no saturation
    ph_isat: f32,
    // air/radiation rolloff: fixed one-pole LP ~10 kHz. The 44.1 kHz VSCO
    // check caught the render +17 dB above 8 kHz (NSynth's 16 kHz refs are
    // blind there): board directivity + air absorption kill the top octave.
    air_c: f32,
    air_lp: f32,
    // Board antiresonance dip (round 3): a FIXED ~270 Hz, −13 dB, Q≈1.25
    // peaking dip on the string-radiation path. The Salamander attack windows
    // show p1/p2 tilts no f0-tracked filter can make (C2 −17.6, C3 +8.1,
    // F#3 −5.3, C4 −3.1, C5 +21 dB): one fixed dip at ~270 Hz reproduces five
    // keys at once (it catches C4's p1, C3's p2 and C2's p4 — all ≈262 Hz —
    // while sparing C3's p1 and C5's p1). Physically: a driving-point mobility
    // antiresonance between low board modes (Giordano, JASA 1998: measured
    // bridge mobility valleys through 200–400 Hz). Without it the render's C4
    // fundamental sat +13 dB hot vs p2 → centroid 283 vs the ref's 480.
    // RBJ peaking biquad, transposed DF2 state.
    br_b0: f32,
    br_b1: f32,
    br_b2: f32,
    br_a1: f32,
    br_a2: f32,
    br_z1: f32,
    br_z2: f32,
    rng: Lcg,
    sr: f32,
    key: f32,
    vel: f32,
    life: u64,
    age: u64,
}

impl PianoVoice {
    pub fn start(midi: u32, f0_et: f32, vel: f32, sr: f32, seed: u32) -> Self {
        let key = ((midi as f32) - 21.0) / 87.0; // 0 = A0 … 1 = C8
        let mkey = midi as f32;
        // ---- P1 per-key calibration (see PIANO_CAL_* tables) ----
        // Railsback stretch: the loop is tuned so partial 1 lands on the
        // MEASURED per-key offset (−16 c at A0 … +88 c at C8), not equal
        // temperament — aurally-tuned octaves align partials, and the
        // stretch is as much a part of "sounding like a piano" as B itself.
        let tune_c = piano_cal(mkey, &PIANO_CAL_TUNE_C);
        let f0 = f0_et * (tune_c / 1200.0).exp2();
        let b = 10.0f32.powf(piano_cal(mkey, &PIANO_CAL_LOG10_B));
        let t60 = piano_cal(mkey, &PIANO_CAL_T60L_F0);
        let t60_hi = piano_cal(mkey, &PIANO_CAL_T60L_HI).min(t60);
        let t60_prompt = piano_cal(mkey, &PIANO_CAL_T60E_F0).min(t60);
        let dive_extra_dbps = piano_cal(mkey, &PIANO_CAL_DIVE_DBPS).max(0.0);
        let trim_db = piano_cal(mkey, &PIANO_CAL_TRIM_DB);
        let k_vel = piano_cal(mkey, &PIANO_CAL_K_VEL);
        let contact_ms = piano_cal(mkey, &PIANO_CAL_CONTACT_MS).max(0.05);
        let detune_spread = piano_cal(mkey, &PIANO_CAL_DETUNE_C).max(0.05);
        let strike_q = piano_cal(mkey, &PIANO_CAL_STRIKE_Q).clamp(0.03, 0.3);
        // Aftersound t60: with true bridge coupling the per-loop loss states
        // only the string's internal + air damping — the anti-phase modes
        // decay at exactly this rate. P1: solved per note from the per-key
        // MEASURED late ladder (t60 at f0 + at min(8f0, 4 kHz)), replacing the
        // r2/r3 anchor laws. The two-point solve fixes the r3 imbalance where
        // the singing range's mid partials died ~2× fast while p1 lingered.
        let f_hi = (8.0 * f0).min(4000.0);
        let (loss, lp_c) = solve_piano_loss(f0, sr, t60, t60_hi, f_hi);
        // Stiffness (inharmonicity): per-note-solved allpass cascade from the
        // per-key measured B (see design_piano_dispersion). Replaces the r3
        // two-stage fixed cascade + tau0 law, which measured 4–6× too flat
        // through the bass/mid keyboard (C4 p10 +4.4 c vs Salamander's +24.6).
        let (disp_n, disp_p) = design_piano_dispersion(b, f0, sr, lp_c);
        let disp_a = -disp_p;

        let n_strings = if midi < 32 { 2 } else { 3 };
        // Unison mistuning (per-key table): sets the RATE at which the coupled
        // state rotates out of the fast (in-phase) subspace. Measured from the
        // references' tail-envelope beat lines (~1.0–1.3 c mid, ~2.5 c top;
        // wound-string false beats gated out — those are winding nonuniformity,
        // not unison detune).
        // Weinreich 1977 roles, EMERGENT: all strings carry the same internal
        // t60; the bridge coupling (bridge_g below) drains the in-phase
        // component fast (prompt) while the detuned pair's anti-phase remnant
        // sings on (aftersound). P1: all strings share the SOLVED loop filter
        // — velocity→brightness now lives entirely in the felt-hammer law
        // (where the physics puts it), not in a velocity-voiced loop filter.
        let cfg: [f32; 3] = [0.0, detune_spread, -0.8 * detune_spread];
        // TWO coupling terms (see bridge_g field docs).
        // g0 — broadband, purely real: no phase, no detune; owns the composite
        // early envelope. Extra decay rate ≈ 8.686·N·g0·f0 dB/s at every
        // partial. P1: prompt t60 target is the per-key measured early ladder.
        // Deep-bass gap cap (P1): with a single broadband g0, the ff prompt
        // rate and the singing-tail rate cannot both be honored below ~C2 —
        // unison rotation keeps leaking the aftersound back into the bridge
        // drain, so a gap sized for the measured −8…−9 dB/s prompt leaves the
        // tail 2–3× overdamped vs Salamander. The tail is the perceptually
        // dominant axis for held bass notes; cap the gap at 5 dB/s below C2,
        // tapering out by C3 (P2's frequency-shaped admittance owns doing
        // both).
        let kb = (key / 0.31).min(1.0);
        let gap_cap = 3.0 + 60.0 * kb * kb;
        let rate_gap = (60.0 / t60_prompt - 60.0 / t60).max(0.0).min(gap_cap);
        let bridge_g0 = rate_gap / (8.686 * n_strings as f32 * f0);
        // g1 — through the admittance lowpass: gives the FUNDAMENTAL its extra
        // dive while upper partials keep ~the broadband rate. P1: the dive is
        // a per-key MEASURED quantity (strong only around C3–G3 — C3 p1 dives
        // at −52 dB/s vs −9 for its mids; zero across most of the keyboard),
        // not a broad bass Gaussian. The lowpass phase LAG pulls the fast mode
        // SHARP by ≈ N·g1·Im H(f0) rad/round-trip (Weinreich's "twang", §VI),
        // so g1 stays capped at 0.04 rad — the pull applies only to the fast
        // mode; the aftersound that carries perceived pitch is untouched.
        let bridge_fc = (0.25 * f0).max(40.0);
        let xb = f0 / bridge_fc;
        let h_re = 1.0 / (1.0 + xb * xb);
        let h_mag = h_re.sqrt();
        let h_im = xb * h_re;
        // twang cap 0.02 rad (halved from r3's 0.04): the C3 prompt measured
        // +10 c sharp vs the reference at the old cap — half the dive depth
        // where the cap binds is the better trade (P2 owns a real admittance
        // mechanism for the dive).
        let bridge_g1 = (dive_extra_dbps / (8.686 * n_strings as f32 * f0 * h_mag))
            .min(0.02 / (n_strings as f32 * h_im));
        let rng = Lcg(seed | 1);
        // Prompt string (the struck vertical polarization) gets its OWN loop
        // filter solved from the measured early HI-band t60 (its high
        // partials die 3–8× faster than the aftersound ladder — without this
        // the render's bass p10–12 sustain ran 8× hot relative to p2–4 vs
        // Salamander). At f0 it keeps the shared internal t60, so the
        // fundamental's aftersound is untouched. This is r3's bright-prompt/
        // transparent-pair Weinreich split, now per-key calibrated.
        let t60e_hi = piano_cal(mkey, &PIANO_CAL_T60E_HI).clamp(0.1, t60);
        let (loss0, lp_c0) = solve_piano_loss(f0, sr, t60, t60e_hi.min(t60 * 0.99), f_hi);
        let mut strings = [StringLoop::new(f0, 0.0, sr, loss0, lp_c0, disp_n, disp_a); 3];
        let mut strike_off = [0usize; 3];
        for (i, s) in strings.iter_mut().enumerate().take(n_strings) {
            let (ls, c) = if i == 0 { (loss0, lp_c0) } else { (loss, lp_c) };
            *s = StringLoop::new(f0, cfg[i], sr, ls, c, disp_n, disp_a);
            strike_off[i] = ((strike_q * s.len as f32) as usize).clamp(1, s.len - 1);
        }

        // Hammer-string collision (the anti-harpsichord): the string starts at REST
        // and a felt hammer strikes it — force F = K·compression^p injected over the
        // contact. Contact time then EMERGES from velocity and register instead of
        // being painted onto a pre-filled pluck. Nondimensionalized: target a
        // register-scaled contact time at mezzo-forte, let the nonlinearity make
        // hard hits shorter/brighter and soft hits longer/darker.
        // Felt exponent: raised 2.3+0.7k → 2.7+0.4k (P1). Higher p = contact
        // time falls faster with velocity (τ ∝ v^((1−p)/(1+p)), Hertz-type):
        // pp strikes lengthen/darken relative to mf. Salamander's pp bass
        // attack carries relatively LESS p10–12 than the 2.3-felt rendered —
        // the F#1 v16/v4 phantom-band superlinearity read 1.33× vs the ref's
        // 1.68× purely from pp linear brightness. Stulov (JASA 1995) felt
        // fits span p ≈ 2.2–3.5.
        let h_p = 2.7 + 0.4 * key;
        let h_v0 = 0.010 + 0.115 * vel; // displacement units per sample
        // Stiffness normalized at a fixed REFERENCE velocity (mf): the felt law then
        // does its real job — harder hits compress more → shorter contact → brighter.
        // (Normalizing at the actual velocity pins contact time and kills the
        // velocity→timbre physics — measured mistake, see decision log.)
        // Contact target AT the reference velocity: per-key calibration table
        // (init = the r3 law, iterated against the references' attack-centroid
        // curves; real contacts run ~1.5 ms bass → 0.3–0.5 ms treble at ff —
        // Hall & Askenfelt).
        let v_ref = 0.010 + 0.115 * 0.6;
        let omega = core::f32::consts::PI / (contact_ms * 1e-3 * sr);
        let comp_ref = (v_ref / omega).max(1e-6);
        let h_k = omega * omega * comp_ref.powf(1.0 - h_p);

        // body knock: dense soundboard mode cloud (see below)
        let mut v = Self {
            strings,
            strike_off,
            // Weinreich fig. 4 balance: the hammer strike is vertical, so the
            // PROMPT polarization owns the onset; the aftersound plateau is
            // what remains once it dumps — refs hold that plateau ~7 dB under
            // the peak at 1 s (1.6/3.6 = −7.0 dB). Round 1 had this inverted
            // (pair carried 83% of onset), which made the composite early
            // decay read the pair's slow t60 at every key.
            // Aftersound plateau level, envelope-grid fit: the pair sits
            // −16 dB below the prompt at the bass break, −12 dB mid, −9 dB
            // treble (round 1 had the pair 5 dB ABOVE the prompt — the whole
            // note read as the pair's slow t60). The pair is deliberately
            // UNEQUAL (60/40): bridge coupling makes the unison normal modes
            // asymmetric, and equal weights (100% beat modulation) parked a
            // beat null in the 0.8–1.8 s window (C3 t60_late read 4.8 vs 12).
            out_w: {
                // Aftersound plateau level: per-key calibration table (P1),
                // fed back from the references' measured level-at-1 s per
                // partial band. Replaces the r3 three-segment law — at the
                // top of the keyboard the law's −2 dB plateau flattened the
                // composite early decay ~35 dB/s short of the references.
                let a_db = piano_cal(mkey, &PIANO_CAL_PLATEAU_DB);
                if n_strings == 2 {
                    let a = 1.8 * 10f32.powf(a_db / 20.0);
                    [1.8, a, 0.0]
                } else {
                    let a = 2.0 * 10f32.powf(a_db / 20.0);
                    // 72/28: the two slow normal modes must reach the output
                    // with clearly UNEQUAL amplitude, or their mode-locked
                    // ~0.15 Hz residual beat digs a −40 dB null into the tail
                    // (measured 4.75 s @ C4) that no real piano shows —
                    // Weinreich's measured unison modes are asymmetric mixes.
                    [2.0, 0.78 * a, 0.22 * a]
                }
            },
            bridge_g0,
            bridge_g1,
            bridge_lp: 0.0,
            bridge_c: 1.0 - (-core::f32::consts::TAU * bridge_fc / sr).exp(),
            n_strings,
            // Velocity→loudness curve, pinned at the vel-0.8 makeup calibration
            // point. P1: the per-key exponent K_VEL is calibrated against the
            // references' measured pp→ff LUFS span (14–33 dB, GROWING toward
            // the treble — the old NSynth-era "nearly flat" reading was an
            // artifact of level-normalized references; Salamander preserves
            // the natural layer gains). Per-key radiated-level trim is the
            // TRIM_DB calibration table (LUFS-matched per key, iterated).
            level: {
                2.4 / (n_strings as f32)
                    * (-k_vel * (vel - 0.8)).exp()
                    * 10f32.powf(trim_db / 20.0)
            },
            h_x: 0.0,
            h_v: h_v0,
            h_k,
            h_p,
            h_gain: 260.0, // force→displacement-wave coupling, tuned via piano-audition peaks
            h_active: true,
            h_tau: 1.5e-4 * sr,
            h_cp1: 0.0,
            body_a1: [0.0; PIANO_BOARD_MODES],
            body_r2: [0.0; PIANO_BOARD_MODES],
            body_y1: [0.0; PIANO_BOARD_MODES],
            body_y2: [0.0; PIANO_BOARD_MODES],
            body_g: [0.0; PIANO_BOARD_MODES],
            body_pulse_pos: 0,
            // hammer-bridge force pulse: ~3 ms on heavy bass hammers, ~1 ms in
            // the treble (Askenfelt & Jansson 1990 contact times) — the shorter
            // pulse is what lets the cloud's upper modes speak at all
            body_pulse_len: (((0.003 - 0.002 * key) * sr) as u32).max(2),
            body_live: (0.9 * sr) as u64,
            thump_env: 1.0,
            thump_decay: t60_gain(0.010, sr),
            // Key/action thump: roughly constant mechanical energy per stroke
            // while the string's radiated output falls toward the treble
            // (short low-mass strings) — so its RELATIVE level grows with key
            // (Askenfelt & Jansson 1990; Salamander C7's attack carries as
            // much 20–250 Hz thump as string tone, measured −18 dB in our
            // render before this). Colored DARK (≈800 Hz one-pole): the
            // reference thump is LF, the old white noise read as "click".
            // ∝ √vel: the mechanical key/action impact shrinks sub-linearly
            // with touch (Askenfelt & Jansson: touch noise is RELATIVELY
            // prominent at pp) — with the linear law the pp top-octave
            // renders were void where Salamander's C8 v1 is nearly pure
            // action noise.
            // 0.02 → 0.012 (−4.4 dB) per Keunwoo listening 2026-07-12: "the
            // noise/stomping part is too strong" — level trimmed, the fitted
            // per-key growth and √vel touch law kept. (The remaining bass
            // "stomp" is the 20–60 Hz radiation excess, a P2 soundboard item.)
            thump_amp: 0.012 * vel.max(0.0).sqrt() * (1.0 + 6.0 * key * key * key),
            noise_lp: 0.0,
            noise_lp_c: 0.10,
            bloom: 0.0,
            bloom_c: 1.0 - (-f0.max(50.0) / (2.5 * sr)).exp(),
            // r3: 0.35/80 → 0.4/100 — Salamander's bass fundamentals sit lower
            // still (A0 attack p1 −31 dB rel p3; C2 p1 −18 rel p2)
            rad_c: 1.0 - (-core::f32::consts::TAU * (0.35 * f0).max(88.0) / sr).exp(),
            rad_lp: 0.0,
            rad_lp2: 0.0,
            // gain: register-tapered (off above key≈0.62 — Salamander A4 ff
            // still holds its dominant phantom at −58 dB rel peak) with a
            // GENTLE velocity factor. P3 measurement: the discrete phantom
            // clusters grow ~6–16 dB mf→ff in the refs, which the s² source
            // provides by itself (parent amplitude squared); the old
            // 0.05+0.95v² gain curve DOUBLE-counted velocity (render grew
            // 31–42 dB mf→ff where Salamander says 16). The remaining linear
            // factor keeps pp phantoms soft (Conklin hears phantoms "at
            // forte"; refs' pp clusters sit near the floor).
            // Velocity factor (0.04+0.96v): the s² source scales the
            // rel-peak phantom axis ~linearly with in-loop amplitude
            // (≈ −6 dB mf→ff intrinsic); the refs' formant-projection table
            // demands ~30–45 dB pp→ff spread in the bass (pp phantoms are
            // essentially floor — Conklin hears phantoms "at forte") while
            // mf sits only ~3–6 dB under ff, which a LINEAR gain-in-velocity
            // with a small floor reproduces; v² double-counted (it2/it3
            // measurement, scratchpad piano-p3).
            ph_gain: {
                let reg = ((0.62 - key) / 0.62).clamp(0.0, 1.0);
                6.0 * reg * (0.04 + 0.96 * vel)
            },
            ph_c: 1.0 - (-core::f32::consts::TAU * (6.0 * f0).min(0.1 * sr) / sr).exp(),
            ph_lp1: 0.0,
            ph_lp2: 0.0,
            lg_a1: 0.0,
            lg_r2: 0.0,
            lg_g: 0.0,
            lg_y1: 0.0,
            lg_y2: 0.0,
            // 0.5 -> 0.35 (it10): the broadband floor polluted the C2/D#2
            // attack third on the K-weighted log-mel while the formant axis
            // read calibrated — refs keep the between-cluster floor lower.
            ph_fw: 0.35,
            ph_c3: 1.0 - (-core::f32::consts::TAU * (7_000.0f32).min(0.3 * sr) / sr).exp(),
            ph_lp3: 0.0,
            // Per-key drive-saturation scale, fitted to the refs' velocity
            // ladders (piano-p3): all wound-bass keys show a threshold-then-
            // saturate shape — near-floor at v1, a 25–35 dB jump by v4, then
            // 3–5 dB/layer (F#1 v7→v16 = +5.3 dB where the unsaturated
            // quadratic renders +12) — while mid keys stay sub-knee until ff
            // (C4 v7→v16 = +21.7 dB). Knee ≈ the v≈0.25 drive in the bass,
            // reached only at ff by C4.
            // Two-segment log-linear in key: ~450 through the wound bass
            // (key ≤ .19), falling fast to ~70 by A2 (.28) — its ladder reads
            // v7→v16 = +15 dB, only lightly saturated — then to ~22 by C4.
            ph_isat: {
                let l = if key < 0.28 {
                    2.75 - 0.90 * ((key - 0.21) / 0.07).clamp(0.0, 1.0)
                } else {
                    1.85 - 0.50 * ((key - 0.28) / 0.17).clamp(0.0, 1.0)
                };
                10f32.powf(l)
            },
            air_c: 1.0 - (-core::f32::consts::TAU * (10_000.0f32).min(0.4 * sr) / sr).exp(),
            air_lp: 0.0,
            br_b0: 1.0,
            br_b1: 0.0,
            br_b2: 0.0,
            br_a1: 0.0,
            br_a2: 0.0,
            br_z1: 0.0,
            br_z2: 0.0,
            rng,
            sr,
            key,
            vel,
            // cap: the long mid-register aftersound params would otherwise hold
            // voices ~36 s (pool exhaustion under pedal); inaudible past ~18 s
            life: (((t60 * 1.4 + 0.1).min(18.0)) * sr) as u64,
            age: 0,
        };
        // Knock/thump are a subtle PRECURSOR in real recordings (Askenfelt &
        // Jansson 1990), ~8 dB below the string plateau. Round 1 shipped 3 lonely
        // case modes (85/172/318) — audibly "a sine knock", not wood. Round 2:
        // a dense mode CLOUD, i.e. an init-time-synthesized soundboard impulse
        // response. Physics: board modal spacing is ~25–40 Hz low down and the
        // modal overlap passes 1 above ~1 kHz, where discrete modes blur into a
        // diffuse cloud (Suzuki JASA 1986; Giordano JASA 1998). Twelve modes on
        // a geometric ladder 88 Hz → 2.6 kHz with a seeded ±5% scatter (strike
        // position moves along the bridge, so every note meets a slightly
        // different board response), T60 0.55 s → 0.13 s (radiation damping
        // grows with f), amplitude bell centered ~300 Hz (the board's best
        // radiating band). The key-tracked hammer pulse (3 ms bass → 1 ms
        // treble) lowpasses the cloud naturally: bass knock stays dark, treble
        // knock speaks up to ~2 kHz.
        // board antiresonance dip coefficients (see field docs): RBJ peaking
        // EQ, fc 270 Hz, gain −13 dB, Q 1.25
        {
            let a = 10f32.powf(-13.0 / 40.0);
            let w0 = core::f32::consts::TAU * 270.0 / sr;
            let alpha = w0.sin() / (2.0 * 1.25);
            let a0 = 1.0 + alpha / a;
            v.br_b0 = (1.0 + alpha * a) / a0;
            v.br_b1 = -2.0 * w0.cos() / a0;
            v.br_b2 = (1.0 - alpha * a) / a0;
            v.br_a1 = -2.0 * w0.cos() / a0;
            v.br_a2 = (1.0 - alpha / a) / a0;
        }
        // P3 longitudinal resonator design (see PIANO_LONG_* block): center at
        // the per-key measured phantom formant, Q ≈ 12 (cluster width read off
        // the refs' dominant-vs-neighbor phantom levels: F#1 14 dB down at
        // −17% detune, C2 11 dB at −12% → Q ~ 12–16; longitudinal modes are
        // heavily damped, the resonator is a formant, not a ringer). Gain is
        // exact-peak-normalized: |D(e^{jw0})| = (1−r)·√(1−2r·cos2w0+r²).
        if v.ph_gain > 0.0 {
            let f_long = piano_anchor_interp(mkey, &PIANO_LONG_MIDI, &PIANO_LONG_HZ)
                .min(0.35 * sr);
            let trim = piano_anchor_interp(mkey, &PIANO_LONG_MIDI, &PIANO_LONG_DB);
            let wl = core::f32::consts::TAU * f_long / sr;
            let rl = (-core::f32::consts::PI * (f_long / 12.0) / sr).exp();
            let peak = (1.0 - rl) * (1.0 - 2.0 * rl * (2.0 * wl).cos() + rl * rl).sqrt();
            v.lg_a1 = 2.0 * rl * wl.cos();
            v.lg_r2 = rl * rl;
            v.lg_g = 2.0 * peak;
            v.ph_gain *= 10f32.powf(trim / 20.0);
        }
        let knock = 0.6 * vel * (1.0 - 0.55 * key);
        let mut jrng = Lcg(seed.wrapping_mul(0x9E37) | 1);
        let mut bf = 88.0f32;
        for i in 0..PIANO_BOARD_MODES {
            let f = bf * (1.0 + 0.05 * jrng.next());
            let bt = 0.55 * (88.0 / f).powf(0.4);
            let a_rel = if f < 300.0 { (f / 300.0).powf(0.3) } else { (300.0 / f).powf(0.55) };
            // Treble keys must not ring the LOW board modes: the hammer pulse
            // enters at the treble bridge, whose driving-point coupling to the
            // ~90–300 Hz modes is weak — Salamander's C6/C7 show NO low knock
            // line, while our 91 Hz mode was C6's strongest low-frequency
            // component (round-3 heterodyne). Quadratic fade below 0.3·f0.
            let reach = (f / (0.3 * f0)).min(1.0);
            let r = t60_gain(bt, sr);
            let w = core::f32::consts::TAU * f / sr;
            v.body_a1[i] = 2.0 * r * w.cos();
            v.body_r2[i] = r * r;
            v.body_g[i] = 0.079 * a_rel * (1.0 - r) * knock * reach * reach;
            bf *= 1.36;
        }
        v
    }

    pub fn render(&mut self, out: &mut [f32]) -> bool {
        let inv_pulse = 1.0 / self.body_pulse_len as f32;
        let inv_n = 1.0 / self.n_strings as f32;
        let body_on = self.age < self.body_live;
        for o in out.iter_mut() {
            // felt-hammer collision: F = K·compression^p while in contact, integrated
            // per sample (hammer mass 1). Ends when the hammer rebounds clear.
            if self.h_active {
                let mut y_s = 0.0;
                for (i, st) in self.strings.iter().enumerate().take(self.n_strings) {
                    y_s += st.read_at(self.strike_off[i]);
                }
                y_s *= inv_n;
                let comp = self.h_x - y_s;
                let cp = if comp > 0.0 { comp.powf(self.h_p) } else { 0.0 };
                // Stulov hysteresis (see field docs): boost the loading edge,
                // relax the unloading edge; force stays repulsive (≥ 0)
                let f = (self.h_k * (cp + self.h_tau * (cp - self.h_cp1))).max(0.0);
                self.h_cp1 = cp;
                self.h_v -= f;
                self.h_x += self.h_v;
                if f > 0.0 {
                    // Asymmetric strike (Weinreich 1977 §V): hammer-crown
                    // irregularities and string leveling mean the unisons never
                    // receive equal force — the imbalance injects the coupled
                    // system's SLOW (anti-phase) modes directly, a primary
                    // source of the aftersound plateau's level. Key-tapered:
                    // with the same asymmetry the treble plateau measured
                    // ~10 dB above Salamander (rotation leak also grows with
                    // f0), so the direct injection shrinks up the keyboard.
                    let aw = 1.0 - 0.55 * self.key;
                    let strike_w = [1.0 + 0.10 * aw, 1.0 - 0.04 * aw, 1.0 - 0.18 * aw];
                    let inj = f * self.h_gain * inv_n;
                    for i in 0..self.n_strings {
                        let off = self.strike_off[i];
                        self.strings[i].inject(off, inj * strike_w[i]);
                    }
                } else if self.h_v < 0.0 {
                    self.h_active = false; // hammer moving away, contact over
                }
            }
            let mut s = 0.0;
            let mut ph_src = 0.0;
            // bridge coupling (see bridge_g field docs): run every loop's
            // filters, then reflect the CONCURRENT bridge-incident values
            // through R = I − gJ before the loops write back
            let mut w = [0.0f32; 3];
            let mut w_sum = 0.0f32;
            for (i, st) in self.strings.iter_mut().enumerate().take(self.n_strings) {
                let (y, wi) = st.tick();
                w[i] = wi;
                w_sum += wi;
                if i == 0 {
                    ph_src += y;
                } else if i == 1 {
                    // aftersound at reduced weight: full weight left the ff-bass
                    // SUSTAINED centroid ~2× the ref (t03 295 vs 156) while the
                    // attack was right — the cross/after² sum-terms are what
                    // persist, so they get the trim
                    ph_src += 0.6 * y;
                }
                s += y * self.out_w[i];
            }
            // two-term bridge coupling (see bridge_g0 field docs): broadband
            // real term + admittance-lowpass term for the fundamental's dive
            self.bridge_lp += self.bridge_c * (w_sum - self.bridge_lp);
            let gsum = self.bridge_g0 * w_sum + self.bridge_g1 * self.bridge_lp;
            for (i, st) in self.strings.iter_mut().enumerate().take(self.n_strings) {
                st.commit(w[i] - gsum);
            }
            // phantom partials (see field docs): quadratic tension tap off the
            // prompt + ONE aftersound string. Squaring the full detuned pair
            // parked a 2·f0 beat chorus in the tail (measured lm_tail
            // 3.21→3.55); prompt-only phantoms died before the 0.25–0.75 s
            // window where the references still hold them (ref G1-ff keeps
            // 492/541 Hz at −13…−17 dB there). One aftersound string gives
            // sustained sum-terms with a single, slow cross-beat.
            if self.ph_gain > 0.0 {
                let s2 = ph_src * ph_src;
                self.ph_lp1 += self.ph_c * (s2 - self.ph_lp1);
                let h1 = s2 - self.ph_lp1;
                self.ph_lp2 += self.ph_c * (h1 - self.ph_lp2);
                let d0 = h1 - self.ph_lp2;
                // longitudinal drive saturation: the refs' phantom growth
                // SATURATES with amplitude (D#2 v4->v13 spread 6 dB vs the
                // unsaturated quadratic's 24; longitudinal displacement is
                // physically bounded). Rational soft-sat: quadratic regime
                // for small drive, compressed at deep-bass ff amplitudes.
                let drive = d0 / (1.0 + d0.abs() * self.ph_isat);
                // longitudinal-mode formant (P3, Bank & Sujbert): the bridge
                // transduces the string's longitudinal response — free ringing
                // near the mode plus forced phantoms shaped by its resonance
                let yl = self.lg_a1 * self.lg_y1 - self.lg_r2 * self.lg_y2 + self.lg_g * drive;
                self.lg_y2 = self.lg_y1;
                self.lg_y1 = yl;
                // broadband forced floor, top end bounded by the ~7 kHz pole
                self.ph_lp3 += self.ph_c3 * (drive - self.ph_lp3);
                s += self.ph_gain * (self.ph_fw * self.ph_lp3 + yl);
            }
            // air/radiation top-octave rolloff (see field docs)
            self.air_lp += self.air_c * (s - self.air_lp);
            s = self.air_lp;
            // soundboard radiation buildup (see field docs): strings bloom in,
            // the percussive knock/thump below stay immediate
            self.bloom += self.bloom_c * (1.0 - self.bloom);
            s *= self.bloom;
            // radiation highpass (see field docs): two cascaded one-poles
            self.rad_lp += self.rad_c * (s - self.rad_lp);
            s -= self.rad_lp;
            self.rad_lp2 += self.rad_c * (s - self.rad_lp2);
            s -= self.rad_lp2;
            // board antiresonance dip (see field docs), transposed DF2
            let br_y = self.br_b0 * s + self.br_z1;
            self.br_z1 = self.br_b1 * s - self.br_a1 * br_y + self.br_z2;
            self.br_z2 = self.br_b2 * s - self.br_a2 * br_y;
            s = br_y;
            // hammer pulse into the body modes (case knock) + key thump noise
            if body_on {
                let mut x = 0.0;
                if self.body_pulse_pos < self.body_pulse_len {
                    let ph = self.body_pulse_pos as f32 * inv_pulse;
                    x = 0.5 * (1.0 - (core::f32::consts::TAU * ph).cos());
                    self.body_pulse_pos += 1;
                }
                for m in 0..PIANO_BOARD_MODES {
                    let y = self.body_a1[m] * self.body_y1[m] - self.body_r2[m] * self.body_y2[m]
                        + self.body_g[m] * x;
                    self.body_y2[m] = self.body_y1[m];
                    self.body_y1[m] = y;
                    s += y;
                }
            }
            if self.thump_amp > 1e-5 && self.thump_env > 1e-4 {
                let nw = self.rng.next();
                self.noise_lp += self.noise_lp_c * (nw - self.noise_lp);
                s += self.thump_amp * self.thump_env * self.noise_lp;
                self.thump_env *= self.thump_decay;
            }
            *o += s * self.level;
        }
        for st in self.strings.iter_mut().take(self.n_strings) {
            st.flush();
        }
        for m in 0..PIANO_BOARD_MODES {
            self.body_y1[m] = flush_denormal(self.body_y1[m]);
            self.body_y2[m] = flush_denormal(self.body_y2[m]);
        }
        self.bridge_lp = flush_denormal(self.bridge_lp);
        self.rad_lp = flush_denormal(self.rad_lp);
        self.rad_lp2 = flush_denormal(self.rad_lp2);
        self.air_lp = flush_denormal(self.air_lp);
        self.br_z1 = flush_denormal(self.br_z1);
        self.br_z2 = flush_denormal(self.br_z2);
        self.ph_lp1 = flush_denormal(self.ph_lp1);
        self.ph_lp2 = flush_denormal(self.ph_lp2);
        self.ph_lp3 = flush_denormal(self.ph_lp3);
        self.lg_y1 = flush_denormal(self.lg_y1);
        self.lg_y2 = flush_denormal(self.lg_y2);
        self.age += out.len() as u64;
        self.age < self.life
    }

    /// Damper falls: fast but not instant. Felt dampers grip thin treble strings
    /// almost immediately but take ~0.3 s to stop a heavy wound bass string, and
    /// the NSynth references keep decaying ~1 s past note-off (damper + soundboard
    /// ring + room). A hard stop at +0.25 s left the whole reference tail third
    /// compared against digital silence (measured −240 dB vs the ref's −27…−60 dB).
    pub fn damp(&mut self) {
        let key = self.key;
        // Dampers settle, they don't clamp: felt compresses for 100–300 ms
        // before the string stills (Askenfelt & Jansson 1990). Ref post-off
        // slopes read t60 ≈ 0.5 s bass → ~0.3 s treble.
        let t_damp = 0.50 - 0.30 * key;
        for st in self.strings.iter_mut().take(self.n_strings) {
            // per-pass (round-trip) loss, same bookkeeping as StringLoop::new
            st.loss = (-6.907_755 * st.len as f32 / (t_damp * self.sr)).exp();
        }
        // Damper-felt landing + key-action release (Askenfelt & Jansson 1990):
        // in the refs the release transient sits ~−28 dB below the note's peak
        // at f/ff (mostly masked by the still-ringing string) but reaches
        // −5 dB at pp — the mechanical thud doesn't shrink with a soft touch,
        // so its RELATIVE level explodes as velocity falls. (1−vel)³ base +
        // divide out the velocity-compensated voice level. Dark noise (felt,
        // not click) + a softer, longer re-knock of the board mode cloud.
        let soft = 1.0 - self.vel;
        let base = 0.0035 + 0.087 * soft * soft * soft;
        let rel = (base * (1.0 - 0.55 * key)) / self.level.max(1e-3);
        self.thump_amp = rel;
        self.thump_env = 1.0;
        self.thump_decay = t60_gain(0.09 - 0.05 * key, self.sr);
        self.noise_lp_c = 0.12; // ≈ 1 kHz one-pole: felt, not click
        // The body gate FROZE the mode states when it expired (longest board
        // T60 is 0.55 s — they are physically silent by now); clear them so the
        // re-knock doesn't resume a stale attack-time ring.
        if self.age >= self.body_live {
            self.body_y1 = [0.0; PIANO_BOARD_MODES];
            self.body_y2 = [0.0; PIANO_BOARD_MODES];
        }
        self.body_pulse_pos = 0;
        self.body_pulse_len = ((0.006 * self.sr) as u32).max(2);
        // Damper-landing re-knock is a MECHANICAL event (damper weight + key
        // return): its absolute level does not scale with how hard the note
        // was struck. body_g carries the attack's 0.6·vel factor — divide it
        // out (measured round 3: the old flat ×0.5 left the pp release knock
        // 11× too quiet relative to ff, hiding the Askenfelt pp prominence).
        let re_knock = 0.5 / self.vel.max(0.15);
        for g in self.body_g.iter_mut() {
            *g *= re_knock;
        }
        self.body_live = self.age + (0.9 * self.sr) as u64;
        self.life = self.age + (1.5 * self.sr) as u64;
    }
}

// ---------------------------------------------------------------------------
// ElectricVoice — the electric-guitar string (agent-tuned; split from PluckVoice at merge
// so the acoustic and electric reference-matched implementations stay byte-faithful)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct ElectricVoice {
    buf: [f32; PLUCK_BUF],
    len: usize,
    pos: usize,
    /// loop one-pole lowpass state + coefficient (brightness)
    lp: f32,
    lp_c: f32,
    /// per-sample loop loss
    loss: f32,
    /// fractional-delay allpass
    ap_c: f32,
    ap_x1: f32,
    ap_y1: f32,
    // Second string polarization (electrics only; pol2_on=false elsewhere and the
    // whole block is skipped). Weinreich 1977: the vertical polarization couples
    // strongly to the bridge and decays fast, the horizontal one rings on,
    // slightly detuned — the two-stage decay every real electric shows.
    pol2_on: bool,
    // magnetic pickup senses string VELOCITY, not displacement (Zollner ch.4):
    // first-difference differentiator, unity-normalized at f0. 0.0 = off.
    diff_g: f32,
    diff_x1: f32,
    buf2: [f32; PLUCK_BUF],
    len2: usize,
    pos2: usize,
    lp2: f32,
    loss2: f32,
    ap2_c: f32,
    ap2_x1: f32,
    ap2_y1: f32,
    // Amp supply-rail sag (pre-drive gain shaping, per-voice stand-in for the
    // shared rail): a tube amp's plate supply droops under signal current with
    // the rectifier/filter RC (~25 ms) and recovers slowly (~300 ms), so gain
    // follows the INVERSE of a slow signal envelope — hard onsets duck, and the
    // note BLOOMS as the string decays into the recovered rail. Modulation is
    // sub-audio-rate by construction (band-limited; no extra aliasing).
    sag_env: f32,
    sag_a: f32,
    sag_r: f32,
    sag_k: f32,
    // Voicing/cab biquad (RBJ lowpass, transposed DF2): tone-stack + speaker
    // voicing. A static circuit is fixed-Hz: 550 Hz for the dark clean rig —
    // it stacks with the bus pickup (1500 Hz) + tone one-pole into the refs'
    // measured ~−37 dB/oct cliff above 1 kHz, and holds their TIME-FLAT
    // centroid (spectrum is circuit-shaped, not string-shaped).
    vf_on: bool,
    vf_b0: f32,
    vf_b1: f32,
    vf_b2: f32,
    vf_a1: f32,
    vf_a2: f32,
    vf_z1: f32,
    vf_z2: f32,
    // Cab/box resonance (r3, 028 refit): closed-back cab = underdamped
    // 2nd-order highpass (fc ≈ 95 Hz, Q ≈ 1.4 → +3 dB hump at ~125–250 Hz,
    // sub rolloff below). This fixed low-mid emphasis is what keeps the refs'
    // H2–H4 +10…+15 dB over H1 at B1–E2 yet leaves E3+ fundamental-dominant —
    // register-resolved evidence that it is a cabinet, not a string, feature.
    hp_b0: f32,
    hp_b1: f32,
    hp_b2: f32,
    hp_a1: f32,
    hp_a2: f32,
    hp_z1: f32,
    hp_z2: f32,
    // Fret-release transient (r3 refit, 028 refs measured in the 3.0–3.2 s
    // window): the burst is dominated by a LOW thump (0–300 Hz ≈ all of its
    // energy — the finger lift re-excites the damped string; injected into
    // the loop at damp() so it rings at f0 and dies at the damped t60) plus a
    // 700–1400 Hz scrape band ~10 dB down (two cascaded one-poles below;
    // round 2's flat >900 Hz noise put −4 dB of the burst at 1.2–3 kHz where
    // the refs keep −29…−64). Injected ON THE STRING (pre-voicing).
    rel_rng: Lcg,
    rel_amp: f32,
    rel_c: f32,
    rel_hp_c: f32,
    rel_lp: f32,
    rel_lp2: f32,
    /// distorted-channel voice (drive-90 bus): scales the release injection
    /// down — r3's burst calibration was vs the CLEAN 028 refs; through the
    /// amp round's gain-ride (at cap by note-off) + tanh limiter + presence
    /// EQ the same injection re-saturated the limiter and read +4.7 dB ABOVE
    /// the note's own attack (measured 2026-07-12; a pop on every note-off).
    dist: bool,
    vel: f32,
    level: f32,
    life: u64,
    age: u64,
    sr: f32,
}

impl ElectricVoice {
    pub fn start(f0: f32, vel: f32, sr: f32, t60: f32, bright: f32, pick_pos: f32, seed: u32) -> Self {
        // Loop-filter tuning compensation (Jaffe-Smith): the one-pole loop lowpass
        // delays the loop by ~(1-c)/c samples, and c depends on brightness/velocity —
        // a fixed compensation detunes the string with velocity. Subtract the actual
        // filter delay, and bias the allpass fraction into [0.5, 1.5) so its pole
        // stays well inside the unit circle (ill-conditioned as frac→0).
        let lp_c = (bright * (0.35 + 0.6 * vel)).clamp(0.05, 0.995);
        let lp_delay = (1.0 - lp_c) / lp_c;
        let period = sr / f0;
        let total = (period - lp_delay).max(3.0);
        let len = ((total - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac = (total - len as f32).clamp(0.1, 1.5);
        let ap_c = (1.0 - frac) / (1.0 + frac);
        let mut v = Self {
            buf: [0.0; PLUCK_BUF],
            len,
            pos: 0,
            lp: 0.0,
            lp_c,
            loss: t60_gain(t60, sr),
            ap_c,
            ap_x1: 0.0,
            ap_y1: 0.0,
            pol2_on: false,
            diff_g: 0.0,
            diff_x1: 0.0,
            buf2: [0.0; PLUCK_BUF],
            len2: 2,
            pos2: 0,
            lp2: 0.0,
            loss2: 0.0,
            ap2_c: 0.0,
            ap2_x1: 0.0,
            ap2_y1: 0.0,
            sag_env: 0.0,
            sag_a: 0.0,
            sag_r: 0.0,
            sag_k: 0.0,
            vf_on: false,
            vf_b0: 0.0,
            vf_b1: 0.0,
            vf_b2: 0.0,
            vf_a1: 0.0,
            vf_a2: 0.0,
            vf_z1: 0.0,
            vf_z2: 0.0,
            hp_b0: 1.0,
            hp_b1: 0.0,
            hp_b2: 0.0,
            hp_a1: 0.0,
            hp_a2: 0.0,
            hp_z1: 0.0,
            hp_z2: 0.0,
            rel_rng: Lcg(seed ^ 0x9E37_79B9),
            rel_amp: 0.0,
            rel_c: 0.0,
            rel_hp_c: 0.0,
            rel_lp: 0.0,
            rel_lp2: 0.0,
            dist: false,
            vel,
            level: 0.5 * (0.35 + 0.65 * vel),
            life: ((t60 * 1.5) * sr) as u64,
            age: 0,
            sr,
        };
        // Excitation pre-loaded into the delay line: velocity-lowpassed noise,
        // comb-filtered at the pick position (Jaffe-Smith), DC-removed.
        let mut rng = Lcg(seed | 1);
        let mut lp = 0.0f32;
        let exc_c = 0.25 + 0.72 * vel; // soft pluck = duller excitation
        let mut tmp = [0.0f32; PLUCK_BUF];
        for t in tmp.iter_mut().take(len) {
            lp += exc_c * (rng.next() - lp);
            *t = lp;
        }
        let p = ((pick_pos * len as f32) as usize).clamp(1, len - 1);
        let mut mean = 0.0;
        for i in 0..len {
            let comb = tmp[i] - 0.9 * tmp[(i + len - p) % len];
            v.buf[i] = comb;
            mean += comb;
        }
        mean /= len as f32;
        for b in v.buf.iter_mut().take(len) {
            *b -= mean;
        }
        v
    }

    /// Electric solid-body voicing (GuitarElectric / GuitarDistorted only — the
    /// acoustic `start` laws above are untouched). Differences, physically:
    /// - Loss is dominated by the bridge/internal damping (`loss`), NOT the loop
    ///   lowpass: a solid body radiates almost nothing, so t60 is long and nearly
    ///   register-flat (NSynth guitar_electronic refs: t60_early ≈ 3.4–4.8 s from
    ///   E1 to C5). The loop lowpass applies once per round trip (f0 times/sec),
    ///   so its per-pass attenuation must shrink as f0 rises or trebles die —
    ///   key-track lp_c toward 1.0 up the neck (Jaffe & Smith 1983 loss scaling).
    ///
    /// CLEAN VOICING (round-2 decision): the NSynth clean refs split into two
    /// distinct rigs. The DEFAULT is the dark/saggy one (ref cluster 022:
    /// neck-pickup jazz tone, centroid ≈ 1.3–1.65·f0 time-flat, deep rail sag,
    /// t60_late/early ≈ 4.7) — reference-fit here. The alternative BRIGHT/STIFF
    /// rig (cluster 028: centroid ≈ 3.2–4.2·f0 at low register, sag ratio only
    /// ≈ 1.3–1.7, audible fret-release squeak) is a future preset row; measured
    /// directions if exposed: diff_g ON (velocity pickup tilt), vf_on false or
    /// vfc ≈ 2.6 kHz, sag_k ≈ 2.0, t60_slow ≈ 9 s, texture corner ×2, pickup
    /// row ≈ (2400 Hz, Q 1.8), tone ≈ 3.2 kHz. The release squeak passes its
    /// open voicing automatically (it is injected on the string, pre-voicing).
    pub fn start_electric(midi: u32, f0: f32, vel: f32, sr: f32, dist: bool, seed: u32) -> Self {
        let key = ((midi as f32) - 40.0) / 44.0; // 0 = E2 … 1 = C6
        // dist: heavy strings + amp compression read as longer sustain; the pick
        // signal into a high-gain chain is bright (bridge pickup, tone full up)
        // fast (vertical) polarization: strongly bridge-coupled, dies at the
        // refs' measured early rate; sustain grows up the neck (NSynth electrics
        // t60_early ≈ 3.4 s at E1 → 4.8 s at C5, round-1 measurement)
        let t60 = (if dist { 4.5 } else { 3.4 }) + 1.4 * key.max(0.0);
        // per-pass brightness: refs lose ~35 dB/s at 1 kHz in the low register
        // while H2..H5 barely decay — a steep loop corner, key-tracked so the
        // per-second HF decay stays register-flat (Valimaki et al. 1996 loop fit)
        // r3 (028 refit): key law linear 0.56·k → quadratic 0.33·k+0.17·k²
        // above E2 — the sustained centroid ran +40% bright at E3 (488 vs
        // 342 Hz) and +27% at C5 (903 vs 713 Hz); measured trade between
        // slopes 0.45 and 0.38 showed no single slope holds both ends (the
        // loop-loss law is convex in key). BELOW E2 the old 0.56 slope stays:
        // the quadratic accidentally brightened B1 (sustained H10+ ran
        // +15…+30 dB hot at 0.8 s vs the refs' collapse above ~600 Hz).
        let kf = if key < 0.0 {
            0.56 * key
        } else {
            0.33 * key + 0.17 * key * key
        };
        let mut lp_c = (0.51 + kf + 0.06 * vel).clamp(0.30, 0.985);
        if dist {
            lp_c = (lp_c + 0.08).min(0.985);
        }
        let lp_delay = (1.0 - lp_c) / lp_c;
        let period = sr / f0;
        let total = (period - lp_delay).max(3.0);
        let len = ((total - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac = (total - len as f32).clamp(0.1, 1.5);
        let ap_c = (1.0 - frac) / (1.0 + frac);
        // Second polarization (Weinreich 1977): rings ~2.5× longer, +0.5 cents
        // detuned (bridge admittance differs in the two planes), plucked at ~0.3
        // of the main amplitude — gives the fast-early/slow-late two-stage decay
        // measured in every NSynth electric ref (t60 0.1–0.4 s ≈ 3.5 s but
        // 0.8–1.8 s ≈ 6–20 s).
        let t60_slow = if dist { 18.0 } else { 9.0 };
        let f2 = f0 * 1.000289; // +0.5 cents
        let total2 = (sr / f2 - lp_delay).max(3.0);
        let len2 = ((total2 - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac2 = (total2 - len2 as f32).clamp(0.1, 1.5);
        let mut v = Self {
            buf: [0.0; PLUCK_BUF],
            len,
            pos: 0,
            lp: 0.0,
            lp_c,
            // Loop loss is applied once per ROUND TRIP (f0 times a second), not per
            // sample — the per-period gain for a wanted t60 is g = 10^(−3·P/t60)
            // (Jaffe & Smith 1983), i.e. t60_gain evaluated at rate f0, not sr.
            // (t60_gain(t60, sr) here would leave the fundamental ringing ~sr/f0
            //  times too long — measured 40 s instead of 4 s at f0 = 87 Hz.)
            loss: t60_gain(t60, f0),
            ap_c,
            ap_x1: 0.0,
            ap_y1: 0.0,
            pol2_on: true,
            // Magnetic pickups sense string VELOCITY (Zollner ch.4) — but the
            // clean 022 refs show a displacement-like 1/n² knee at ~1.5·f0:
            // the rolled-off tone circuit + amp input coupling integrate the
            // tilt back out. Net transfer ≈ displacement → diff off for clean.
            // The distorted channel keeps the velocity tilt (bright bridge
            // pickup feeding the drive is what makes palm-tone chugs cut).
            // bright-rig clean (owner 2026-07-12: dark 022 voicing read as an
            // e-piano): velocity-pickup tilt ON for both channels now
            diff_g: 1.0 / (2.0 * (core::f32::consts::PI * f0 / sr).sin()).max(1e-3),
            diff_x1: 0.0,
            buf2: [0.0; PLUCK_BUF],
            len2,
            pos2: 0,
            lp2: 0.0,
            loss2: t60_gain(t60_slow, f2),
            ap2_c: (1.0 - frac2) / (1.0 + frac2),
            ap2_x1: 0.0,
            ap2_y1: 0.0,
            // rail sag: rectifier charge ~25 ms, filter-cap recovery ~300 ms
            // (Fender/Marshall RC supplies land in this decade; the audible spec
            // is the two-slope decay every NSynth electric ref shows).
            // Depth k is the amp's stiffness: high-gain supplies sag deeper.
            sag_env: 0.0,
            sag_a: 1.0 - (-1.0 / (0.025 * sr)).exp(),
            sag_r: 1.0 - (-1.0 / (0.900 * sr)).exp(),
            // depth: low notes draw more supply current (more stored string energy),
            // so sag scales down the neck — deep on E1, mild at C5
            sag_k: (if dist { 9.0 } else { 2.0 }) * (1.0 - 0.55 * key.clamp(0.0, 1.0)),
            vf_on: true,
            vf_b0: 0.0,
            vf_b1: 0.0,
            vf_b2: 0.0,
            vf_a1: 0.0,
            vf_a2: 0.0,
            vf_z1: 0.0,
            vf_z2: 0.0,
            hp_b0: 1.0,
            hp_b1: 0.0,
            hp_b2: 0.0,
            hp_a1: 0.0,
            hp_a2: 0.0,
            hp_z1: 0.0,
            hp_z2: 0.0,
            rel_rng: Lcg(seed ^ 0x9E37_79B9),
            rel_amp: 0.0,
            rel_c: t60_gain(0.22, sr),
            // scrape band: LP 1400 minus LP 700 (see field comment)
            rel_hp_c: 1.0 - (-core::f32::consts::TAU * 1400.0 / sr).exp(),
            rel_lp: 0.0,
            rel_lp2: 0.0,
            dist,
            vel,
            // Velocity moves loudness far less than timbre on an electric (NSynth
            // layer spread ≈ 5 LU, most of it spectral): keep the level curve
            // shallow and let the pick corner carry the dynamics. Mild key boost
            // compensates the short upper strings' lower energy (P72 was −18 LU).
            level: 0.52 * (0.72 + 0.28 * vel) * (1.0 + 0.35 * key.max(0.0)),
            life: ((t60_slow + 0.5) * sr) as u64,
            age: 0,
            sr,
        };
        // voicing/cab corner. Clean: FIXED-Hz 2-pole at 550 Hz — the measured 022
        // mid-third curve is (a) the triangle's sin(nπ·0.13)/n² law through H6,
        // (b) a −26 dB texture plateau 450 Hz–1 kHz, then (c) a ~−37 dB/oct
        // cliff: this pole + the bus pickup (1500 Hz) + tone one-pole stack to
        // exactly that cliff order. A static circuit is fixed-Hz — the earlier
        // key-tracked corner was wrong (it crushed every harmonic of low notes).
        // Distorted runs the voicing open at a FIXED 3.2 kHz (bridge-pickup +
        // presence feed into the drive — a keyed corner choked low power chords
        // at ~500 Hz and left the channel lifeless; the post-drive cab rolloff
        // is the bus tone row).
        let vfc = if dist { 3200.0 } else { 2600.0 };
        let wv = core::f32::consts::TAU * vfc / sr;
        let (sv, cv) = wv.sin_cos();
        let alpha = sv / (2.0 * 0.707);
        let a0 = 1.0 + alpha;
        v.vf_b0 = ((1.0 - cv) / 2.0) / a0;
        v.vf_b1 = (1.0 - cv) / a0;
        v.vf_b2 = v.vf_b0;
        v.vf_a1 = (-2.0 * cv) / a0;
        v.vf_a2 = (1.0 - alpha) / a0;
        // cab/box resonance highpass (see field comment): RBJ HP, fc 95, Q 1.4
        // (r3 note: a dist-only 60 Hz/Q2 "depth" bump was tried against the
        // FreePats chug refs and measured NEUTRAL-to-worse — pre-clip LF
        // boosts cannot out-vote the velocity-pickup tilt at drive 90; the
        // refs' LF-dominant chug needs a POST-clip cab bump, see report recs)
        let wh = core::f32::consts::TAU * 95.0 / sr;
        let (sh, ch) = wh.sin_cos();
        let alpha_h = sh / (2.0 * 1.4);
        let a0h = 1.0 + alpha_h;
        v.hp_b0 = ((1.0 + ch) / 2.0) / a0h;
        v.hp_b1 = -(1.0 + ch) / a0h;
        v.hp_b2 = v.hp_b0;
        v.hp_a1 = (-2.0 * ch) / a0h;
        v.hp_a2 = (1.0 - alpha_h) / a0h;
        // excitation: a pick pluck is a DETERMINISTIC released displacement
        // triangle (Smith PASP: harmonics ∝ sin(nπ·pick)/n²) plus a small
        // lowpassed-noise texture layer. Round-2 finding: pure noise excitation
        // has σ ≈ 7.7 dB note-to-note H2/H1 variance (measured, 40 seeds) — a
        // tone lottery in exactly the harmonics the dark clean voicing exposes.
        // Bridge-side electric picking. r3 refit vs the 028 cluster: the pick
        // sits at a fixed physical DISTANCE from the bridge, so its fraction
        // of the speaking length doubles per octave up the neck (L ∝ 1/f0).
        // The refs' comb structure demands exactly that: lobe cutoff ~H18-20
        // at B1, ~H14 at A2, ~H7 at E3, ~H4.5 at C5 — no fixed fraction fits.
        // Base 0.07 at E2; clamp [0.085, 0.22] — measured: at B1 fractions
        // below ~0.08 over-brighten the SUSTAIN (the wider lobe feeds
        // harmonics the loop preserves: 235-281 vs 212 Hz at 0.8 s), while
        // 0.10 starved the attack's second lobe; the floor holds the low
        // strings near the H11-12 null. E3 fits at ~0.14, C5 at the 0.22
        // ceiling (null ~H4.5 — the refs' H5 sits at −23 dB there).
        // The H8 notch is the pickup comb's, fraction-flat in the refs.
        let pick_pos = (0.07 * (44.0 * key / 12.0).exp2()).clamp(0.085, 0.22);
        let mut rng = Lcg(seed | 1);
        // texture corner: flesh-soft ≈ 200 Hz → hard plectrum ≈ 1.6 kHz,
        // register-tracked (as in round 1); the deterministic shape underneath
        // keeps the low-harmonic balance stable across velocity and seed.
        let mut fc = ((350.0 + 500.0 * vel) * (1.0 + 1.0 * key)).clamp(150.0, 0.35 * sr);
        if dist {
            fc = (fc * 2.5).min(0.35 * sr);
        }
        let exc_c = 1.0 - (-core::f32::consts::TAU * fc / sr).exp();
        let p = ((pick_pos * len as f32) as usize).clamp(1, len - 1);
        let mut lp = 0.0f32;
        let mut lp2 = 0.0f32;
        let mut tmp = [0.0f32; PLUCK_BUF];
        for (i, t) in tmp.iter_mut().enumerate().take(len) {
            // released triangle: 0→1 over [0,p], back to 0 over [p,len)
            let tri = if i < p {
                i as f32 / p as f32
            } else {
                (len - i) as f32 / (len - p) as f32
            };
            // texture: TWO-pole noise (−12 dB/oct above fc). The scrape is a
            // displacement-domain disturbance with a finite contact patch —
            // with the velocity pickup ON (+6 dB/oct), a one-pole texture came
            // out FLAT to Nyquist and buried the comb notches under broadband
            // noise (r3 baseline: attack centroid 1.4 kHz vs refs' 350 Hz).
            // Post-diff the layer now falls −6 dB/oct like the refs' lobes.
            lp += exc_c * (rng.next() - lp);
            lp2 += exc_c * (lp - lp2);
            *t = tri + 0.35 * lp2;
        }
        // Pickup-POSITION comb (Zollner ch. 5): a magnetic pickup samples the
        // string at q of the speaking length → |sin(nπq)| response. The 028
        // refs show its H8 null (~20 dB deep, so q ≈ 0.125) and the H1
        // suppression that keeps H2–H4 on top even in the sustain — the single
        // spectral cue that most separates a guitar pickup from an EP tine.
        // String + loop are LTI, so the comb commutes into the excitation
        // (Karjalainen/Smith commuted synthesis) at zero runtime cost. Depth
        // 0.92 models the pickup aperture (finite sensing width bounds the
        // null depth; the refs' notch floor is ≈ −20 dB, not −∞).
        let q = ((0.125 * len as f32) as usize).clamp(1, len - 1);
        let mut mean = 0.0;
        for i in 0..len {
            let s = tmp[i] - 0.92 * tmp[(i + len - q) % len];
            v.buf[i] = s;
            mean += s;
        }
        mean /= len as f32;
        for b in v.buf.iter_mut().take(len) {
            *b -= mean;
        }
        // pick displaces mostly one plane; ~0.3 leaks into the slow polarization
        for i in 0..v.len2 {
            v.buf2[i] = 0.35 * v.buf[i % len];
        }
        v
    }

    pub fn render(&mut self, out: &mut [f32]) -> bool {
        for o in out.iter_mut() {
            let y = self.buf[self.pos];
            // loop lowpass (string damping / brightness)
            self.lp += self.lp_c * (y - self.lp);
            // fractional-delay allpass keeps the string in tune
            let ap = self.ap_c * (self.lp - self.ap_y1) + self.ap_x1;
            self.ap_x1 = self.lp;
            self.ap_y1 = ap;
            self.buf[self.pos] = ap * self.loss;
            self.pos = (self.pos + 1) % self.len;
            let mut s = y;
            // slow horizontal polarization (electrics only)
            if self.pol2_on {
                let y2 = self.buf2[self.pos2];
                self.lp2 += self.lp_c * (y2 - self.lp2);
                let ap2 = self.ap2_c * (self.lp2 - self.ap2_y1) + self.ap2_x1;
                self.ap2_x1 = self.lp2;
                self.ap2_y1 = ap2;
                self.buf2[self.pos2] = ap2 * self.loss2;
                self.pos2 = (self.pos2 + 1) % self.len2;
                s += y2;
            }
            if self.diff_g > 0.0 {
                let d = (s - self.diff_x1) * self.diff_g;
                self.diff_x1 = s;
                s = d;
            }
            // fret-release scrape: 700-1400 Hz noise band on the string (see fields)
            if self.rel_amp > 1e-5 {
                let w = self.rel_rng.next();
                self.rel_lp += self.rel_hp_c * (w - self.rel_lp);
                self.rel_lp2 += 0.5 * self.rel_hp_c * (self.rel_lp - self.rel_lp2);
                s += (self.rel_lp - self.rel_lp2) * self.rel_amp;
                self.rel_amp *= self.rel_c;
            }
            let mut u = s * self.level;
            // voicing/cab biquad (see fields; static per note, fixed-Hz corner)
            if self.vf_on {
                let y = self.vf_b0 * u + self.vf_z1;
                self.vf_z1 = self.vf_b1 * u - self.vf_a1 * y + self.vf_z2;
                self.vf_z2 = self.vf_b2 * u - self.vf_a2 * y;
                u = y;
                // cab/box resonance highpass (see fields)
                let yh = self.hp_b0 * u + self.hp_z1;
                self.hp_z1 = self.hp_b1 * u - self.hp_a1 * yh + self.hp_z2;
                self.hp_z2 = self.hp_b2 * u - self.hp_a2 * yh;
                u = yh;
            }
            // supply-rail sag: slow follower, gain = 1/(1 + k·env) (see fields)
            if self.sag_k > 0.0 {
                let a = u.abs();
                let c = if a > self.sag_env { self.sag_a } else { self.sag_r };
                self.sag_env += c * (a - self.sag_env);
                u /= 1.0 + self.sag_k * self.sag_env;
            }
            *o += u;
        }
        self.sag_env = flush_denormal(self.sag_env);
        self.vf_z1 = flush_denormal(self.vf_z1);
        self.vf_z2 = flush_denormal(self.vf_z2);
        self.hp_z1 = flush_denormal(self.hp_z1);
        self.hp_z2 = flush_denormal(self.hp_z2);
        self.diff_x1 = flush_denormal(self.diff_x1);
        self.lp = flush_denormal(self.lp);
        self.ap_y1 = flush_denormal(self.ap_y1);
        self.lp2 = flush_denormal(self.lp2);
        self.ap2_y1 = flush_denormal(self.ap2_y1);
        self.age += out.len() as u64;
        self.age < self.life
    }

    pub fn damp(&mut self) {
        if self.pol2_on {
            // electric fret/finger release: the 028 refs' post-off burst+string
            // decay ~260 dB/s → t60 ≈ 0.25 s. Loss basis is per round trip
            // (rate f0 = sr/len), see above. Life 0.45 s: NSynth refs hard-gate
            // ~0.3 s after the off — ringing past that is pure tail error.
            let f0 = self.sr / self.len.max(2) as f32;
            self.loss = t60_gain(0.25, f0);
            self.loss2 = self.loss;
            self.life = self.age + (0.45 * self.sr) as u64;
            // fret-release transient, r3 calibration against the 028 refs
            // (burst rms re note attack: −14 dB at vel 50, −16 at vel 25,
            // −21 at vel 127, −22 by E3, absent at C5; burst spectrum ≈ ALL
            // 0–300 Hz + a 700–1400 Hz scrape lobe ~10 dB down; bump decays
            // with the damped string, ~0.35 s). The lift force is ~constant
            // (press force, not pick force) so relative to the velocity-
            // scaled note the burst FALLS with velocity, and it is a WOUND-
            // string feature fading across the wound/plain boundary.
            // Thump = dark re-pluck INTO the loop: rings at f0, fundamental-
            // dominant (1/n²), dies at the damped t60 — the refs' burst.
            // Scrape = short 700–1400 Hz noise band (t60 0.22 s, render()).
            let key = ((12.0 * (f0 / 440.0).log2() + 69.0) - 40.0) / 44.0;
            let wound = (1.0 - 1.9 * key.max(0.0)).clamp(0.0, 1.0);
            let force = (1.0 - 0.55 * self.vel) * wound;
            // scrape stays velocity-flat: the refs' 700–1400 lobe is
            // proportionally LARGER at ff (−9 re burst) than at mp (−20)
            // dist: the r3 thump is an H1-PURE injection (raised cosine) — on
            // the drive-90 bus the limiter re-saturates on it and the whole
            // burst lands on the post-drive 105 Hz cab bump (+9 dB), reading
            // +3.6…+4.7 dB ABOVE the note's own attack (measured: a −9 dB
            // injection cut only moved the output −1 dB — a limiter eats
            // amplitude changes, only spectrum survives). A high-gain note-off
            // is a SCRAPE, not a boom: drop the thump, keep a reduced scrape.
            let dist_rel = if self.dist { 0.35 } else { 1.0 };
            self.rel_amp = 0.20 * wound * dist_rel;
            // thump = pure-fundamental injection (raised cosine minus its
            // mean is a clean H1 with zero DC): the refs put ~99% of the
            // burst below 300 Hz; a triangle re-pluck leaked −8 dB of
            // harmonics into 300–700 Hz where the refs keep −15…−18.
            let thump = if self.dist { 0.0 } else { 0.17 * force };
            let len = self.len;
            let w = core::f32::consts::TAU / len as f32;
            for (i, b) in self.buf.iter_mut().enumerate().take(len) {
                *b -= thump * (w * i as f32).cos();
            }
        } else {
            self.loss = t60_gain(0.07, self.sr);
            self.loss2 = t60_gain(0.07, self.sr);
            self.life = self.age + (0.1 * self.sr) as u64;
        }
    }
}

// ---------------------------------------------------------------------------
// Sympathetic resonance bank (pedal bloom) — the sound of a piano's lifted
// dampers: undamped strings driven by everything else on the track. Twelve
// dark, long-ringing string loops at harmonic-rich tunings; input is injected
// INTO the loops so resonance builds and sings (Bank 2003 efficient
// sympathetic simulation; Rings-style resonator lineage).
// ---------------------------------------------------------------------------

pub const SYMP_STRINGS: usize = 12;
const SYMP_BUF: usize = 1024;
/// C2 G2 C3 E3 G3 A#3 C4 D4 E4 G4 A4 C5 — spread of common overtone anchors
const SYMP_TUNING: [u32; SYMP_STRINGS] = [36, 43, 48, 52, 55, 58, 60, 62, 64, 67, 69, 72];

/// Always-on send fraction: duplex/aliquot string segments are never damped
/// (Conklin), so a sliver of every note reaches the bank even pedal-up.
const DUPLEX_SEND: f32 = 0.12;

#[derive(Clone)]
pub struct SympBank {
    bufs: [[f32; SYMP_BUF]; SYMP_STRINGS],
    len: [usize; SYMP_STRINGS],
    pos: [usize; SYMP_STRINGS],
    lp: [f32; SYMP_STRINGS],
    loss_open: [f32; SYMP_STRINGS],
    loss_damped: [f32; SYMP_STRINGS],
    open: bool,
    /// smoothed input send (ramps with the pedal so engagement doesn't click)
    send: f32,
    send_target: f32,
    send_c: f32,
    /// recent-input activity follower (lets silent tracks skip the bank even
    /// though the duplex floor keeps a small send alive)
    hot: f32,
    pub enabled: bool,
    /// true while configured as the guitar open-string bank (lets lib.rs
    /// restore the piano tuning when a track switches back without touching
    /// still-ringing piano banks on repeated set_track calls)
    pub guitar: bool,
    wet: f32,
}

impl SympBank {
    pub fn new(sr: f32) -> Self {
        let mut b = Self {
            bufs: [[0.0; SYMP_BUF]; SYMP_STRINGS],
            len: [2; SYMP_STRINGS],
            pos: [0; SYMP_STRINGS],
            lp: [0.0; SYMP_STRINGS],
            loss_open: [0.0; SYMP_STRINGS],
            loss_damped: [0.0; SYMP_STRINGS],
            open: false,
            send: DUPLEX_SEND,
            send_target: DUPLEX_SEND,
            send_c: 1.0 - (-1.0 / (0.015 * sr)).exp(),
            hot: 0.0,
            enabled: false,
            guitar: false,
            wet: 0.4,
        };
        for (i, &m) in SYMP_TUNING.iter().enumerate() {
            let f0 = midi_to_hz(m as f32);
            b.len[i] = ((sr / f0 - 0.5) as usize).clamp(2, SYMP_BUF - 1);
            // per-period loss (the fleet's convergent lesson): long open ring;
            // pedal-up is NOT dead — duplex/aliquot segments and the bridge
            // keep a short undamped ring (~0.4 s; Conklin's duplex scaling)
            b.loss_open[i] = 10f32.powf(-3.0 / (5.0 * f0));
            b.loss_damped[i] = 10f32.powf(-3.0 / (0.4 * f0));
        }
        b
    }

    /// Reconfigure as the guitar's six open strings (E2 A2 D3 G3 B3 E4):
    /// every fretted note rings the other strings through the bridge. This is
    /// the always-stable FEEDFORWARD sympathetic bank (Jaffe-Smith 1983; the
    /// HUT coupling matrix of Karjalainen/Valimaki/Tolonen CMJ 1998 Eq. 16 is
    /// the same idea inside a dual-polarization string). Coupling is subtle -
    /// audible on strums and staccato phrases, not solo held notes (Woodhouse,
    /// Euphonics 7.3: guitar bridge coupling exceeds intrinsic damping only
    /// near the strong soundboard resonances). Slots 6..12 are parked (len 2,
    /// skipped by tick's len guard). Piano banks are untouched: lib.rs calls
    /// this only for guitar-family tracks.
    pub fn retune_guitar(&mut self, sr: f32) {
        const GUITAR_OPEN: [u32; 6] = [40, 45, 50, 55, 59, 64];
        for i in 0..SYMP_STRINGS {
            self.bufs[i] = [0.0; SYMP_BUF];
            self.lp[i] = 0.0;
            self.pos[i] = 0;
            if i < GUITAR_OPEN.len() {
                let f0 = midi_to_hz(GUITAR_OPEN[i] as f32);
                self.len[i] = ((sr / f0 - 0.5) as usize).clamp(2, SYMP_BUF - 1);
                // undamped open strings ring a few seconds while the hand is
                // playing; the flesh chokes them when the last voice dies
                // (lib.rs drives open/damped from track voice activity)
                self.loss_open[i] = 10f32.powf(-3.0 / (3.0 * f0));
                self.loss_damped[i] = 10f32.powf(-3.0 / (0.3 * f0));
            } else {
                self.len[i] = 2;
            }
        }
        self.open = true;
        self.send = 1.0;
        self.send_target = 1.0;
        // subtle: single sampled notes (NSynth) carry no cross-string ring;
        // the halo should surface on strums/phrases, not solo-note metrics
        self.wet = 0.05;
        self.hot = 0.0;
        self.guitar = true;
    }

    pub fn set_pedal(&mut self, on: bool) {
        self.open = on;
        // pedal-up keeps the duplex floor: undamped string segments ring
        // regardless of the dampers (the faint metallic halo of a real piano)
        self.send_target = if on { 1.0 } else { DUPLEX_SEND };
    }

    /// True while the bank could still be audible (skip processing otherwise).
    pub fn ringing(&self) -> bool {
        self.open || self.send > DUPLEX_SEND + 1e-3 || self.hot > 1e-5
    }

    #[inline]
    pub fn tick(&mut self, input: f32) -> f32 {
        self.hot = (self.hot * 0.9995).max(input.abs());
        self.send += self.send_c * (self.send_target - self.send);
        let inj = input * self.send * (1.0 / SYMP_STRINGS as f32) * 0.9;
        let mut sum = 0.0;
        for i in 0..SYMP_STRINGS {
            if self.len[i] < 3 {
                continue; // parked slot (guitar bank uses 6 of 12)
            }
            let p = self.pos[i];
            let y = self.bufs[i][p];
            // dark loop: sympathetic strings answer mostly with their lower partials
            self.lp[i] += 0.22 * (y - self.lp[i]);
            let loss = if self.open { self.loss_open[i] } else { self.loss_damped[i] };
            self.bufs[i][p] = self.lp[i] * loss + inj;
            self.pos[i] = (p + 1) % self.len[i];
            sum += y;
        }
        sum * self.wet
    }

    pub fn flush(&mut self) {
        for i in 0..SYMP_STRINGS {
            self.lp[i] = flush_denormal(self.lp[i]);
        }
    }
}

// ---------------------------------------------------------------------------
// Subtractive synth pad — two polyBLEP saws → 2-pole lowpass → ADSR
// (classic-synth track per PRINCIPLES #5; polyBLEP keeps the top end alias-free,
//  which is the producer persona's #1 dismissal criterion)
// ---------------------------------------------------------------------------

/// polyBLEP residual: subtracts the aliased step at the saw wrap.
#[inline(always)]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy)]
pub struct SynthVoice {
    phase: [f32; 2],
    dphase: [f32; 2],
    lp1: f32,
    lp2: f32,
    cutoff_hz: f32,
    // ADSR
    env: f32,
    stage: u8, // 0 attack, 1 decay, 2 sustain, 3 release
    attack_c: f32,
    decay_c: f32,
    sustain: f32,
    release_c: f32,
    level: f32,
    sr: f32,
    age: u64,
    life: u64,
}

impl SynthVoice {
    pub fn start(f0: f32, vel: f32, sr: f32) -> Self {
        let detune = 1.004; // ~7 cents apart
        Self {
            phase: [0.0, 0.37],
            dphase: [f0 / (sr * detune), f0 * detune / sr],
            lp1: 0.0,
            lp2: 0.0,
            cutoff_hz: (300.0 + 2800.0 * vel * vel + f0 * 1.5).min(0.4 * sr),
            env: 0.0,
            stage: 0,
            attack_c: 1.0 - (-1.0 / (0.12 * sr)).exp(),
            decay_c: 1.0 - (-1.0 / (0.45 * sr)).exp(),
            sustain: 0.72,
            release_c: 1.0 - (-1.0 / (0.35 * sr)).exp(),
            level: 0.16 * (0.4 + 0.6 * vel),
            sr,
            age: 0,
            life: (30.0 * sr) as u64, // safety cap; normally ends via release
        }
    }

    pub fn render(&mut self, out: &mut [f32]) -> bool {
        // block-rate filter coefficient (envelope moves slowly; no audible zipper)
        let fc = self.cutoff_hz * (0.35 + 0.65 * self.env);
        let c = 1.0 - (-core::f32::consts::TAU * fc / self.sr).exp();
        for o in out.iter_mut() {
            let mut s = 0.0;
            for v in 0..2 {
                let t = self.phase[v];
                s += (2.0 * t - 1.0) - poly_blep(t, self.dphase[v]);
                self.phase[v] += self.dphase[v];
                if self.phase[v] >= 1.0 {
                    self.phase[v] -= 1.0;
                }
            }
            // ADSR
            match self.stage {
                0 => {
                    self.env += self.attack_c * (1.02 - self.env);
                    if self.env >= 1.0 {
                        self.env = 1.0;
                        self.stage = 1;
                    }
                }
                1 => {
                    self.env += self.decay_c * (self.sustain - self.env);
                    if (self.env - self.sustain).abs() < 1e-3 {
                        self.stage = 2;
                    }
                }
                2 => {}
                _ => self.env += self.release_c * (0.0 - self.env),
            }
            self.lp1 += c * (s - self.lp1);
            self.lp2 += c * (self.lp1 - self.lp2);
            *o += self.lp2 * self.env * self.level;
        }
        self.lp1 = flush_denormal(self.lp1);
        self.lp2 = flush_denormal(self.lp2);
        self.env = flush_denormal(self.env);
        self.age += out.len() as u64;
        let dead = self.stage == 3 && self.env < 1e-4;
        !dead && self.age < self.life
    }

    pub fn release(&mut self) {
        self.stage = 3;
    }
}

// ---------------------------------------------------------------------------
// Cymbals — banded-noise resonator bank with post-attack bloom
// ---------------------------------------------------------------------------
// A cymbal is a thin nonlinear plate: hundreds of inharmonic modes whose density
// increases with frequency, coupled nonlinearly so strike energy migrates upward
// for ~50–200 ms after contact (the audible "bloom"), then decays over seconds
// with strong beating — dense-modal (NOT white-noise) even above 3 kHz.
// Real-time approximation from the banded-noise lineage (Essl & Cook, banded
// waveguides for cymbals/gongs, ICMC 1999/2000; Serra & Smith stochastic
// modeling; Risset inharmonic partial stacks): N narrow two-pole resonators fed
// by shared white noise. Per band: an immediate strike-burst gain, a sustained
// "wash" gain gated by a DELAYED one-pole bloom envelope (stands in for the
// nonlinear upward energy transfer), and an independent decay t60.
// Reference targets measured from CC0 recordings (see 2026-07-11 cymbal report):
// ride centroid FALLS 8k→3k over 300 ms while the 1–2 kHz band peaks late
// (~60 ms); crash centroid RISES 1.8k→7.7k into ~50 ms; in-band spectral
// flatness stays 0.2–0.5 everywhere (never pure noise, never a lone sine).

pub const CYM_BANDS: usize = 22;

#[derive(Clone, Copy)]
pub struct CymbalVoice {
    n_bands: usize,
    // two-pole resonator bank: y = a1·y1 − r2·y2 + x
    a1: [f32; CYM_BANDS],
    r2: [f32; CYM_BANDS],
    y1: [f32; CYM_BANDS],
    y2: [f32; CYM_BANDS],
    /// strike-burst input gain (immediate, stick contact + ping)
    g_burst: [f32; CYM_BANDS],
    /// sustained wash input gain (reached after bloom)
    g_wash: [f32; CYM_BANDS],
    /// per-band wash decay envelope (t60 of the band's tail)
    wash_env: [f32; CYM_BANDS],
    wash_dec: [f32; CYM_BANDS],
    /// bloom state: starts at the delayed fraction β, decays to 0;
    /// the band's wash input is scaled by (1 − bloom)
    bloom: [f32; CYM_BANDS],
    bloom_c: [f32; CYM_BANDS],
    burst_env: f32,
    burst_dec: f32,
    amp: f32,
    /// Σ|initial ring amplitude| — worst-case (phase-aligned) strike sum
    strike_sum: f32,
    /// slosh AM (r4): two hi-hat plates rattle against each other, gating the
    /// wash with a slow chaotic wobble (detrended envelope std: open-hat ref
    /// 0.76 dB vs ride 0.65 — and the r3 renders were INVERTED, 0.57 vs 0.79).
    /// slosh = LP'd noise state, slosh_c its coefficient (~5 Hz), slosh_d the
    /// modulation depth (0 = off: rides/crashes are single stable plates).
    slosh: f32,
    slosh_c: f32,
    slosh_d: f32,
    rng: Lcg,
    life: u64,
    age: u64,
}

/// Per-band design parameters handed to [`CymbalVoice::push_band`].
struct CymBand {
    freq: f32,
    /// resonator ring t60 (bandwidth: ~2.2/ring_t60 Hz) — texture, not decay
    ring_t60: f32,
    burst: f32,
    /// contact-scrape noise gain (attack HF bed, decays in ~30 ms)
    chick: f32,
    wash: f32,
    /// wash decay t60 seconds
    decay_t60: f32,
    /// delayed fraction of the wash (0 = immediate, 1 = fully bloomed-in)
    bloom_frac: f32,
    /// bloom time constant seconds
    bloom_tau: f32,
}

impl CymbalVoice {
    fn new(seed: u32) -> Self {
        Self {
            n_bands: 0,
            a1: [0.0; CYM_BANDS],
            r2: [0.0; CYM_BANDS],
            y1: [0.0; CYM_BANDS],
            y2: [0.0; CYM_BANDS],
            g_burst: [0.0; CYM_BANDS],
            g_wash: [0.0; CYM_BANDS],
            wash_env: [1.0; CYM_BANDS],
            wash_dec: [0.0; CYM_BANDS],
            bloom: [0.0; CYM_BANDS],
            bloom_c: [0.0; CYM_BANDS],
            burst_env: 1.0,
            burst_dec: 0.0,
            amp: 0.0,
            strike_sum: 0.0,
            slosh: 0.0,
            slosh_c: 0.0,
            slosh_d: 0.0,
            rng: Lcg(seed | 1),
            life: 0,
            age: 0,
        }
    }

    /// Deterministic headroom guard: if the worst-case (phase-aligned) sum of
    /// initial ring amplitudes exceeds `cap`, scale the strike states down so
    /// no first-millisecond spike can exceed it regardless of phase draws.
    fn cap_strike(&mut self, cap: f32) {
        if self.strike_sum > cap {
            let k = cap / self.strike_sum;
            for i in 0..self.n_bands {
                self.y1[i] *= k;
                self.y2[i] *= k;
            }
        }
    }

    /// Hi-hat choke / hand mute: collapse every band's tail fast (pedal closing
    /// clamps the plates), kill the strike bed, end the voice shortly after.
    /// The clamp damps the RESONATORS too, not just the noise feed: each band
    /// is re-poled to a ~45 ms t60 at its own frequency (recover cos ω from
    /// a1/2r), otherwise the tonal skeleton (ring t60 up to ~3 s) rings on
    /// until the life cutoff and the choke ends in an audible hard cut.
    fn choke(&mut self, sr: f32) {
        let d = t60_gain(0.045, sr);
        let r_new = d;
        for i in 0..self.n_bands {
            self.wash_dec[i] = d;
            let r_old = self.r2[i].max(1e-12).sqrt();
            let cos_w = (self.a1[i] / (2.0 * r_old)).clamp(-1.0, 1.0);
            self.a1[i] = 2.0 * r_new * cos_w;
            self.r2[i] = r_new * r_new;
        }
        self.burst_env = 0.0;
        self.life = self.age + (0.15 * sr) as u64;
    }

    fn push_band(&mut self, b: CymBand, sr: f32) {
        if self.n_bands == CYM_BANDS || b.freq >= 0.45 * sr {
            return; // Nyquist guard — never synthesize above 0.45·sr
        }
        let i = self.n_bands;
        let r = t60_gain(b.ring_t60, sr);
        let w = core::f32::consts::TAU * b.freq / sr;
        self.a1[i] = 2.0 * r * w.cos();
        self.r2[i] = r * r;
        // (1−r)·sin(ω) normalizes resonance peak gain across bandwidth AND
        // center frequency (two-pole peak gain ≈ g / ((1−r)·sin ω); without the
        // sin ω term low bands come out ~28 dB hot — iteration-1 measurement)
        let norm = (1.0 - r) * w.sin();
        // Strike = impulse initial condition: the stick imparts velocity to
        // every mode at t=0 (a ms-scale noise burst cannot charge a band whose
        // rise time is 1/BW — iteration-5 measurement). Ring peak ≈ b.burst,
        // boosted for fast-ringing bands so perceived (frame-energy) strike
        // level is even across the spectrum (iteration-6 calibration).
        // Ring phase is randomized per band: phase-aligned rings beat
        // constructively ~40 ms in and delay the envelope peak (iteration 16).
        let c = b.burst * (0.045 / b.ring_t60).sqrt().clamp(1.0, 4.5);
        let phi = core::f32::consts::PI * self.rng.next();
        self.y1[i] = c * (phi - w).sin();
        self.y2[i] = c * (phi - 2.0 * w).sin();
        self.strike_sum += c.abs();
        // sustained-noise "chick": bright contact scrape that carries the
        // attack's HF for the first ~30 ms, then hands over to the wash
        // (reference onset energy concentrates at 6–18 kHz — iteration 20)
        self.g_burst[i] = b.chick * norm;
        self.g_wash[i] = b.wash * norm;
        self.wash_dec[i] = t60_gain(b.decay_t60, sr);
        self.bloom[i] = b.bloom_frac.clamp(0.0, 1.0);
        self.bloom_c[i] = if b.bloom_tau > 0.0 { (-1.0 / (b.bloom_tau * sr)).exp() } else { 0.0 };
        self.n_bands += 1;
    }

    /// Render one block, ADD into `out` (pre-scaled by `self.amp`).
    /// Returns false when spent.
    fn render(&mut self, out: &mut [f32]) -> bool {
        for o in out.iter_mut() {
            let mut n = self.rng.next();
            let nb = n * self.burst_env;
            self.burst_env *= self.burst_dec;
            // slosh AM: slow chaotic gate on the sustained wash feed (plate
            // rattle) — the strike burst above stays un-modulated
            if self.slosh_d > 0.0 {
                self.slosh += self.slosh_c * (n - self.slosh);
                n *= 1.0 + self.slosh_d * self.slosh;
            }
            let mut s = 0.0;
            for i in 0..self.n_bands {
                // squared bloom gate = sigmoid onset: nonlinear upward energy
                // transfer is autocatalytic, so HF stays near-silent for ~τ/2
                // then floods in (a plain exponential leaks HF into the first
                // 30 ms — the crash reference is dark until then; iteration 26)
                let bl = 1.0 - self.bloom[i];
                let x = self.g_burst[i] * nb + self.g_wash[i] * bl * bl * self.wash_env[i] * n;
                let y = self.a1[i] * self.y1[i] - self.r2[i] * self.y2[i] + x;
                self.y2[i] = self.y1[i];
                self.y1[i] = y;
                s += y;
                self.wash_env[i] *= self.wash_dec[i];
                self.bloom[i] *= self.bloom_c[i];
            }
            *o += s * self.amp;
            self.age += 1;
        }
        self.burst_env = flush_denormal(self.burst_env);
        self.slosh = flush_denormal(self.slosh);
        for i in 0..self.n_bands {
            self.y1[i] = flush_denormal(self.y1[i]);
            self.y2[i] = flush_denormal(self.y2[i]);
            self.wash_env[i] = flush_denormal(self.wash_env[i]);
            self.bloom[i] = flush_denormal(self.bloom[i]);
        }
        self.age < self.life
    }

    /// Configure slosh AM: `depth` ≈ effective modulation index (the LP'd
    /// noise is normalized to unit RMS via the one-pole's noise gain).
    fn set_slosh(&mut self, depth: f32, rate_hz: f32, sr: f32) {
        let c = 1.0 - (-core::f32::consts::TAU * rate_hz / sr).exp();
        self.slosh_c = c;
        // uniform ±1 white RMS 0.577; one-pole passes ~sqrt(c/2) of it
        self.slosh_d = depth / (0.577 * (0.5 * c).sqrt().max(1e-6));
    }

    /// Ride cymbal (GM 51/59): clear inharmonic stick ping over a mid-blooming
    /// wash with a very long, frequency-sloped decay.
    ///
    /// Kit voicing follows production convention, not guesswork (the standard
    /// manufacturer voicing axes — jazz rides "dark/warm/washy", rock rides
    /// "bright/cutting ping", e.g. Zildjian K vs A lineage; Owsinski,
    /// *Recording Engineer's Handbook*, genre drum-sound notes): rock = louder
    /// stick ping + brighter contact; jazz = wash-forward, darker top, longer
    /// tail, and ~1 dB hotter because the ride carries the time.
    fn ride(vel: f32, sr: f32, seed: u32, kit: KitStyle) -> Self {
        // (ping, wash, chick, decay, amp, HF shelf >5 kHz)
        // jazz r4 (owner: "consider brushy ride behavior — wash-forward,
        // less ping"): voiced toward the Swirly/DRS brush-ride refs
        // (CC0/CC-BY-4.0): ping cluster halved, wash up, HF shelf FLIPPED
        // dark->airy (ref body leads at 4-10 kHz, -3.5 dB rel, with the
        // low gong at -22), low gong cut below. Still a stick-able ride —
        // not a literal brush patch (it keeps the time).
        let (m_ping, m_wash, m_chick, m_dec, m_amp, m_hf) = match kit {
            KitStyle::Pop => (1.0f32, 0.92f32, 1.0f32, 1.0f32, 1.0f32, 0.75f32),
            KitStyle::Rock => (1.55, 1.05, 1.25, 1.0, 1.1, 0.85),
            KitStyle::Jazz => (0.55, 1.35, 0.90, 1.08, 1.12, 1.0),
        };
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.030, sr);
        v.amp = 0.9 * m_amp * (0.25 + 0.75 * vel);
        v.life = (5.0 * sr) as u64;
        let mut jit = Lcg(seed ^ 0x51de | 1);
        // strike-impulse scale: rings must sit just above the noise-fed wash
        // bed (whose RMS is only ~gain·√(BW/nyq)) yet top the bloomed wash so
        // the envelope peaks AT the strike, not 60 ms in (iterations 7/9)
        let imp = 0.19;

        // Long-ringing ping bands: the ride's tonal skeleton, measured from the
        // CC0 reference sustain (0.15–0.65 s FFT): a beating cluster at
        // ~444/462/467 Hz (4.5 Hz beat), one strong mode ~1221 Hz, light
        // support ~2244 Hz. The spectrum DIPS at 200–350 and 500–1000 Hz.
        // (levels pre-compensated ×1.8 for the cap_strike renormalization)
        for (f, ring, burst) in [
            (402.0, 2.0, 0.13f32),
            (444.0, 2.5, 0.20),
            (462.0, 3.5, 0.34),
            (466.5, 3.0, 0.27),
            (1221.0, 2.2, 0.43),
            (2244.0, 1.4, 0.40),
        ] {
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst: burst * imp * m_ping * vel.powf(0.8),
                    chick: 0.0,
                    wash: 0.0,
                    decay_t60: ring,
                    bloom_frac: 0.0,
                    bloom_tau: 0.0,
                },
                sr,
            );
        }
        // Wash bands: geometric 300 Hz → 16 kHz with seeded jitter (the strike
        // sizzle lives at 6–18 kHz in the reference).
        let n_wash = CYM_BANDS - v.n_bands;
        for k in 0..n_wash {
            let t = k as f32 / (n_wash - 1) as f32;
            let f = 300.0 * (16000.0f32 / 300.0).powf(t) * (1.0 + 0.05 * jit.next());
            // texture: bandwidth ≈ f/20 — measured sweet spot between a sine
            // comb (f/213: flatness 0.05, reads as a chord) and hiss (f/8:
            // flatness 0.9); real ride in-band flatness is 0.2–0.5 (dense
            // modal, never white). Bands charge from noise in ~1/BW ≈ 20/f s.
            let ring = 44.0 / f;
            // decay: LF rings ~5.6 s, 12 kHz ~1.8 s (measured slope of CC0 ride)
            let decay = (5.2 * (1500.0 / f).sqrt()).clamp(1.7, 5.6) * m_dec;
            // bloom: mids arrive latest/deepest (1–2 kHz band peaks ~60 ms late)
            let (bloom_frac, bloom_tau) = if f < 700.0 {
                (0.15, 0.015)
            } else if f < 3000.0 {
                (0.85 * (0.5 + 0.5 * vel), 0.040)
            } else {
                (0.45 * (0.5 + 0.5 * vel), 0.022)
            };
            // burst: stick contact is broadband and instant (reference: every
            // band peaks in the first frame except the blooming mids), with a
            // bright bump peaked ~2.5 kHz
            // measured ride spectrum dips ~500–1000 Hz between the low "gong"
            // hump and the stick band — a plate-response property, so it
            // shapes BOTH the strike and the wash (iteration-11 measurement)
            let lnd = (f / 750.0).ln();
            let dip = 1.0 - 0.35 * (-lnd * lnd / 0.5).exp();
            // stick contact is treble-tilted: small broadband floor, strong
            // bump around 7 kHz (reference onset energy peaks at 9–13 kHz)
            let lnr = (f / 7000.0).ln();
            let burst = (0.12 + 1.6 * (-lnr * lnr / 0.9).exp()) * vel.powf(0.7) * imp * dip;
            // below ~500 Hz the real wash rolls off steeply (−26 dB/bin at
            // 200–350 Hz; the LF body is the tonal cluster, not noise); the
            // stick band ~3 kHz sits ~3 dB proud; gentle shelf above 5 kHz
            let lnw = (f / 3000.0).ln();
            let gong_roll = if matches!(kit, KitStyle::Jazz) { 2.2 } else { 1.5 };
            let shape = (1.0 + 0.45 * (-lnw * lnw / 0.8).exp())
                * if f < 500.0 { (f / 500.0).powf(gong_roll) } else { 1.0 }
                * if f > 5000.0 { m_hf } else { 1.0 };
            let wash = 0.5 * m_wash * dip * shape;
            // chick: treble-tilted contact noise, ~2× the wash gain up top
            // chick: treble-tilted contact noise, ~2x the wash gain up top
            // (r4 note: brightening this to chase the ref's -4.9 dB onset
            // sizzle WORSENED the K-weighted fit 0.96->1.08 — the attack
            // window is owned by the ping rings, so hat/ride contrast is
            // won on the hat side instead; tried and reverted)
            let chick = 1.4 * m_chick * (f / 6000.0).powf(0.8).min(2.2) * vel.powf(1.2);
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst,
                    chick,
                    wash,
                    decay_t60: decay,
                    bloom_frac,
                    bloom_tau,
                },
                sr,
            );
        }
        v.cap_strike(1.6);
        v
    }

    /// Crash cymbal (GM 49/57): explosive wash whose brightness BLOOMS after
    /// the hit (reference centroid rises 1.8→7.7 kHz into ~50 ms; every band
    /// peaks 45–170 ms late, lower-mids last), then a fast, bright decay.
    ///
    /// Kit voicing: rock crashes are bigger/heavier and mixed "washier" —
    /// longer sustain, more wash (Owsinski, genre conventions); jazz crashes
    /// are thin and fast, closer to accents than explosions.
    fn crash(vel: f32, sr: f32, seed: u32, kit: KitStyle) -> Self {
        // (wash, decay, amp, life s)
        let (m_wash, m_dec, m_amp, life_s) = match kit {
            KitStyle::Pop => (1.0f32, 1.0f32, 1.0f32, 3.8f32),
            // rock amp 1.05->1.18 (r4 "louder more aggressive" crash)
            KitStyle::Rock => (1.18, 1.25, 1.18, 4.6),
            KitStyle::Jazz => (1.0, 0.85, 0.92, 3.4),
        };
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.045, sr);
        v.amp = 1.15 * m_amp * (0.25 + 0.75 * vel);
        v.life = (life_s * sr) as u64;
        let mut jit = Lcg(seed ^ 0xc4a5 | 1);
        let imp = 0.08;

        // light tonal skeleton — crashes are wash-dominated. Rock r4: the
        // Muldjord crash OPENS with a 500-1k CLANG (-2.5 dB rel, its
        // loudest attack band; ours led at 10-16k) — a big heavy plate hit
        // hard. Skeleton x2.6 for rock only.
        let m_skel = if matches!(kit, KitStyle::Rock) { 2.6 } else { 1.0 };
        for (f, ring, burst) in [(524.0, 1.8, 0.14f32), (713.0, 1.5, 0.10), (1173.0, 1.4, 0.12)] {
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst: burst * m_skel * imp * vel.powf(0.8),
                    chick: 0.0,
                    wash: 0.0,
                    decay_t60: ring,
                    bloom_frac: 0.0,
                    bloom_tau: 0.0,
                },
                sr,
            );
        }
        let n_wash = CYM_BANDS - v.n_bands;
        for k in 0..n_wash {
            let t = k as f32 / (n_wash - 1) as f32;
            let f = 380.0 * (16000.0f32 / 380.0).powf(t) * (1.0 + 0.05 * jit.next());
            let ring = 44.0 / f;
            // faster, brighter decay than the ride: mids ~3 s, top ~1.4 s
            let decay = (2.8 * (2000.0 / f).powf(0.35)).clamp(1.4, 3.4) * m_dec;
            // deep bloom everywhere; lower-mids arrive LAST (ref: 500–1 kHz
            // band peaks at ~170 ms, HF at ~50 ms)
            // near-total bloom: the crash starts as a dark thud and the
            // brightness floods in (ref: −7 dB HF at 20 ms, full by ~50 ms)
            let depth = 0.95 * (0.75 + 0.25 * vel);
            let (bloom_frac, bloom_tau) = if f < 1000.0 {
                (depth, 0.070)
            } else if f < 2500.0 {
                (depth, 0.045)
            } else {
                (depth, 0.028)
            };
            // crash onset is DARK (ref centroid 1.8 kHz at 20 ms) — the
            // brightness arrives via the bloom, not the contact (iteration 24)
            let lnr = (f / 2500.0).ln();
            let burst = (0.12 + 0.9 * (-lnr * lnr / 1.2).exp()) * vel.powf(0.7) * imp;
            let wash = 0.62
                * m_wash
                * if f < 700.0 { (f / 700.0).powf(1.2) } else { 1.0 }
                // rock r4: sustained wash lives at 3-10 kHz (Muldjord body
                // -4.3/-4.6 dB there vs -15..-18 below 2 k)
                * if matches!(kit, KitStyle::Rock) {
                    if (3000.0..10000.0).contains(&f) { 1.4 } else if f < 1500.0 { 0.75 } else { 1.0 }
                } else {
                    1.0
                };
            let chick = 0.12 * (f / 6000.0).powf(0.8).min(2.0) * vel.powf(1.2);
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst,
                    chick,
                    wash,
                    decay_t60: decay,
                    bloom_frac,
                    bloom_tau,
                },
                sr,
            );
        }
        v.cap_strike(1.6);
        v
    }

    /// Ride BELL (GM 53): striking the stiff cup excites a sparse cluster of
    /// long-ringing partials 1.5–5 kHz (measured on the CC0 virtuosity bell,
    /// sustain FFT 0.15–0.65 s: dominant beating pair 2062/2128 Hz, support at
    /// 1578/2340/2662/2990/3360/4120/4900; in-band flatness above 3 kHz is 0.06
    /// — discrete partials, almost no noise) over only a LIGHT wash: the cup is
    /// far stiffer than the bow, so little strike energy migrates into the
    /// plate's dense mode field. Distinct from GM 51 (bow: wash-forward, low
    /// 440–467 Hz cluster) by cluster register and wash share.
    fn bell(vel: f32, sr: f32, seed: u32) -> Self {
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.020, sr);
        v.amp = 1.05 * (0.25 + 0.75 * vel);
        v.life = (4.5 * sr) as u64;
        let mut jit = Lcg(seed ^ 0xbe11 | 1);
        let imp = 0.23;
        // cup partials: (freq, ring t60, hard-hit level, soft-hit level) — the
        // soft reference redistributes energy UP (2326/4218/4900 dominate soft)
        for (f, ring, hard, soft) in [
            (1578.0, 5.5, 0.26f32, 0.10f32),
            (2062.0, 6.0, 0.48, 0.25),
            (2128.0, 4.5, 0.27, 0.24),
            (2340.0, 3.8, 0.26, 0.42),
            (2662.0, 3.2, 0.30, 0.20),
            (2990.0, 2.8, 0.33, 0.14),
            (3360.0, 2.4, 0.22, 0.30),
            (4120.0, 2.1, 0.32, 0.14),
            (4900.0, 1.8, 0.16, 0.30),
            // low plate modes: the cup strike rocks the whole cymbal — the
            // reference's 200–500 band is its LOUDEST (+11.7 dB) and rings
            // longest (t60 ≈ 9 s)
            (262.0, 8.5, 0.20, 0.08),
            (331.0, 8.0, 0.17, 0.07),
            (416.0, 7.0, 0.14, 0.06),
        ] {
            let ring = ring * (0.60 + 0.40 * vel);
            // cup partials fall steeply with velocity while the low plate
            // modes barely move (ref soft-vs-hard band levels) — extra vel^0.6
            // on the cup cluster only
            let vk = if f > 1000.0 { vel.powf(0.6) } else { 1.0 };
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst: (soft + (hard - soft) * vel) * imp * vk,
                    chick: 0.0,
                    wash: 0.0,
                    decay_t60: ring,
                    bloom_frac: 0.0,
                    bloom_tau: 0.0,
                },
                sr,
            );
        }
        // light wash bed: the plate still shivers under the cup (measured
        // 200–500 Hz band is broadband, and 1–2 kHz still blooms late), at
        // roughly a third of the bow ride's wash level, scaled up with vel
        let n_wash = CYM_BANDS - v.n_bands;
        for k in 0..n_wash {
            let t = k as f32 / (n_wash - 1) as f32;
            let f = 250.0 * (15500.0f32 / 250.0).powf(t) * (1.0 + 0.05 * jit.next());
            let ring = 44.0 / f;
            // measured bell-hit band t60s: 8.9 s at 200–500 (low plate modes
            // ring LONGEST under a cup strike), ~4 s at 8–16 k
            let decay = (5.5 * (1000.0 / f).powf(0.3)).clamp(2.2, 7.5);
            let (bloom_frac, bloom_tau) =
                if f < 3000.0 && f > 700.0 { (0.7 * (0.5 + 0.5 * vel), 0.045) } else { (0.2, 0.015) };
            let lnr = (f / 6000.0).ln();
            let burst = (0.08 + 1.0 * (-lnr * lnr / 0.9).exp()) * vel.powf(0.7) * imp * 0.5;
            let wash = 0.38
                * (0.4 + 0.6 * vel)
                * if f > 7000.0 { 1.9 } else { 1.0 }; // sustained HF sizzle (ref 8–16k −4.3 dB, t60 3.9 s)
            let chick = 1.4 * (f / 6000.0).powf(0.8).min(2.0) * vel.powf(0.5);
            v.push_band(
                CymBand { freq: f, ring_t60: ring, burst, chick, wash, decay_t60: decay, bloom_frac, bloom_tau },
                sr,
            );
        }
        v.cap_strike(1.6);
        v
    }

    /// China (GM 52): upturned-edge plate — strongly nonlinear contact floods
    /// energy across the spectrum almost instantly (bloom taus ~⅓ of a
    /// crash's), the mode field is denser/noisier (wider bands: ring 30/f vs
    /// 44/f), the low skeleton is a dissonant cluster, and the trash decays
    /// fast (~60% of crash). Rossing, Science of Percussion Instruments
    /// ch. 20 (china/swish); no license-clean reference exists (same search
    /// posture as splash, r2) — voiced from the crash under these physics.
    fn china(vel: f32, sr: f32, seed: u32) -> Self {
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.030, sr);
        v.amp = 1.1 * (0.25 + 0.75 * vel);
        v.life = (2.6 * sr) as u64;
        let mut jit = Lcg(seed ^ 0xc41a | 1);
        let imp = 0.08;
        // dissonant low cluster (near-tritone spacing, beating)
        for (f, ring, burst) in [(438.0, 1.4, 0.15f32), (593.0, 1.2, 0.13), (617.0, 1.1, 0.10)] {
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst: burst * imp * vel.powf(0.8),
                    chick: 0.0,
                    wash: 0.0,
                    decay_t60: ring,
                    bloom_frac: 0.0,
                    bloom_tau: 0.0,
                },
                sr,
            );
        }
        let n_wash = CYM_BANDS - v.n_bands;
        for k in 0..n_wash {
            let t = k as f32 / (n_wash - 1) as f32;
            let f = 500.0 * (16500.0f32 / 500.0).powf(t) * (1.0 + 0.08 * jit.next());
            let ring = 30.0 / f; // wider bands = trashier texture
            let decay = (1.7 * (2000.0 / f).powf(0.35)).clamp(0.8, 2.0);
            let depth = 0.9 * (0.75 + 0.25 * vel);
            let (bloom_frac, bloom_tau) = if f < 2500.0 { (depth, 0.016) } else { (depth, 0.009) };
            let lnr = (f / 3000.0).ln();
            let burst = (0.14 + 0.9 * (-lnr * lnr / 1.4).exp()) * vel.powf(0.7) * imp;
            let wash = 0.66 * if f < 900.0 { (f / 900.0).powf(1.2) } else { 1.0 };
            let chick = 0.3 * (f / 6000.0).powf(0.8).min(2.0) * vel.powf(1.2);
            v.push_band(
                CymBand { freq: f, ring_t60: ring, burst, chick, wash, decay_t60: decay, bloom_frac, bloom_tau },
                sr,
            );
        }
        v.cap_strike(1.6);
        v
    }

    /// Splash (GM 55): an 8–10" thin crash. No license-clean reference exists
    /// (see references ledger), so this is the crash model under plate scaling
    /// physics: modal frequencies scale ~h/d² (Rossing, Science of Percussion
    /// Instruments, ch. 20 — halving diameter ≈ 3–4× higher modes), stored
    /// energy and radiating area shrink, so decay and bloom shorten by roughly
    /// the same factor. Production convention agrees: "fast bright attack,
    /// quick decay" is THE splash descriptor (manufacturer voicing language;
    /// Owsinski, Recording Engineer's Handbook, cymbal-miking notes).
    fn splash(vel: f32, sr: f32, seed: u32) -> Self {
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.030, sr);
        v.amp = 0.95 * (0.25 + 0.75 * vel);
        v.life = (1.6 * sr) as u64;
        let mut jit = Lcg(seed ^ 0x5b1a | 1);
        let imp = 0.08;
        // scaled tonal skeleton (crash 524/1173 Hz × ~3.2)
        for (f, ring, burst) in [(1680.0, 1.0, 0.14f32), (3760.0, 0.8, 0.12)] {
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst: burst * imp * vel.powf(0.8),
                    chick: 0.0,
                    wash: 0.0,
                    decay_t60: ring,
                    bloom_frac: 0.0,
                    bloom_tau: 0.0,
                },
                sr,
            );
        }
        let n_wash = CYM_BANDS - v.n_bands;
        for k in 0..n_wash {
            let t = k as f32 / (n_wash - 1) as f32;
            let f = 1200.0 * (17000.0f32 / 1200.0).powf(t) * (1.0 + 0.05 * jit.next());
            let ring = 44.0 / f;
            // whole tail lives in ~1 s: mids ~0.9 s, top ~0.45 s
            let decay = (0.85 * (4000.0 / f).powf(0.35)).clamp(0.4, 1.0);
            // bloom compressed to ~1/3 of the crash's (smaller plate, shorter
            // energy-migration path): full depth, 10–25 ms taus
            let depth = 0.9 * (0.75 + 0.25 * vel);
            let (bloom_frac, bloom_tau) = if f < 3000.0 {
                (depth, 0.025)
            } else if f < 7000.0 {
                (depth, 0.016)
            } else {
                (depth, 0.010)
            };
            let lnr = (f / 6000.0).ln();
            let burst = (0.12 + 0.9 * (-lnr * lnr / 1.2).exp()) * vel.powf(0.7) * imp;
            let wash = 0.60 * if f < 2000.0 { (f / 2000.0).powf(1.2) } else { 1.0 };
            let chick = 0.15 * (f / 8000.0).powf(0.8).min(2.0) * vel.powf(1.2);
            v.push_band(
                CymBand { freq: f, ring_t60: ring, burst, chick, wash, decay_t60: decay, bloom_frac, bloom_tau },
                sr,
            );
        }
        v.cap_strike(1.6);
        v
    }

    /// Hi-hats (GM 42/44 closed, 46 open): small bright plates. Closed chokes
    /// in ~0.4 s with no bloom; open sizzles for seconds with a mild mid bloom
    /// (reference band t60s hump at 1–2 kHz).
    ///
    /// Velocity depth (round-1 producer finding: kit velocity was level-only):
    /// harder closed hits are brighter AND sizzle longer — plates are never
    /// fully clamped, hard sticks push them apart (CC0 closed-hat refs: onset
    /// centroid 6.9 kHz at vl3 vs 3.2 kHz at vl1). Rock hats brighter
    /// (production convention, Owsinski).
    ///
    /// Jazz (r4, owner: "hat is too hard"): measured on the DRSKit close-mic
    /// hats, the "hard" read was a LOW-MID CLANG, not brightness — the ref's
    /// 500–2 kHz body sits 6–15 dB below ours while its 4–10 kHz air leads
    /// (centroid 8.4 kHz vs our 5.7), it rings shorter (broad t60 0.28 vs
    /// 0.62 s) and the pedal clamp kills mids FASTER than the HF sizzle. So
    /// jazz = thin-plate voicing: mid-wash shelf cut, flipped closed-decay
    /// slope, softer stick (less burst, steeper treble-tilted chick).
    fn hat(open: bool, vel: f32, sr: f32, seed: u32, kit: KitStyle) -> Self {
        // (amp, chick, closed-decay, mid-wash shelf <2.5k, chick tilt exp)
        // rock r4 (owner: kit "too similar with the pop kit"): Muldjord's
        // close-mic closed hat is TIGHTER (broad t60 0.39 vs our 0.55) with
        // clamped mids (500-2k body -22 dB rel vs our -5.5) and a 4-10 kHz
        // lead — same thin-plate physics as the jazz fix, harder stick
        let (m_amp, m_chick, m_cdec, m_mid, tilt) = match kit {
            KitStyle::Pop => (1.0f32, 1.0f32, 1.0f32, 1.0f32, 0.7f32),
            KitStyle::Rock => (1.05, 1.2, 0.70, 0.55, 0.9),
            KitStyle::Jazz => (0.82, 0.60, 1.0, 0.45, 1.1),
        };
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.012, sr);
        v.amp = if open { 0.68 } else { 0.5 } * m_amp * (0.3 + 0.7 * vel);
        v.life = if open { (3.2 * sr) as u64 } else { (0.9 * sr) as u64 };
        let mut jit = Lcg(seed ^ 0x4a75 | 1);
        if open {
            // plate-collision slosh: the open pair rattles chaotically
            // (r4 hat-vs-ride axis; a ride is one stable plate — no slosh)
            // jazz thin hats slosh most; rock heavies barely move
            v.set_slosh(
                match kit {
                    KitStyle::Jazz => 0.20,
                    KitStyle::Pop => 0.15,
                    KitStyle::Rock => 0.08,
                },
                5.0,
                sr,
            );
            // tonal skeleton measured from the CC0 open-hat sustain:
            // 614/634 Hz beating pair, 1003, 1114, 1771 Hz. r4: bursts ×~2.3 —
            // the ref's ATTACK leads in the 500–1 kHz band (−4.1 dB rel, its
            // strongest) because the strike claps the plates together, a
            // low-mid CLANG the r3 render barely had (−8.5); this attack
            // clang vs the ride's HF sizzle+ping is the loudest of the
            // owner's "differentiate open hat vs ride" axes.
            let mb = match kit {
                KitStyle::Jazz => 0.6,
                KitStyle::Pop => 1.0,
                KitStyle::Rock => 1.35, // heavier plates, harder clap
            };
            for (f, ring, burst) in [
                (614.0, 2.1, 0.036f32),
                (634.0, 2.4, 0.047),
                (862.0, 2.0, 0.034),
                (1003.0, 2.2, 0.030),
                (1114.0, 2.1, 0.027),
                (1771.0, 1.8, 0.020),
            ] {
                v.push_band(
                    CymBand {
                        freq: f,
                        ring_t60: ring,
                        burst: burst * mb * vel.powf(0.8),
                        chick: 0.0,
                        wash: 0.0,
                        decay_t60: ring,
                        bloom_frac: 0.0,
                        bloom_tau: 0.0,
                    },
                    sr,
                );
            }
        }
        let n_bands = CYM_BANDS - v.n_bands;
        for k in 0..n_bands {
            let t = k as f32 / (n_bands - 1) as f32;
            let f = 550.0 * (15500.0f32 / 550.0).powf(t) * (1.0 + 0.05 * jit.next());
            let ring = (44.0 / f).min(0.03);
            let decay = if open {
                // measured hump: ~5 s at 1.6 kHz, ~1.5 s at 12 kHz; softer
                // strokes ring the open plates a little shorter. r4: hump
                // capped 4.8->3.0 s — the open hat must DIE noticeably faster
                // than the ride's wash (its plates keep grinding), another
                // hat-vs-ride axis
                let lnh = (f / 1600.0).ln();
                (0.8 + 2.7 * (-lnh * lnh / 2.4).exp()).clamp(0.8, 3.0) * (0.75 + 0.35 * vel)
            } else if matches!(kit, KitStyle::Jazz) {
                // flipped slope (DRS close mic: 1 kHz t60 0.21 s, 3k/8k
                // 0.34 s): the pedal clamp chokes the mids first, the HF
                // sizzle outlives them
                (0.30 * (f / 3000.0).powf(0.12)).clamp(0.16, 0.34) * (0.80 + 0.35 * vel)
            } else {
                // velocity-openness: hard closed hits sizzle longer
                (0.45 * (3000.0 / f).powf(0.25)).clamp(0.22, 0.55)
                    * (0.75 + 0.50 * vel)
                    * m_cdec
            };
            let (bloom_frac, bloom_tau) = if open && f > 800.0 && f < 3500.0 {
                (0.5 * (0.4 + 0.6 * vel), 0.050)
            } else {
                (0.0, 0.0)
            };
            let lnr = (f / 7000.0).ln();
            // jazz: lighter stick, softer impulse into the plates
            let m_burst = if matches!(kit, KitStyle::Jazz) { 0.6 } else { 1.0 };
            let burst = (0.10 + 1.1 * (-lnr * lnr / 1.0).exp())
                * vel.powf(0.7)
                * 0.13
                * m_burst
                * if open { 1.5 } else { 1.0 };
            let wash = 0.5
                * if f < 1200.0 { (f / 1200.0).powf(1.4) } else { 1.0 }
                // thin-plate mid cut (jazz: m_mid < 1 — the clang lived here)
                * if f < 2500.0 { m_mid } else { 1.0 }
                // r4 hat-vs-ride: the open hat BODY is mid-forward with a
                // rolled top (ref body: 500–2 k at −5.6…−5.8 dB, 10–16 k at
                // −15.3) while the ride's body peaks at 2–4 k (ping) — push
                // the sustained wash down-spectrum, keep attack HF via chick
                * if open && f < 2000.0 { 1.3 } else { 1.0 }
                * if open && f > 6000.0 { 0.7 } else { 1.0 }
                // velocity-brightness: the closed hat's top end scales with
                // velocity beyond the level curve (onset centroid doubles
                // vl1→vl3 in the refs)
                * if !open && f > 5000.0 { 0.55 + 0.55 * vel } else { 1.0 };
            let chick = if open { 1.35 } else { 1.5 }
                * m_chick
                * (f / 6000.0).powf(tilt).min(2.0)
                * vel.powf(1.1);
            v.push_band(
                CymBand {
                    freq: f,
                    ring_t60: ring,
                    burst,
                    chick,
                    wash,
                    decay_t60: decay,
                    bloom_frac,
                    bloom_tau,
                },
                sr,
            );
        }
        v.cap_strike(1.6);
        v
    }
}

// ---------------------------------------------------------------------------
// Drum kit (GM pitches) — sine-sweep kick, mode+noise snare, banded cymbals
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct DrumVoice {
    kind: DrumKind,
    phase: f32,
    freq: f32,
    freq_end: f32,
    sweep: f32,
    amp: f32,
    env: f32,
    decay: f32,
    hp: f32,
    hp_c: f32,
    noise_amt: f32,
    tone_amt: f32,
    /// snare-wire noise upper band edge (one-pole lowpass after the hp
    /// highpass = broad bandpass): wires are head-driven and radiate
    /// ~300 Hz–6 kHz, NOT white to Nyquist (white hiss was round-2's
    /// "weak" snare: all energy above the crack band)
    lp: f32,
    lp_c: f32,
    /// kick beater-click gain (velocity-shaped; hp/hp_c double as the click's
    /// brightness lowpass for the Kick kind)
    click: f32,
    /// beater contact ramp on the kick body: raised-cosine 0→1 over the
    /// contact time (felt compresses ~5–8 ms, hard beaters ~4 ms) — the refs
    /// peak 6–10 ms AFTER onset; an instant-on sine is the "electronic" edge
    atk_ph: f32,
    atk_dp: f32,
    /// beater "slap" resonator: an impulse-rung two-pole at the head's
    /// overtone region (~500–700 Hz, t60 ~30 ms) — the mid-band knock that
    /// separates a hard beater hit from the pitched fundamental (CC0 hard-kick
    /// refs hold the 400–1500 Hz attack band within ~2 dB of the fundamental)
    sl_a1: f32,
    sl_r2: f32,
    sl_y1: f32,
    sl_y2: f32,
    /// LF noise bed (r4): shell-wall + dense high-order head modes + room LF —
    /// the broadband floor under a real kick's discrete partials (close-mic
    /// refs read 40-300 Hz spectral flatness 0.03-0.10 and tone/noise ≈ -3…+2
    /// dB where the r3 model had literally zero noise after beater contact →
    /// autocorr pitch salience pinned at 1.00 = the owner's "tonality").
    /// lfn = one-pole LP state, lfn_c its coefficient, lfn_env·lfn_dec the
    /// bed's own decay envelope. Band-limited by construction (LP'd white).
    lfn: f32,
    /// second LP stage (12 dB/oct): one pole leaked audible 1-5 kHz hiss into
    /// the felt-dark jazz kick (it3: overall K-weighted logmel rose while the
    /// LF-zoom view improved)
    lfn2: f32,
    lfn_c: f32,
    lfn_env: f32,
    lfn_dec: f32,
    /// body-sine gain (kick kind, 1.0 elsewhere): a feathered jazz stroke
    /// drives the head into a modal MIX, not one clean fundamental — the DRS
    /// soft ref holds the undertone at -1.7 dB rel fund and its dominant
    /// partial drifts window-to-window
    sine_g: f32,
    modal: ModalVoice,
    has_modal: bool,
    cym: CymbalVoice,
    rng: Lcg,
    life: u64,
    age: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrumKind {
    Kick,
    Noise, // snare wires, hats
    Cymbal,
}

impl DrumVoice {
    /// Choke this drum voice (engine-level note interaction, e.g. closed hat
    /// cutting a ringing open hat).
    pub fn choke(&mut self, sr: f32) {
        if self.kind == DrumKind::Cymbal {
            self.cym.choke(sr);
        }
        self.decay = t60_gain(0.03, sr);
        self.life = self.life.min(self.age + (0.12 * sr) as u64);
    }

    pub fn start(gm_note: u32, vel: f32, sr: f32, seed: u32, kit: KitStyle) -> Self {
        let mut v = Self {
            kind: DrumKind::Noise,
            phase: 0.0,
            freq: 0.0,
            freq_end: 0.0,
            sweep: 0.0,
            amp: vel,
            env: 1.0,
            decay: t60_gain(0.2, sr),
            hp: 0.0,
            hp_c: 0.2,
            noise_amt: 1.0,
            tone_amt: 0.0,
            lp: 0.0,
            lp_c: 1.0, // pass-through unless a kind sets a band edge
            click: 0.0,
            atk_ph: 1.0,
            atk_dp: 0.0,
            sl_a1: 0.0,
            sl_r2: 0.0,
            sl_y1: 0.0,
            sl_y2: 0.0,
            lfn: 0.0,
            lfn2: 0.0,
            lfn_c: 0.0,
            lfn_env: 0.0,
            lfn_dec: 0.0,
            sine_g: 1.0,
            modal: ModalVoice::start(200.0, 0.0, sr, &[], 1.0, 0.0, 0.0, seed),
            has_modal: false,
            cym: CymbalVoice::new(seed),
            rng: Lcg(seed | 1),
            life: (0.6 * sr) as u64,
            age: 0,
        };
        match gm_note {
            35 | 36 => {
                // Kick — three genuinely different INSTRUMENTS per kit (owner
                // verdict 2026-07-12: the r3 kicks were one recorded drum
                // re-voiced three ways). Each arm is fit to its own
                // license-clean reference drum (ledger:
                // scratchpad references/kick/SOURCES.txt):
                //   Jazz — 18"-class open bop kick, felt beater: DrumGizmo
                //     DRSKit close kick mic (CC-BY-4.0).
                //   Rock — 22" pillow-damped, hard-beater: DrumGizmo
                //     MuldjordKit (Tama Superstar) close mic (CC-BY-4.0),
                //     Naked Drums 22" DI as sub anchor (CC-BY-4.0).
                //   Pop  — unchanged r3 fit (CC0 virtuosity kick).
                // Shared physics both arms keep (de-danced r3 law): tension-
                // modulation pitch glide decays with amplitude SQUARED
                // (Rossing/Fletcher membrane nonlinearity) → tau ~5-6 ms,
                // start elevation <= ~1.35x; anything slower reads as a 909.
                v.kind = DrumKind::Kick;
                match kit {
                    KitStyle::Jazz => {
                        // MEMBRANE-MODAL topology: the voice is a ringing
                        // coupled-head pair, not a thump machine. Measured on
                        // DRSKit close mic: partial ladder 70-86 Hz fund with
                        // {1.36, 1.79, 2.07, 2.48, 2.98} overtones (air-loaded
                        // two-head membrane — compressed vs ideal-membrane
                        // ratios, Rossing ch.3), a WEAK ~0.66x undertone
                        // (both-heads-in-phase air mode) that is prominent on
                        // soft hits, fundamental t60 0.5-0.8 s (no pillow),
                        // centroid(30 ms) 70-85 Hz (felt = almost no click),
                        // crest 4.5-8 (round, never spiky).
                        let f1 = 76.0;
                        v.freq = f1 * (1.08 + 0.12 * vel);
                        v.freq_end = f1;
                        v.sweep = (-1.0 / (0.005 * sr)).exp();
                        // amplitude-dependent damping: soft strokes ring
                        // relatively longer (DRS LF at 0.3 s: -9 dB soft,
                        // -17 mid, -21.5 hard rel the first 30 ms)
                        v.decay = t60_gain(0.78 - 0.25 * vel, sr);
                        // felt beater: long compliant contact, 10 ms base
                        let contact_ms = 10.0 * (1.35 - 0.5 * vel).max(0.5);
                        v.atk_ph = 0.0;
                        v.atk_dp = 1000.0 / (contact_ms * sr);
                        // feathering law: jazz players live at pp — measured
                        // DRS layers span only ~10.6 LU (soft->hard), i.e.
                        // amp is ~linear in vel, NOT the rock/pop supralinear
                        // (r4: x(1.30-0.30v) refunds the sine_g duck's ~1.6 LU cost at
                        // pp so the measured feathering span stays ~16 LU)
                        v.amp = 0.85 * vel.powf(1.05) * (1.30 - 0.30 * vel);
                        // felt click: tiny and dark (2-6 kHz sits ~-33 dB
                        // under LF in the refs' first 30 ms)
                        v.click = 0.06 + 0.30 * vel * vel;
                        v.hp_c = 0.05 + 0.20 * vel;
                        // felt-on-head knock, mild (400-1500 Hz band reads
                        // -19..-24 dB rel LF on the refs; it2 was +17 dB hot
                        // at a=0.86/t60 0.3 — felt barely knocks)
                        let sf = 700.0;
                        let r = t60_gain(0.15, sr);
                        let w = core::f32::consts::TAU * sf / sr;
                        v.sl_a1 = 2.0 * r * w.cos();
                        v.sl_r2 = r * r;
                        let a = 0.18 * vel.powf(1.05);
                        let phi = core::f32::consts::PI * Lcg(seed ^ 0x51a9 | 1).next();
                        v.sl_y1 = a * (phi - w).sin();
                        v.sl_y2 = a * (phi - 2.0 * w).sin();
                        // LF noise bed (r4 "very subtle" verdict): the DRS
                        // close-mic floor is even stronger than pop's ref
                        // (tone/noise +1.6…+2.2 dB, LF flatness 0.03-0.08) —
                        // darker (140 Hz) and longer (0.5 s: open shell)
                        v.lfn_c = 1.0 - (-core::f32::consts::TAU * 140.0 / sr).exp();
                        v.lfn_env = 3.4 * (0.85 + 0.15 * vel);
                        // bed decays WITH the drum (amplitude-dependent
                        // damping law above): a fixed 0.5 s bed died under
                        // the pp fundamental (t60 0.72) and left the feathered
                        // tail a naked sine again (it3 measurement)
                        v.lfn_dec = t60_gain(0.74 - 0.24 * vel, sr);
                        // feathered strokes read as a modal mix, not one
                        // clean fundamental: duck the body sine at pp (the
                        // undertone/partner cluster below rises to meet it)
                        v.sine_g = 0.62 + 0.38 * vel;
                        v.has_modal = true;
                        v.modal = ModalVoice::start(
                            f1,
                            vel,
                            sr,
                            &[
                                // undertone: soft-forward (soft ref holds it
                                // at -1.7 dB rel fund; hard at -13) — r4 trim:
                                // at pp it matched the fundamental and the two
                                // sines locked a "crazy clear" pitch (owner);
                                // salience mean 0.87 vs the DRS ref's 0.70
                                // …and it is the LONG survivor: the DRS
                                // tail wanders BELOW the fundamental (pitch
                                // track 52->44->40 Hz over 0.1-0.4 s), so the
                                // 0.6 s two-stage ring is this inharmonic air
                                // mode, not a clean near-fundamental sine
                                ModeDef {
                                    ratio: 0.66,
                                    amp: 0.55 * (1.15 - 0.6 * vel),
                                    t60: 0.95,
                                },
                                // resonant-head partner: the open two-stage
                                // tail (rings past 0.6 s at every velocity).
                                // r4: detuned off the fundamental (1.04->1.035
                                // = a slow unlocked beat) and reined in from
                                // t60 1.45-0.40v — a feathered stroke rang a
                                // clean 79 Hz sine for 1.3 s
                                ModeDef {
                                    ratio: 1.035,
                                    amp: 0.55 * (1.25 - 0.5 * vel),
                                    t60: 0.95 - 0.28 * vel,
                                },
                                // overtone ladder measured on the ff DRS hit
                                // (101/125/145/174/209 Hz over 76): the open
                                // heads hold these within -8..-25 dB of the
                                // fundamental for the whole first half-second
                                ModeDef { ratio: 1.33, amp: 0.70, t60: 0.90 },
                                ModeDef { ratio: 1.65, amp: 0.55, t60: 0.75 },
                                ModeDef { ratio: 1.91, amp: 0.42, t60: 0.65 },
                                ModeDef { ratio: 2.28, amp: 0.40, t60: 0.55 },
                                ModeDef { ratio: 2.75, amp: 0.25, t60: 0.45 },
                            ],
                            0.6,
                            0.0,
                            0.0,
                            seed ^ 0x6b1c,
                        );
                        v.life = (1.40 * sr) as u64;
                    }
                    KitStyle::Rock => {
                        // BEATER-TRANSIENT topology: hard slap + shell knock
                        // over a short damped sub thump. Measured on the
                        // MuldjordKit close mic: deep partials {~0.65, 1,
                        // 1.72, 2.5}x of ~48 Hz, 2-6 kHz beater band at
                        // -16.5 dB rel LF in the first 30 ms of ff (still
                        // -21 dB at mf — the click is ALWAYS there), LF dead
                        // ~29 dB down by 0.3 s (pillow), crest 15 at ff.
                        let f1 = 48.0;
                        v.freq = f1 * (1.12 + 0.20 * vel);
                        v.freq_end = f1;
                        v.sweep = (-1.0 / (0.006 * sr)).exp();
                        // temporal shape: fast-decaying main thump with a
                        // quiet slow LF residue (ring mode below); softer
                        // hits ring freer (ref soft t60 1.04 vs hard 0.76)
                        v.decay = t60_gain(0.75 - 0.26 * vel, sr);
                        // hard beater: stiff, short contact
                        let contact_ms = 2.5 * (1.35 - 0.5 * vel).max(0.5);
                        v.atk_ph = 0.0;
                        v.atk_dp = 1000.0 / (contact_ms * sr);
                        // stomp law: rock kicks are played loud; the head
                        // saturates at the top (measured +9.2 dB soft->mid
                        // but only +2.3 dB mid->hard) — compressive knee
                        v.amp = 2.3 * vel.powf(1.5) / (1.0 + 1.3 * vel * vel);
                        // beater slap: bright, velocity-squared. The contact
                        // burst alone is ~3 ms — the measured 2-6 kHz band
                        // (-16.5 dB rel LF over 30 ms at ff) comes from the
                        // batter head's HF modal cluster ringing on after the
                        // beater leaves (pillow-damped t60 ~25 ms): post-
                        // contact click ring via lp (level) / lp_c (decay)
                        v.click = 0.10 + 3.20 * vel * vel;
                        // the ref's attack noise RISES toward 2-6 kHz (hard
                        // beater on a coated head): highpassed click (flag =
                        // tone_amt) with ~1.2 kHz cut, vs the felt voicings'
                        // lowpassed thud
                        v.tone_amt = 1.0;
                        v.hp_c = 0.08 + 0.10 * vel;
                        v.lp = 0.8;
                        v.lp_c = t60_gain(0.025, sr);
                        // shell knock resonator (~540 Hz), damped fast
                        // (it2: a=1.53 measured +28 dB over the close-mic ref)
                        let sf = 540.0;
                        let r = t60_gain(0.085, sr);
                        let w = core::f32::consts::TAU * sf / sr;
                        v.sl_a1 = 2.0 * r * w.cos();
                        v.sl_r2 = r * r;
                        let a_raw = 0.08 * vel.powf(1.05);
                        let a = a_raw / (1.0 + 0.18 * a_raw);
                        let phi = core::f32::consts::PI * Lcg(seed ^ 0x51a9 | 1).next();
                        v.sl_y1 = a * (phi - w).sin();
                        v.sl_y2 = a * (phi - 2.0 * w).sin();
                        v.has_modal = true;
                        let ms = 0.75 + 0.45 * vel; // mode-sustain vel scale
                        v.modal = ModalVoice::start(
                            f1,
                            vel,
                            sr,
                            &[
                                // sub undertone (in-phase air mode, ~31 Hz —
                                // the strongest sustained partial on the
                                // close-mic ref's 0.15-0.55 s window). Fast-
                                // mode sustain scales with velocity (hard
                                // strokes load the head, loosening the
                                // pillow's grip — it6/it8 per-layer fits)
                                ModeDef { ratio: 0.65, amp: 0.40, t60: 0.42 * ms },
                                // quiet SLOW tail (ref: ~-21 dB residue at
                                // t60 ~1 s under the fast thump), kept well
                                // under the jazz kick's open ring at 0.6 s
                                // (two-stage kit contrast)
                                ModeDef { ratio: 1.04, amp: 0.12, t60: 0.90 },
                                // 83/121 Hz pair: the pillow kills >150 Hz
                                // but the ref holds these at -7 dB rel fund
                                // through the half-second window
                                ModeDef { ratio: 1.72, amp: 0.85, t60: 0.38 * ms },
                                ModeDef { ratio: 2.52, amp: 1.80, t60: 0.34 * ms },
                                // wood shell knock + upper head cluster
                                // (a 22" head's (1,2)/(3,1)-family modes at
                                // 150-250 Hz ring through the ref's tail)
                                ModeDef { ratio: 3.40, amp: 0.30, t60: 0.30 },
                                ModeDef { ratio: 3.90, amp: 0.35, t60: 0.45 },
                            ],
                            0.6,
                            0.0,
                            0.0,
                            seed ^ 0x6b1c,
                        );
                        v.life = (0.75 * sr) as u64;
                    }
                    KitStyle::Pop => {
                        // r3 fit + r4 tonality trim only (owner 2026-07-12:
                        // "kick: reduce the tonality" — the thump/click stay)
                        let f1 = 54.0;
                        v.freq = f1 * (1.10 + 0.22 * vel);
                        v.freq_end = f1;
                        v.sweep = (-1.0 / (0.006 * sr)).exp();
                        v.decay = t60_gain(0.22, sr);
                        let contact_ms = 5.0 * (1.35 - 0.5 * vel).max(0.5);
                        v.atk_ph = 0.0;
                        v.atk_dp = 1000.0 / (contact_ms * sr);
                        v.amp = vel.powf(1.7) * 0.9;
                        v.click = 0.22 + 0.85 * vel * vel;
                        v.hp_c = 0.08 + 0.55 * vel;
                        let (sf, sa, open) = (620.0, 2.0, 1.6f32);
                        let r = t60_gain(0.060 * open, sr);
                        let w = core::f32::consts::TAU * sf / sr;
                        v.sl_a1 = 2.0 * r * w.cos();
                        v.sl_r2 = r * r;
                        let a_raw = sa * vel.powf(1.05) / (open / 1.6f32).sqrt();
                        let a = a_raw / (1.0 + 0.18 * a_raw);
                        let phi = core::f32::consts::PI * Lcg(seed ^ 0x51a9 | 1).next();
                        v.sl_y1 = a * (phi - w).sin();
                        v.sl_y2 = a * (phi - 2.0 * w).sin();
                        // LF noise bed: fc ~420 Hz (the overhead-mic'd CC0 ref
                        // hears brighter kit/room rustle than a close mic —
                        // the darker 170 Hz bed measurably WORSENED the fit),
                        // t60 ~0.35 s — the floor the ref keeps under its
                        // partials (tone/noise -3.5 dB; r3 had literally none)
                        v.lfn_c = 1.0 - (-core::f32::consts::TAU * 420.0 / sr).exp();
                        // near-flat velocity law: the ref's SOFT hit is the
                        // less tonal one (salience 0.55 vs 0.71 hard) — a
                        // light stroke excites the fundamental less cleanly,
                        // it does not remove the shell/room floor
                        v.lfn_env = 1.5 * (0.85 + 0.15 * vel);
                        v.lfn_dec = t60_gain(0.35, sr);
                        v.has_modal = true;
                        v.modal = ModalVoice::start(
                            f1,
                            vel,
                            sr,
                            &[
                                // r4 tonality trim: the resonant-head partner
                                // rang a CLEAN 56 Hz sine for half a second
                                // after the thump died (autocorr salience 1.00
                                // at 100-400 ms vs the CC0 ref's 0.71;
                                // tone/noise +10.4 dB vs the ref's -3.5).
                                // Damped 0.50->0.26 s, 0.30->0.22, detuned
                                // 1.04->1.046 so the residue beats against the
                                // fundamental instead of locking one pitch.
                                ModeDef { ratio: 1.046, amp: 0.22, t60: 0.26 },
                                ModeDef { ratio: 1.58, amp: 0.5, t60: 0.055 * open },
                                ModeDef { ratio: 3.4, amp: 0.28, t60: 0.035 * open },
                            ],
                            0.6,
                            0.0,
                            0.0,
                            seed ^ 0x6b1c,
                        );
                        v.life = ((0.22f32 * 1.6).max(0.50 * 1.05).max(0.5) * sr) as u64;
                    }
                }
            }
            38 | 40 => {
                // Jazz GM 38 = BRUSH tap (GM 40 keeps the stick jazz snare).
                // A brush is dozens of thin bristles arriving distributed over
                // 10–30 ms: noise-dominated, soft onset, little modal ping —
                // the defining jazz comp texture. Measured on the CC0
                // Frankensnare brush refs (close mic): time-to-peak 22 ms
                // (hard) to 42 ms (soft) vs 4 ms for sticks; soft taps
                // wire-bright (cen30 ~3.3 kHz), hard taps head-forward
                // (~0.9 kHz); t60 0.18–0.24 s.
                let brush = matches!(kit, KitStyle::Jazz) && gm_note == 38;
                if brush {
                    let dyn_g = 0.30 + 0.70 * vel;
                    v.decay = t60_gain(0.21, sr);
                    // wire band closes with velocity: light taps are bright
                    // bristle-on-wire sizzle (ref cen30 ~3.3 kHz), hard taps
                    // drive the head and read dark (~0.9 kHz)
                    let lp_hz = 5800.0 - 3800.0 * vel;
                    v.hp_c = 1.0 - (-core::f32::consts::TAU * 400.0 / sr).exp();
                    v.lp_c = 1.0 - (-core::f32::consts::TAU * lp_hz / sr).exp();
                    v.amp = 0.75 * dyn_g;
                    v.noise_amt = 0.30 + 0.55 * vel;
                    // bristle-arrival ramp on the noise (soft attack)
                    let arrive_ms = 30.0 * (1.35 - 0.6 * vel).max(0.4);
                    v.atk_ph = 0.0;
                    v.atk_dp = 1000.0 / (arrive_ms * sr);
                    // quiet head tone: a plain enveloped sine gated by the
                    // SAME bristle ramp — a modal bank fed a long soft pulse
                    // leaks the pulse shape as a sub-200 Hz "pat" thump at
                    // t≈4 ms (measured; quasi-static resonator response), so
                    // no ModalVoice here. Tone grows with velocity (hard taps
                    // read head-forward in the refs).
                    v.freq = 214.0;
                    v.tone_amt = 0.04 + 0.42 * vel;
                    v.life = (0.40 * sr) as u64;
                    return v;
                }
                // Snare, kit-voiced: shell fundamental + coupled-head partials
                // (ratios 1.3–3.0 measured on the CC0 virtuosity snare, 182 Hz
                // fundamental). Conventions: pop = tight/bright/damped; rock =
                // deeper shell tuned lower, longer ring; jazz = tuned high with
                // the shell singing through the wires (Drum Tuning Bible;
                // Owsinski). Velocity: wires dominate soft hits, shell tone
                // grows with velocity (soft ref is relatively wire-bright) —
                // wires get a level floor, the modal shell scales with vel.
                // (shell Hz, decay, wire band lo Hz, wire band hi Hz,
                //  noise base, noise vel span, modal gain, life s)
                // rock row hardened 2026-07-12 (owner: "too weak"); round 3
                // moved the wires from white hiss into a 300 Hz–6.5 kHz band
                // (head-driven wires; refs hold 400–1500 Hz at +14…+23 dB
                // over LF while our hiss put it 18 dB UNDER)
                // r4 (owner: rock kit "too similar with the pop kit"): the
                // rock snare is now ITS OWN DRUM, fit to the MuldjordKit
                // (Tama Superstar) close-mic snare, CC-BY-4.0 (ledger:
                // scratchpad references/rockkit/SOURCES.txt) — not a pop
                // re-voicing. Measured: 200–500 Hz owns attack AND body
                // (−1 dB rel; centroid 560–635 Hz vs our old 2–4.7 kHz!),
                // dominant partial 447 Hz ((1,1) batter cluster) with
                // 694/806 support over a 191/215/312 low cluster, ring t60
                // ~1.0 s, wires dark and sustained. Soft hits flip emphasis
                // to the low cluster (199/229/309 lead, 447 at −8.6).
                if matches!(kit, KitStyle::Rock) {
                    let dyn_g = 0.30 + 0.70 * vel;
                    v.decay = t60_gain(0.55, sr);
                    v.hp_c = 1.0 - (-core::f32::consts::TAU * 250.0 / sr).exp();
                    // wires: dark, 12 dB/oct (second stage in render), and
                    // the band OPENS with velocity — the soft ref is nearly
                    // all drum (2-4 kHz at -20 dB rel), hard adds rattle
                    let wire_hz = 1100.0 + 1200.0 * vel;
                    v.lp_c = 1.0 - (-core::f32::consts::TAU * wire_hz / sr).exp();
                    v.lfn_c = v.lp_c;
                    v.amp = 1.45 * dyn_g;
                    v.noise_amt = 0.10 + 0.55 * vel;
                    v.has_modal = true;
                    // crack cluster grows with velocity well beyond the
                    // ModalVoice brightness law (ref: 447 Hz moves from
                    // −8.6 dB rel the low cluster at soft to +14 at hard)
                    let ck = 0.30 + 0.85 * vel;
                    v.modal = ModalVoice::start(
                        195.0,
                        vel,
                        sr,
                        &[
                            ModeDef { ratio: 1.0, amp: 0.55 * dyn_g, t60: 0.75 },
                            ModeDef { ratio: 1.10, amp: 0.45 * dyn_g, t60: 0.90 },
                            ModeDef { ratio: 1.58, amp: 0.50 * dyn_g, t60: 0.65 },
                            ModeDef { ratio: 2.29, amp: 1.40 * ck * dyn_g, t60: 1.00 },
                            ModeDef { ratio: 2.94, amp: 0.40 * ck * dyn_g, t60: 0.70 },
                            ModeDef { ratio: 3.56, amp: 0.90 * ck * dyn_g, t60: 1.40 },
                            ModeDef { ratio: 4.13, amp: 1.00 * ck * dyn_g, t60: 1.20 },
                        ],
                        0.85,
                        0.0,
                        0.0,
                        seed ^ 0x9e37,
                    );
                    v.life = (1.1 * sr) as u64;
                    return v;
                }
                let (shell, dec, hp_hz, lp_hz, n0, nv, mg, life) = match kit {
                    KitStyle::Pop => (186.0, 0.14, 400.0, 6000.0, 0.30, 0.60, 0.40, 0.32),
                    KitStyle::Rock => unreachable!("rock snare handled above"),
                    KitStyle::Jazz => (214.0, 0.20, 450.0, 6500.0, 0.24, 0.55, 0.60, 0.40),
                };
                v.decay = t60_gain(dec, sr);
                v.hp_c = 1.0 - (-core::f32::consts::TAU * hp_hz / sr).exp();
                v.lp_c = 1.0 - (-core::f32::consts::TAU * lp_hz / sr).exp();
                // velocity mostly lives HERE (wires floor + span) and in the
                // shell's modal excitation — plus a mild global dynamic factor
                // (refs span ~22 dB vl6→vl34; wires-only gave 8) that keeps
                // the wire/shell balance (soft hits stay wire-forward, as in
                // the CC0 soft snare).
                let dyn_g = 0.30 + 0.70 * vel;
                v.amp = 0.95 * dyn_g;
                v.noise_amt = n0 + nv * vel;
                v.has_modal = true;
                let dl = dec / 0.14; // shell ring scales with the kit's looseness
                // dyn_g on every mode amp = same global dynamic factor as the
                // wires: keeps the soft-hit wire/shell balance while widening
                // the total span (mg is the strike pulse LENGTH — don't scale it)
                v.modal = ModalVoice::start(
                    shell,
                    vel,
                    sr,
                    &[
                        ModeDef { ratio: 1.0, amp: 1.0 * dyn_g, t60: 0.11 * dl },
                        ModeDef { ratio: 1.55, amp: 0.6 * dyn_g, t60: 0.08 * dl },
                        ModeDef { ratio: 2.1, amp: 0.3 * dyn_g, t60: 0.06 * dl },
                        // coupled-head "crack" modes: the ring at 400–1300 Hz
                        // ((2,1)/(0,2)/(3,1)-family, air-load stretched) that
                        // the refs hold +14…+23 dB over LF — strongly velocity
                        // -grown (ModalVoice brightens uppers ~vel^(1+))
                        ModeDef { ratio: 2.65, amp: 0.55 * dyn_g, t60: 0.055 * dl },
                        ModeDef { ratio: 3.3, amp: 0.50 * dyn_g, t60: 0.045 * dl },
                        ModeDef { ratio: 4.4, amp: 0.35 * dyn_g, t60: 0.035 * dl },
                    ],
                    mg,
                    0.0,
                    0.0,
                    seed ^ 0x9e37,
                );
                v.life = (life * sr) as u64;
            }
            42 | 44 => {
                // closed hat (44 = pedal: slightly softer/shorter via velocity)
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::hat(false, if gm_note == 44 { vel * 0.8 } else { vel }, sr, seed, kit);
                v.life = v.cym.life;
            }
            46 => {
                // open hat
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::hat(true, vel, sr, seed, kit);
                v.life = v.cym.life;
            }
            49 | 57 => {
                // crash
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::crash(vel, sr, seed, kit);
                v.life = v.cym.life;
            }
            51 | 59 => {
                // ride BOW: banded-noise resonator bank (see CymbalVoice) — the old
                // 7-mode cluster + hiss read as a test tone (owner note 2026-07-11)
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::ride(vel, sr, seed, kit);
                v.life = v.cym.life;
            }
            53 => {
                // ride BELL: cup ping cluster, light wash
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::bell(vel, sr, seed);
                v.life = v.cym.life;
            }
            52 => {
                // china: trashy fast-flood accent
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::china(vel, sr, seed);
                v.life = v.cym.life;
            }
            55 => {
                // splash: small fast crash
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::splash(vel, sr, seed);
                v.life = v.cym.life;
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                // Toms, kit-voiced (round 3): reuse the acoustic kick machinery
                // (swept body + contact ramp + force click + overtone modal).
                // Tunings follow size/genre convention (Drum Tuning Bible;
                // Owsinski): rock 12/13/16" tuned LOW with long sustain, jazz
                // 8/12/14" tuned a fourth-ish higher, shorter and darker, pop
                // between. GM ladder 41→50 spans floor→high rack.
                let idx = match gm_note {
                    41 => 0,
                    43 => 1,
                    45 => 2,
                    47 => 3,
                    48 => 4,
                    _ => 5,
                } as f32;
                // (f of GM41, per-step ratio, t60, click brightness, amp)
                // r4 rock refit vs MuldjordKit close-mic toms (CC-BY-4.0,
                // references/rockkit/SOURCES.txt): floor tom fund 66 Hz,
                // rack 87.5 — big drums tuned LOW and CLOSE together
                // (f_lo 56.5 / step 1.12 puts GM43 at 63 Hz and GM48 at
                // 89), ringing t60 1.6-4 s where our r3 tom died at 0.75 s
                // jazz r4 (owner: "how about making others all played by
                // brush?"): jazz toms are BRUSH TAPS now — measured on the
                // Swirly Drums brush kit (CC0) + DRSKit whisker toms
                // (CC-BY-4.0, references/jazzkit/SOURCES.txt): bristle
                // arrival 8-20 ms (vs 2-4 ms stick), attack = broadband
                // 500-4k scrape with rolled top (a "shhk", no click), body
                // skin-dominant (>=1 kHz collapses -19..-37 dB while
                // 200-500 leads), and the undamped head RINGS (t60 1.7 s).
                let (f_lo, step, t60, ckb, amp) = match kit {
                    KitStyle::Pop => (70.0, 1.165f32, 0.38, 0.45, 0.85),
                    KitStyle::Rock => (56.5, 1.120, 1.50, 0.50, 0.95),
                    KitStyle::Jazz => (82.0, 1.160, 1.00, 0.45, 0.75),
                };
                let f1 = f_lo * step.powf(idx);
                v.kind = DrumKind::Kick;
                v.freq_end = f1;
                // gentle head bend: much smaller and a bit slower than a kick
                v.freq = f1 * (1.04 + 0.06 * vel);
                v.sweep = (-1.0 / (0.020 * sr)).exp();
                v.decay = t60_gain(t60, sr);
                let dyn_g = 0.30 + 0.70 * vel;
                v.amp = amp * dyn_g;
                // contact: stick = ~3 ms ramp + force click; jazz brush =
                // 16 ms bristle arrival + scrape noise, no click snap
                let brush = matches!(kit, KitStyle::Jazz);
                let contact_ms = if brush { 30.0 * (1.35 - 0.6 * vel).max(0.4) } else { 3.0 * (1.35 - 0.5 * vel).max(0.5) };
                v.atk_ph = 0.0;
                v.atk_dp = 1000.0 / (contact_ms * sr);
                v.click = if brush { 1.9 + 2.1 * vel } else { 0.10 + 0.45 * vel * vel };
                v.hp_c = 0.06 + ckb * vel;
                if brush {
                    // bristles SETTLE on the head after the tap (brush
                    // technique): quiet lowpassed scrape residue via the
                    // post-contact ring path (the Swirly refs hold 500-1k
                    // at -7 dB rel through the body window)
                    v.lp = 0.22;
                    v.lp_c = t60_gain(0.15, sr);
                }
                // stick "thwack": head-knock resonator tracked to the drum
                // (~4.5×f1, clamped to the 450–950 Hz knock region) — low toms
                // otherwise have NO energy above 400 Hz (modal tops out at
                // 2.14×f1) and read as sine blobs
                let sf = (4.5 * f1).clamp(450.0, 950.0);
                let sa = match kit {
                    KitStyle::Jazz => 0.0, // brush: no stick thwack ring
                    // r4: Muldjord attack holds 500-1k at -8 dB rel (big
                    // stick, coated head) — 1.5 measured 11 dB shy
                    KitStyle::Rock => 2.8,
                    KitStyle::Pop => 1.5,
                };
                let r = t60_gain(0.050, sr);
                let w = core::f32::consts::TAU * sf / sr;
                v.sl_a1 = 2.0 * r * w.cos();
                v.sl_r2 = r * r;
                let a = sa * vel.powf(1.4);
                let phi = core::f32::consts::PI * Lcg(seed ^ 0x70a9 | 1).next();
                v.sl_y1 = a * (phi - w).sin();
                v.sl_y2 = a * (phi - 2.0 * w).sin();
                // membrane overtones + a resonant-head ring like the kick's
                // two-stage tail. Rock gets the ladder MEASURED on the
                // Muldjord rack tom (ratios 1.29/1.66/2.0/2.32/2.65/3.12
                // held at -13..-28 dB rel fund through the 0.25-0.75 s
                // window — the r3 render's overtones sat at -67 dB there,
                // a sine blob); pop/jazz keep the generic 1.59/2.14 series.
                v.has_modal = true;
                if matches!(kit, KitStyle::Rock) {
                    v.modal = ModalVoice::start(
                        f1,
                        vel,
                        sr,
                        &[
                            ModeDef { ratio: 1.05, amp: 0.35 * dyn_g, t60: t60 * 1.2 },
                            ModeDef { ratio: 1.29, amp: 0.35 * dyn_g, t60: 1.30 },
                            ModeDef { ratio: 1.66, amp: 0.65 * dyn_g, t60: 1.50 },
                            ModeDef { ratio: 2.00, amp: 0.30 * dyn_g, t60: 1.00 },
                            ModeDef { ratio: 2.32, amp: 0.40 * dyn_g, t60: 1.00 },
                            ModeDef { ratio: 2.65, amp: 0.35 * dyn_g, t60: 0.90 },
                            ModeDef { ratio: 3.12, amp: 0.55 * dyn_g, t60: 1.20 },
                        ],
                        0.6,
                        0.0,
                        0.0,
                        seed ^ 0x70e5,
                    );
                } else {
                    v.modal = ModalVoice::start(
                        f1,
                        vel,
                        sr,
                        &[
                            ModeDef { ratio: 1.05, amp: 0.35 * dyn_g, t60: t60 * 1.3 },
                            ModeDef { ratio: 1.59, amp: 0.5 * dyn_g, t60: t60 * 0.45 },
                            ModeDef { ratio: 2.14, amp: 0.25 * dyn_g, t60: t60 * 0.28 },
                        ],
                        0.6,
                        0.0,
                        0.0,
                        seed ^ 0x70e5,
                    );
                }
                v.life = ((t60 * 1.5).max(0.45) * sr) as u64;
            }
            _ => {
                // tom-ish fallback: pitched mode by GM note
                v.has_modal = true;
                v.decay = t60_gain(0.25, sr);
                v.noise_amt = 0.25;
                let f = midi_to_hz(gm_note as f32) * 0.5;
                v.modal = ModalVoice::start(
                    f.clamp(60.0, 400.0),
                    vel,
                    sr,
                    &[
                        ModeDef { ratio: 1.0, amp: 1.0, t60: 0.3 },
                        ModeDef { ratio: 1.5, amp: 0.4, t60: 0.15 },
                    ],
                    0.8,
                    0.15,
                    0.0,
                    seed ^ 0x51ed,
                );
                v.life = (0.5 * sr) as u64;
            }
        }
        v
    }

    pub fn render(&mut self, out: &mut [f32], sr: f32) -> bool {
        if self.kind == DrumKind::Cymbal {
            return self.cym.render(out);
        }
        let dt = 1.0 / sr;
        for o in out.iter_mut() {
            let mut s;
            match self.kind {
                DrumKind::Kick => {
                    self.freq = self.freq_end + (self.freq - self.freq_end) * self.sweep;
                    self.phase = (self.phase + self.freq * dt).fract();
                    // raised-cosine beater-contact ramp (body only: click IS
                    // the contact, slap resonator IS the contact knock)
                    let atk = if self.atk_ph < 1.0 {
                        let g = 0.5 * (1.0 - (core::f32::consts::PI * self.atk_ph).cos());
                        self.atk_ph += self.atk_dp;
                        g
                    } else {
                        1.0
                    };
                    s = (core::f32::consts::TAU * self.phase).sin() * self.env * atk * self.sine_g;
                    // beater-contact click: velocity-shaped gain (click) through
                    // a velocity-opened lowpass (hp/hp_c), modulated by the
                    // contact FORCE pulse sin²(π·ph) — noise exists only while
                    // beater and head touch, peaking mid-contact (felt thud at
                    // pp, hard slap at ff)
                    // click noise tilt: tone_amt > 0.5 selects HIGHPASSED
                    // noise (hard beater — spectrum rises toward 2-6 kHz),
                    // else the lowpassed felt/plastic thud
                    if self.atk_ph < 1.0 {
                        let force = (core::f32::consts::PI * self.atk_ph).sin();
                        let n = self.rng.next();
                        self.hp += self.hp_c * (n - self.hp);
                        let cn = if self.tone_amt > 0.5 { n - self.hp } else { self.hp };
                        s += self.click * cn * force * force;
                    } else if self.lp > 1e-4 {
                        // post-contact beater ring (rock voicing): the batter
                        // head's HF modal cluster keeps sounding briefly after
                        // the beater leaves — lp is its level, lp_c its decay
                        let n = self.rng.next();
                        self.hp += self.hp_c * (n - self.hp);
                        let cn = if self.tone_amt > 0.5 { n - self.hp } else { self.hp };
                        s += self.click * cn * self.lp;
                        self.lp *= self.lp_c;
                    }
                    // beater slap ring (impulse initial condition, decays on its own)
                    let y = self.sl_a1 * self.sl_y1 - self.sl_r2 * self.sl_y2;
                    self.sl_y2 = self.sl_y1;
                    self.sl_y1 = y;
                    s += y;
                    // LF noise bed (r4 de-pitching): dark broadband floor under
                    // the partials — see field docs. Runs on its own envelope.
                    if self.lfn_env > 1e-5 {
                        let n = self.rng.next();
                        self.lfn += self.lfn_c * (n - self.lfn);
                        self.lfn2 += self.lfn_c * (self.lfn - self.lfn2);
                        s += self.lfn2 * self.lfn_env;
                        self.lfn_env *= self.lfn_dec;
                    }
                }
                DrumKind::Cymbal => s = 0.0, // handled by early return above
                DrumKind::Noise => {
                    let n = self.rng.next();
                    self.hp += self.hp_c * (n - self.hp); // lowpass...
                    let hp = n - self.hp; // ...subtracted = one-pole highpass
                    self.lp += self.lp_c * (hp - self.lp); // band upper edge
                    // optional second LP stage (12 dB/oct wires): the
                    // Muldjord rock snare's close-mic noise falls >20 dB by
                    // 2-4 kHz — one pole leaves it 15 dB too bright (r4)
                    let band = if self.lfn_c > 0.0 {
                        self.lfn += self.lfn_c * (self.lp - self.lfn);
                        self.lfn
                    } else {
                        self.lp
                    };
                    // bristle-arrival ramp (brush voicings; 1.0 = no-op else)
                    let atk = if self.atk_ph < 1.0 {
                        let g = 0.5 * (1.0 - (core::f32::consts::PI * self.atk_ph).cos());
                        self.atk_ph += self.atk_dp;
                        g
                    } else {
                        1.0
                    };
                    s = band * self.env * self.noise_amt * atk;
                    if self.tone_amt > 0.0 {
                        self.phase = (self.phase + self.freq * dt).fract();
                        s += self.tone_amt * (core::f32::consts::TAU * self.phase).sin() * self.env * atk;
                    }
                }
            }
            self.env *= self.decay;
            *o += s * self.amp;
            self.age += 1;
        }
        self.hp = flush_denormal(self.hp);
        self.lp = flush_denormal(self.lp);
        self.env = flush_denormal(self.env);
        self.sl_y1 = flush_denormal(self.sl_y1);
        self.sl_y2 = flush_denormal(self.sl_y2);
        self.lfn = flush_denormal(self.lfn);
        self.lfn2 = flush_denormal(self.lfn2);
        self.lfn_env = flush_denormal(self.lfn_env);
        if self.has_modal {
            self.modal.render(out);
        }
        self.age < self.life
    }
}

// ---------------------------------------------------------------------------
// Voice = one note on one track
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Kernel {
    Off,
    Modal(ModalVoice),
    Pluck(PluckVoice),
    EPluck(ElectricVoice),
    Drum(DrumVoice),
    Synth(SynthVoice),
    Piano(PianoVoice),
}

#[derive(Clone, Copy)]
pub struct Voice {
    pub kernel: Kernel,
    pub track: u8,
    pub midi: u8,
    pub releasing: bool,
    /// note-off arrived while the sustain pedal was down; release on pedal-up
    pub pedal_held: bool,
    /// stereo placement of THIS note (-1..1): pianos/mallets spread by key,
    /// drums by kit layout, acoustics by a small seeded micro-offset
    pub pan: f32,
    pub age: u64,
}

impl Voice {
    pub const fn off() -> Self {
        Self {
            kernel: Kernel::Off,
            track: 0,
            midi: 0,
            releasing: false,
            pedal_held: false,
            pan: 0.0,
            age: 0,
        }
    }
    pub fn active(&self) -> bool {
        !matches!(self.kernel, Kernel::Off)
    }
}

/// Stereo placement per note (audience perspective). Electrics return 0 — the amp
/// chain is genuinely mono, like the instrument.
pub fn voice_pan(inst: Instrument, midi: u32, seed: u32) -> f32 {
    let key = |lo: f32, hi: f32| (((midi as f32) - lo) / (hi - lo)).clamp(0.0, 1.0) - 0.5;
    match inst {
        Instrument::Piano => key(21.0, 108.0) * 0.60,
        Instrument::Marimba | Instrument::Vibraphone => key(45.0, 96.0) * 0.50,
        Instrument::Glockenspiel | Instrument::MusicBox => key(60.0, 108.0) * 0.35,
        Instrument::EPiano => key(28.0, 96.0) * 0.30,
        Instrument::Drums | Instrument::DrumsRock | Instrument::DrumsJazz => match midi {
            35 | 36 => 0.0,       // kick center
            38 | 40 => 0.05,      // snare just off-center
            42 | 44 => 0.28,      // hats player-left (audience right)
            46 => 0.30,
            49 | 57 => -0.25,     // crash
            55 => -0.32,          // splash (mounted near the crash side)
            51 | 53 | 59 => 0.40, // ride (bell = same cymbal)
            41 | 43 | 45 => -0.18, // low toms
            47 | 48 | 50 => 0.12, // high toms
            _ => ((seed >> 7) as f32 / 33554432.0 - 0.5) * 0.2,
        },
        Instrument::Guitar | Instrument::GuitarSteel => {
            // per-note micro-offset: strings sit at slightly different spots
            // across the neck (round 2: widened a touch — the stereo body
            // banks render the spread naturally now)
            ((seed >> 9) as f32 / 8388608.0 - 0.5) * 0.16
        }
        // bass sits at the middle of the mix like a DI track: barely off-center
        Instrument::Bass => ((seed >> 9) as f32 / 8388608.0 - 0.5) * 0.04,
        _ => 0.0,
    }
}

/// Per-instrument note-on: builds the right kernel with per-family preset numbers.
pub fn start_voice(inst: Instrument, midi: u32, vel: f32, sr: f32, seed: u32) -> Kernel {
    let f0 = midi_to_hz(midi as f32);
    let vel = vel.clamp(0.0, 1.0);
    match inst {
        Instrument::Marimba => {
            // rosewood bar, tuned 1:4:10 (Rossing); short bright contact
            let key = (midi as f32 - 45.0) / 40.0; // decay shortens up the keyboard
            let t = (1.4 - key).clamp(0.35, 1.6);
            Kernel::Modal(ModalVoice::start(
                f0,
                vel,
                sr,
                &[
                    ModeDef { ratio: 1.0, amp: 1.0, t60: t },
                    ModeDef { ratio: 3.98, amp: 0.42, t60: t * 0.30 },
                    ModeDef { ratio: 10.2, amp: 0.16, t60: t * 0.12 },
                    ModeDef { ratio: 17.9, amp: 0.05, t60: t * 0.07 },
                ],
                1.1,
                0.10,
                0.0,
                seed,
            ))
        }
        Instrument::Vibraphone => {
            let t = 6.0 - 3.0 * ((midi as f32 - 53.0) / 36.0).clamp(0.0, 1.0);
            let mut m = ModalVoice::start(
                f0,
                vel,
                sr,
                &[
                    ModeDef { ratio: 1.0, amp: 1.0, t60: t },
                    ModeDef { ratio: 4.0, amp: 0.28, t60: t * 0.22 },
                    ModeDef { ratio: 10.0, amp: 0.10, t60: t * 0.07 },
                ],
                1.6,
                0.05,
                0.0,
                seed,
            );
            // rotary-motor tremolo — the vibraphone's signature (≈4.5 Hz, medium fan)
            m.trem_rate = core::f32::consts::TAU * 4.5 / sr;
            m.trem_depth = 0.35;
            Kernel::Modal(m)
        }
        Instrument::Glockenspiel => Kernel::Modal(ModalVoice::start(
            f0,
            vel,
            sr,
            // free-bar ratios (steel, no arch tuning)
            &[
                ModeDef { ratio: 1.0, amp: 1.0, t60: 2.6 },
                ModeDef { ratio: 2.756, amp: 0.35, t60: 1.4 },
                ModeDef { ratio: 5.404, amp: 0.18, t60: 0.7 },
                ModeDef { ratio: 8.933, amp: 0.06, t60: 0.35 },
            ],
            0.7,
            0.12,
            0.0,
            seed,
        )),
        Instrument::MusicBox => Kernel::Modal(ModalVoice::start(
            f0,
            vel,
            sr,
            // plucked steel comb tooth: slightly inharmonic shimmer
            &[
                ModeDef { ratio: 1.0, amp: 1.0, t60: 2.2 },
                ModeDef { ratio: 2.02, amp: 0.22, t60: 1.1 },
                ModeDef { ratio: 5.7, amp: 0.10, t60: 0.5 },
                ModeDef { ratio: 9.1, amp: 0.04, t60: 0.25 },
            ],
            0.5,
            0.16,
            0.0,
            seed,
        )),
        Instrument::Guitar => {
            // nylon: near-lossless bright loop (refs 010/014: all harmonics decay
            // at similar slow rates at E2); darkness comes from the finger-release
            // excitation, not loop damping. Fundamental t60 rises with register
            // in the NSynth nylon sources.
            let key = (((midi as f32) - 40.0) / 44.0).clamp(0.0, 1.0);
            let p = AcPluck {
                f0,
                vel,
                // refs split by source: 010 ≈ 6–15 s early t60, 014 ≈ 22–27 s;
                // round-1's 8+4·key sat at the floor of both (render −20 dB vs
                // 014 at the 3 s mark). Compromise law raised round 2.
                // superseded by the damping law (eta_f > 0); kept as doc of the
                // r2 hand fit the law replaces
                t60_f0: 11.0 + 6.0 * key,
                lp_c: 0.97 - 0.10 * key + 0.02 * vel,
                hf_floor_t60: 0.0,
                hf_knee_hz: 2400.0,
                pick_pos: 0.20,
                // r2's 0.045 band-limited the excitation to ~n=22 (1.8 kHz at
                // E2); the refs keep 1.7-3.4 kHz attack content +17..+31 dB
                // above the render (pooled envdelta r3). Woodhouse 2012 uses a
                // ~7.5 mm contact on a 650 mm string = 0.0115 - our wider value
                // also stood in for the missing radiation tilt, now present.
                // Velocity-steepened: soft tirando is all flesh (wide patch),
                // hard plucks release nearer the nail.
                contact: 0.036 - 0.011 * vel,
                snap: 0.5,
                scrape: 0.06,
                // fingertip/nail release tick (r3): at the nylon transient
                // scale (tr_lvl = p.level, no differencer renorms) this is
                // subtle - metrics prefer it slightly over 0 and single-note
                // peaks are unchanged (0.042)
                click: 5.0,
                // fingertip/nail contact: lower band than a pick, and only a
                // light slide component (ref mf scrape/body sits 13 dB below
                // the old render's — tirando is mostly flesh)
                click_slow: 0.8,
                click_hz: 2000.0,
                // fingertip flesh: slow contact (soft strokes slower still)
                click_ramp: 0.012 - 0.006 * vel,
                // nylon refs' post-off slopes (68-171 dB/s) are contaminated
                // by the NSynth release fade/gate (014 mids are hard-gated;
                // steel 015's 31-51 dB/s proves slower decays survive the
                // pipeline, so nylon's true damp is ≥ ~70 dB/s). A 0.55-0.75 s
                // law overshot (tail logmel 1.29→1.45, it2); 0.30 s keeps an
                // audible finger-damp ring without ringing past the refs.
                rel_t60: 0.30 - 0.06 * vel,
                rel_click: 0.25,
                rel_ramp: 0.0, // legacy instant damp (bit-identical; see AcPluck)
                pol_mix: 0.35,
                pol_detune_cents: 2.2,
                pol_t60_ratio: 0.55,
                // nylon source 014 measures B ≈ 4.6e-5 (E2) → 5.2e-5 (C4), h10
                // ≈ +5 cents; source 010 is near-flexible. Split the difference
                // low — nylon stretch is subtle but real (bfit 2026-07-11 r2).
                stiff_b: 3.5e-5,
                // no tension glide: nylon refs show none, and the glide beat
                // against the polarization detune measurably hurt C5
                tm_cents: 0.0,
                // Woodhouse 2004 II Table 1 (D'Addario Pro Arte): monofilament
                // trebles eta_f 14-40e-5, wound basses 2-7e-5 (key-interp);
                // eta_a ~1.2. eta_b fit to the NSynth sources instead of the
                // table's 2e-2 polymer value: refs keep 2-3.4 kHz ringing
                // through the mid window (t60(2k) ~5-7 s), implying ~2.5e-3 —
                // the table value killed HF 50 dB below ref (it2d, 2026-07-12).
                // couple sized so eta at A0 roughly doubles intrinsic loss.
                eta_f: 0.0, // BISECT: legacy path
                eta_b: 2.5e-3,
                eta_a: 1.2,
                couple: 4.0e-3,
                // displacement tap: the NSynth nylon sources are fundamental-
                // dominant in mid/high register; low-register h2 emphasis comes
                // from the body's T1 mode, not a global force tilt
                br_rho: 0.0,
                acc_rho: 0.0, // body-round radiation design (bass-merge silently reverted this; drift-check caught it)
                // radiated sound only: monopole HP (Woodhouse 2012 f_c ~250 Hz)
                rad_hz: 250.0,
                thump: 0.0,
                thump_hz: 110.0,
                // register slope ~12 dB/key (within-source NSynth slope is
                // ~9 dB/key; cross-source fits inflate it); mild velocity curve
                level: 0.5 * (0.55 + 0.45 * vel) * (1.4 * key.min(0.9)).exp(),
            };
            Kernel::Pluck(PluckVoice::start_acoustic(&p, sr, seed))
        }
        Instrument::Bass => {
            // ELECTRIC bass guitar (owner 2026-07-12: DI/amp identity, not
            // upright): fingered roundwound bass through a velocity-sensing
            // magnetic pickup chain — reference-matched against the CC0
            // darkblack fingered DI corpus (44.1k, E1/A1/D2/G2 × p/mf/f;
            // growly + FreePats YR as held-out cross-instruments). Pooled
            // K-weighted log-mel 2.48 → 1.02; held-out −75/−76/−77%.
            let key = (((midi as f32) - 28.0) / 32.0).clamp(0.0, 1.0);
            let p = AcPluck {
                f0,
                vel,
                // electric bass sustains LONG: darkblack (CC0 DI corpus)
                // envelope t60_late runs 39 s at E1-p falling to ~16-18 s at
                // G2; the old 14−4·key measured 10-13 s across the board.
                t60_f0: 26.0 - 16.0 * key,
                lp_c: 0.50 + 0.10 * vel,
                hf_floor_t60: 0.0,
                hf_knee_hz: 0.0,
                // fingers pluck ~0.2 of the speaking length from the bridge
                // (over/behind the neck pickup). 0.30 put the excitation comb
                // null nearly ON h3 (sin 0.9π ≈ 0.31) — the darkblack refs'
                // early ladders are flat through h3-h4 (e2_mf: h3 −0.3 dB)
                // with a measured null at n≈5 (g3_mf h5 −41 dB → q = 0.2).
                pick_pos: 0.20,
                // wide flesh contact keeps the SUSTAIN ladder dark (C2/G2
                // sustain centroids match refs); narrowing it to fingertip
                // scale (0.045-0.02v) was tried and reverted — through the
                // velocity-tap differencer it became a giant onset edge
                // (crest 11 vs ref 5.3) that died into the dark loop within
                // periods. Attack brightness lives in the click transient.
                contact: 0.08,
                // ZERO time-domain buffer injections: snap/scrape are loaded
                // off the mode grid, so their first recirculation through the
                // warm loop filters is discontinuous at the wrap seam — and
                // the double-differencer renorm (~×3900 at E1) turns that
                // seam into a ±full-scale 2-sample pop at exactly one period
                // (measured 0.45 FS at 24.0 ms, E1). Fingers have no pick
                // corner; the contact noise lives in the out-of-loop click.
                snap: 0.0,
                scrape: 0.0,
                // fingertip thump + string-against-fret contact (the owner
                // verdict: "attack is too soft" — this was 0.0). The click
                // bypasses the pickup differencers whose renorms scale the
                // string level up steeply toward low f0, so the register law
                // must be steep to keep the thump register-honest (darkblack
                // refs' onset/body: 5.0 at E1-f → 8.5 at G2-f). Velocity:
                // refs show NO click at p (onset/body 2.4-3.3 = body crest)
                // — the machinery's 10% quadratic floor rides the E1 boost,
                // so divide it out for a cube law (soft fingers slip, hard
                // fingers snap off the fret).
                click: 0.05 * (f0 / 41.2).powf(1.7) * vel * vel * vel
                    / (0.1 + 0.9 * vel * vel),
                // real finger mute (~0.18 s), safe now that the loss GLIDES:
                // the legacy instant step read back through the double-
                // differencer renorm as a clipped ±FS doublet one period
                // after EVERY note-off (0.12 s and even 30/f0 both clipped)
                rel_t60: 0.18,
                // finger-mute thud: rides tr_lvl (the 1/f0-boosted pre-acc
                // level), so it needs the same register compensation as the
                // onset click (flat 0.03 measured a release spike 10x the
                // decayed body at E1 — file ttp read 3.9 s = note-off)
                rel_click: 0.004 * (f0 / 41.2).powf(1.5),
                rel_ramp: 0.02,
                // slope-extrapolated differencer priming: the legacy prime
                // clipped a +/-FS doublet on every mid-stream note-on (AcPluck)
                pol_mix: 0.25,
                pol_detune_cents: 1.2,
                pol_t60_ratio: 0.5,
                // real roundwound inharmonicity: darkblack B fits 0.75e-4
                // (G string) … 2.2e-4 (A), growly similar → h10 +4…+18 cents
                // (the 16 kHz NSynth fit said ≤2e-5; the 44.1k corpus wins).
                // Side effect: the deeper dispersion cascade also halved the
                // carrier wrap-seam residual at E1 (13.3× → 5.9× body rms).
                stiff_b: 1.2e-4,
                tm_cents: 0.0,
                // magnetic pickup senses string VELOCITY (Faraday: EMF ∝ dΦ/dt),
                // not displacement — the DI tilt is +6 dB/oct. Baseline rendered
                // centroid = f0 at every register (a sine); refs sit 1.5-2.3×f0.
                br_rho: 0.995,
                // second differencer = COMMUTED pickup-position comb: the honest
                // pickup ladder is sin(nπβ)·sin(nπq)/n (velocity at the pole
                // piece, q ≈ 0.2 for a neck pickup); with β = q = 0.2 it tops at
                // h2-h3 and nulls at h5. Triangle/n² × both differencers (n²)
                // = sin(nπβ): the same flat-top ladder within ~2 dB through the
                // contact rolloff — darkblack e2_mf measures −5.3/0/−0.3/−5.2
                // (h1..h4), i.e. literally sin(nπ·0.2). No single-comb setting
                // can put h2 above h1 (cap: cos(πβ) ≤ 1), so this tilt is
                // structural, not cosmetic. (A tighter 0.998 leak — floor
                // below E1's |2sin(ω/2)| — fixed the last +5 dB of h1 excess
                // at E1 but read as a net K-weighted regression: kept at the
                // guitar's 0.995.)
                acc_rho: 0.995,
                rad_hz: 0.0, // DI bass: no radiation HP (an 80 Hz DI low-cut
                // was tried for the refs' weak E1 fundamental and reverted:
                // it passes the onset edge while gutting the sustain — E1
                // crest blew up 3.8→8.5 vs ref 3.7, attack logmel +0.3)
                eta_f: 0.0,  // legacy hand-fit loss (DI bass, no body)
                eta_b: 0.0,
                eta_a: 0.0,
                couple: 0.0,
                // velocity span fit to the darkblack LUFS ladder: refs span
                // ~12.7 LU p→f (the old 0.5+0.5v gave 4.9). Register: the
                // corpus' own layers disagree (p falls −4.3 LU by G2, f is
                // flat-to-rising — per-string recording levels), so a full
                // e^key fit overshot at f by +6-8 LU; a modest +2 dB slope
                // keeps lines even without chasing library idiosyncrasy.
                level: 0.5 * (0.12 + 0.88 * vel).powf(1.7) * (0.5 * key).exp(),
                ..Default::default()
            };
            Kernel::Pluck(PluckVoice::start_acoustic(&p, sr, seed))
        }
        Instrument::EPiano => {
            // tine/reed + tonebar through an asymmetric flux pickup — see
            // ModalVoice::start_epiano (EP round 2026-07-12; the old 1:3.97
            // bar ladder + symmetric tanh was the owner's "too much like
            // marimba": a tanh is odd-symmetric and CANNOT make the
            // even-harmonic bark that is the EP's velocity axis)
            Kernel::Modal(ModalVoice::start_epiano(midi, f0, vel, sr, seed))
        }
        Instrument::Drums => Kernel::Drum(DrumVoice::start(midi, vel, sr, seed, KitStyle::Pop)),
        Instrument::DrumsRock => Kernel::Drum(DrumVoice::start(midi, vel, sr, seed, KitStyle::Rock)),
        Instrument::DrumsJazz => Kernel::Drum(DrumVoice::start(midi, vel, sr, seed, KitStyle::Jazz)),
        Instrument::SynthPad => Kernel::Synth(SynthVoice::start(f0, vel, sr)),
        Instrument::Piano => Kernel::Piano(PianoVoice::start(midi, f0, vel, sr, seed)),
        Instrument::GuitarSteel => {
            // steel: darker loop than nylon in RELATIVE terms (h12/h1 t60 ratio
            // ~1/3 in ref 015) but a much brighter pick excitation near the
            // bridge; fundamental t60 falls from ~12 s (E2) to ~2.3 s (C5).
            let key = (((midi as f32) - 40.0) / 44.0).clamp(0.0, 1.0);
            let p = AcPluck {
                f0,
                vel,
                t60_f0: (12.0 - 13.0 * key).clamp(2.0, 12.0),
                // sustain brightness is velocity-independent in the refs —
                // velocity lives in the excitation, not the loop. lp_c is
                // superseded by the blend loss filter (hf_floor below).
                lp_c: 0.57 + 0.28 * key,
                // blend fit to ref 015's measured t60 ladder (12 s at f0 →
                // 9.8 s at 410 → 4.2 s at 1 kHz → ~2 s floor at 3 kHz+): knee
                // 2850 Hz, floor 1.5 s at E2, both within ~15% across the
                // ladder (offline grid fit 2026-07-11 round 2)
                hf_floor_t60: (1.5 * (82.41 / f0).sqrt()).clamp(0.3, 1.8),
                hf_knee_hz: 2850.0,
                pick_pos: 0.14,
                contact: 0.010,
                // split round 2: only a sliver of the pick noise persists as
                // string modes (in-loop, sized to the refs' −28 dB late-HF
                // remnant); the rest radiates directly as the click transient
                // (with the HF loss floor, all-in-loop ran +30…+47 dB hot)
                snap: 0.12,
                scrape: 0.008,
                // Re-fit at the r3 transient scale (tr_lvl = pre-acceleration
                // level): r2's 1.35 rode a scale ~30-46x hotter at E2 and was
                // soft-clipping every ff onset. Sweep 1.5-16 (2026-07-12): the
                // metric optimum (8-16) rides the clip ceiling again; 3.0 is
                // the best clean-crest point (solo ff onset peak ~0.5).
                click: 3.0,
                // scrape: refs' 1-6.5 kHz attack-band/body ratio stays ~flat
                // from pp to ff (+1..2 dB) — the slide friction keeps its
                // energy at low velocity, spread over a longer stroke
                click_slow: 1.0,
                click_hz: 2800.0,
                click_ramp: 0.0015,
                // measured on 015 post-note-off envelopes (body round,
                // 2026-07-12): E2 ff 33.6 dB/s (t60 1.8), D2 mf 31.7, E2 mf
                // 37.8, G2 ff 51.3 (1.17), F#2 pp 70.1 (0.86) — slower ring
                // low/hard, quicker high/soft. r3's flat 0.9 halved the ff
                // ring ("notes stop dead"). 021 refs are hard-gated at ~3.2 s
                // and carry no release information (corpus artifact).
                rel_t60: (1.7 * (82.41 / f0).powf(1.2) * (0.7 + 0.3 * vel))
                    .clamp(0.4, 2.0),
                rel_click: 0.5,
                rel_ramp: 0.0, // legacy instant damp (bit-identical; see AcPluck)
                // two-stage decay, Weinreich roles corrected round 2: the
                // strongly-coupled (plucked) polarization decays FAST; the
                // orthogonal one couples weakly to the bridge and carries the
                // slow aftersound — refs flatten to it ~2 s in (render sat
                // −12 dB under ref 015 at the 3 s mark with the old 0.55).
                pol_mix: 0.22,
                pol_detune_cents: 1.5,
                pol_t60_ratio: 1.6,
                // steel source 015: B = 2.70e-4 @73 Hz → 3.15e-4 @97 Hz (h10
                // +31…36 cents), rising with key as B ∝ 1/l² on a fretted
                // string ⇒ ≈ √(f0/E2) law (bfit fits, 2026-07-11 round 2)
                stiff_b: 3.0e-4 * (f0 / 82.41).sqrt(),
                // hard low plucks start a few cents sharp and settle — the steel
                // "twang" onset (Tolonen/Välimäki/Karjalainen 2000); NSynth refs
                // show ≤3 cents, so this stays subtle
                tm_cents: 4.0,
                // Woodhouse 2012 Table (Martin 80/20 bronze): eta_f 5-11e-5,
                // eta_a 1.0-2.5. eta_b fit to ref 015 instead of the table's
                // wound-string 4e-3: its 2-6 kHz partials ring ~2 s, implying
                // eta_b ~1.2e-3 (the table's aged-wound value would kill them
                // in 0.1 s). couple sized so the coupling humps roughly double
                // the intrinsic loss at A0/T1 (Woodhouse 2004 I Fig. 7).
                eta_f: 8.0e-5,
                eta_b: 1.2e-3,
                eta_a: 1.0,
                // 3e-3, not the Fig.-7 peak ~8e-3: source 015's G2 (98 Hz, ON
                // the A0) still rings 6 s — 8e-3 choked it to 2.3 s (it5)
                couple: 3.0e-3,
                br_rho: 0.995,
                acc_rho: 0.995,
                rad_hz: 250.0,
                // body-pump (see AcPluck::thump): sized to the ff attack A0
                // deficit (−10.8 dB vs ref 015, body round 2026-07-12); vel²
                // law keeps pp clean (refs' pp attack A0 is already matched)
                thump: 0.085,
                thump_hz: 105.0,
                level: 0.5 * (0.55 + 0.45 * vel) * (0.83 * key).exp(),
            };
            Kernel::Pluck(PluckVoice::start_acoustic(&p, sr, seed))
        }
        Instrument::GuitarElectric | Instrument::GuitarDistorted => Kernel::EPluck(
            ElectricVoice::start_electric(midi, f0, vel, sr, inst == Instrument::GuitarDistorted, seed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_pluck(inst: Instrument, midi: u32, vel: f32, sr: f32, secs: f32) -> Vec<f32> {
        let mut k = start_voice(inst, midi, vel, sr, 12345);
        let total = (secs * sr) as usize;
        let mut out = vec![0.0f32; total];
        for chunk in out.chunks_mut(128) {
            if let Kernel::Pluck(p) = &mut k {
                p.render(chunk);
            }
        }
        out
    }

    fn autocorr_pitch(x: &[f32], sr: f32, lo: f32, hi: f32) -> f32 {
        let min_lag = (sr / hi) as usize;
        let max_lag = ((sr / lo) as usize).min(x.len() / 2);
        let n = x.len() - max_lag;
        let corr = |lag: usize| -> f32 { (0..n).map(|i| x[i] * x[i + lag]).sum() };
        let mut best_lag = min_lag;
        let mut best = f32::NEG_INFINITY;
        for lag in min_lag..=max_lag {
            let c = corr(lag);
            if c > best {
                best = c;
                best_lag = lag;
            }
        }
        let (a, b, c) = (corr(best_lag - 1), corr(best_lag), corr(best_lag + 1));
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
        sr / (best_lag as f32 + delta.clamp(-0.5, 0.5))
    }

    fn render_piano(midi: u32, vel: f32, sr: f32, secs: f32, damp_at: Option<f32>) -> Vec<f32> {
        let mut v = PianoVoice::start(midi, midi_to_hz(midi as f32), vel, sr, 777);
        let total = (secs * sr) as usize;
        let mut out = vec![0.0f32; total];
        let damp_i = damp_at.map(|t| (t * sr) as usize);
        let mut i = 0usize;
        for chunk in out.chunks_mut(128) {
            if let Some(d) = damp_i {
                if i <= d && d < i + chunk.len() {
                    v.damp();
                }
            }
            v.render(chunk);
            i += chunk.len();
        }
        out
    }

    /// Goertzel-style band energy: sum of |projection| onto sin/cos at freqs.
    fn band_energy(x: &[f32], sr: f32, freqs: &[f32]) -> f32 {
        let n = x.len() as f32;
        freqs
            .iter()
            .map(|&f| {
                let w = core::f32::consts::TAU * f / sr;
                let (mut c, mut s) = (0.0f32, 0.0f32);
                for (i, &xi) in x.iter().enumerate() {
                    let ph = w * i as f32;
                    c += xi * ph.cos();
                    s += xi * ph.sin();
                }
                (c * c + s * s) / (n * n)
            })
            .sum()
    }

    /// Bridge coupling (Weinreich 1977, round 3): the fundamental must show a
    /// TWO-STAGE decay — the in-phase prompt drains fast through the coupled
    /// bridge, then the detuned pair's anti-phase remnant sings at the much
    /// slower internal rate. Painted per-string t60s can't produce this on a
    /// single partial; the coupling matrix does.
    #[test]
    fn piano_fundamental_decays_in_two_stages() {
        let sr = 48_000.0;
        let f0 = midi_to_hz(60.0);
        let out = render_piano(60, 0.7, sr, 4.0, None);
        // heterodyne envelope of the fundamental
        let hop = (0.05 * sr) as usize;
        let n = out.len() / hop;
        let w = core::f32::consts::TAU * f0 / sr;
        let mut env_db = vec![0.0f32; n];
        for (k, e) in env_db.iter_mut().enumerate() {
            let (mut c, mut s) = (0.0f32, 0.0f32);
            for i in 0..hop {
                let ph = w * ((k * hop + i) as f32);
                c += out[k * hop + i] * ph.cos();
                s += out[k * hop + i] * ph.sin();
            }
            *e = 10.0 * ((c * c + s * s) / (hop * hop) as f32 + 1e-18).log10();
        }
        let rate = |t0: f32, t1: f32| {
            let (i0, i1) = ((t0 / 0.05) as usize, (t1 / 0.05) as usize);
            (env_db[i0] - env_db[i1]) / (t1 - t0)
        };
        let early = rate(0.10, 0.70);
        let late = rate(1.60, 3.60);
        assert!(
            early > late + 8.0,
            "C4 fundamental should dive then sing: early {early} dB/s, late {late} dB/s"
        );
        assert!(late < 16.0, "aftersound should sing at the internal rate: late {late} dB/s");
    }

    /// Stiffness dispersion must stretch partials SHARP of n·f0 (Fletcher
    /// 1964: f_n = n·f0·√(1+Bn²)); the pre-round-3 allpass sign dragged them
    /// FLAT. Guard partial 8 of C4 at both deploy sample rates against the
    /// Salamander-fit window (+14.8 cents measured; ref +14.1).
    #[test]
    fn piano_partials_stretch_sharp() {
        for sr in [44_100.0f32, 48_000.0] {
            let f0 = midi_to_hz(60.0);
            let out = render_piano(60, 0.6, sr, 1.4, None);
            let seg = &out[(0.3 * sr) as usize..(1.3 * sr) as usize];
            // scan ±40 cents around 8·f0 with a fine projection grid
            let mut best = (0.0f32, f32::NEG_INFINITY);
            let mut cents = -40.0f32;
            while cents <= 40.0 {
                let f = 8.0 * f0 * (cents / 1200.0).exp2();
                let e = band_energy(seg, sr, &[f]);
                if e > best.1 {
                    best = (cents, e);
                }
                cents += 2.0;
            }
            assert!(
                (5.0..30.0).contains(&best.0),
                "sr {sr}: partial 8 stretch {} cents, want +5…+30 (sharp)",
                best.0
            );
        }
    }

    /// The coupling matrix R = I − gJ is only energy-passive for N·g ≤ 2 —
    /// enforce a wide margin across the whole keyboard and velocity range.
    #[test]
    fn piano_bridge_coupling_stays_passive() {
        for midi in (21..=108).step_by(3) {
            for vel in [0.05f32, 0.5, 1.0] {
                for sr in [44_100.0f32, 48_000.0] {
                    let v = PianoVoice::start(midi, midi_to_hz(midi as f32), vel, sr, 42);
                    let ng = v.n_strings as f32 * (v.bridge_g0 + v.bridge_g1);
                    assert!(
                        (0.0..0.5).contains(&ng),
                        "midi {midi} vel {vel} sr {sr}: N·g = {ng} outside passive margin"
                    );
                }
            }
        }
    }

    /// P3 longitudinal formant (Bank & Sujbert 2005): the 6+7 pair-sum
    /// phantom cluster at C4 ff sits in the interpartial gap near 13.2·f0
    /// (the per-key formant table's C4 anchor) — assert the cluster is
    /// present at ff, grows strongly against pp (Conklin: phantoms at
    /// forte), and radiates at the SAME level at both deploy sample rates
    /// (the resonator design is exact-peak-normalized; measured parity
    /// 0.015 dB, gate 2 dB).
    #[test]
    fn piano_ff_formant_phantom_cluster_present_and_rate_stable() {
        let mut ratios = [0.0f32; 4];
        for (si, sr) in [44_100.0f32, 48_000.0].iter().enumerate() {
            let sr = *sr;
            let f0_et = midi_to_hz(60.0);
            let b = 10.0f32.powf(-3.086); // C4 calibrated inharmonicity
            let f0 = f0_et * (3.475f32 / 1200.0).exp2(); // calibrated tuning
            // predicted 6+7 phantom: f6+f7 = 13.23 f0, safely between the
            // stretched f12 (12.69 f0) and f13 (13.87 f0)
            let fsum = f0 * (6.0 * (1.0 + 36.0 * b).sqrt() + 7.0 * (1.0 + 49.0 * b).sqrt());
            let mut freqs = [0.0f32; 7];
            for (i, f) in freqs.iter_mut().enumerate() {
                *f = fsum * ((i as f32 - 3.0) * 10.0 / 1200.0).exp2();
            }
            let mut lo = [0.0f32; 30];
            for (i, f) in lo.iter_mut().enumerate() {
                *f = 2.0 * f0 + (2.5 * f0) * (i as f32) / 29.0;
            }
            for (vi, vel) in [0.2f32, 1.0].iter().enumerate() {
                let out = render_piano(60, *vel, sr, 1.0, None);
                let (a, bnd) = ((0.1 * sr) as usize, (0.6 * sr) as usize);
                ratios[si * 2 + vi] = band_energy(&out[a..bnd], sr, &freqs)
                    / band_energy(&out[a..bnd], sr, &lo).max(1e-12);
            }
        }
        for (si, sr) in [44_100.0f32, 48_000.0].iter().enumerate() {
            let (pp, ff) = (ratios[si * 2], ratios[si * 2 + 1]);
            assert!(ff > 2.5e-4, "sr {sr}: formant cluster missing at ff ({ff})");
            assert!(ff > 4.0 * pp, "sr {sr}: cluster not forte-dominant (ff {ff} pp {pp})");
        }
        let parity_db = 10.0 * (ratios[1] / ratios[3]).log10();
        assert!(
            parity_db.abs() < 2.0,
            "44.1k/48k formant parity broken: {parity_db} dB"
        );
    }

    /// Phantom partials (Conklin 1999): an ff bass note must carry MUCH more
    /// energy around partials 10–12 than pp, beyond the linear model's
    /// velocity-brightening — the quadratic tension tap is what provides it.
    #[test]
    fn piano_ff_bass_grows_phantom_partials() {
        let sr = 48_000.0;
        let f0 = midi_to_hz(31.0);
        // dense grid across partials 9.5–12.5: dispersion shifts the real
        // partials off n·f0, and a 0.5 s projection is only ~2 Hz wide
        let mut freqs = [0.0f32; 50];
        for (i, f) in freqs.iter_mut().enumerate() {
            *f = 9.5 * f0 + (3.0 * f0) * (i as f32) / 49.0;
        }
        // reference band on the low partials (2–4): any LINEAR stage (board
        // dip, radiation HP, air pole…) applies identical per-band gains at
        // both velocities, so normalizing the phantom band by this band makes
        // the superlinearity measure filter-immune (the old total-energy
        // denominator broke when the board antiresonance dip reshaped pp vs
        // ff totals differently).
        let mut lo = [0.0f32; 30];
        for (i, f) in lo.iter_mut().enumerate() {
            *f = 2.0 * f0 + (2.5 * f0) * (i as f32) / 29.0;
        }
        let ff = render_piano(31, 1.0, sr, 1.0, None);
        let pp = render_piano(31, 0.2, sr, 1.0, None);
        let (a, b) = ((0.1 * sr) as usize, (0.6 * sr) as usize);
        let r_ff = band_energy(&ff[a..b], sr, &freqs) / band_energy(&ff[a..b], sr, &lo).max(1e-12);
        let r_pp = band_energy(&pp[a..b], sr, &freqs) / band_energy(&pp[a..b], sr, &lo).max(1e-12);
        assert!(r_ff > 1e-7, "no phantom-band energy at ff: {r_ff}");
        // The band holds transverse partials too (they scale ~linearly against
        // the reference band, so the ratio cancels); the superlinear EXCESS is
        // the phantom tap's signature — with ph_gain = 0 this ratio-of-ratios
        // measures ≈ 1.0. Threshold 1.35 → 1.15 (P1): the per-key-calibrated
        // hammer-contact/prompt-loss tables shift the pp LINEAR content in
        // this band by a few % per calibration round (observed 1.27–1.33
        // across rounds; the tap itself unchanged) — the gate now sits
        // decisively above the ph_gain=0 null and below the calibration
        // jitter floor, guarding the MECHANISM rather than the calibration
        // state. Honest gap on record (P1, re-measured after P3): Salamander
        // F#1 v16/v4 shows 1.68 on the FFT-band version of this axis; the
        // render reads 1.20 (P1: 1.33) and its ABSOLUTE phantom-band/low-band
        // ratio runs ~5.6× the reference (P1: ~5×) — P3 measurement located
        // this band (9.5–12.5·f0 ≈ 437–575 Hz at F#1) BELOW the longitudinal
        // formant (1204 Hz), where the excess is LINEAR pp/mf excitation
        // shape, a P2 (hammer pulse/knock voicing) item; the physically
        // dominant phantom clusters at/above the formant are calibrated to
        // ±3 dB across velocity by the P3 tables (scratchpad piano-p3).
        assert!(
            r_ff > 1.15 * r_pp,
            "phantom band should grow superlinearly with velocity: ff {r_ff} vs pp {r_pp}"
        );
    }

    /// Damper felt/key release (Askenfelt & Jansson 1990): after note-off a
    /// broadband thud must appear BETWEEN the partials (where the harmonic
    /// string can't put energy), decay away, and be RELATIVELY louder on a
    /// soft note (the mechanism doesn't shrink with a soft touch).
    ///
    /// Round 3 reposing: the transient is isolated as the sample-exact
    /// DIFFERENCE against a null twin — the same voice (same seed → identical
    /// string/RNG state) with the release mechanisms (thump noise + board
    /// re-knock) suppressed at note-off. The old gap-band proxy really
    /// measured the held string's spectral skirt; once bridge coupling gave
    /// the string its reference-matched decay that background moved while the
    /// thud physics (level relative to note peak) was unchanged.
    #[test]
    fn piano_release_thud_present_decaying_and_velocity_relative() {
        let sr = 48_000.0f32;
        let f0 = midi_to_hz(48.0);
        let render_with = |vel: f32, null_release: bool| -> Vec<f32> {
            let mut v = PianoVoice::start(48, f0, vel, sr, 777);
            let total = (2.4 * sr) as usize;
            let mut out = vec![0.0f32; total];
            let damp_i = (1.5 * sr) as usize;
            let mut i = 0usize;
            for chunk in out.chunks_mut(128) {
                if i <= damp_i && damp_i < i + chunk.len() {
                    v.damp();
                    if null_release {
                        v.thump_amp = 0.0;
                        v.body_pulse_pos = v.body_pulse_len; // cancel the re-knock
                    }
                }
                v.render(chunk);
                i += chunk.len();
            }
            out
        };
        let energy = |x: &[f32], t0: f32, t1: f32| -> f32 {
            let seg = &x[(t0 * sr) as usize..(t1 * sr) as usize];
            seg.iter().map(|s| s * s).sum::<f32>() / seg.len() as f32
        };
        let mut rel = [0.0f32; 2];
        for (i, vel) in [0.25f32, 1.0].iter().enumerate() {
            let out = render_with(*vel, false);
            let null = render_with(*vel, true);
            let diff: Vec<f32> = out.iter().zip(&null).map(|(a, b)| a - b).collect();
            let thud = energy(&diff, 1.50, 1.65);
            let late = energy(&diff, 2.10, 2.25);
            let peak = energy(&out, 0.05, 0.25);
            assert!(thud > 1e-12, "vel {vel}: no release transient ({thud})");
            assert!(
                late < 0.25 * thud,
                "vel {vel}: release transient does not decay ({thud} -> {late})"
            );
            rel[i] = thud / peak.max(1e-15);
        }
        // Askenfelt & Jansson 1990: the mechanical release thud does not shrink
        // with a soft touch, so RELATIVE to the (much quieter) pp note it must
        // be far louder than at ff.
        assert!(
            rel[0] > 2.0 * rel[1],
            "soft-note release should be relatively louder: pp {} vs ff {}",
            rel[0],
            rel[1]
        );
    }

    /// Steel-string tuning at both deploy sample rates (the acoustic constructor
    /// has its own delay budget: exact loop-lowpass phase delay + tension-mod
    /// fraction bias) — protocol: delay math changes need 44.1k AND 48k coverage.
    #[test]
    /// Guards the r3 radiation chain (bridge force -> acceleration differencer
    /// -> monopole HP): the steel attack must be BRIGHT like the refs (attack
    /// centroid ~1.1 kHz at E2 ff in NSynth 015; the r2 chain sat at 274 Hz).
    #[test]
    fn steel_attack_is_radiation_bright() {
        for sr in [44_100.0f32, 48_000.0f32] {
            let out = render_pluck(Instrument::GuitarSteel, 40, 1.0, sr, 0.5);
            let n = (0.1 * sr) as usize;
            let seg = &out[..n];
            // spectral centroid over the first 100 ms via Goertzel probes
            let (mut num, mut den) = (0.0f64, 0.0f64);
            let mut f = 90.0f32;
            while f < 5000.0 {
                let e = band_energy(seg, sr, &[f]) as f64;
                num += (f as f64) * e;
                den += e;
                f *= 1.12;
            }
            let centroid = num / den.max(1e-12);
            assert!(
                centroid > 600.0,
                "sr={sr}: attack centroid {centroid:.0} Hz — radiation tilt missing"
            );
        }
    }

    /// Guards the coupling-shelf delay compensation: G2 sits ON the A0 hump
    /// (shelf active, extra in-loop phase delay) and must stay in tune.
    #[test]
    fn steel_g2_in_tune_with_coupling_shelf_active() {
        for sr in [44_100.0f32, 48_000.0f32] {
            let out = render_pluck(Instrument::GuitarSteel, 43, 0.9, sr, 1.0);
            let tail = &out[(0.5 * sr) as usize..];
            let want = midi_to_hz(43.0);
            let f = peak_freq(tail, sr, want * 0.97, want * 1.03);
            assert!(
                (f - want).abs() < want * 0.015,
                "sr={sr}: {f} Hz, want {want}"
            );
        }
    }

    #[test]
    fn steel_e3_in_tune_both_rates_and_velocities() {
        for sr in [44_100.0f32, 48_000.0f32] {
            for vel in [0.25f32, 1.0f32] {
                let out = render_pluck(Instrument::GuitarSteel, 52, vel, sr, 1.0);
                let tail = &out[(0.5 * sr) as usize..];
                // exact DFT peak: a raw ACF is biased sharp by the stretched
                // partials ringing on the HF loss floor (round 2)
                let want = midi_to_hz(52.0);
                let f = peak_freq(tail, sr, want * 0.97, want * 1.03);
                assert!(
                    (f - want).abs() < want * 0.015,
                    "sr={sr} vel={vel}: {f} Hz, want {want}"
                );
            }
        }
    }

    /// Guards the per-period loss calibration: a nylon E2 must actually decay
    /// (the per-sample t60_gain bug made guitar envelopes flat for seconds).
    #[test]
    fn nylon_e2_decays_like_its_t60() {
        let sr = 48_000.0f32;
        let out = render_pluck(Instrument::Guitar, 40, 0.8, sr, 3.5);
        let rms = |a: &[f32]| (a.iter().map(|s| s * s).sum::<f32>() / a.len() as f32).sqrt();
        let early = rms(&out[(0.2 * sr) as usize..(0.7 * sr) as usize]);
        let late = rms(&out[(2.8 * sr) as usize..(3.3 * sr) as usize]);
        let drop_db = 20.0 * (early / late.max(1e-9)).log10();
        // t60_f0 = 8 s → fundamental alone drops ~19.5 dB over 2.6 s; brighter
        // partials decay faster, so demand at least ~12 dB and at most ~45 dB
        assert!(
            (12.0..45.0).contains(&drop_db),
            "E2 envelope drop {drop_db} dB over 2.6 s (want physical decay)"
        );
    }

    /// The acoustic loop must be stable everywhere: loss ≤ 1 by construction
    /// even where the requested loop filter is darker than the sustain target.
    #[test]
    fn acoustic_loop_gain_never_exceeds_unity() {
        for midi in [28u32, 40, 52, 64, 76, 88] {
            for vel in [0.1f32, 1.0] {
                for inst in [Instrument::Guitar, Instrument::GuitarSteel, Instrument::Bass] {
                    if let Kernel::Pluck(p) = start_voice(inst, midi, vel, 48_000.0, 7) {
                        assert!(p.loss <= 1.0, "{inst:?} midi={midi} loss={}", p.loss);
                        assert!(p.loss2 <= 1.0, "{inst:?} midi={midi} loss2={}", p.loss2);
                    }
                }
            }
        }
    }

    /// Hann-windowed DFT magnitude at an arbitrary frequency (f64 phase).
    fn dft_mag(x: &[f32], sr: f32, f: f32) -> f32 {
        let n = x.len() as f64;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        let wstep = core::f64::consts::TAU * f as f64 / sr as f64;
        for (i, &s) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (core::f64::consts::TAU * i as f64 / n).cos();
            let ph = wstep * i as f64;
            re += s as f64 * w * ph.cos();
            im -= s as f64 * w * ph.sin();
        }
        ((re * re + im * im) as f32).sqrt()
    }

    /// Strongest spectral peak in [lo, hi], via grid + parabolic refinement.
    fn peak_freq(x: &[f32], sr: f32, lo: f32, hi: f32) -> f32 {
        let n = 48;
        let step = (hi - lo) / n as f32;
        let mut best = (lo, 0.0f32);
        for i in 0..=n {
            let f = lo + i as f32 * step;
            let m = dft_mag(x, sr, f);
            if m > best.1 {
                best = (f, m);
            }
        }
        let (a, b, c) = (
            dft_mag(x, sr, best.0 - step),
            best.1,
            dft_mag(x, sr, best.0 + step),
        );
        let denom = a - 2.0 * b + c;
        let d = if denom.abs() > 1e-12 { 0.5 * (a - c) / denom } else { 0.0 };
        best.0 + d.clamp(-1.0, 1.0) * step
    }

    /// Dispersion cascade: steel E2 upper partials must stretch like the
    /// measured inharmonicity (source 015: B ≈ 3e-4 ⇒ h10 ≈ +25 cents), with
    /// the fundamental still in tune — at BOTH deploy sample rates (the
    /// cascade's phase delay is compensated in the loop length).
    #[test]
    fn steel_partials_stretch_like_the_measured_inharmonicity() {
        for sr in [44_100.0f32, 48_000.0] {
            let out = render_pluck(Instrument::GuitarSteel, 40, 1.0, sr, 1.5);
            let seg = &out[(0.25 * sr) as usize..(1.25 * sr) as usize];
            let want = midi_to_hz(40.0);
            let f1 = peak_freq(seg, sr, want * 0.98, want * 1.02);
            assert!(
                (f1 - want).abs() < want * 0.006,
                "sr={sr}: f1={f1}, want {want}"
            );
            let cents = |fn_meas: f32, n: f32| 1200.0 * (fn_meas / (n * f1)).log2();
            let f5 = peak_freq(seg, sr, 5.0 * f1 * 0.999, 5.0 * f1 * 1.012);
            let c5 = cents(f5, 5.0);
            let f10 = peak_freq(seg, sr, 10.0 * f1 * 1.004, 10.0 * f1 * 1.032);
            let c10 = cents(f10, 10.0);
            assert!((3.0..14.0).contains(&c5), "sr={sr}: h5 stretch {c5} cents");
            assert!((17.0..40.0).contains(&c10), "sr={sr}: h10 stretch {c10} cents");
            assert!(c10 > c5 + 5.0, "sr={sr}: stretch not progressive ({c5} → {c10})");
        }
    }

    /// Release transient ("pluck-off"): note-off on a ringing steel string must
    /// re-brighten the output briefly (fret/finger click) and then decay on the
    /// release t60 — not chop to silence in 90 ms. Nylon still chokes fast.
    #[test]
    fn steel_release_clicks_and_rings_nylon_chokes() {
        let sr = 48_000.0f32;
        let hf = |seg: &[f32]| {
            // crude HF energy: first-difference RMS
            let mut acc = 0.0f32;
            for i in 1..seg.len() {
                let d = seg[i] - seg[i - 1];
                acc += d * d;
            }
            (acc / seg.len() as f32).sqrt()
        };
        let rms =
            |seg: &[f32]| (seg.iter().map(|s| s * s).sum::<f32>() / seg.len() as f32).sqrt();
        let run = |inst: Instrument| -> (f32, f32, f32, f32) {
            let mut k = start_voice(inst, 45, 0.9, sr, 4242);
            let mut out = vec![0.0f32; (2.2 * sr) as usize];
            let off = (1.0 * sr) as usize;
            let mut i = 0;
            while i < out.len() {
                let end = (i + 128).min(out.len());
                if let Kernel::Pluck(p) = &mut k {
                    if i <= off && off < end {
                        p.damp();
                    }
                    p.render(&mut out[i..end]);
                }
                i = end;
            }
            let pre_hf = hf(&out[off - 4800..off]);
            let post_hf = hf(&out[off..off + 4800]);
            let at_02 = rms(&out[off + (0.2 * sr) as usize..off + (0.3 * sr) as usize]);
            let pre = rms(&out[off - 4800..off]);
            (pre_hf, post_hf, pre, at_02)
        };
        let (pre_hf, post_hf, pre, at_02) = run(Instrument::GuitarSteel);
        assert!(
            post_hf > pre_hf * 1.3,
            "steel release click missing: HF {pre_hf} -> {post_hf}"
        );
        // release t60 0.35 s ⇒ still ringing at +0.25 s, but clearly decaying
        assert!(
            at_02 > pre * 1e-4 && at_02 < pre * 0.6,
            "steel release tail off: pre {pre}, at+0.25 {at_02}"
        );
        let (_, _, pre_n, at_02n) = run(Instrument::Guitar);
        assert!(
            at_02n < pre_n * 0.05,
            "nylon must choke fast: pre {pre_n}, at+0.25 {at_02n}"
        );
    }

    /// Steel HF loss saturates on the blend filter's bypass floor: the 2.3–2.7
    /// kHz band must still ring at t ≈ 1.3 s (ref 015 measures t60 ≈ 2 s there;
    /// the old pure one-pole left digital silence). Guards the blend wiring.
    #[test]
    fn steel_hf_partials_ring_on_the_loss_floor() {
        let sr = 48_000.0f32;
        let out = render_pluck(Instrument::GuitarSteel, 40, 1.0, sr, 1.6);
        let band = |t0: f32, t1: f32| {
            let seg = &out[(t0 * sr) as usize..(t1 * sr) as usize];
            let n = seg.len();
            // Goertzel-ish: RMS after a crude 2.3–2.7 kHz projection
            let mut acc = 0.0f64;
            for f in [2350.0f32, 2500.0, 2650.0] {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, &s) in seg.iter().enumerate() {
                    let ph = (core::f32::consts::TAU * f * i as f32 / sr) as f64;
                    re += s as f64 * ph.cos();
                    im += s as f64 * ph.sin();
                }
                acc += (re * re + im * im) / (n * n) as f64;
            }
            (acc.sqrt() as f32).max(1e-12)
        };
        let early = band(0.15, 0.45);
        let late = band(1.15, 1.45);
        let drop_db = 20.0 * (early / late).log10();
        // floor t60 ≈ 2.2 s ⇒ ~27 dB/s: expect a 10–45 dB drop over 1 s, not
        // the >70 dB collapse of the one-pole
        assert!(
            (8.0..48.0).contains(&drop_db),
            "2.5 kHz band dropped {drop_db} dB over 1 s"
        );
    }

    /// Fret-release squeak (028 refs): note-off on an electric must burst
    /// broadband HF over the decayed string before dying (t60 ≈ 0.15 s).
    /// Round-3 regression guard: the squeak was silently absent from renders.
    #[test]
    fn electric_release_squeak_bursts() {
        let sr = 48_000.0f32;
        let midi = 36u32;
        let mut v =
            ElectricVoice::start_electric(midi, midi_to_hz(midi as f32), 0.5, sr, false, 42);
        let total = (3.4 * sr) as usize;
        let mut out = vec![0.0f32; total];
        let damp_i = (3.0 * sr) as usize;
        let mut i = 0usize;
        for chunk in out.chunks_mut(128) {
            if i <= damp_i && damp_i < i + chunk.len() {
                v.damp();
            }
            v.render(chunk);
            i += chunk.len();
        }
        let w = (0.15 * sr) as usize;
        let pre = &out[damp_i - w..damp_i];
        let post = &out[damp_i..damp_i + w];
        // The release thump re-excites the FUNDAMENTAL (a pure-H1 injection —
        // the 028 refs put ~99% of the burst below 300 Hz): fundamental-band
        // energy must jump at the off, while a burst-less damp only decays.
        let f0 = midi_to_hz(midi as f32);
        let (a, b) = (band_energy(pre, sr, &[f0]), band_energy(post, sr, &[f0]));
        assert!(
            b > 2.0 * a,
            "release thump missing: f0-band pre {a:e} post {b:e} (want >2x)"
        );
    }

    /// Nylon stretch stays subtle (B ≈ 3.5e-5 ⇒ h10 ≈ +3 cents); bass carries
    /// REAL roundwound stretch (darkblack CC0 corpus, 2026-07-12: B fit
    /// 0.75–2.2e-4 across strings, h10 = +4…+18 cents — the old ≤6-cent cap
    /// encoded the 16 kHz NSynth fit, superseded by the 44.1 kHz corpus).
    #[test]
    fn nylon_and_bass_stretch_match_corpus() {
        let sr = 48_000.0f32;
        for (inst, midi, lo, hi) in [
            (Instrument::Guitar, 40u32, -2.0f32, 8.0f32),
            (Instrument::Bass, 28, 3.0, 18.0),
        ] {
            let out = render_pluck(inst, midi, 0.9, sr, 1.5);
            let seg = &out[(0.25 * sr) as usize..(1.25 * sr) as usize];
            let want = midi_to_hz(midi as f32);
            let f1 = peak_freq(seg, sr, want * 0.98, want * 1.02);
            assert!((f1 - want).abs() < want * 0.006, "{inst:?}: f1={f1}");
            let f10 = peak_freq(seg, sr, 10.0 * f1 * 0.995, 10.0 * f1 * 1.012);
            let c10 = 1200.0 * (f10 / (10.0 * f1)).log2();
            assert!(
                (lo..hi).contains(&c10),
                "{inst:?}: h10 stretch {c10} cents (want {lo}..{hi})"
            );
        }
    }

    /// Note-off must not emit the wrap-seam doublet: an instant release-loss
    /// step reads back one period later through the bass's double-differencer
    /// renorm (~×3900 at E1) as a clipped ±FS 2-sample pop (measured ratio
    /// 60–126× pre-off rms before the rel_ramp loss glide). Both rates.
    #[test]
    fn bass_release_has_no_seam_pop() {
        for sr in [48_000.0f32, 44_100.0] {
            for midi in [28u32, 43] {
                let mut v = start_voice(Instrument::Bass, midi, 0.7, sr, 2024);
                let total = (3.0 * sr) as usize;
                let mut out = vec![0.0f32; total];
                let damp_i = (2.0 * sr) as usize;
                let mut i = 0usize;
                for chunk in out.chunks_mut(128) {
                    if i <= damp_i && damp_i < i + chunk.len() {
                        if let Kernel::Pluck(ref mut p) = v {
                            p.damp();
                        }
                    }
                    if let Kernel::Pluck(ref mut p) = v {
                        p.render(chunk);
                    }
                    i += chunk.len();
                }
                let pre = &out[damp_i - (sr * 0.45) as usize..damp_i];
                let pre_rms =
                    (pre.iter().map(|x| x * x).sum::<f32>() / pre.len() as f32).sqrt();
                let post_peak = out[damp_i..damp_i + (sr * 0.4) as usize]
                    .iter()
                    .fold(0.0f32, |m, &x| m.max(x.abs()));
                assert!(
                    post_peak < 8.0 * pre_rms.max(1e-6),
                    "midi {midi} sr {sr}: post-off peak {post_peak} vs pre rms {pre_rms}"
                );
            }
        }
    }
    // -----------------------------------------------------------------------
    // Electric piano (EP round 2026-07-12: kill the "too much like marimba"
    // tells — tine/tonebar + asymmetric pickup, NOT a struck bar). These lock
    // the mechanisms that separate an EP from the modal mallets it used to
    // resemble; if a later round legitimately shifts an axis, move the number
    // WITH a justification in the round report (loop-protocol hard rule).
    // -----------------------------------------------------------------------

    fn render_epiano(midi: u32, vel: f32, sr: f32, secs: f32, off_at: Option<f32>) -> Vec<f32> {
        let f0 = midi_to_hz(midi as f32);
        let mut m = ModalVoice::start_epiano(midi, f0, vel, sr, 12345);
        let total = (secs * sr) as usize;
        let off = off_at.map(|t| (t * sr) as usize);
        let mut out = vec![0.0f32; total];
        let mut done = 0usize;
        for chunk in out.chunks_mut(128) {
            if let Some(o) = off {
                if done < o && done + chunk.len() >= o {
                    m.damp(sr);
                }
            }
            m.render(chunk);
            done += chunk.len();
        }
        out
    }

    /// h_k / h1 in dB over an early body window (Goertzel at exact k·f0).
    fn ep_h_rel_db(x: &[f32], sr: f32, f0: f32, k: u32, t0: f32, t1: f32) -> f32 {
        let seg = &x[(t0 * sr) as usize..((t1 * sr) as usize).min(x.len())];
        let h1 = band_energy(seg, sr, &[f0]);
        let hk = band_energy(seg, sr, &[k as f32 * f0]);
        10.0 * (hk / (h1 + 1e-30)).log10()
    }

    /// The velocity→timbre axis IS the pickup nonlinearity: the second
    /// harmonic (even ⇒ only an ASYMMETRIC transfer can make it — a tanh
    /// cannot) must GROW monotonically pp→mf→ff by a wide margin. The old
    /// modal preset drove a symmetric tanh: h2 was a fixed painted mode, near
    /// velocity-flat and tiny. Measured at C3 both deploy rates.
    #[test]
    fn epiano_pickup_bark_grows_with_velocity() {
        for sr in [48_000.0f32, 44_100.0] {
            let f0 = midi_to_hz(48.0);
            let h2: Vec<f32> = [0.15f32, 0.6, 0.95]
                .iter()
                .map(|&v| {
                    let out = render_epiano(48, v, sr, 1.0, None);
                    ep_h_rel_db(&out, sr, f0, 2, 0.05, 0.45)
                })
                .collect();
            assert!(
                h2[1] > h2[0] + 4.0 && h2[2] > h2[1] + 4.0,
                "sr {sr}: h2/h1 must grow with velocity (pickup bark), got {h2:?} dB"
            );
            // ff is genuinely bark-y: h2 within ~12 dB of the fundamental
            assert!(h2[2] > -12.0, "sr {sr}: ff h2/h1 too clean at {} dB", h2[2]);
        }
    }

    /// NOT a tuned bar. The old modal preset was a SPARSE inharmonic ladder
    /// (f0, 3.97·f0, 6.24·f0 — no h2, no h3, no h5: exactly a marimba/vibe
    /// mode set, the tell the owner heard). The EP is a FILLED harmonic comb
    /// through the pickup: assert two things a struck bar cannot do —
    ///   (a) the even comb is present — h2 is NOT dwarfed by the ~4·f0 band
    ///       (on a bar the 3.98 mode dominates and h2 is ~40 dB down);
    ///   (b) the true inter-harmonic gaps (3.5·f0, 4.5·f0) are dead, i.e. the
    ///       partials sit ON the harmonic comb, not at inharmonic bar ratios.
    #[test]
    fn epiano_is_a_comb_not_a_struck_bar() {
        let sr = 48_000.0f32;
        let f0 = midi_to_hz(48.0);
        let out = render_epiano(48, 0.95, sr, 1.0, None);
        let seg = &out[(0.05 * sr) as usize..(0.45 * sr) as usize];
        let h1 = band_energy(seg, sr, &[f0]);
        let h2 = band_energy(seg, sr, &[2.0 * f0]);
        let bar_band = band_energy(seg, sr, &[3.9 * f0, 4.0 * f0]);
        // (a) even comb present: h2 at least as strong as the 4·f0 region
        let comb = 10.0 * (h2 / (bar_band + 1e-30)).log10();
        assert!(comb > -3.0, "even comb missing (bar-like): h2 vs 4f0 {comb} dB");
        // (b) inter-harmonic gaps dead ⇒ no inharmonic bar mode
        let gap = band_energy(seg, sr, &[3.5 * f0, 4.5 * f0]);
        let gap_rel = 10.0 * (gap / (h1 + 1e-30)).log10();
        assert!(gap_rel < -25.0, "inharmonic energy in the gaps: {gap_rel} dB rel f0");
    }

    /// Long singing sustain with a two-stage decay: the tine drains while the
    /// mistuned tonebar sings on (aftersound). A struck bar is a single fast
    /// exponential with no sustain stage. Assert the note is still clearly
    /// alive well after a marimba would be silent, at both rates.
    #[test]
    fn epiano_sings_with_two_stage_sustain() {
        for sr in [48_000.0f32, 44_100.0] {
            let out = render_epiano(36, 0.8, sr, 3.0, None);
            let rms = |a: f32, b: f32| -> f32 {
                let s = &out[(a * sr) as usize..(b * sr) as usize];
                (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
            };
            let early = rms(0.1, 0.3);
            let late = rms(2.0, 2.5);
            // still singing at 2 s (a marimba bass note is long dead)
            assert!(late > early * 0.02, "sr {sr}: EP sustain died: late {late} early {early}");
            assert!(out.iter().all(|s| s.is_finite()), "sr {sr}: non-finite");
        }
    }

    /// The tine+tonebar pair beats slowly (mistuned resonators), and the note
    /// damps on release (felt). Assert a slow amplitude modulation exists in
    /// the sustain and that note-off actually shortens the tail.
    #[test]
    fn epiano_beats_and_damps_on_release() {
        let sr = 48_000.0f32;
        // held: measure fundamental-band modulation depth in the sustain
        let held = render_epiano(48, 0.7, sr, 3.0, None);
        let f0 = midi_to_hz(48.0);
        let w = core::f32::consts::TAU * f0 / sr;
        // heterodyne envelope (0.5..2.5 s), boxcar-smoothed
        let (a, b) = ((0.5 * sr) as usize, (2.5 * sr) as usize);
        let mut env = Vec::new();
        let win = (0.04 * sr) as usize;
        let mut i = a;
        while i + win < b {
            let mut re = 0.0f32;
            let mut im = 0.0f32;
            for j in 0..win {
                let ph = w * (i + j) as f32;
                re += held[i + j] * ph.cos();
                im += held[i + j] * ph.sin();
            }
            env.push((re * re + im * im).sqrt());
            i += win;
        }
        let emax = env.iter().cloned().fold(0.0f32, f32::max);
        let emin = env.iter().cloned().fold(f32::INFINITY, f32::min);
        let depth = 20.0 * (emax / (emin + 1e-12)).log10();
        assert!(depth > 1.0, "no tine/tonebar beat in sustain: {depth} dB");
        // released tail must be quieter than the held tail at the same time
        let released = render_epiano(48, 0.7, sr, 3.0, Some(1.0));
        let tail = |x: &[f32]| -> f32 {
            let s = &x[(2.0 * sr) as usize..(2.5 * sr) as usize];
            (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        assert!(
            tail(&released) < tail(&held) * 0.6,
            "release did not damp the tine: released {} held {}",
            tail(&released),
            tail(&held)
        );
    }

    /// Budget guard (loop-protocol: ≤40 µs/quantum for 8 EP voices). The EP
    /// path is a 3-mode resonator + one exp() + one exp() in the pickup +
    /// a one-pole per sample — comfortably under. Printed, not asserted on
    /// wall-clock (CI jitter), but flags a regression by eye.
    #[test]
    fn probe_epiano_render_cost() {
        use std::time::Instant;
        let mut voices: Vec<ModalVoice> = (0..8)
            .map(|i| ModalVoice::start_epiano(36 + i * 6, midi_to_hz((36 + i * 6) as f32), 0.9, 48_000.0, 99 + i))
            .collect();
        let mut block = [0.0f32; 128];
        for _ in 0..100 {
            for v in voices.iter_mut() {
                v.render(&mut block);
            }
        }
        let n = 2000;
        let t0 = Instant::now();
        for _ in 0..n {
            block.fill(0.0);
            for v in voices.iter_mut() {
                v.render(&mut block);
            }
            core::hint::black_box(&block);
        }
        let us = t0.elapsed().as_micros() as f64 / n as f64;
        println!("epiano: {us:.1} us/quantum @8 voices (native)");
    }
}

// ---------------------------------------------------------------------------
// Cymbal tests (ride/crash/hat code paths only)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod cymbal_tests {
    use super::*;

    /// Render a drum GM note to a mono buffer until the voice ends (cap 8 s).
    fn render_drum(gm: u32, vel: f32, sr: f32) -> Vec<f32> {
        render_drum_kit(gm, vel, sr, KitStyle::Pop)
    }

    fn render_drum_kit(gm: u32, vel: f32, sr: f32, kit: KitStyle) -> Vec<f32> {
        let mut v = DrumVoice::start(gm, vel, sr, 0x1234_5678, kit);
        let mut out = Vec::new();
        let mut block = [0.0f32; 128];
        for _ in 0..(8.0 * sr / 128.0) as usize {
            block.fill(0.0);
            let alive = v.render(&mut block, sr);
            out.extend_from_slice(&block);
            if !alive {
                return out;
            }
        }
        panic!("drum voice for GM {gm} did not terminate within 8 s");
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Two-pole bandpass RMS in a time window — same resonator form as the DSP.
    fn band_rms(x: &[f32], f: f32, sr: f32, t0: f32, t1: f32) -> f32 {
        let r = t60_gain(0.030, sr);
        let w = core::f32::consts::TAU * f / sr;
        let g = (1.0 - r) * w.sin();
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        let (a1, r2) = (2.0 * r * w.cos(), r * r);
        let (i0, i1) = ((t0 * sr) as usize, ((t1 * sr) as usize).min(x.len()));
        let mut acc = 0.0;
        for (i, &s) in x.iter().enumerate().take(i1) {
            let y = a1 * y1 - r2 * y2 + g * s;
            y2 = y1;
            y1 = y;
            if i >= i0 {
                acc += y * y;
            }
        }
        (acc / (i1 - i0).max(1) as f32).sqrt()
    }

    #[test]
    fn cymbals_finite_bounded_and_terminate_at_both_rates() {
        for &sr in &[44100.0f32, 48000.0] {
            for &gm in &[42u32, 44, 46, 49, 51, 52, 53, 55, 57, 59] {
                let out = render_drum(gm, 1.0, sr);
                for (i, &s) in out.iter().enumerate() {
                    assert!(s.is_finite(), "GM {gm} sr {sr}: non-finite at {i}");
                    assert!(s.abs() <= 2.0, "GM {gm} sr {sr}: |{s}| > 2 at {i}");
                }
            }
        }
    }

    #[test]
    fn cymbal_velocity_is_monotonic() {
        for &gm in &[42u32, 46, 49, 51, 52, 53, 55] {
            let soft = rms(&render_drum(gm, 0.3, 48000.0));
            let hard = rms(&render_drum(gm, 1.0, 48000.0));
            assert!(
                hard > soft * 1.3,
                "GM {gm}: hard hit ({hard}) not louder than soft ({soft})"
            );
        }
    }

    /// Ride bloom signature measured on the CC0 reference: the 1–2 kHz wash
    /// arrives late (band peaks ~60 ms after the strike, NOT in the first
    /// frames) while the strike itself is immediate.
    #[test]
    fn ride_mid_band_blooms_after_strike() {
        let out = render_drum(51, 0.8, 48000.0);
        let early = band_rms(&out, 1500.0, 48000.0, 0.005, 0.030);
        let late = band_rms(&out, 1500.0, 48000.0, 0.045, 0.110);
        assert!(
            late > early * 1.05,
            "ride 1.5 kHz band should bloom after the strike: early {early}, late {late}"
        );
    }

    /// Crash bloom: the whole envelope keeps RISING for >35 ms after the hit
    /// (reference peaks at ~46 ms; brightness floods in via mode coupling).
    #[test]
    fn crash_envelope_blooms() {
        let out = render_drum(49, 1.0, 48000.0);
        let early = rms(&out[0..(0.035 * 48000.0) as usize]);
        let late = rms(&out[(0.060 * 48000.0) as usize..(0.120 * 48000.0) as usize]);
        assert!(late > early, "crash should still be blooming at 60–120 ms: {early} vs {late}");
    }

    /// Kernel-level choke: DrumVoice::choke on a ringing open hat collapses
    /// the tail fast and terminates the voice within ~0.25 s — on every kit.
    #[test]
    fn drum_choke_collapses_open_hat_tail() {
        let sr = 48_000.0f32;
        for kit in [KitStyle::Pop, KitStyle::Rock, KitStyle::Jazz] {
            let mut v = DrumVoice::start(46, 0.9, sr, 0x777, kit);
            let mut block = [0.0f32; 128];
            for _ in 0..(0.4 * sr / 128.0) as usize {
                block.fill(0.0);
                v.render(&mut block, sr);
            }
            block.fill(0.0);
            v.render(&mut block, sr);
            let pre = rms(&block);
            assert!(pre > 1e-4, "{kit:?}: open hat should still ring at 0.4 s");
            v.choke(sr);
            let mut blocks_alive = 0usize;
            let mut post = 0.0f32;
            loop {
                block.fill(0.0);
                let alive = v.render(&mut block, sr);
                blocks_alive += 1;
                if blocks_alive == (0.10 * sr / 128.0) as usize {
                    post = rms(&block); // 100 ms after the choke
                }
                if !alive {
                    break;
                }
                assert!(
                    (blocks_alive as f32) < 0.25 * sr / 128.0,
                    "{kit:?}: choked voice failed to terminate within 0.25 s"
                );
            }
            assert!(
                post < pre * 0.05,
                "{kit:?}: choked tail too loud 100 ms in: pre {pre} post {post}"
            );
        }
    }

    /// The ride must actually RING: audible (> −50 dB rel peak) past 2.5 s.
    #[test]
    fn ride_tail_rings_long() {
        let out = render_drum(51, 0.8, 48000.0);
        assert!(out.len() as f32 / 48000.0 > 3.0, "ride voice ends too early");
        let tail = rms(&out[(2.4 * 48000.0) as usize..(2.6 * 48000.0) as usize]);
        assert!(tail > 1e-4, "ride tail inaudible at 2.5 s: rms {tail}");
        // closed hat, by contrast, must be short
        let hat = render_drum(42, 0.8, 48000.0);
        assert!((hat.len() as f32) < 1.0 * 48000.0, "closed hat rings too long");
    }
}

// ---------------------------------------------------------------------------
// Drum-kit round-3 regression tests (kick transient truth, snare crack,
// brush articulation, kit-voiced toms)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod drum_kit_tests {
    use super::*;

    fn render_kit(gm: u32, vel: f32, sr: f32, kit: KitStyle, secs: f32) -> Vec<f32> {
        let mut v = DrumVoice::start(gm, vel, sr, 0x2468_ace0, kit);
        let mut out = Vec::new();
        let mut block = [0.0f32; 128];
        for _ in 0..(secs * sr / 128.0) as usize {
            block.fill(0.0);
            let alive = v.render(&mut block, sr);
            out.extend_from_slice(&block);
            if !alive {
                break;
            }
        }
        out
    }

    /// Goertzel-style band magnitude over a time window.
    fn band_mag(x: &[f32], sr: f32, f0: f32, f1: f32, t0: f32, t1: f32) -> f32 {
        let (i0, i1) = ((t0 * sr) as usize, ((t1 * sr) as usize).min(x.len()));
        let seg = &x[i0..i1];
        let mut acc = 0.0f32;
        let mut f = f0;
        while f < f1 {
            let w = core::f32::consts::TAU * f / sr;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (n, &s) in seg.iter().enumerate() {
                let ph = w * n as f32;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            acc += re * re + im * im;
            f *= 1.12;
        }
        acc.sqrt()
    }

    /// Dominant frequency in [lo, hi] via dense Goertzel scan.
    fn peak_hz(x: &[f32], sr: f32, lo: f32, hi: f32, t0: f32, t1: f32) -> f32 {
        let (i0, i1) = ((t0 * sr) as usize, ((t1 * sr) as usize).min(x.len()));
        let seg = &x[i0..i1];
        let mut best = (0.0f32, lo);
        let mut f = lo;
        while f < hi {
            let w = core::f32::consts::TAU * f / sr;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (n, &s) in seg.iter().enumerate() {
                let ph = w * n as f32;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            let m = re * re + im * im;
            if m > best.0 {
                best = (m, f);
            }
            f *= 1.01;
        }
        best.1
    }

    fn env_peak_ms(x: &[f32], sr: f32) -> f32 {
        let hop = (0.002 * sr) as usize;
        let mut best = (0.0f32, 0usize);
        for (k, ch) in x.chunks(hop).enumerate().take(60) {
            let r: f32 = ch.iter().map(|s| s * s).sum::<f32>() / ch.len() as f32;
            if r > best.0 {
                best = (r, k);
            }
        }
        best.1 as f32 * 2.0
    }

    /// Round-3 kick truth: the pitch reads as a TRANSIENT (settled within
    /// 12% of steady by ~20 ms; start elevation bounded) — not a dance sweep.
    /// Checked at both rates on every kit.
    #[test]
    fn kick_glide_is_a_transient() {
        for &sr in &[44_100.0f32, 48_000.0] {
            for kit in [KitStyle::Pop, KitStyle::Rock, KitStyle::Jazz] {
                let out = render_kit(36, 1.0, sr, kit, 0.6);
                let early = peak_hz(&out, sr, 35.0, 130.0, 0.020, 0.080);
                let steady = peak_hz(&out, sr, 35.0, 130.0, 0.080, 0.240);
                let ratio = early / steady;
                assert!(
                    (0.85..1.13).contains(&ratio),
                    "{kit:?} sr {sr}: pitch not settled by 20 ms (early {early:.1} vs steady {steady:.1})"
                );
            }
        }
    }

    /// Round-3 two-stage tail: the open jazz kick still rings at 0.6 s
    /// (resonant-head mode) while the muffled rock kick has died.
    #[test]
    fn kick_two_stage_tail_is_kit_voiced() {
        let sr = 48_000.0f32;
        let jazz = render_kit(36, 1.0, sr, KitStyle::Jazz, 1.5);
        let rock = render_kit(36, 1.0, sr, KitStyle::Rock, 1.5);
        let late = |x: &[f32]| {
            let (i0, i1) = ((0.55 * sr) as usize, (0.70 * sr) as usize);
            if x.len() < i1 {
                return 0.0;
            }
            (x[i0..i1].iter().map(|s| s * s).sum::<f32>() / (i1 - i0) as f32).sqrt()
        };
        let (lj, lr) = (late(&jazz), late(&rock));
        assert!(lj > 1e-4, "jazz kick tail dead at 0.6 s: {lj}");
        assert!(lr < lj * 0.5, "rock kick should be muffled vs jazz: rock {lr} jazz {lj}");
    }

    /// Kick-specialist round (2026-07-12): the three kits' kicks are
    /// DIFFERENT INSTRUMENTS, each fit to its own reference drum —
    /// jazz 18"-class bop kick ~76 Hz (DRSKit), rock 22" ~48 Hz
    /// (MuldjordKit), pop tight 54 Hz (virtuosity). Checked at both rates.
    #[test]
    fn kick_kits_are_different_instruments() {
        for &sr in &[44_100.0f32, 48_000.0] {
            let jazz = render_kit(36, 1.0, sr, KitStyle::Jazz, 0.8);
            let rock = render_kit(36, 1.0, sr, KitStyle::Rock, 0.8);
            let pop = render_kit(36, 1.0, sr, KitStyle::Pop, 0.8);
            let fj = peak_hz(&jazz, sr, 35.0, 130.0, 0.08, 0.24);
            let fr = peak_hz(&rock, sr, 35.0, 130.0, 0.08, 0.24);
            let fp = peak_hz(&pop, sr, 35.0, 130.0, 0.08, 0.24);
            assert!((65.0..95.0).contains(&fj), "jazz kick fundamental {fj} Hz (want 65-95)");
            assert!((40.0..56.0).contains(&fr), "rock kick fundamental {fr} Hz (want 40-56)");
            assert!((48.0..62.0).contains(&fp), "pop kick fundamental {fp} Hz (want 48-62)");
            assert!(
                fj / fr > 1.35,
                "sr {sr}: jazz ({fj}) must sit a clear interval above rock ({fr})"
            );
        }
    }

    /// Felt vs hard beater: the rock kick's 2–6 kHz beater band (first 30 ms,
    /// ff) stands at least 10 dB closer to its LF body than the jazz kick's
    /// on the coarse Goertzel scale (this fit measures ~12.5 dB; FFT band
    /// energies read rock −19.7 dB rel LF vs jazz −43). And the rock hit
    /// is the spikier waveform (refs: muld ff crest 15.2, DRS ff crest 8.3).
    #[test]
    fn kick_beater_contrast_is_kit_voiced() {
        let sr = 48_000.0f32;
        let jazz = render_kit(36, 0.95, sr, KitStyle::Jazz, 0.3);
        let rock = render_kit(36, 0.95, sr, KitStyle::Rock, 0.3);
        let rel = |x: &[f32]| {
            let lf = band_mag(x, sr, 30.0, 150.0, 0.0, 0.030);
            let hf = band_mag(x, sr, 2_000.0, 6_000.0, 0.0, 0.030);
            20.0 * (hf / lf.max(1e-9)).log10()
        };
        let (rj, rr) = (rel(&jazz), rel(&rock));
        assert!(
            rr > rj + 10.0,
            "hard-beater rock ({rr:.1} dB rel LF) vs felt jazz ({rj:.1}) contrast too small"
        );
        let crest = |x: &[f32]| {
            let n = ((0.5 * sr) as usize).min(x.len());
            let seg = &x[..n];
            let pk = seg.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            let rms = (seg.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt();
            pk / rms.max(1e-9)
        };
        let (cj, cr) = (crest(&jazz), crest(&rock));
        assert!(
            cr > cj * 1.35,
            "rock kick should be spikier than jazz: crest rock {cr:.1} jazz {cj:.1}"
        );
    }

    /// Round-3 rock snare authority: the 400–1500 Hz crack band holds within
    /// 8 dB of the 40–150 Hz band over the first 30 ms of an ff backbeat.
    #[test]
    fn rock_snare_has_crack_band() {
        let sr = 48_000.0f32;
        let out = render_kit(38, 0.95, sr, KitStyle::Rock, 0.4);
        let lf = band_mag(&out, sr, 40.0, 150.0, 0.0, 0.030);
        let crack = band_mag(&out, sr, 400.0, 1500.0, 0.0, 0.030);
        let rel = 20.0 * (crack / lf.max(1e-9)).log10();
        // regression tripwire: the round-2 white-hiss snare measured ~-20 dB
        // on this scale; the round-3 fit sits around -10
        assert!(rel > -12.0, "rock snare crack band too weak: {rel:.1} dB rel LF");
    }

    /// Round-3 jazz brush: GM 38 on the jazz kit peaks late (bristle arrival,
    /// >8 ms) and darker than the stick snare on GM 40 (which stays fast).
    #[test]
    fn jazz_gm38_is_a_brush() {
        let sr = 48_000.0f32;
        let brush = render_kit(38, 0.8, sr, KitStyle::Jazz, 0.5);
        let stick = render_kit(40, 0.8, sr, KitStyle::Jazz, 0.5);
        let (tb, ts) = (env_peak_ms(&brush, sr), env_peak_ms(&stick, sr));
        assert!(tb >= 8.0, "brush attack too fast: peak at {tb} ms");
        assert!(ts <= 6.0, "stick snare attack too slow: peak at {ts} ms");
        let hf_b = band_mag(&brush, sr, 6_000.0, 12_000.0, 0.0, 0.05);
        let mid_b = band_mag(&brush, sr, 500.0, 3_000.0, 0.0, 0.05);
        assert!(mid_b > hf_b, "brush should be mid-forward, not hissy");
    }

    /// Round-3 toms: kit-voiced tuning (jazz higher than rock on the same GM
    /// note) and kit-voiced length (rock rings longer). Both rates.
    #[test]
    fn toms_are_kit_voiced() {
        for &sr in &[44_100.0f32, 48_000.0] {
            let rock = render_kit(45, 0.85, sr, KitStyle::Rock, 1.2);
            let jazz = render_kit(45, 0.85, sr, KitStyle::Jazz, 1.2);
            let fr = peak_hz(&rock, sr, 50.0, 260.0, 0.05, 0.30);
            let fj = peak_hz(&jazz, sr, 50.0, 260.0, 0.05, 0.30);
            assert!(
                fj > fr * 1.15,
                "sr {sr}: jazz tom ({fj:.1} Hz) should sit well above rock ({fr:.1} Hz)"
            );
            let late = |x: &[f32]| {
                let (i0, i1) = ((0.35 * sr) as usize, (0.45 * sr) as usize);
                if x.len() < i1 {
                    return 0.0;
                }
                (x[i0..i1].iter().map(|s| s * s).sum::<f32>() / (i1 - i0) as f32).sqrt()
            };
            assert!(
                late(&rock) > late(&jazz),
                "sr {sr}: rock tom should ring longer than jazz"
            );
        }
    }
}

#[cfg(test)]
mod bench_probe {
    use super::*;
    use std::time::Instant;

    #[test]
    fn probe_render_cost() {
        for (name, inst, base) in [
            ("nylon", Instrument::Guitar, 40u32),
            ("steel", Instrument::GuitarSteel, 40),
            ("bass", Instrument::Bass, 28),
        ] {
            let mut voices: Vec<Kernel> = (0..8)
                .map(|i| start_voice(inst, base + i * 3, 0.9, 48_000.0, 99 + i))
                .collect();
            let mut block = [0.0f32; 128];
            // warmup
            for _ in 0..100 {
                for v in voices.iter_mut() {
                    if let Kernel::Pluck(p) = v {
                        p.render(&mut block);
                    }
                }
            }
            let t0 = Instant::now();
            let n = 2000;
            for _ in 0..n {
                block.fill(0.0);
                for v in voices.iter_mut() {
                    if let Kernel::Pluck(p) = v {
                        p.render(&mut block);
                    }
                }
                core::hint::black_box(&block);
            }
            println!("{name}: {:.1} us/quantum @8 voices (native)", t0.elapsed().as_micros() as f64 / n as f64);
        }
    }

    #[test]
    fn probe_note_on_cost() {
        for (name, inst, base) in [
            ("nylon", Instrument::Guitar, 40u32),
            ("steel", Instrument::GuitarSteel, 40),
            ("bass", Instrument::Bass, 28),
        ] {
            for midi in [base, base + 12, base + 24] {
                let t0 = Instant::now();
                for i in 0..50 {
                    let k = start_voice(inst, midi, 0.9, 48_000.0, 1234 + i);
                    core::hint::black_box(&k);
                }
                println!("{name} midi {midi}: {:.1} us/note-on", t0.elapsed().as_micros() as f64 / 50.0);
            }
        }
    }
}
