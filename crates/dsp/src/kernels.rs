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
        // high gain: compression holds the note while the string decays >20 dB
        Instrument::GuitarDistorted => (11.0, 4200.0),
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

pub const MAX_BODY_MODES: usize = 8;

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
        // Nylon (refs 010/014): A0 ~100 Hz, broad T1 region 165–210 Hz (the ref
        // cluster's strongest formant — E2's h2 rides it), gentle mid ladder,
        // nothing above ~1.1 kHz (nylon HF dies in the string, not the body).
        Instrument::Guitar => (
            0.45,
            &[
                (100.0, 0.30, 0.042),  // P=1.6 A0 Helmholtz
                (170.0, 0.22, 0.116),  // P=2.6 T1 lower skirt
                (208.0, 0.20, 0.131),  // P=2.4 T1
                (300.0, 0.14, 0.078),  // P=1.0 T2/back
                (425.0, 0.12, 0.145),  // P=1.3
                (560.0, 0.10, 0.139),  // P=0.95
                (720.0, 0.09, 0.198),  // P=1.05
                (1100.0, 0.06, 0.187), // P=0.65
            ],
        ),
        // Steel (refs 015/030/021): weak below ~170 Hz, broad 230–1800 plateau
        // with peaks near 400/900/1500, slow rolloff to 5 kHz (dreadnought-ish).
        Instrument::GuitarSteel => (
            0.35,
            &[
                (105.0, 0.30, 0.019), // P=0.7 A0
                (235.0, 0.22, 0.080), // P=1.3 T1
                (400.0, 0.16, 0.230), // P=2.2
                (620.0, 0.12, 0.195), // P=1.2
                (900.0, 0.10, 0.470), // P=2.0
                (1400.0, 0.08, 0.658), // P=1.8
                (2200.0, 0.06, 0.686), // P=1.2
                (3600.0, 0.045, 0.908), // P=1.0
            ],
        ),
        // piano: soundboard low-mode ladder (broad, subtle — per-voice knock stays)
        Instrument::Piano => (
            0.75,
            &[
                (62.0, 0.30, 0.7),
                (110.0, 0.24, 0.9),
                (175.0, 0.18, 0.8),
                (255.0, 0.13, 0.6),
                (370.0, 0.10, 0.45),
                (520.0, 0.08, 0.3),
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
        Instrument::Guitar => 0.194,       // re-baked 2026-07-11 (acoustic rework ran -13.3 LUFS at 0.85)
        Instrument::Bass => 0.66,         // was -25.3 LUFS
        Instrument::EPiano => 1.47,       // was -26.6 LUFS
        Instrument::Drums => 0.61,        // was -27.4 LUFS
        Instrument::SynthPad => 0.48,     // was -26.5 LUFS
        Instrument::Piano => 0.067, // piano agent re-measure (v4 knock/level rework)
        Instrument::GuitarSteel => 0.50,    // acoustic agent re-bake
        Instrument::GuitarElectric => 0.73, // electric agent re-measure
        Instrument::GuitarDistorted => 0.21, // electric agent re-measure (high gain)
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
    /// stiffness dispersion allpass (steel strings; 0 = off)
    disp_c: f32,
    d1_x1: f32,
    d1_y1: f32,
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
    /// pluck point as a fraction of string length (comb is inherent in the shape)
    pub pick_pos: f32,
    /// contact-patch width as a fraction of string length (finger flesh ≫ pick
    /// tip); band-limits the WHOLE initial condition to ~len/width harmonics
    pub contact: f32,
    /// localized release-snap bump (velocity component of the initial condition)
    pub snap: f32,
    /// pick/finger contact noise mixed into the initial shape
    pub scrape: f32,
    /// second-polarization output level (0 = single string)
    pub pol_mix: f32,
    pub pol_detune_cents: f32,
    /// polarization-2 fundamental t60 = t60_f0 × this ratio
    pub pol_t60_ratio: f32,
    /// stiffness dispersion allpass coefficient (0 = none)
    pub disp_c: f32,
    /// initial tension-mod sharpening at vel = 1 (cents; small — refs show ≤3c)
    pub tm_cents: f32,
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

/// First-order allpass (a + z⁻¹)/(1 + a z⁻¹) phase delay in samples at ω.
#[inline]
fn allpass_delay(a: f32, w: f32) -> f32 {
    let (sw, cw) = w.sin_cos();
    let th_n = (-sw).atan2(a + cw);
    let th_d = (-a * sw).atan2(1.0 + a * cw);
    -(th_n - th_d) / w
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
            disp_c: 0.0,
            d1_x1: 0.0,
            d1_y1: 0.0,
            br_rho: 0.0,
            br_x1: 0.0,
            tm_dev: 0.0,
            tm_env: 0.0,
            tm_c: 0.0,
            tm_norm: 0.0,
            frac1: 0.0,
            frac2: 0.0,
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
        let mut lp_c = p.lp_c.clamp(0.05, 0.995);
        while onepole_mag(lp_c, w0) < g0 && lp_c < 0.99 {
            lp_c += 0.01;
        }
        let loss = (g0 / onepole_mag(lp_c, w0)).min(0.99995);

        // Tuning: subtract exact loop-filter + dispersion phase delays at f0.
        let disp_on = p.disp_c > 0.0;
        let d_lp = onepole_delay(lp_c, w0);
        let d_disp = if disp_on { allpass_delay(p.disp_c, w0) } else { 0.0 };
        let total = (period - d_lp - d_disp).max(3.0);
        // Bias the fraction high when tension-mod wants sharpening headroom.
        let bias = if p.tm_cents > 0.0 { 1.45 } else { 0.5 };
        let len = ((total - bias).ceil() as usize).clamp(2, PLUCK_BUF - 1);
        let frac1 = (total - len as f32).clamp(0.1, 1.5);

        // Second polarization: detuned by a couple cents, faster decay
        // (vertical motion pumps the bridge harder — Weinreich 1977 two-stage).
        let f2 = p.f0 * (p.pol_detune_cents / 1200.0).exp2();
        let total2 = (sr / f2 - d_lp).max(3.0);
        let len2 = ((total2 - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac2 = (total2 - len2 as f32).clamp(0.1, 1.5);
        let g2 = per_period_gain(p.t60_f0 * p.pol_t60_ratio.max(0.05), f2);
        let loss2 = (g2 / onepole_mag(lp_c, w0)).min(0.99995);

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
            loss,
            ap_c: (1.0 - frac1) / (1.0 + frac1),
            level,
            life: ((p.t60_f0 * 1.6 + 0.5) * sr) as u64,
            sr,
            len2,
            loss2,
            ap2_c: (1.0 - frac2) / (1.0 + frac2),
            pol_mix: p.pol_mix,
            disp_c: if disp_on { p.disp_c } else { 0.0 },
            br_rho: p.br_rho,
            tm_dev,
            tm_c: 1.0 - (-core::f32::consts::TAU * 6.0 / sr).exp(),
            frac1,
            frac2,
            f0: p.f0,
            ..Self::blank()
        };

        // --- pick-release excitation (displacement initial condition) ---
        // A pluck is a RELEASE of a displaced string: triangle peaked at the pick
        // point (harmonic amps ∝ sin(nπβ)/n² — the pick-position comb is inherent),
        // corner rounded by pick/finger compliance, plus a localized release-snap
        // bump (velocity component) and a dash of contact noise.
        let mut rng = Lcg(seed | 1);
        let pk = ((p.pick_pos * len as f32) as usize).clamp(2, len - 2);
        let mut tmp = [0.0f32; PLUCK_BUF];
        for (i, t) in tmp.iter_mut().enumerate().take(len) {
            *t = if i <= pk {
                i as f32 / pk as f32
            } else {
                (len - i) as f32 / (len - pk) as f32
            };
        }
        // release snap: narrow raised-cosine bump at the pick point (the corner
        // the pick leaves as it lets go); NSynth refs show attack brightness
        // grows with velocity but far less than linearly
        if p.snap > 0.0 {
            let wdt = ((len as f32 * 0.008) as usize + 2).min(len / 4);
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
        // so the excitation bandwidth is a physical fraction of f0 — the fixed
        // [1,2,1] smoothing this replaces barely filtered long strings.
        let cw = ((p.contact * (1.2 - 0.2 * p.vel) * len as f32) as usize).clamp(1, len / 4);
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
        // DC removal, then load both polarizations (pol2 gets the same shape —
        // it IS the same string; only its loop differs)
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
        for o in out.iter_mut() {
            let y = self.buf[self.pos];
            // loop lowpass (string damping / brightness)
            self.lp += self.lp_c * (y - self.lp);
            // stiffness dispersion (steel): first-order allpass delays highs
            let s = if self.disp_c > 0.0 {
                let d = self.disp_c * (self.lp - self.d1_y1) + self.d1_x1;
                self.d1_x1 = self.lp;
                self.d1_y1 = d;
                d
            } else {
                self.lp
            };
            // fractional-delay allpass keeps the string in tune
            let ap = self.ap_c * (s - self.ap_y1) + self.ap_x1;
            self.ap_x1 = s;
            self.ap_y1 = ap;
            self.buf[self.pos] = ap * self.loss;
            self.pos = (self.pos + 1) % self.len;
            let mut mix = y;
            // second polarization (own loop; summed at the bridge)
            if self.pol_mix > 0.0 {
                let y2 = self.buf2[self.pos2];
                self.lp2 += self.lp_c * (y2 - self.lp2);
                let ap2 = self.ap2_c * (self.lp2 - self.ap2_y1) + self.ap2_x1;
                self.ap2_x1 = self.lp2;
                self.ap2_y1 = ap2;
                self.buf2[self.pos2] = ap2 * self.loss2;
                self.pos2 = (self.pos2 + 1) % self.len2;
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
        }
        self.lp = flush_denormal(self.lp);
        self.ap_y1 = flush_denormal(self.ap_y1);
        if self.pol_mix > 0.0 {
            self.lp2 = flush_denormal(self.lp2);
            self.ap2_y1 = flush_denormal(self.ap2_y1);
        }
        if self.disp_c > 0.0 {
            self.d1_y1 = flush_denormal(self.d1_y1);
        }
        self.tm_env = flush_denormal(self.tm_env);
        self.br_x1 = flush_denormal(self.br_x1);
        self.age += out.len() as u64;
        self.age < self.life
    }

    pub fn damp(&mut self) {
        if self.f0 > 0.0 {
            // acoustic path: per-period loss for a ~90 ms release (finger/palm
            // damping), voice retired shortly after
            self.loss = per_period_gain(0.09, self.f0);
            self.loss2 = self.loss;
            self.life = self.age + (0.25 * self.sr) as u64;
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

#[derive(Clone, Copy)]
pub struct PianoVoice {
    strings: [StringLoop; 3],
    strike_off: [usize; 3],
    // radiated mix per string: the aftersound pair couples into the board for
    // seconds while the prompt polarization dumps fast; equal 1:1:1 weighting
    // left our post-prompt plateau ~15 dB under the peak where the references
    // hold ~7 dB (env at 1 s: ref −15 dB vs render −30 dB, C3 f).
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
    // soundboard/case knock: 3 fixed low modes excited by the hammer pulse
    body_a1: [f32; 3],
    body_r2: [f32; 3],
    body_y1: [f32; 3],
    body_y2: [f32; 3],
    body_g: [f32; 3],
    body_pulse_pos: u32,
    body_pulse_len: u32,
    thump_env: f32,
    thump_decay: f32,
    thump_amp: f32,
    // soundboard radiation buildup: the board is a driven resonant radiator whose
    // low-frequency output rises over several string periods (Suzuki, JASA 1986
    // soundboard mobility; driven-resonator transient). NSynth refs peak 5–9
    // periods after onset (G1 ≈ 160 ms, C3 ≈ 40 ms, C5 ≈ 20 ms) — the strings'
    // radiated sum gets a 1−e^(−t/τ) rise, τ ≈ 2.5 periods; knock/thump bypass.
    bloom: f32,
    bloom_c: f32,
    // radiation highpass at 0.35·f0: the board cannot radiate far below the
    // string's fundamental (dipole rolloff below the first board modes), and the
    // hammer's unipolar injection otherwise leaks a subsonic pedestal transient
    // (measured: a ~20 Hz component 7 dB ABOVE C5's fundamental in the attack).
    rad_c: f32,
    rad_lp: f32,
    rng: Lcg,
    sr: f32,
    key: f32,
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
        // asymmetric bump: decay falls off faster below the low-mid peak than above
        let bw = if key < 0.30 { 0.145 } else { 0.20 };
        let bump = (-((key - 0.30) / bw) * ((key - 0.30) / bw)).exp();
        let taper = 1.0 - 0.55 * ((key - 0.7).max(0.0) / 0.3);
        let t60 = (3.5 + 22.5 * bump) * taper.max(0.2);
        let lp_c = (0.32 + 0.44 * key + 0.18 * vel).clamp(0.25, 0.95);
        // stiffness (inharmonicity): audible on wound bass strings, mild in mid
        let disp_c =
            if key < 0.35 { 0.20 * (1.0 - key / 0.35) + 0.05 } else { 0.035 + 0.04 * (key - 0.35) };

        // Two-stage decay: string 0 is the SUSTAIN mode (darker loop, full t60);
        // the others are the ATTACK stage (brighter, faster, detuned — Weinreich
        // 1977 coupled-string prompt sound vs aftersound). All are struck by the
        // SAME hammer. Refs show prompt t60 ≈ half the aftersound t60 across the
        // keyboard (C2 3.2→7.0, C3 6.7→12.0, C5 3.5→5.9), slightly faster when
        // struck harder (bridge coupling grows with amplitude).
        let t_attack = (0.45 * t60).min(6.0) * (1.05 - 0.15 * vel);
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
        let cfg: [(f32, f32, f32); 3] = [
            (0.0, t_attack, lp_c * 1.40), // (detune cents, t60 s, lp_c) prompt
            (detune_spread, t60, 0.82),   // aftersound +
            (-0.8 * detune_spread, t60 * 0.92, 0.82), // aftersound −
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
        let contact_ms = 1.7 - 1.1 * key; // contact target AT the reference velocity
        let v_ref = 0.010 + 0.115 * 0.6;
        let omega = core::f32::consts::PI / (contact_ms * 1e-3 * sr);
        let comp_ref = (v_ref / omega).max(1e-6);
        let h_k = omega * omega * comp_ref.powf(1.0 - h_p);

        // body knock: fixed case/soundboard modes (85/172/318 Hz), short decay
        let mut v = Self {
            strings,
            strike_off,
            out_w: if n_strings == 2 { [0.5, 1.5, 0.0] } else { [0.5, 1.25, 1.25] },
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
            body_a1: [0.0; 3],
            body_r2: [0.0; 3],
            body_y1: [0.0; 3],
            body_y2: [0.0; 3],
            body_g: [0.0; 3],
            body_pulse_pos: 0,
            body_pulse_len: ((0.003 * sr) as u32).max(2),
            thump_env: 1.0,
            thump_decay: t60_gain(0.010, sr),
            thump_amp: 0.02 * vel,
            bloom: 0.0,
            bloom_c: 1.0 - (-f0.max(50.0) / (2.5 * sr)).exp(),
            rad_c: 1.0 - (-core::f32::consts::TAU * 0.35 * f0 / sr).exp(),
            rad_lp: 0.0,
            rng,
            sr,
            key,
            // cap: the long mid-register aftersound params would otherwise hold
            // voices ~36 s (pool exhaustion under pedal); inaudible past ~18 s
            life: (((t60 * 1.4 + 0.1).min(18.0)) * sr) as u64,
            age: 0,
        };
        // Knock/thump are a subtle PRECURSOR in real recordings (Askenfelt &
        // Jansson 1990), ~8 dB below the string plateau; at 2.5×vel the 85 Hz
        // knock mode sat ~10 dB ABOVE it and owned the first 150 ms of every ff
        // note (and buried the treble attack centroid: C5 read 99 Hz vs ref 665).
        // Key taper: the case knock shrinks toward the short treble strings.
        let body = [(85.0f32, 0.40f32, 0.30f32), (172.0, 0.28, 0.20), (318.0, 0.18, 0.13)];
        let knock = 0.6 * vel * (1.0 - 0.55 * key);
        for (i, &(bf, bt, ba)) in body.iter().enumerate() {
            let r = t60_gain(bt, sr);
            let w = core::f32::consts::TAU * bf / sr;
            v.body_a1[i] = 2.0 * r * w.cos();
            v.body_r2[i] = r * r;
            v.body_g[i] = ba * (1.0 - r) * knock;
        }
        v
    }

    pub fn render(&mut self, out: &mut [f32]) -> bool {
        let inv_pulse = 1.0 / self.body_pulse_len as f32;
        let inv_n = 1.0 / self.n_strings as f32;
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
            for (i, st) in self.strings.iter_mut().enumerate().take(self.n_strings) {
                s += st.tick() * self.out_w[i];
            }
            // soundboard radiation buildup (see field docs): strings bloom in,
            // the percussive knock/thump below stay immediate
            self.bloom += self.bloom_c * (1.0 - self.bloom);
            s *= self.bloom;
            // radiation highpass at 0.35·f0 (see field docs)
            self.rad_lp += self.rad_c * (s - self.rad_lp);
            s -= self.rad_lp;
            // hammer pulse into the body modes (case knock) + key thump noise
            let mut x = 0.0;
            if self.body_pulse_pos < self.body_pulse_len {
                let ph = self.body_pulse_pos as f32 * inv_pulse;
                x = 0.5 * (1.0 - (core::f32::consts::TAU * ph).cos());
                self.body_pulse_pos += 1;
            }
            for m in 0..3 {
                let y = self.body_a1[m] * self.body_y1[m] - self.body_r2[m] * self.body_y2[m]
                    + self.body_g[m] * x;
                self.body_y2[m] = self.body_y1[m];
                self.body_y1[m] = y;
                s += y;
            }
            if self.thump_amp > 1e-5 && self.thump_env > 1e-4 {
                s += self.thump_amp * self.thump_env * self.rng.next();
                self.thump_env *= self.thump_decay;
            }
            *o += s * self.level;
        }
        for st in self.strings.iter_mut().take(self.n_strings) {
            st.flush();
        }
        for m in 0..3 {
            self.body_y1[m] = flush_denormal(self.body_y1[m]);
            self.body_y2[m] = flush_denormal(self.body_y2[m]);
        }
        self.rad_lp = flush_denormal(self.rad_lp);
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
        let t_damp = 0.32 - 0.20 * key; // s: bass 0.32 → treble 0.12
        for st in self.strings.iter_mut().take(self.n_strings) {
            // per-pass (round-trip) loss, same bookkeeping as StringLoop::new
            st.loss = (-6.907_755 * st.len as f32 / (t_damp * self.sr)).exp();
        }
        self.life = self.age + (1.2 * self.sr) as u64;
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
    pub fn start_electric(midi: u32, f0: f32, vel: f32, sr: f32, dist: bool, seed: u32) -> Self {
        let key = ((midi as f32) - 40.0) / 44.0; // 0 = E2 … 1 = C6
        // dist: heavy strings + amp compression read as longer sustain; the pick
        // signal into a high-gain chain is bright (bridge pickup, tone full up)
        let t60 = if dist { 6.0 } else { 4.6 };
        // per-pass brightness: refs lose ~35 dB/s at 1 kHz in the low register
        // while H2..H5 barely decay — a steep loop corner, key-tracked so the
        // per-second HF decay stays register-flat (Valimaki et al. 1996 loop fit)
        let mut lp_c = (0.42 + 0.62 * key + 0.06 * vel).clamp(0.30, 0.985);
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
        let t60_slow = if dist { 12.0 } else { 9.0 };
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
            // Velocity moves loudness far less than timbre on an electric (NSynth
            // layer spread ≈ 5 LU, most of it spectral): keep the level curve
            // shallow and let the pick corner carry the dynamics. Mild key boost
            // compensates the short upper strings' lower energy (P72 was −18 LU).
            level: 0.52 * (0.72 + 0.28 * vel) * (1.0 + 0.35 * key.max(0.0)),
            life: ((t60_slow + 0.5) * sr) as u64,
            age: 0,
            sr,
        };
        // excitation: a pick pluck is a released displacement triangle ≈ 1/n²
        // harmonic tilt (−12 dB/oct; Smith PASP, pluck excitation), so shape the
        // noise with TWO cascaded one-pole lowpasses. Velocity moves the corner
        // (flesh-soft ≈ 200 Hz → hard plectrum ≈ 1.6 kHz), matching the NSynth
        // refs where the spectral knee scales with velocity but the cliff stays.
        // pick point at 0.28 of the sounding length (first comb dip ≈ H3.6);
        // bridge-side and velocity-tracked variants both measured worse
        let pick_pos = 0.28;
        let mut rng = Lcg(seed | 1);
        // corner is flatter in velocity than energy is (NSynth layers: the knee
        // moves ~1 octave from pp to ff, not 3) and tracks register upward
        let mut fc = ((120.0 + 300.0 * vel) * (1.0 + 1.0 * key)).clamp(80.0, 0.35 * sr);
        if dist {
            fc = (fc * 2.5).min(0.35 * sr);
        }
        let exc_c = 1.0 - (-core::f32::consts::TAU * fc / sr).exp();
        let mut lp1 = 0.0f32;
        let mut lp = 0.0f32;
        let mut tmp = [0.0f32; PLUCK_BUF];
        for t in tmp.iter_mut().take(len) {
            lp1 += exc_c * (rng.next() - lp1);
            lp += exc_c * (lp1 - lp);
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
        // pick displaces mostly one plane; ~0.3 leaks into the slow polarization
        for i in 0..v.len2 {
            v.buf2[i] = 0.3 * v.buf[i % len];
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
            *o += s * self.level;
        }
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
            send: 0.0,
            send_target: 0.0,
            send_c: 1.0 - (-1.0 / (0.015 * sr)).exp(),
            enabled: false,
            wet: 0.4,
        };
        for (i, &m) in SYMP_TUNING.iter().enumerate() {
            let f0 = midi_to_hz(m as f32);
            b.len[i] = ((sr / f0 - 0.5) as usize).clamp(2, SYMP_BUF - 1);
            // per-period loss (the fleet's convergent lesson): long open ring,
            // fast collapse when the dampers fall back
            b.loss_open[i] = 10f32.powf(-3.0 / (5.0 * f0));
            b.loss_damped[i] = 10f32.powf(-3.0 / (0.15 * f0));
        }
        b
    }

    pub fn set_pedal(&mut self, on: bool) {
        self.open = on;
        self.send_target = if on { 1.0 } else { 0.0 };
    }

    /// True while the bank could still be audible (skip processing otherwise).
    pub fn ringing(&self) -> bool {
        self.send > 1e-4 || self.open
    }

    #[inline]
    pub fn tick(&mut self, input: f32) -> f32 {
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
        Instrument::Guitar | Instrument::GuitarSteel | Instrument::Bass => {
            // per-note micro-offset: strings sit at slightly different spots
            ((seed >> 9) as f32 / 8388608.0 - 0.5) * 0.12
        }
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
                t60_f0: 8.0 + 4.0 * key,
                lp_c: 0.97 - 0.10 * key + 0.02 * vel,
                pick_pos: 0.20,
                contact: 0.045,
                snap: 0.5,
                scrape: 0.06,
                pol_mix: 0.35,
                pol_detune_cents: 2.2,
                pol_t60_ratio: 0.55,
                disp_c: 0.0,
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
                t60_f0: 6.5 - 2.5 * key,
                lp_c: 0.50 + 0.10 * vel,
                pick_pos: 0.30,
                contact: 0.10,
                snap: 0.35,
                scrape: 0.03,
                pol_mix: 0.25,
                pol_detune_cents: 1.2,
                pol_t60_ratio: 0.5,
                disp_c: 0.0,
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
                // velocity lives in the excitation, not the loop
                lp_c: 0.57 + 0.28 * key,
                pick_pos: 0.14,
                contact: 0.010,
                snap: 2.5,
                scrape: 0.35,
                pol_mix: 0.3,
                pol_detune_cents: 1.5,
                pol_t60_ratio: 0.55,
                disp_c: 0.0,
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

    /// Steel-string tuning at both deploy sample rates (the acoustic constructor
    /// has its own delay budget: exact loop-lowpass phase delay + tension-mod
    /// fraction bias) — protocol: delay math changes need 44.1k AND 48k coverage.
    #[test]
    fn steel_e3_in_tune_both_rates_and_velocities() {
        for sr in [44_100.0f32, 48_000.0f32] {
            for vel in [0.25f32, 1.0f32] {
                let out = render_pluck(Instrument::GuitarSteel, 52, vel, sr, 1.0);
                let tail = &out[(0.5 * sr) as usize..];
                let f = autocorr_pitch(tail, sr, 100.0, 400.0);
                let want = midi_to_hz(52.0);
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
                for inst in [Instrument::Guitar, Instrument::GuitarSteel] {
                    if let Kernel::Pluck(p) = start_voice(inst, midi, vel, 48_000.0, 7) {
                        assert!(p.loss <= 1.0, "{inst:?} midi={midi} loss={}", p.loss);
                        assert!(p.loss2 <= 1.0, "{inst:?} midi={midi} loss2={}", p.loss2);
                    }
                }
            }
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
