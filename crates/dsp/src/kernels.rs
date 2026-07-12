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
            _ => Self::Marimba,
        }
    }
}

/// Default amp-stage settings per instrument: (drive pre-gain, tone lowpass Hz).
/// drive 0.0 = bypass. Electric guitars are DEFINED by their amp — the drive lives
/// on the track bus so simultaneous notes intermodulate like a real amplifier.
pub fn amp_defaults(inst: Instrument) -> (f32, f32) {
    match inst {
        Instrument::GuitarElectric => (1.6, 5200.0),
        Instrument::GuitarDistorted => (6.5, 3800.0),
        _ => (0.0, 0.0),
    }
}

/// Magnetic-pickup resonance per instrument: (resonant-lowpass Hz, Q). The RLC
/// resonance of a real pickup is the core "electric" tone; 0.0 = bypass.
pub fn pickup_defaults(inst: Instrument) -> (f32, f32) {
    match inst {
        Instrument::GuitarElectric => (4200.0, 2.6),
        Instrument::GuitarDistorted => (3400.0, 3.2),
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
        // acoustic guitars: A0 Helmholtz air mode, T1 top plate, mid-mode ladder
        Instrument::Guitar => (
            0.55,
            &[
                (98.0, 0.28, 1.5),
                (196.0, 0.20, 1.9),
                (292.0, 0.14, 1.1),
                (428.0, 0.10, 0.85),
                (555.0, 0.08, 0.6),
                (712.0, 0.06, 0.45),
                (1050.0, 0.045, 0.3),
            ],
        ),
        Instrument::GuitarSteel => (
            0.55,
            &[
                (102.0, 0.26, 1.4),
                (208.0, 0.18, 1.8),
                (315.0, 0.13, 1.0),
                (460.0, 0.09, 0.8),
                (610.0, 0.075, 0.6),
                (890.0, 0.055, 0.45),
                (1280.0, 0.04, 0.3),
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
        Instrument::Guitar => 0.85,       // was -26.7 LUFS
        Instrument::Bass => 0.70,         // was -25.3 LUFS
        Instrument::EPiano => 1.47,       // was -26.6 LUFS
        Instrument::Drums => 0.61,        // was -27.4 LUFS
        Instrument::SynthPad => 0.48,     // was -26.5 LUFS
        Instrument::Piano => 0.11,          // v3 hammer runs hot; measured -13.5 LUFS pre-correction
        Instrument::GuitarSteel => 0.76,    // was -25.2 LUFS
        Instrument::GuitarElectric => 1.5,  // was -31.7 LUFS
        Instrument::GuitarDistorted => 0.57, // was -27.3 LUFS
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
}

impl PluckVoice {
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
    pub fn start_electric(midi: u32, f0: f32, vel: f32, sr: f32, seed: u32) -> Self {
        let key = ((midi as f32) - 40.0) / 44.0; // 0 = E2 … 1 = C6
        let t60 = 4.0;
        // per-pass brightness: gentle at the bottom, nearly lossless loop on top
        let lp_c = (0.60 + 0.42 * key + 0.06 * vel).clamp(0.35, 0.985);
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
            // Loop loss is applied once per ROUND TRIP (f0 times a second), not per
            // sample — the per-period gain for a wanted t60 is g = 10^(−3·P/t60)
            // (Jaffe & Smith 1983), i.e. t60_gain evaluated at rate f0, not sr.
            // (t60_gain(t60, sr) here would leave the fundamental ringing ~sr/f0
            //  times too long — measured 40 s instead of 4 s at f0 = 87 Hz.)
            loss: t60_gain(t60, f0),
            ap_c,
            ap_x1: 0.0,
            ap_y1: 0.0,
            level: 0.5 * (0.35 + 0.65 * vel),
            life: ((t60 * 1.5) * sr) as u64,
            age: 0,
            sr,
        };
        // excitation: a pick pluck is a released displacement triangle ≈ 1/n²
        // harmonic tilt (−12 dB/oct; Smith PASP, pluck excitation), so shape the
        // noise with TWO cascaded one-pole lowpasses. Velocity moves the corner
        // (flesh-soft ≈ 200 Hz → hard plectrum ≈ 1.6 kHz), matching the NSynth
        // refs where the spectral knee scales with velocity but the cliff stays.
        let pick_pos = 0.28;
        let mut rng = Lcg(seed | 1);
        let fc = 180.0 + 1450.0 * vel * vel;
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
            *o += y * self.level;
        }
        self.lp = flush_denormal(self.lp);
        self.ap_y1 = flush_denormal(self.ap_y1);
        self.age += out.len() as u64;
        self.age < self.life
    }

    pub fn damp(&mut self) {
        self.loss = t60_gain(0.07, self.sr);
        self.life = self.age + (0.1 * self.sr) as u64;
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

impl StringLoop {
    /// `detune_cents` shifts this string against the nominal pitch (unison beating).
    fn new(f0: f32, detune_cents: f32, sr: f32, t60: f32, lp_c: f32, disp_c: f32) -> Self {
        let f = f0 * (detune_cents / 1200.0).exp2();
        // Total loop delay budget: buffer + tuning-allpass fraction + loop-lowpass
        // phase delay + 2× dispersion-allpass DC delay (Jaffe-Smith compensation —
        // without the dispersion term the stiff strings would all play flat).
        let lp_delay = (1.0 - lp_c) / lp_c;
        let disp_delay = 2.0 * (1.0 - disp_c) / (1.0 + disp_c);
        let total = (sr / f - lp_delay - disp_delay).max(3.0);
        let len = ((total - 0.5).floor() as usize).clamp(2, PLUCK_BUF - 1);
        let frac = (total - len as f32).clamp(0.1, 1.5);
        Self {
            buf: [0.0; PLUCK_BUF],
            len,
            pos: 0,
            lp: 0.0,
            lp_c,
            loss: t60_gain(t60, sr),
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
        // DC blocker (loop must not carry the hammer's unipolar injection)
        let dc = ap - self.dc_x1 + 0.9975 * self.dc_y1;
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
    rng: Lcg,
    sr: f32,
    life: u64,
    age: u64,
}

impl PianoVoice {
    pub fn start(midi: u32, f0: f32, vel: f32, sr: f32, seed: u32) -> Self {
        let key = ((midi as f32) - 21.0) / 87.0; // 0 = A0 … 1 = C8
        // register scaling: long singing bass → short bright top
        let t60 = (11.0 * (1.0 - key).powf(1.7) + 0.9).min(11.0);
        let lp_c = (0.32 + 0.44 * key + 0.18 * vel).clamp(0.25, 0.95);
        // stiffness (inharmonicity): audible on wound bass strings, mild in mid
        let disp_c =
            if key < 0.35 { 0.20 * (1.0 - key / 0.35) + 0.05 } else { 0.035 + 0.04 * (key - 0.35) };

        // Two-stage decay: string 0 is the SUSTAIN mode (darker loop, full t60);
        // the others are the ATTACK bloom (brighter, short, detuned — Weinreich
        // coupling/polarization). All are struck by the SAME hammer.
        let t_attack = ((0.35 + 1.0 * (1.0 - key)) * (0.4 + 0.6 * vel)).min(0.45 * t60);
        let n_strings = if midi < 32 { 2 } else { 3 };
        let detune_spread = if midi < 32 { 0.35 } else { 1.5 - 0.7 * key };
        let cfg: [(f32, f32, f32); 3] = [
            (0.0, t60, 0.80),                       // (detune cents, t60 s, lp_c mult) sustain
            (detune_spread, t_attack, 1.40),        // attack bloom +
            (-0.8 * detune_spread, t_attack * 1.15, 1.28), // attack bloom −
        ];
        let rng = Lcg(seed | 1);
        let mut strings = [StringLoop::new(f0, 0.0, sr, t60, lp_c, disp_c); 3];
        let mut strike_off = [0usize; 3];
        for (i, s) in strings.iter_mut().enumerate().take(n_strings) {
            let (cents, t_sec, lp_mul) = cfg[i];
            *s = StringLoop::new(f0, cents, sr, t_sec, (lp_c * lp_mul).min(0.97), disp_c);
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
            n_strings,
            level: 2.4 / (n_strings as f32),
            h_x: 0.0,
            h_v: h_v0,
            h_k,
            h_p,
            h_gain: 260.0, // force→displacement-wave coupling, tuned via piano-audition peaks
            h_active: true,
            body_a1: [0.0; 3],
            body_r2: [0.0; 3],
            body_y1: [0.0; 3],
            body_y2: [0.0; 3],
            body_g: [0.0; 3],
            body_pulse_pos: 0,
            body_pulse_len: ((0.003 * sr) as u32).max(2),
            thump_env: 1.0,
            thump_decay: t60_gain(0.010, sr),
            thump_amp: 0.05 * vel,
            rng,
            sr,
            life: ((t60 * 1.4 + 0.1) * sr) as u64,
            age: 0,
        };
        let body = [(85.0f32, 0.40f32, 0.30f32), (172.0, 0.28, 0.20), (318.0, 0.18, 0.13)];
        for (i, &(bf, bt, ba)) in body.iter().enumerate() {
            let r = t60_gain(bt, sr);
            let w = core::f32::consts::TAU * bf / sr;
            v.body_a1[i] = 2.0 * r * w.cos();
            v.body_r2[i] = r * r;
            v.body_g[i] = ba * (1.0 - r) * 2.5 * vel;
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
                let f = if comp > 0.0 { self.h_k * comp.powf(self.h_p) } else { 0.0 };
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
            for st in self.strings.iter_mut().take(self.n_strings) {
                s += st.tick();
            }
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
        self.age += out.len() as u64;
        self.age < self.life
    }

    /// Damper falls: fast but not instant (real dampers take ~0.1 s to kill a string).
    pub fn damp(&mut self) {
        for st in self.strings.iter_mut().take(self.n_strings) {
            st.loss = t60_gain(0.12, self.sr);
        }
        self.life = self.age + (0.25 * self.sr) as u64;
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
// Drum kit (GM pitches) — sine-sweep kick, mode+noise snare, filtered-noise cymbals
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
    modal: ModalVoice,
    has_modal: bool,
    rng: Lcg,
    life: u64,
    age: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrumKind {
    Kick,
    Noise, // snare wires, hats, cymbals
}

impl DrumVoice {
    pub fn start(gm_note: u32, vel: f32, sr: f32, seed: u32) -> Self {
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
            modal: ModalVoice::start(200.0, 0.0, sr, &[], 1.0, 0.0, 0.0, seed),
            has_modal: false,
            rng: Lcg(seed | 1),
            life: (0.6 * sr) as u64,
            age: 0,
        };
        match gm_note {
            35 | 36 => {
                // kick: 110→43 Hz sweep, soft knee
                v.kind = DrumKind::Kick;
                v.freq = 110.0;
                v.freq_end = 43.0;
                v.sweep = (-1.0 / (0.035 * sr)).exp();
                v.decay = t60_gain(0.42, sr);
                v.amp = vel * 0.9;
                v.life = (0.6 * sr) as u64;
            }
            38 | 40 => {
                // snare: two shell modes + bright wire noise
                v.decay = t60_gain(0.16, sr);
                v.hp_c = 0.35;
                v.noise_amt = 0.5 + 0.5 * vel;
                v.has_modal = true;
                v.modal = ModalVoice::start(
                    186.0,
                    vel,
                    sr,
                    &[
                        ModeDef { ratio: 1.0, amp: 0.9, t60: 0.11 },
                        ModeDef { ratio: 1.78, amp: 0.55, t60: 0.08 },
                    ],
                    0.4,
                    0.0,
                    0.0,
                    seed ^ 0x9e37,
                );
                v.life = (0.35 * sr) as u64;
            }
            42 | 44 => {
                // closed hat
                v.decay = t60_gain(0.055, sr);
                v.hp_c = 0.72;
                v.amp = vel * 0.55;
                v.life = (0.12 * sr) as u64;
            }
            46 => {
                // open hat
                v.decay = t60_gain(0.38, sr);
                v.hp_c = 0.7;
                v.amp = vel * 0.5;
                v.life = (0.7 * sr) as u64;
            }
            49 | 57 => {
                // crash
                v.decay = t60_gain(1.6, sr);
                v.hp_c = 0.5;
                v.amp = vel * 0.5;
                v.life = (2.2 * sr) as u64;
            }
            51 | 59 => {
                // ride: inharmonic metallic ping cluster (cymbal modes are dense and
                // irrationally spaced) over a restrained wash — a sine over noise
                // reads as a test tone, not a cymbal (listening note 2026-07-11)
                v.decay = t60_gain(1.25, sr);
                v.hp_c = 0.62;
                v.amp = vel * 0.16;
                v.noise_amt = 0.6;
                v.has_modal = true;
                v.modal = ModalVoice::start(
                    905.0,
                    (vel * 0.9).min(1.0),
                    sr,
                    &[
                        ModeDef { ratio: 1.0, amp: 0.85, t60: 1.7 },
                        ModeDef { ratio: 1.594, amp: 0.60, t60: 1.15 },
                        ModeDef { ratio: 2.137, amp: 0.48, t60: 0.85 },
                        ModeDef { ratio: 2.781, amp: 0.34, t60: 0.62 },
                        ModeDef { ratio: 3.417, amp: 0.25, t60: 0.45 },
                        ModeDef { ratio: 4.312, amp: 0.16, t60: 0.32 },
                        ModeDef { ratio: 5.483, amp: 0.10, t60: 0.22 },
                    ],
                    0.35,
                    0.06,
                    0.0,
                    seed ^ 0x51de,
                );
                v.life = (2.0 * sr) as u64;
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
        let dt = 1.0 / sr;
        for o in out.iter_mut() {
            let mut s;
            match self.kind {
                DrumKind::Kick => {
                    self.freq = self.freq_end + (self.freq - self.freq_end) * self.sweep;
                    self.phase = (self.phase + self.freq * dt).fract();
                    s = (core::f32::consts::TAU * self.phase).sin() * self.env;
                    // contact click for the first ~4 ms
                    if self.age < (0.004 * sr) as u64 {
                        s += 0.3 * self.rng.next() * self.env;
                    }
                }
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
    pub age: u64,
}

impl Voice {
    pub const fn off() -> Self {
        Self { kernel: Kernel::Off, track: 0, midi: 0, releasing: false, pedal_held: false, age: 0 }
    }
    pub fn active(&self) -> bool {
        !matches!(self.kernel, Kernel::Off)
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
            let key = ((midi as f32) - 40.0) / 44.0;
            let t60 = (4.2 - 2.6 * key).clamp(0.8, 4.2);
            Kernel::Pluck(PluckVoice::start(f0, vel, sr, t60, 0.55, 0.28, seed))
        }
        Instrument::Bass => {
            // warm fingered upright/electric hybrid: dark loop, pluck near the neck
            // (user listening note 2026-07-11: bridge-picked bright bass read as
            // "loud and weird" — rounder is right for the default)
            let t60 = 5.0 - 2.0 * (((midi as f32) - 28.0) / 32.0).clamp(0.0, 1.0);
            Kernel::Pluck(PluckVoice::start(f0, vel, sr, t60, 0.26, 0.31, seed))
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
        Instrument::Drums => Kernel::Drum(DrumVoice::start(midi, vel, sr, seed)),
        Instrument::SynthPad => Kernel::Synth(SynthVoice::start(f0, vel, sr)),
        Instrument::Piano => Kernel::Piano(PianoVoice::start(midi, f0, vel, sr, seed)),
        Instrument::GuitarSteel => {
            // bright bronze-wound pluck, pick closer to the bridge than the nylon
            let key = ((midi as f32) - 40.0) / 44.0;
            let t60 = (5.0 - 2.8 * key).clamp(1.0, 5.0);
            Kernel::Pluck(PluckVoice::start(f0, vel, sr, t60, 0.68, 0.20, seed))
        }
        Instrument::GuitarElectric | Instrument::GuitarDistorted => {
            // solid-body voicing lives in start_electric; the amp/pickup character
            // comes from the track bus stages (pickup_defaults / amp_defaults)
            Kernel::Pluck(PluckVoice::start_electric(midi, f0, vel, sr, seed))
        }
    }
}

#[cfg(test)]
mod diag {
    use super::*;
    #[test]
    fn diag_electric_kernel_decay() {
        let sr = 48000.0;
        let mut v = PluckVoice::start_electric(41, midi_to_hz(41.0), 1.0, sr, 12345);
        let mut out = vec![0.0f32; (4.0 * sr) as usize];
        for chunk in out.chunks_mut(128) {
            v.render(chunk);
        }
        let hop = (0.1 * sr) as usize;
        let mut dbs = Vec::new();
        for w in out.chunks(hop).take(20) {
            let rms = (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt();
            dbs.push(format!("{:.1}", 20.0 * (rms + 1e-9).log10()));
        }
        eprintln!("KERNEL-ONLY env dB: {}", dbs.join(" "));
    }
}
