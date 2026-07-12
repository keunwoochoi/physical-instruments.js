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
        Instrument::GuitarElectric => (1.6, 1800.0),
        // high gain: preamp gain must keep the tanh saturated for seconds so the
        // note SINGS while the string decays >20 dB (drive 11 fell linear after
        // ~1 s — a crunch, not a lead channel); tone at 3.4 kHz keeps the
        // regenerated clip harmonics under the fizz gate
        Instrument::GuitarDistorted => (45.0, 3400.0),
        _ => (0.0, 0.0),
    }
}

/// Magnetic-pickup resonance per instrument: (resonant-lowpass Hz, Q). The RLC
/// resonance of a real pickup is the core "electric" tone; 0.0 = bypass.
pub fn pickup_defaults(inst: Instrument) -> (f32, f32) {
    match inst {
        // NSynth guitar_electronic refs cliff at a pitch-independent ~1.2-1.5 kHz:
        // a heavily loaded pickup + rolled tone pot pulls the RLC resonance down
        // and damps its Q (Zollner, Physics of the Electric Guitar, ch. 5).
        Instrument::GuitarElectric => (1500.0, 1.2),
        // distorted: vocal mid hump BEFORE the drive (TS-style pre-emphasis; the
        // in-voice differentiator already tightens the lows pre-drive)
        Instrument::GuitarDistorted => (1600.0, 2.8),
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
        Instrument::Guitar => (
            0.22,
            &[
                (100.0, 0.30, 0.0236),  // P=0.9 A0 Helmholtz
                (190.0, 0.25, 0.0696),  // P=1.4 T1
                (285.0, 0.16, 0.1194),  // P=1.6 T2/back
                (340.0, 0.14, 0.1602),  // P=1.8
                (425.0, 0.12, 0.2113),  // P=1.9
                (520.0, 0.11, 0.2041),  // P=1.5
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
        // Steel (refs 015/030/021), 16-mode round-2 refit (same method): refs
        // concentrate energy near 258 and 705 Hz far more than the old plateau;
        // 90–235 / 300–390 / 480–610 / ~1850 trimmed relative to those anchors;
        // 2.4k/3.6k presence kept (round-1 attack match).
        Instrument::GuitarSteel => (
            0.28,
            &[
                (100.0, 0.28, 0.0131),  // P=0.5 A0
                (190.0, 0.20, 0.0274),  // P=0.55
                (258.0, 0.22, 0.1216),  // P=1.8 T1' ref peak
                (295.0, 0.18, 0.1081),  // P=1.4
                (350.0, 0.15, 0.0641),  // P=0.7
                (415.0, 0.14, 0.1304),  // P=1.2
                (505.0, 0.12, 0.1058),  // P=0.8
                (630.0, 0.12, 0.2639),  // P=1.6
                (705.0, 0.10, 0.3507),  // P=1.9 ref peak
                (810.0, 0.09, 0.2121),  // P=1.0
                (940.0, 0.085, 0.3199), // P=1.3
                (1180.0, 0.07, 0.2780), // P=0.9
                (1450.0, 0.065, 0.4151), // P=1.1
                (1850.0, 0.055, 0.3357), // P=0.7
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
        Instrument::Glockenspiel => 28.0, // was -39.6 LUFS (tiny raw kernel level)
        Instrument::MusicBox => 14.8,     // was -35.6 LUFS
        Instrument::Guitar => 0.126,       // round-2 re-bake (body refit + release; was -22.1 LUFS at 0.194)
        Instrument::Bass => 0.63,         // round-2 re-bake (DI tilt body)
        Instrument::EPiano => 1.47,       // was -26.6 LUFS
        Instrument::Drums => 0.61,        // was -27.4 LUFS
        Instrument::SynthPad => 0.48,     // was -26.5 LUFS
        Instrument::Piano => 0.084, // piano r2 re-bake (decay-geometry rework)
        Instrument::GuitarSteel => 0.46,    // acoustics r2 re-bake (HF floor + 16-mode body)
        Instrument::GuitarElectric => 0.78, // electric r2 re-bake (022 dark voicing)
        Instrument::GuitarDistorted => 0.23, // electric r2 re-bake (drive 45 lead channel)
        Instrument::DrumsRock => 0.43,      // measured 2026-07-11 (pyloudnorm -22.6 pre-bake)
        Instrument::DrumsJazz => 0.59,      // measured 2026-07-11 (pyloudnorm -25.5 pre-bake)
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

    /// Render one block, ADD into `out`. Returns false when the voice is spent.
    pub fn render(&mut self, out: &mut [f32]) -> bool {
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
        self.life = self.age + (0.12 * sr) as u64;
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
    /// tension modulation: hard plucks start sharp and settle (band-limited —
    /// the fractional-delay allpass coefficient follows a smoothed env²).
    tm_dev: f32,
    tm_env: f32,
    tm_c: f32,
    tm_norm: f32,
    frac1: f32,
    frac2: f32,
    /// direct contact-click transient: decaying HP-shaped noise added to the
    /// output (attack splash / release pluck-off), not stored in the string
    tr_env: f32,
    tr_dec: f32,
    tr_hp: f32,
    tr_rng: Lcg,
    /// release voicing: post-note-off t60 and pluck-off click level
    rel_t60: f32,
    rel_click: f32,
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
    /// the HF loss floor, keeping it all in-loop ran +30…+47 dB hot mid-note
    pub click: f32,
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
    /// bridge differencer leak (0 = raw displacement out)
    pub br_rho: f32,
    pub level: f32,
}

/// One-pole loop lowpass y += c(x−y): magnitude at ω (for loss calibration).
#[inline]
fn onepole_mag(c: f32, w: f32) -> f32 {
    let b = 1.0 - c;
    c / (1.0 - 2.0 * b * w.cos() + b * b).sqrt()
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
    let b = 1.0 - c;
    let (sw, cw) = w.sin_cos();
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
fn design_dispersion(b: f32, f0: f32, sr: f32, lp_c: f32, lp_mix: f32) -> (usize, f32) {
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
    // loop lowpass already contributes
    let d_geom = n0 * (1.0 / (1.0 + b).sqrt() - 1.0 / (1.0 + b * n_star * n_star).sqrt());
    let d_lp = blend_delay(lp_c, lp_mix, w1) - blend_delay(lp_c, lp_mix, wn);
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
            let d_lp = -him.atan2(hre) / w;
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
        let hlp = (lp_c as f64 * dre / dd, lp_c as f64 * dim / dd);
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
            tm_dev: 0.0,
            tm_env: 0.0,
            tm_c: 0.0,
            tm_norm: 0.0,
            frac1: 0.0,
            frac2: 0.0,
            tr_env: 0.0,
            tr_dec: 0.0,
            tr_hp: 0.0,
            tr_rng: Lcg(1),
            rel_t60: 0.0,
            rel_click: 0.0,
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
        // Keep |H_lp(f0)| ≥ target per-period gain so loss ≤ 1 (loop stable at
        // every frequency including DC). If the requested loop filter is too dark
        // to sustain the fundamental, brighten it minimally — physically, thin
        // sustaining trebles are never felt-dark.
        let g0 = per_period_gain(p.t60_f0, p.f0);
        // Loss filter: pure one-pole, or (steel) the blend ladder-over-floor —
        // knee placed via |H_lp(knee)| = ½ (bisected), bypass from the target
        // HF-floor t60 relative to the fundamental's.
        let (mut lp_c, lp_mix) = if p.hf_floor_t60 > 0.0 {
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
        while blend_mag(lp_c, lp_mix, w0) < g0 && lp_c < 0.99 {
            lp_c += 0.01;
        }
        let loss = (g0 / blend_mag(lp_c, lp_mix, w0)).min(0.99995);

        // Stiffness dispersion cascade (solved per note), then tuning: subtract
        // the exact loop-filter + cascade phase delays at f0 so partial 1 stays
        // on pitch while uppers stretch.
        let (disp_n, disp_p) = design_dispersion(p.stiff_b, p.f0, sr, lp_c, lp_mix);
        let disp_a = -disp_p;
        let d_lp = blend_delay(lp_c, lp_mix, w0);
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
        let g2 = per_period_gain(p.t60_f0 * p.pol_t60_ratio.max(0.05), f2);
        let loss2 = (g2 / blend_mag(lp_c, lp_mix, w0)).min(0.99995);

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
        let level = if p.br_rho > 0.0 {
            let w3 = (3.0 * w0).min(core::f32::consts::FRAC_PI_2);
            let mag = (1.0 - 2.0 * p.br_rho * w3.cos() + p.br_rho * p.br_rho).sqrt();
            p.level / mag.max(1e-3)
        } else {
            p.level
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
            life: (((p.t60_f0 * p.pol_t60_ratio.max(1.0) * 1.1 + 0.5).min(18.0)) * sr) as u64,
            sr,
            len2,
            loss2,
            ap2_c: (1.0 - frac2) / (1.0 + frac2),
            pol_mix: p.pol_mix,
            disp_a,
            disp_n: disp_n as u8,
            br_rho: p.br_rho,
            tm_dev,
            tm_c: 1.0 - (-core::f32::consts::TAU * 6.0 / sr).exp(),
            frac1,
            frac2,
            // contact click: ~35 ms HP-shaped noise burst, velocity-scaled like
            // the snap; scaled by `level` since it bypasses the loop tap
            tr_env: p.click * (0.35 + 0.65 * p.vel) * level,
            tr_dec: t60_gain(0.035, sr),
            tr_rng: Lcg(seed.rotate_left(13) | 1),
            rel_t60: p.rel_t60,
            rel_click: p.rel_click,
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
            v.ap_c,
            p.f0,
            p.stiff_b,
            sr,
            p.pick_pos,
            cw,
            n_syn,
        );
        v.lp = st.lp as f32;
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
                v.ap2_c,
                f2,
                p.stiff_b,
                sr,
                p.pick_pos,
                cw,
                n_syn,
            );
            v.lp2 = st2.lp as f32;
            v.ap2_x1 = st2.apx as f32;
            v.ap2_y1 = st2.apy as f32;
            for k in 0..disp_n {
                v.ds2x[k] = st2.dsx[k] as f32;
                v.ds2y[k] = st2.dsy[k] as f32;
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
        for o in out.iter_mut() {
            let y = self.buf[self.pos];
            // blend loss filter: one-pole ladder over a flat HF bypass floor
            // (m·y + (1−m)·lp ≡ lp + m(y−lp); m = 0 is the pure one-pole)
            lp += self.lp_c * (y - lp);
            // stiffness dispersion: M-stage allpass cascade delays lows vs highs
            // (pole at +p ⇒ stretched partials, see design_dispersion)
            let mut s = self.lp_mix.mul_add(y - lp, lp);
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
            self.buf[self.pos] = ap * self.loss;
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
                for k in 0..disp_n {
                    let d = self.disp_a * (s2 - ds2y[k]) + ds2x[k];
                    ds2x[k] = s2;
                    ds2y[k] = d;
                    s2 = d;
                }
                let ap2 = self.ap2_c * (s2 - ap2_y1) + ap2_x1;
                ap2_x1 = s2;
                ap2_y1 = ap2;
                self.buf2[self.pos2] = ap2 * self.loss2;
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
            let outv = if self.br_rho > 0.0 {
                let f = mix - self.br_rho * self.br_x1;
                self.br_x1 = mix;
                f
            } else {
                mix
            };
            *o += outv * self.level;
            // direct contact-click transient (bypasses the string loop)
            if self.tr_env > 1e-7 {
                let n = self.tr_rng.next();
                *o += self.tr_env * 0.5 * (n - self.tr_hp);
                self.tr_hp = n;
                self.tr_env *= self.tr_dec;
            }
        }
        self.dsx = dsx;
        self.dsy = dsy;
        self.ds2x = ds2x;
        self.ds2y = ds2y;
        self.lp = flush_denormal(lp);
        self.ap_x1 = ap_x1;
        self.ap_y1 = flush_denormal(ap_y1);
        if self.pol_mix > 0.0 {
            self.lp2 = flush_denormal(lp2);
            self.ap2_x1 = ap2_x1;
            self.ap2_y1 = flush_denormal(ap2_y1);
        }
        for k in 0..self.disp_n as usize {
            self.dsy[k] = flush_denormal(self.dsy[k]);
            self.ds2y[k] = flush_denormal(self.ds2y[k]);
        }
        self.tm_env = flush_denormal(self.tm_env);
        self.br_x1 = flush_denormal(self.br_x1);
        self.age += out.len() as u64;
        self.age < self.life
    }

    pub fn damp(&mut self) {
        if self.f0 > 0.0 {
            // acoustic path: per-period loss toward the instrument's release
            // t60 — finger/palm damping is not instantaneous (steel refs ring
            // ~0.5 s after note-off); voice retired once the tail is spent
            let rel = self.rel_t60.max(0.05);
            self.loss = per_period_gain(rel, self.f0);
            self.loss2 = self.loss;
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
                let amp = self.rel_click * rms.min(1.0) * self.level;
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
    // two cascaded dispersion allpasses (stiffness → stretched partials)
    disp_c: f32,
    d1_x1: f32,
    d1_y1: f32,
    d2_x1: f32,
    d2_y1: f32,
    // in-loop DC blocker (~19 Hz): hammer force injection is unipolar and would
    // otherwise park a slowly-decaying DC pedestal in the loop
    dc_x1: f32,
    dc_y1: f32,
}

/// In-loop DC-blocker pole (~3 Hz at 48 k). Shared by `tick` and the delay budget:
/// the blocker's phase LEAD at f0 shortens the effective loop delay and must be
/// compensated or every string plays sharp (worst in the bass, where the old 19 Hz
/// blocker left A1 audibly sharp — found while fixing its fundamental damping).
const DC_POLE: f32 = 0.9996;

impl StringLoop {
    /// `detune_cents` shifts this string against the nominal pitch (unison beating).
    fn new(f0: f32, detune_cents: f32, sr: f32, t60: f32, lp_c: f32, disp_c: f32) -> Self {
        let f = f0 * (detune_cents / 1200.0).exp2();
        // Total loop delay budget: buffer + tuning-allpass fraction + loop-lowpass
        // phase delay + 2× dispersion-allpass DC delay (Jaffe-Smith compensation —
        // without the dispersion term the stiff strings would all play flat).
        let lp_delay = (1.0 - lp_c) / lp_c;
        let disp_delay = 2.0 * (1.0 - disp_c) / (1.0 + disp_c);
        // DC blocker H(z) = (1−z⁻¹)/(1−Rz⁻¹): exact phase lead at ω, in samples
        // (lead = negative group delay contribution → ADD to the target length).
        let w = core::f32::consts::TAU * f / sr;
        let dc_lead = ((core::f32::consts::PI - w) / 2.0
            - (DC_POLE * w.sin()).atan2(1.0 - DC_POLE * w.cos()))
            / w;
        let total = (sr / f - lp_delay - disp_delay + dc_lead).max(3.0);
        let len = ((total - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac = (total - len as f32).clamp(0.1, 1.5);
        // Loss is applied once per ROUND TRIP (each slot is rewritten every `len`
        // samples), so the per-pass gain must be exp(-6.9077·period/(t60·sr)) — a
        // per-SAMPLE t60_gain here silently inflates t60 by ×len (measured: a 49 Hz
        // fundamental rang ~flat for 3 s once the old DC blocker stopped damping it).
        // Divide out the loop filters' own magnitude at f0 so the t60 parameter
        // states the FUNDAMENTAL's decay; upper partials still die faster through
        // the lowpass (Jaffe–Smith 1983 loss factor; Välimäki et al. 1996 SDL loop).
        let period = sr / f;
        let target = (-6.907_755 * period / (t60 * sr)).exp();
        let a = 1.0 - lp_c;
        let lp_mag = lp_c / (1.0 + a * a - 2.0 * a * w.cos()).sqrt();
        let dc_mag = (2.0 - 2.0 * w.cos()).sqrt()
            / (1.0 + DC_POLE * DC_POLE - 2.0 * DC_POLE * w.cos()).sqrt();
        // cap: loop gain never reaches 1 at any frequency (|lp|,|dc| ≤ 1 elsewhere,
        // but treble strings can ask for more f0 gain than the lowpass leaves)
        let loss = (target / (lp_mag * dc_mag).max(1e-3)).min(0.999_95);
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
            disp_c,
            d1_x1: 0.0,
            d1_y1: 0.0,
            d2_x1: 0.0,
            d2_y1: 0.0,
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

    #[inline(always)]
    fn tick(&mut self) -> f32 {
        let y = self.buf[self.pos];
        // loop lowpass (frequency-dependent loss)
        self.lp += self.lp_c * (y - self.lp);
        // dispersion: two cascaded first-order allpasses delay highs vs lows
        let d1 = self.disp_c * (self.lp - self.d1_y1) + self.d1_x1;
        self.d1_x1 = self.lp;
        self.d1_y1 = d1;
        let d2 = self.disp_c * (d1 - self.d2_y1) + self.d2_x1;
        self.d2_x1 = d1;
        self.d2_y1 = d2;
        // fractional tuning allpass
        let ap = self.ap_c * (d2 - self.ap_y1) + self.ap_x1;
        self.ap_x1 = d2;
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
        self.buf[self.pos] = dc * self.loss;
        self.pos = (self.pos + 1) % self.len;
        y
    }

    fn flush(&mut self) {
        self.lp = flush_denormal(self.lp);
        self.ap_y1 = flush_denormal(self.ap_y1);
        self.d1_y1 = flush_denormal(self.d1_y1);
        self.d2_y1 = flush_denormal(self.d2_y1);
        self.dc_y1 = flush_denormal(self.dc_y1);
    }
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
    // air/radiation rolloff: fixed one-pole LP ~10 kHz. The 44.1 kHz VSCO
    // check caught the render +17 dB above 8 kHz (NSynth's 16 kHz refs are
    // blind there): board directivity + air absorption kill the top octave.
    air_c: f32,
    air_lp: f32,
    rng: Lcg,
    sr: f32,
    key: f32,
    vel: f32,
    life: u64,
    age: u64,
}

impl PianoVoice {
    pub fn start(midi: u32, f0: f32, vel: f32, sr: f32, seed: u32) -> Self {
        let key = ((midi as f32) - 21.0) / 87.0; // 0 = A0 … 1 = C8
        // register scaling of the aftersound t60: real decay times PEAK in the
        // low-mid register, not in the deep bass (NSynth acoustic refs, t60 of the
        // 0.8–1.8 s window: G1≈7 s, E♭2≈11 s, C3≈12 s, C5≈6 s; same shape in
        // Fletcher & Rossing fig. for piano decay vs key). Gaussian bump over key,
        // tapered above key≈0.7 where real strings shorten rapidly.
        // Amplitude/width set so the COMPOSITE late decay (aftersound pair mixed
        // with the still-audible prompt string) lands on the refs: the aftersound
        // param must exceed the composite target (~20 s at C3 for a measured ~12 s;
        // real mid-register aftersound runs tens of seconds — Fletcher & Rossing).
        // Fit 2026-07-11 r2 to the measured envelope grid (RMS dB at fixed
        // times, G1/C2/C3/C5 refs): late-phase t60 G1≈18, C2≈19, C3≈32, C5≈6.3.
        // Round 1's narrow bass-side bump priced G1 at 7.9 s where the ref
        // rings ~18 s once the prompt is gone.
        let bw = if key < 0.30 { 0.28 } else { 0.20 };
        let bump = (-((key - 0.30) / bw) * ((key - 0.30) / bw)).exp();
        let taper = 1.0 - 0.55 * ((key - 0.7).max(0.0) / 0.3);
        let t60 = (3.5 + 26.5 * bump) * taper.max(0.2);
        let lp_c = (0.32 + 0.44 * key + 0.18 * vel).clamp(0.25, 0.95);
        // stiffness (inharmonicity): audible on wound bass strings, mild in mid
        let disp_c =
            if key < 0.35 { 0.20 * (1.0 - key / 0.35) + 0.05 } else { 0.035 + 0.04 * (key - 0.35) };

        // Two-stage decay: string 0 is the PROMPT sound (bright, velocity-voiced,
        // faster decay); the others are the AFTERSOUND pair (full t60,
        // near-transparent loop) — Weinreich 1977. Envelope-grid fit of the
        // prompt:aftersound t60 ratio: mid slopes give prompt t60 G1≈5.5,
        // C2≈4, C3≈10, C5≈3.4 → r ≈ 0.27/0.20/0.33/0.4+ — roughly a third
        // everywhere, dipping at the bass break (key≈0.17) and rising toward
        // the treble where prompt and aftersound converge.
        let dk = (key - 0.17) / 0.06;
        let r_prompt = 0.28 - 0.10 * (-dk * dk).exp() + 0.27 * ((key - 0.35).max(0.0) / 0.65);
        let t_attack = (r_prompt * t60).min(11.0) * (1.05 - 0.15 * vel);
        let n_strings = if midi < 32 { 2 } else { 3 };
        let detune_spread = if midi < 32 { 0.35 } else { 1.5 - 0.7 * key };
        // Weinreich 1977 roles: the PROMPT sound is one bright, velocity-voiced,
        // faster-decaying string; the AFTERSOUND is the detuned unison pair at the
        // full t60 whose mutual beating stretches the composite late decay (the
        // reference's p2 decays at ~3 dB/s through pair beating while singles do
        // 8–15 dB/s). Aftersound loop filter is near-transparent (lp_c 0.82 fixed):
        // real loop losses are ~flat through the low kHz (Välimäki et al. 1996),
        // and aftersound brightness is not strike-dependent — the prompt string
        // carries the velocity timbre.
        // Aftersound loop at 0.93: near-transparent, so partials 2–6 inherit
        // the fundamental's long t60. The refs carry the late plateau on the
        // MID partials (C3's strongest tail peak is p2, G1's are p1/p4/p2) —
        // at 0.82 the pair's upper partials died and the "plateau" sagged at
        // 10 dB/s (measured C3 env −22.9 dB at 2.6 s vs ref −11.9).
        let cfg: [(f32, f32, f32); 3] = [
            (0.0, t_attack, lp_c * 1.40), // (detune cents, t60 s, lp_c) prompt
            (detune_spread, t60, 0.93),   // aftersound +
            (-0.8 * detune_spread, t60 * 0.92, 0.93), // aftersound −
        ];
        let rng = Lcg(seed | 1);
        let mut strings = [StringLoop::new(f0, 0.0, sr, t60, lp_c, disp_c); 3];
        let mut strike_off = [0usize; 3];
        for (i, s) in strings.iter_mut().enumerate().take(n_strings) {
            let (cents, t_sec, c) = cfg[i];
            *s = StringLoop::new(f0, cents, sr, t_sec, c.min(0.97), disp_c);
            strike_off[i] = ((0.12 * s.len as f32) as usize).clamp(1, s.len - 1);
        }

        // Hammer-string collision (the anti-harpsichord): the string starts at REST
        // and a felt hammer strikes it — force F = K·compression^p injected over the
        // contact. Contact time then EMERGES from velocity and register instead of
        // being painted onto a pre-filled pluck. Nondimensionalized: target a
        // register-scaled contact time at mezzo-forte, let the nonlinearity make
        // hard hits shorter/brighter and soft hits longer/darker.
        let h_p = 2.3 + 0.7 * key; // felt hardens up the keyboard
        let h_v0 = 0.010 + 0.115 * vel; // displacement units per sample
        // Stiffness normalized at a fixed REFERENCE velocity (mf): the felt law then
        // does its real job — harder hits compress more → shorter contact → brighter.
        // (Normalizing at the actual velocity pins contact time and kills the
        // velocity→timbre physics — measured mistake, see decision log.)
        // Contact target AT the reference velocity. Piecewise: the round-1
        // linear law is right through the bass/low-mid (a full-key exponential
        // overshot G1-ff centroid 341 vs 261), but linear left C5-ff contact
        // ≈0.7 ms, whose force-pulse null at ~2.1 kHz sat exactly on p4 — the
        // ref's SECOND-loudest attack peak (−12.4 dB; we rendered −24). Real
        // treble contacts run 0.3–0.5 ms at ff (Hall & Askenfelt), so above
        // key 0.35 the target falls exponentially to ~0.15 ms at C8.
        let contact_ms =
            if key < 0.35 { 1.7 - 1.1 * key } else { 1.315 * (-2.8 * (key - 0.35)).exp() };
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
                let a_db = -13.0
                    + 6.0 * ((key - 0.17).max(0.0) / 0.14).min(1.0)
                    + 2.0 * ((key - 0.45).max(0.0) / 0.35).min(1.0);
                if n_strings == 2 {
                    let a = 1.8 * 10f32.powf(a_db / 20.0);
                    [1.8, a, 0.0]
                } else {
                    let a = 2.0 * 10f32.powf(a_db / 20.0);
                    [2.0, 0.6 * a, 0.4 * a]
                }
            },
            n_strings,
            // Velocity→loudness curve, pinned at the vel-0.8 makeup calibration
            // point: the raw collision gives ~12.5 dB across vel 25→127 where the
            // references are nearly flat; −9.5 dB/vel-unit of compensation keeps
            // ~5 dB of musical dynamics without pp vanishing (timbre still tracks
            // velocity through the felt law, which is where piano dynamics live).
            level: 2.4 / (n_strings as f32) * (-1.09 * (vel - 0.8)).exp(),
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
            thump_amp: 0.02 * vel,
            noise_lp: 0.0,
            noise_lp_c: 1.0,
            bloom: 0.0,
            bloom_c: 1.0 - (-f0.max(50.0) / (2.5 * sr)).exp(),
            rad_c: 1.0 - (-core::f32::consts::TAU * (0.35 * f0).max(80.0) / sr).exp(),
            rad_lp: 0.0,
            rad_lp2: 0.0,
            // gain: register-tapered (off above key≈0.55) and velocity-curved —
            // the s² source alone gave pp phantoms ~11 dB hotter relative to
            // their soft references (felt at pp is too soft to pump the
            // longitudinal direction; Conklin hears phantoms "at forte")
            ph_gain: {
                let reg = ((0.55 - key) / 0.55).clamp(0.0, 1.0);
                6.0 * reg * reg * (0.25 + 0.75 * vel * vel)
            },
            ph_c: 1.0 - (-core::f32::consts::TAU * (6.0 * f0).min(0.1 * sr) / sr).exp(),
            ph_lp1: 0.0,
            ph_lp2: 0.0,
            air_c: 1.0 - (-core::f32::consts::TAU * (10_000.0f32).min(0.4 * sr) / sr).exp(),
            air_lp: 0.0,
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
        let knock = 0.6 * vel * (1.0 - 0.55 * key);
        let mut jrng = Lcg(seed.wrapping_mul(0x9E37) | 1);
        let mut bf = 88.0f32;
        for i in 0..PIANO_BOARD_MODES {
            let f = bf * (1.0 + 0.05 * jrng.next());
            let bt = 0.55 * (88.0 / f).powf(0.4);
            let a_rel = if f < 300.0 { (f / 300.0).powf(0.3) } else { (300.0 / f).powf(0.55) };
            let r = t60_gain(bt, sr);
            let w = core::f32::consts::TAU * f / sr;
            v.body_a1[i] = 2.0 * r * w.cos();
            v.body_r2[i] = r * r;
            v.body_g[i] = 0.079 * a_rel * (1.0 - r) * knock;
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
                    let inj = f * self.h_gain * inv_n;
                    for i in 0..self.n_strings {
                        let off = self.strike_off[i];
                        self.strings[i].inject(off, inj);
                    }
                } else if self.h_v < 0.0 {
                    self.h_active = false; // hammer moving away, contact over
                }
            }
            let mut s = 0.0;
            let mut ph_src = 0.0;
            for (i, st) in self.strings.iter_mut().enumerate().take(self.n_strings) {
                let y = st.tick();
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
                s += self.ph_gain * (h1 - self.ph_lp2);
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
        self.rad_lp = flush_denormal(self.rad_lp);
        self.rad_lp2 = flush_denormal(self.rad_lp2);
        self.air_lp = flush_denormal(self.air_lp);
        self.ph_lp1 = flush_denormal(self.ph_lp1);
        self.ph_lp2 = flush_denormal(self.ph_lp2);
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
        self.body_pulse_pos = 0;
        self.body_pulse_len = ((0.006 * self.sr) as u32).max(2);
        for g in self.body_g.iter_mut() {
            *g *= 0.5;
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
    // Fret-release noise (NSynth bright refs: broadband squeak at note-off,
    // ~+5 dB over the decayed string, >800 Hz dominant, t60 ≈ 0.15 s, scales
    // with velocity). Injected ON THE STRING (pre-voicing): the dark clean
    // voicing suppresses it exactly as the dark 022 refs show no burst, while
    // bright/distorted voicings pass it.
    rel_rng: Lcg,
    rel_amp: f32,
    rel_c: f32,
    rel_hp_c: f32,
    rel_lp: f32,
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
            rel_rng: Lcg(seed ^ 0x9E37_79B9),
            rel_amp: 0.0,
            rel_c: 0.0,
            rel_hp_c: 0.0,
            rel_lp: 0.0,
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
        let mut lp_c = (0.51 + 0.56 * key + 0.06 * vel).clamp(0.30, 0.985);
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
        let t60_slow = if dist { 18.0 } else { 15.0 };
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
            diff_g: if dist {
                1.0 / (2.0 * (core::f32::consts::PI * f0 / sr).sin()).max(1e-3)
            } else {
                0.0
            },
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
            sag_k: (if dist { 9.0 } else { 6.0 }) * (1.0 - 0.55 * key.clamp(0.0, 1.0)),
            vf_on: true,
            vf_b0: 0.0,
            vf_b1: 0.0,
            vf_b2: 0.0,
            vf_a1: 0.0,
            vf_a2: 0.0,
            vf_z1: 0.0,
            vf_z2: 0.0,
            rel_rng: Lcg(seed ^ 0x9E37_79B9),
            rel_amp: 0.0,
            // burst t60 ≈ 0.15 s (NSynth 028 release transients)
            rel_c: t60_gain(0.15, sr),
            // squeak brightness: one-pole HP at ~900 Hz (refs: 83-88% energy > 800 Hz)
            rel_hp_c: 1.0 - (-core::f32::consts::TAU * 900.0 / sr).exp(),
            rel_lp: 0.0,
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
        let vfc = if dist { 3200.0 } else { 550.0 };
        let wv = core::f32::consts::TAU * vfc / sr;
        let (sv, cv) = wv.sin_cos();
        let alpha = sv / (2.0 * 0.707);
        let a0 = 1.0 + alpha;
        v.vf_b0 = ((1.0 - cv) / 2.0) / a0;
        v.vf_b1 = (1.0 - cv) / a0;
        v.vf_b2 = v.vf_b0;
        v.vf_a1 = (-2.0 * cv) / a0;
        v.vf_a2 = (1.0 - alpha) / a0;
        // excitation: a pick pluck is a DETERMINISTIC released displacement
        // triangle (Smith PASP: harmonics ∝ sin(nπ·pick)/n²) plus a small
        // lowpassed-noise texture layer. Round-2 finding: pure noise excitation
        // has σ ≈ 7.7 dB note-to-note H2/H1 variance (measured, 40 seeds) — a
        // tone lottery in exactly the harmonics the dark clean voicing exposes.
        // Bridge-side electric picking (0.13 of the speaking length) yields the
        // refs' ~−6 dB/oct low-harmonic slope.
        let pick_pos = 0.13;
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
        let mut mean = 0.0;
        for i in 0..len {
            // released triangle: 0→1 over [0,p], back to 0 over [p,len)
            let tri = if i < p {
                i as f32 / p as f32
            } else {
                (len - i) as f32 / (len - p) as f32
            };
            // texture: ONE-pole noise (fills the refs' −26 dB mid plateau; the
            // fixed 550 Hz voicing pole + bus filters shape its top)
            lp += exc_c * (rng.next() - lp);
            let s = tri + 0.9 * lp;
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
            // fret-release squeak: HP'd noise burst on the string (see fields)
            if self.rel_amp > 1e-5 {
                let w = self.rel_rng.next();
                self.rel_lp += self.rel_hp_c * (w - self.rel_lp);
                s += (w - self.rel_lp) * self.rel_amp;
                self.rel_amp *= self.rel_c;
            }
            let mut u = s * self.level;
            // voicing/cab biquad (see fields; static per note, fixed-Hz corner)
            if self.vf_on {
                let y = self.vf_b0 * u + self.vf_z1;
                self.vf_z1 = self.vf_b1 * u - self.vf_a1 * y + self.vf_z2;
                self.vf_z2 = self.vf_b2 * u - self.vf_a2 * y;
                u = y;
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
            // electric fret/finger release: NSynth refs decay at t60 ≈ 0.3 s after
            // note-off. Loss basis is per round trip (rate f0 = sr/len), see above.
            let f0 = self.sr / self.len.max(2) as f32;
            self.loss = t60_gain(0.30, f0);
            self.loss2 = self.loss;
            self.life = self.age + (0.45 * self.sr) as u64;
            // fret-release squeak: finger-lift friction noise, velocity-scaled
            // (absolute level — press force, not current string amplitude)
            self.rel_amp = 0.05 + 0.13 * self.vel;
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
            let n = self.rng.next();
            let nb = n * self.burst_env;
            self.burst_env *= self.burst_dec;
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
        for i in 0..self.n_bands {
            self.y1[i] = flush_denormal(self.y1[i]);
            self.y2[i] = flush_denormal(self.y2[i]);
            self.wash_env[i] = flush_denormal(self.wash_env[i]);
            self.bloom[i] = flush_denormal(self.bloom[i]);
        }
        self.age < self.life
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
        let (m_ping, m_wash, m_chick, m_dec, m_amp, m_hf) = match kit {
            KitStyle::Pop => (1.0f32, 0.92f32, 1.0f32, 1.0f32, 1.0f32, 0.75f32),
            KitStyle::Rock => (1.55, 1.05, 1.25, 1.0, 1.1, 0.85),
            KitStyle::Jazz => (0.95, 1.22, 0.80, 1.08, 1.12, 0.62),
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
            let shape = (1.0 + 0.45 * (-lnw * lnw / 0.8).exp())
                * if f < 500.0 { (f / 500.0).powf(1.5) } else { 1.0 }
                * if f > 5000.0 { m_hf } else { 1.0 };
            let wash = 0.5 * m_wash * dip * shape;
            // chick: treble-tilted contact noise, ~2× the wash gain up top
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
            KitStyle::Rock => (1.18, 1.25, 1.05, 4.6),
            KitStyle::Jazz => (1.0, 0.85, 0.92, 3.4),
        };
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.045, sr);
        v.amp = 1.15 * m_amp * (0.25 + 0.75 * vel);
        v.life = (life_s * sr) as u64;
        let mut jit = Lcg(seed ^ 0xc4a5 | 1);
        let imp = 0.08;

        // light tonal skeleton — crashes are wash-dominated
        for (f, ring, burst) in [(524.0, 1.8, 0.14f32), (1173.0, 1.4, 0.12)] {
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
            let wash = 0.62 * m_wash * if f < 700.0 { (f / 700.0).powf(1.2) } else { 1.0 };
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
    /// centroid 6.9 kHz at vl3 vs 3.2 kHz at vl1). Jazz hats sit softer and
    /// slightly sloshier; rock hats brighter (production convention, Owsinski).
    fn hat(open: bool, vel: f32, sr: f32, seed: u32, kit: KitStyle) -> Self {
        // (amp, chick, closed-decay)
        let (m_amp, m_chick, m_cdec) = match kit {
            KitStyle::Pop => (1.0f32, 1.0f32, 1.0f32),
            KitStyle::Rock => (1.05, 1.2, 1.0),
            KitStyle::Jazz => (0.82, 0.85, 1.15),
        };
        let mut v = Self::new(seed);
        v.burst_dec = t60_gain(0.012, sr);
        v.amp = if open { 0.68 } else { 0.5 } * m_amp * (0.3 + 0.7 * vel);
        v.life = if open { (3.2 * sr) as u64 } else { (0.9 * sr) as u64 };
        let mut jit = Lcg(seed ^ 0x4a75 | 1);
        if open {
            // tonal skeleton measured from the CC0 open-hat sustain:
            // 614/634 Hz beating pair, 1003, 1114, 1771 Hz
            for (f, ring, burst) in [
                (614.0, 2.8, 0.016f32),
                (634.0, 3.2, 0.021),
                (1003.0, 3.0, 0.019),
                (1114.0, 2.8, 0.018),
                (1771.0, 2.4, 0.014),
            ] {
                v.push_band(
                    CymBand {
                        freq: f,
                        ring_t60: ring,
                        burst: burst * vel.powf(0.8),
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
                // strokes ring the open plates a little shorter
                let lnh = (f / 1600.0).ln();
                (0.8 + 4.0 * (-lnh * lnh / 2.4).exp()).clamp(0.8, 4.8) * (0.75 + 0.35 * vel)
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
            let burst = (0.10 + 1.1 * (-lnr * lnr / 1.0).exp()) * vel.powf(0.7) * 0.13;
            let wash = 0.5
                * if f < 1200.0 { (f / 1200.0).powf(1.4) } else { 1.0 }
                * if open && f > 6000.0 { 0.75 } else { 1.0 }
                // velocity-brightness: the closed hat's top end scales with
                // velocity beyond the level curve (onset centroid doubles
                // vl1→vl3 in the refs)
                * if !open && f > 5000.0 { 0.55 + 0.55 * vel } else { 1.0 };
            let chick = if open { 0.8 } else { 1.5 }
                * m_chick
                * (f / 6000.0).powf(0.7).min(2.0)
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
    /// kick beater-click gain (velocity-shaped; hp/hp_c double as the click's
    /// brightness lowpass for the Kick kind)
    click: f32,
    /// beater "slap" resonator: an impulse-rung two-pole at the head's
    /// overtone region (~500–700 Hz, t60 ~30 ms) — the mid-band knock that
    /// separates a hard beater hit from the pitched fundamental (CC0 hard-kick
    /// refs hold the 400–1500 Hz attack band within ~2 dB of the fundamental)
    sl_a1: f32,
    sl_r2: f32,
    sl_y1: f32,
    sl_y2: f32,
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
            click: 0.0,
            sl_a1: 0.0,
            sl_r2: 0.0,
            sl_y1: 0.0,
            sl_y2: 0.0,
            modal: ModalVoice::start(200.0, 0.0, sr, &[], 1.0, 0.0, 0.0, seed),
            has_modal: false,
            cym: CymbalVoice::new(seed),
            rng: Lcg(seed | 1),
            life: (0.6 * sr) as u64,
            age: 0,
        };
        match gm_note {
            35 | 36 => {
                // Kick, kit-voiced (tuning + damping conventions: rock 22"
                // muffled low w/ hard-beater slap at 2–4 kHz, pop tight and
                // damped, jazz 18" tuned high and left open w/ felt beater —
                // Owsinski, Recording Engineer's Handbook; Drum Tuning Bible;
                // jazz-kick pitch ~78 Hz measured on the CC0 virtuosity kit).
                // Velocity depth: hard hits drive the head further → higher
                // initial pitch (membrane tension modulation, Rossing ch. 2)
                // and a brighter, louder beater click (CC0 kick refs: 30 ms
                // centroid 306 Hz hard vs 148 Hz soft).
                // (f_start base, vel span, f_end, sweep s, t60, click base,
                //  click vel span, click brightness, amp)
                let (f0, f0v, f1, sw, t60, ck0, ckv, ckb, amp) = match kit {
                    KitStyle::Pop => (90.0, 35.0, 45.0, 0.032, 0.30, 0.15, 0.75, 0.55, 0.9),
                    KitStyle::Rock => (95.0, 45.0, 41.0, 0.040, 0.50, 0.20, 1.00, 0.64, 0.95),
                    KitStyle::Jazz => (120.0, 40.0, 72.0, 0.028, 0.35, 0.08, 0.45, 0.38, 0.8),
                };
                v.kind = DrumKind::Kick;
                v.freq = f0 + f0v * vel;
                v.freq_end = f1;
                v.sweep = (-1.0 / (sw * sr)).exp();
                v.decay = t60_gain(t60, sr);
                v.amp = vel * amp;
                v.click = ck0 + ckv * vel * vel;
                v.hp_c = 0.08 + ckb * vel; // click lowpass: felt thud → hard slap
                v.life = ((t60 * 1.6).max(0.5) * sr) as u64;
                // slap resonator: (freq, strength) per kit; amplitude ~vel^2
                // (soft felt strokes barely knock; hard strokes do — refs:
                // slap band −2.4 dB rel at vl4, −12.4 dB at vl1)
                let (sf, sa) = match kit {
                    KitStyle::Pop => (620.0, 1.7),
                    KitStyle::Rock => (540.0, 2.4),
                    KitStyle::Jazz => (700.0, 1.5),
                };
                let r = t60_gain(0.060, sr);
                let w = core::f32::consts::TAU * sf / sr;
                v.sl_a1 = 2.0 * r * w.cos();
                v.sl_r2 = r * r;
                let a = sa * vel.powf(1.05);
                let phi = core::f32::consts::PI * Lcg(seed ^ 0x51a9 | 1).next();
                v.sl_y1 = a * (phi - w).sin();
                v.sl_y2 = a * (phi - 2.0 * w).sin();
            }
            38 | 40 => {
                // Snare, kit-voiced: shell fundamental + coupled-head partials
                // (ratios 1.3–3.0 measured on the CC0 virtuosity snare, 182 Hz
                // fundamental). Conventions: pop = tight/bright/damped; rock =
                // deeper shell tuned lower, longer ring; jazz = tuned high with
                // the shell singing through the wires (Drum Tuning Bible;
                // Owsinski). Velocity: wires dominate soft hits, shell tone
                // grows with velocity (soft ref is relatively wire-bright) —
                // wires get a level floor, the modal shell scales with vel.
                // (shell Hz, decay, hp_c, noise base, noise vel span,
                //  modal gain, life s)
                let (shell, dec, hpc, n0, nv, mg, life) = match kit {
                    KitStyle::Pop => (186.0, 0.14, 0.40, 0.30, 0.60, 0.40, 0.32),
                    KitStyle::Rock => (158.0, 0.22, 0.24, 0.33, 0.62, 0.55, 0.42),
                    KitStyle::Jazz => (214.0, 0.20, 0.32, 0.24, 0.55, 0.60, 0.40),
                };
                v.decay = t60_gain(dec, sr);
                v.hp_c = hpc;
                // velocity lives HERE (wires floor + span) and in the shell's
                // modal excitation — not in v.amp, or the wires pick up a
                // second vel factor and soft hits lose their relative wire
                // brightness (the CC0 soft snare is wire-forward)
                v.amp = 0.95;
                v.noise_amt = n0 + nv * vel;
                v.has_modal = true;
                let dl = dec / 0.14; // shell ring scales with the kit's looseness
                v.modal = ModalVoice::start(
                    shell,
                    vel,
                    sr,
                    &[
                        ModeDef { ratio: 1.0, amp: 1.0, t60: 0.11 * dl },
                        ModeDef { ratio: 1.55, amp: 0.6, t60: 0.08 * dl },
                        ModeDef { ratio: 2.1, amp: 0.3, t60: 0.06 * dl },
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
            55 => {
                // splash: small fast crash
                v.kind = DrumKind::Cymbal;
                v.cym = CymbalVoice::splash(vel, sr, seed);
                v.life = v.cym.life;
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
                    s = (core::f32::consts::TAU * self.phase).sin() * self.env;
                    // beater-contact click for the first ~4 ms: velocity-shaped
                    // gain (click) through a velocity-opened lowpass (hp/hp_c) —
                    // felt-beater thud at pp, hard slap at ff
                    if self.age < (0.010 * sr) as u64 {
                        let n = self.rng.next();
                        self.hp += self.hp_c * (n - self.hp);
                        s += self.click * self.hp * self.env;
                    }
                    // beater slap ring (impulse initial condition, decays on its own)
                    let y = self.sl_a1 * self.sl_y1 - self.sl_r2 * self.sl_y2;
                    self.sl_y2 = self.sl_y1;
                    self.sl_y1 = y;
                    s += y;
                }
                DrumKind::Cymbal => s = 0.0, // handled by early return above
                DrumKind::Noise => {
                    let n = self.rng.next();
                    self.hp += self.hp_c * (n - self.hp); // lowpass...
                    let hp = n - self.hp; // ...subtracted = one-pole highpass
                    s = hp * self.env * self.noise_amt;
                    if self.tone_amt > 0.0 {
                        self.phase = (self.phase + self.freq * dt).fract();
                        s += self.tone_amt * (core::f32::consts::TAU * self.phase).sin() * self.env;
                    }
                }
            }
            self.env *= self.decay;
            *o += s * self.amp;
            self.age += 1;
        }
        self.hp = flush_denormal(self.hp);
        self.env = flush_denormal(self.env);
        self.sl_y1 = flush_denormal(self.sl_y1);
        self.sl_y2 = flush_denormal(self.sl_y2);
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
                t60_f0: 11.0 + 6.0 * key,
                lp_c: 0.97 - 0.10 * key + 0.02 * vel,
                hf_floor_t60: 0.0,
                hf_knee_hz: 0.0,
                pick_pos: 0.20,
                contact: 0.045,
                snap: 0.5,
                scrape: 0.06,
                click: 0.0,
                rel_t60: 0.10,
                rel_click: 0.25,
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
                // displacement tap: the NSynth nylon sources are fundamental-
                // dominant in mid/high register; low-register h2 emphasis comes
                // from the body's T1 mode, not a global force tilt
                br_rho: 0.0,
                // register slope ~12 dB/key (within-source NSynth slope is
                // ~9 dB/key; cross-source fits inflate it); mild velocity curve
                level: 0.5 * (0.55 + 0.45 * vel) * (1.4 * key.min(0.9)).exp(),
            };
            Kernel::Pluck(PluckVoice::start_acoustic(&p, sr, seed))
        }
        Instrument::Bass => {
            // warm fingered upright/electric hybrid, migrated off the legacy
            // per-sample-loss constructor (the flat-envelope bug all three pluck
            // agents flagged) onto the per-period-calibrated acoustic engine.
            // Wide finger-flesh contact + neck position = round; no glide.
            let key = (((midi as f32) - 28.0) / 32.0).clamp(0.0, 1.0);
            let p = AcPluck {
                f0,
                vel,
                // refs' measured ladder (025-028-100): h1 dies 10 dB/s, h3 at
                // 10 dB/s, h6 at 33 dB/s, nothing sustained above ~500 Hz —
                // while their ENVELOPE reads 10–35 s because h2 rings near 0
                // dB/s. Compromise fundamental t60 + a genuinely dark loop
                // (per-period loss barely bites at a 24 ms period: the old
                // lp 0.58 rang 7.5 s at 1 kHz, +20 dB over the ref mid-note).
                t60_f0: 14.0 - 4.0 * key,
                lp_c: 0.50 + 0.10 * vel,
                hf_floor_t60: 0.0,
                hf_knee_hz: 0.0,
                pick_pos: 0.30,
                contact: 0.10,
                snap: 0.35,
                scrape: 0.03,
                click: 0.0,
                rel_t60: 0.12,
                rel_click: 0.15,
                pol_mix: 0.25,
                pol_detune_cents: 1.2,
                pol_t60_ratio: 0.5,
                // NSynth bass_electronic refs measure essentially harmonic
                // (B ≤ 2e-5, h10 ≤ 3 cents) — barely-there stiffness
                stiff_b: 1.2e-5,
                tm_cents: 0.0,
                br_rho: 0.0,
                level: 0.5 * (0.5 + 0.5 * vel),
            };
            Kernel::Pluck(PluckVoice::start_acoustic(&p, sr, seed))
        }
        Instrument::EPiano => {
            // tine + tone-bar partial through a velocity-driven pickup nonlinearity.
            // Drive is key-tracked DOWN in the top octaves: tanh harmonics of a high
            // fundamental would fold past Nyquist (no oversampling yet — issue #8).
            let key = ((midi as f32) - 40.0) / 48.0;
            let t = (7.0 - 4.5 * key).clamp(1.2, 7.0);
            let drive_scale = (1.2 - key).clamp(0.25, 1.0);
            Kernel::Modal(ModalVoice::start(
                f0,
                vel,
                sr,
                &[
                    ModeDef { ratio: 1.0, amp: 1.0, t60: t },
                    ModeDef { ratio: 3.97, amp: 0.14 + 0.5 * vel * vel, t60: 0.5 },
                    ModeDef { ratio: 6.24, amp: 0.05 * vel, t60: 0.2 },
                ],
                2.2,
                0.03,
                (0.5 + 1.6 * vel) * drive_scale,
                seed,
            ))
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
                click: 1.35,
                rel_t60: 0.90,
                rel_click: 0.5,
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
                br_rho: 0.995,
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
        let ff = render_piano(31, 1.0, sr, 1.0, None);
        let pp = render_piano(31, 0.2, sr, 1.0, None);
        let (a, b) = ((0.1 * sr) as usize, (0.6 * sr) as usize);
        let e_ff = band_energy(&ff[a..b], sr, &freqs);
        let e_pp = band_energy(&pp[a..b], sr, &freqs);
        let tot_ff = ff[a..b].iter().map(|s| s * s).sum::<f32>();
        let tot_pp = pp[a..b].iter().map(|s| s * s).sum::<f32>();
        let r_ff = e_ff / tot_ff.max(1e-12);
        let r_pp = e_pp / tot_pp.max(1e-12);
        assert!(r_ff > 1e-7, "no phantom-band energy at ff: {r_ff}");
        // The band holds transverse partials too (they scale ~linearly, so the
        // fraction cancels); the >35% superlinear EXCESS is the phantom tap's
        // signature — with ph_gain = 0 this ratio measures ≈ 1.0.
        assert!(
            r_ff > 1.35 * r_pp,
            "phantom band should grow superlinearly with velocity: ff {r_ff} vs pp {r_pp}"
        );
    }

    /// Damper felt/key release (Askenfelt & Jansson 1990): after note-off a
    /// broadband thud must appear BETWEEN the partials (where the harmonic
    /// string can't put energy), decay away, and be RELATIVELY louder on a
    /// soft note (the mechanism doesn't shrink with a soft touch).
    #[test]
    fn piano_release_thud_present_decaying_and_velocity_relative() {
        let sr = 48_000.0;
        let f0 = midi_to_hz(48.0);
        // inter-partial gap bands: harmonic content is absent here
        let gaps = [2.5 * f0, 3.5 * f0, 4.5 * f0];
        let mut ratio = [0.0f32; 2];
        for (i, vel) in [0.25f32, 1.0].iter().enumerate() {
            let out = render_piano(48, *vel, sr, 2.4, Some(1.5));
            let pre = band_energy(&out[(1.30 * sr) as usize..(1.42 * sr) as usize], sr, &gaps);
            let thud = band_energy(&out[(1.51 * sr) as usize..(1.63 * sr) as usize], sr, &gaps);
            let late = band_energy(&out[(2.10 * sr) as usize..(2.22 * sr) as usize], sr, &gaps);
            assert!(thud > 1e-12, "vel {vel}: no release transient ({thud})");
            assert!(late < thud, "vel {vel}: release does not decay ({thud} -> {late})");
            ratio[i] = thud / pre.max(1e-15);
        }
        assert!(
            ratio[0] > 1.6,
            "pp release thud should rise above the string's gap leakage: {}",
            ratio[0]
        );
        assert!(
            ratio[0] > 1.3 * ratio[1],
            "soft-note release should be relatively louder: pp {} vs ff {}",
            ratio[0],
            ratio[1]
        );
    }

    /// Steel-string tuning at both deploy sample rates (the acoustic constructor
    /// has its own delay budget: exact loop-lowpass phase delay + tension-mod
    /// fraction bias) — protocol: delay math changes need 44.1k AND 48k coverage.
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

    /// Nylon stretch stays subtle (B ≈ 3.5e-5 ⇒ h10 ≈ +3 cents) and bass is
    /// near-harmonic — the cascade must not overshoot on the soft-B rows.
    #[test]
    fn nylon_and_bass_stay_near_harmonic() {
        let sr = 48_000.0f32;
        for (inst, midi, cap) in [
            (Instrument::Guitar, 40u32, 8.0f32),
            (Instrument::Bass, 28, 6.0),
        ] {
            let out = render_pluck(inst, midi, 0.9, sr, 1.5);
            let seg = &out[(0.25 * sr) as usize..(1.25 * sr) as usize];
            let want = midi_to_hz(midi as f32);
            let f1 = peak_freq(seg, sr, want * 0.98, want * 1.02);
            assert!((f1 - want).abs() < want * 0.006, "{inst:?}: f1={f1}");
            let f10 = peak_freq(seg, sr, 10.0 * f1 * 0.995, 10.0 * f1 * 1.008);
            let c10 = 1200.0 * (f10 / (10.0 * f1)).log2();
            assert!(
                (-2.0..cap).contains(&c10),
                "{inst:?}: h10 stretch {c10} cents (cap {cap})"
            );
        }
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
            for &gm in &[42u32, 44, 46, 49, 51, 53, 55, 57, 59] {
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
        for &gm in &[42u32, 46, 49, 51, 53, 55] {
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
