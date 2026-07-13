//! instruments.js DSP core.
//!
//! Constraints (AGENTS.md constitution #4, architecture doc 2026-07-11):
//! - Allocation-free after `Engine::new` — the audio thread never touches the allocator.
//! - All recursive filter state passes through [`flush_denormal`] (WASM has no hardware FTZ).
//! - One engine renders and mixes ALL tracks/instruments; the budget is 2.67 ms per
//!   128-frame quantum at 48 kHz for a full multi-track arrangement.
//! - Sample rate is per-instance, never global, never assumed 48 kHz (iOS locks to 44.1 kHz).
//! - Per-voice SIMD lane batching is deferred (tracked in the architecture doc); the SoA
//!   refactor happens when dsp-bench shows we need it.

#![deny(unsafe_code)]

pub mod kernels;
pub mod wdf;

use kernels::{
    amp_defaults, body_defaults, makeup_gain, pickup_defaults, start_voice, voice_pan, Instrument,
    Kernel, SympBank, Voice, MAX_BLOCK, MAX_BODY_MODES,
};
use wdf::WdfAmp;

pub const MAX_VOICES: usize = 64;
pub const MAX_TRACKS: usize = 16;
pub const QUANTUM_FRAMES: usize = 128;

/// Flush denormals to zero. Denormals in recursive feedback loops are the top WASM perf
/// killer (Letz/Orlarey 2018); every state variable update must pass through this.
#[inline(always)]
pub fn flush_denormal(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

#[derive(Clone, Copy)]
pub struct TrackBus {
    pub instrument: Instrument,
    pub gain: f32,
    pub pan: f32, // -1.0 .. 1.0
    /// sustain pedal (CC64): note-offs are deferred while down
    pub pedal: bool,
    // amp stage (electric guitars): ADAA-antialiased tanh drive + tone lowpass.
    // Lives on the BUS so simultaneous notes intermodulate like a real amplifier.
    drive: f32,
    tone_c: f32,
    amp_x1: f32,
    amp_f1: f32,
    tone_lp: f32,
    // amp gain-ride (kernels::amp_ride_defaults): supply-rail-recovery /
    // bias-shift compression, pre-drive. env = fast-attack/slow-release |x|
    // follower; gain slews toward min(cap, (thr/env)^p) — slow rise (rail
    // recovery), fast fall (sag on a fresh attack). Sub-audio-rate gain by
    // construction: no extra aliasing, ADAA stage untouched.
    ride_thr: f32,
    ride_p: f32,
    ride_cap: f32,
    ride_env: f32,
    ride_g: f32,
    ride_env_a: f32,
    ride_env_r: f32,
    ride_up: f32,
    ride_dn: f32,
    // post-drive cab/presence EQ (kernels::amp_post_eq_defaults): two RBJ
    // peaking biquads after the drive + tone lowpass (clip-generated presence
    // and the cab's LF chug bump cannot come from pre-clip EQ — r3 rec #1).
    // Shared by both amp paths (the WDF "cab" is still these biquads in P1).
    pe_on: bool,
    pe_b: [[f32; 3]; 2],
    pe_a: [[f32; 2]; 2],
    pe_z: [[f32; 2]; 2],
    // WDF circuit-sim amp (wdf::WdfAmp): 12AX7 triode root + Fender TMB tone
    // stack + supply-rail sag. An ALTERNATIVE to the behavioral drive/ride
    // chain above, selected per-track by `wdf_on`. `wdf_voiced` = this
    // instrument has a WDF voicing; behavioral chain stays default (P1).
    wdf_on: bool,
    wdf_voiced: bool,
    wdf_amp: WdfAmp,
    // body resonator bank (acoustic instruments): parallel modes + dry path
    body_n: usize,
    body_dry: f32,
    body_a1: [f32; MAX_BODY_MODES],
    body_r2: [f32; MAX_BODY_MODES],
    body_g: [f32; MAX_BODY_MODES],
    body_y1: [[f32; MAX_BODY_MODES]; 2],
    body_y2: [[f32; MAX_BODY_MODES]; 2],
    // magnetic-pickup resonance (electrics): RBJ resonant lowpass biquad
    pk_on: bool,
    pk_b0: f32,
    pk_b1: f32,
    pk_b2: f32,
    pk_a1: f32,
    pk_a2: f32,
    pk_z1: f32,
    pk_z2: f32,
    // room send (kernels::room_send default; ij_set_room overrides)
    room_send: f32,
    // smoothed equal-power gains (one-pole per block, no zipper noise)
    gl: f32,
    gr: f32,
}

/// Numerically stable ln(cosh(x)) — the antiderivative of tanh, for first-order
/// antiderivative anti-aliasing (ADAA) of the amp waveshaper.
#[inline(always)]
fn ln_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (1.0 + (-2.0 * a).exp()).ln() - core::f32::consts::LN_2
}

/// Global output anchor. The per-family makeup table normalizes families to EQUAL
/// loudness; this constant sets how loud that equal level IS. +5 dB over the
/// original marimba-derived anchor (Keunwoo 2026-07-12: "overall too low volume") —
/// single notes land ~-21 LUFS, leaving master-limiter headroom for arrangements.
const MASTER_LEVEL: f32 = 1.78;

impl TrackBus {
    fn targets(&self) -> (f32, f32) {
        let th = (self.pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
        // measured per-family loudness normalization (kernels::makeup_gain);
        // sqrt(2) compensates the equal-power per-voice spread so a centered voice
        // on a centered track lands at exactly the pre-stereo level
        let g = self.gain * makeup_gain(self.instrument) * MASTER_LEVEL * core::f32::consts::SQRT_2;
        (g * th.cos(), g * th.sin())
    }
}

/// Shared spatial stage (audit 2026-07-12; reverb menu per Keunwoo
/// 2026-07-13 "let's add several reverb choices"): ONE per engine — every
/// track sends into the same space, which is what glues an arrangement into a
/// record instead of N anechoic close mics. Five voicings share one topology
/// (pre-delay → optional input-diffusion allpasses → sparse early taps →
/// 4-line FDN with orthogonal butterfly feedback, per-line HF damping, and
/// optional per-line dispersion allpasses for the spring chirp):
///   Off · Room (default glue, RT60 0.38 s) · Hall (1.9 s, darker, late
///   earlies) · Plate (bright dense 1.3 s, no earlies) · Spring (dispersive
///   boing, dark, 1.6 s).
/// All buffers are allocated ONCE at max capacity in `new`; switching types
/// only rewrites lengths/gains and clears state — the audio path and the
/// control path are both allocation-free.
/// Pre-delays are deliberately generous (room 22 / plate 14 / spring 14 /
/// hall 45 ms; Keunwoo 2026-07-13): the wet onset must sit AFTER the dry
/// transient or fast attacks (piano especially) smear. Pre buffer is sized
/// for the longest (hall) with headroom.
pub const REVERB_OFF: u32 = 0;
pub const REVERB_ROOM: u32 = 1;
pub const REVERB_HALL: u32 = 2;
pub const REVERB_PLATE: u32 = 3;
pub const REVERB_SPRING: u32 = 4;

struct Reverb {
    kind: u32,
    sr: f32,
    wet: f32,
    pre: Vec<f32>,
    pre_len: usize,
    pre_pos: usize,
    // input diffusion (hall/plate/spring): two short feedback allpasses
    apd: [Vec<f32>; 2],
    apd_len: [usize; 2],
    apd_pos: [usize; 2],
    apd_g: f32,
    early: Vec<f32>,
    early_len: usize,
    early_pos: usize,
    taps: [(usize, f32); 6],
    n_taps: usize,
    fdn: [Vec<f32>; 4],
    fdn_len: [usize; 4],
    fdn_pos: [usize; 4],
    fdn_g: [f32; 4],
    damp: [f32; 4],
    damp_c: f32,
    // spring dispersion: 2 first-order allpasses in each feedback path
    disp_g: f32,
    disp_x: [[f32; 4]; 2],
    disp_y: [[f32; 4]; 2],
}

impl Reverb {
    fn new(sr: f32) -> Self {
        let ms = |m: f32| (((m * 1e-3 * sr) as usize) | 1).max(3);
        let mut r = Self {
            kind: REVERB_ROOM,
            sr,
            wet: 1.0,
            pre: vec![0.0; ms(60.0)],
            pre_len: 3,
            pre_pos: 0,
            apd: [vec![0.0; ms(10.0)], vec![0.0; ms(10.0)]],
            apd_len: [3, 3],
            apd_pos: [0, 0],
            apd_g: 0.0,
            early: vec![0.0; ms(95.0)],
            early_len: 3,
            early_pos: 0,
            taps: [(0, 0.0); 6],
            n_taps: 0,
            fdn: [vec![0.0; ms(120.0)], vec![0.0; ms(120.0)], vec![0.0; ms(120.0)], vec![0.0; ms(120.0)]],
            fdn_len: [3; 4],
            fdn_pos: [0; 4],
            fdn_g: [0.0; 4],
            damp: [0.0; 4],
            damp_c: 0.0,
            disp_g: 0.0,
            disp_x: [[0.0; 4]; 2],
            disp_y: [[0.0; 4]; 2],
        };
        r.configure(REVERB_ROOM);
        r
    }

    /// Switch voicing: rewrite lengths/gains, clear state. No allocation —
    /// lengths clamp to the capacities reserved in `new`.
    fn configure(&mut self, kind: u32) {
        let sr = self.sr;
        let ms = |m: f32| (((m * 1e-3 * sr) as usize) | 1).max(3);
        self.kind = kind;
        // (pre ms, apd g, apd ms pair, tap table, fdn ms, rt60, damp hz, disp g, wet)
        struct Cfg {
            pre: f32,
            apd_g: f32,
            apd_ms: [f32; 2],
            taps_ms: [f32; 6],
            taps_g: [f32; 6],
            n_taps: usize,
            fdn_ms: [f32; 4],
            rt60: f32,
            damp_hz: f32,
            disp_g: f32,
            wet: f32,
        }
        let c = match kind {
            REVERB_HALL => Cfg {
                pre: 45.0,
                apd_g: 0.6,
                apd_ms: [5.3, 8.9],
                taps_ms: [19.3, 29.1, 41.9, 57.7, 71.3, 88.9],
                taps_g: [0.38, -0.32, 0.27, -0.22, 0.18, -0.14],
                n_taps: 6,
                fdn_ms: [61.4, 77.9, 91.3, 109.7],
                rt60: 1.9,
                damp_hz: 3200.0,
                disp_g: 0.0,
                wet: 0.9,
            },
            REVERB_PLATE => Cfg {
                pre: 14.0,
                apd_g: 0.7,
                apd_ms: [4.7, 7.3],
                taps_ms: [0.0; 6],
                taps_g: [0.0; 6],
                n_taps: 0,
                fdn_ms: [23.9, 31.7, 41.3, 49.9],
                rt60: 1.3,
                damp_hz: 6500.0,
                disp_g: 0.0,
                wet: 1.0,
            },
            REVERB_SPRING => Cfg {
                pre: 14.0,
                apd_g: 0.5,
                apd_ms: [3.1, 5.9],
                taps_ms: [0.0; 6],
                taps_g: [0.0; 6],
                n_taps: 0,
                fdn_ms: [31.1, 37.3, 43.7, 51.1],
                rt60: 1.6,
                damp_hz: 2800.0,
                disp_g: 0.55,
                wet: 1.1,
            },
            // Room is also the fallback for unknown kinds
            _ => Cfg {
                pre: 22.0,
                apd_g: 0.0,
                apd_ms: [5.0, 8.0],
                taps_ms: [13.1, 19.7, 26.3, 34.9, 43.7, 52.9],
                taps_g: [0.62, -0.50, 0.42, -0.33, 0.26, -0.20],
                n_taps: 6,
                fdn_ms: [41.7, 53.3, 63.1, 74.3],
                rt60: 0.38,
                damp_hz: 4500.0,
                disp_g: 0.0,
                wet: 1.0,
            },
        };
        self.pre_len = ms(c.pre).min(self.pre.len());
        self.apd_g = c.apd_g;
        for k in 0..2 {
            self.apd_len[k] = ms(c.apd_ms[k]).min(self.apd[k].len());
        }
        self.n_taps = c.n_taps;
        let mut max_tap = 3;
        for i in 0..6 {
            let d = ms(c.taps_ms[i].max(0.1)).min(self.early.len() - 1);
            self.taps[i] = (d, c.taps_g[i]);
            if i < c.n_taps && d > max_tap {
                max_tap = d;
            }
        }
        self.early_len = (max_tap + 1).min(self.early.len());
        for k in 0..4 {
            self.fdn_len[k] = ms(c.fdn_ms[k]).min(self.fdn[k].len());
            self.fdn_g[k] = 10f32.powf(-3.0 * self.fdn_len[k] as f32 / (c.rt60 * sr));
        }
        self.damp_c = 1.0 - (-core::f32::consts::TAU * c.damp_hz / sr).exp();
        self.disp_g = c.disp_g;
        self.wet = c.wet;
        // clear state (memset of preallocated buffers — no allocation)
        self.pre.fill(0.0);
        self.early.fill(0.0);
        for k in 0..2 {
            self.apd[k].fill(0.0);
        }
        for k in 0..4 {
            self.fdn[k].fill(0.0);
        }
        self.pre_pos = 0;
        self.early_pos = 0;
        self.apd_pos = [0, 0];
        self.fdn_pos = [0; 4];
        self.damp = [0.0; 4];
        self.disp_x = [[0.0; 4]; 2];
        self.disp_y = [[0.0; 4]; 2];
    }

    /// Adds the wet signal for `input` (mono send bus) into out_l/out_r.
    /// Branch-wraps instead of `%` throughout (the modulo version measured
    /// ~3.8 budget points on the full demo); positions only ever step by 1
    /// and tap offsets are < buffer length, so a conditional subtract is exact.
    fn process(&mut self, input: &[f32], out_l: &mut [f32], out_r: &mut [f32], frames: usize) {
        if self.kind == REVERB_OFF {
            return;
        }
        let el = self.early_len;
        let pl = self.pre_len;
        let wet = self.wet;
        for i in 0..frames {
            // pre-delay
            let mut x = self.pre[self.pre_pos];
            self.pre[self.pre_pos] = input[i];
            self.pre_pos += 1;
            if self.pre_pos >= pl {
                self.pre_pos = 0;
            }
            // input diffusion (feedback allpasses; g=0 bypasses cheaply)
            if self.apd_g > 0.0 {
                for k in 0..2 {
                    let p = self.apd_pos[k];
                    let d = self.apd[k][p];
                    let w = x + self.apd_g * d;
                    self.apd[k][p] = flush_denormal(w);
                    x = d - self.apd_g * w;
                    self.apd_pos[k] = p + 1;
                    if self.apd_pos[k] >= self.apd_len[k] {
                        self.apd_pos[k] = 0;
                    }
                }
            }
            // early reflections from a shared tapped line
            let (mut e_l, mut e_r) = (0.0f32, 0.0f32);
            if self.n_taps > 0 {
                self.early[self.early_pos] = x;
                for (k, (d, g)) in self.taps[..self.n_taps].iter().enumerate() {
                    let mut idx = self.early_pos + el - d;
                    if idx >= el {
                        idx -= el;
                    }
                    let s = self.early[idx] * g;
                    if k & 1 == 0 {
                        e_l += s;
                    } else {
                        e_r += s;
                    }
                }
                self.early_pos += 1;
                if self.early_pos >= el {
                    self.early_pos = 0;
                }
            }
            // FDN tail: read heads, orthogonal butterfly, damped feedback
            let mut r = [0.0f32; 4];
            for k in 0..4 {
                r[k] = self.fdn[k][self.fdn_pos[k]];
            }
            let (a, b) = (r[0] + r[1], r[0] - r[1]);
            let (c, d) = (r[2] + r[3], r[2] - r[3]);
            let mixed = [0.5 * (a + c), 0.5 * (b + d), 0.5 * (a - c), 0.5 * (b - d)];
            for k in 0..4 {
                self.damp[k] += self.damp_c * (mixed[k] - self.damp[k]);
                let mut w = self.damp[k] * self.fdn_g[k];
                // spring chirp: first-order dispersion allpasses in the loop
                if self.disp_g > 0.0 {
                    for st in 0..2 {
                        let y = -self.disp_g * w + self.disp_x[st][k]
                            + self.disp_g * self.disp_y[st][k];
                        self.disp_x[st][k] = w;
                        self.disp_y[st][k] = y;
                        w = y;
                    }
                }
                let w = flush_denormal(w + 0.25 * x);
                self.fdn[k][self.fdn_pos[k]] = w;
                self.fdn_pos[k] += 1;
                if self.fdn_pos[k] >= self.fdn_len[k] {
                    self.fdn_pos[k] = 0;
                }
            }
            out_l[i] += wet * (e_l + 0.7 * (r[0] - r[2]));
            out_r[i] += wet * (e_r + 0.7 * (r[1] - r[3]));
        }
        for k in 0..4 {
            self.damp[k] = flush_denormal(self.damp[k]);
            for st in 0..2 {
                self.disp_y[st][k] = flush_denormal(self.disp_y[st][k]);
            }
        }
    }
}

pub struct Engine {
    pub sample_rate: f32,
    voices: Vec<Voice>,
    tracks: [TrackBus; MAX_TRACKS],
    track_l: [f32; MAX_BLOCK],
    track_r: [f32; MAX_BLOCK],
    voice_buf: [f32; MAX_BLOCK],
    symp: Vec<SympBank>,
    reverb: Reverb,
    room_in: [f32; MAX_BLOCK],
    pub out_l: [f32; MAX_BLOCK],
    pub out_r: [f32; MAX_BLOCK],
    seed: u32,
}

impl Engine {
    /// All allocation happens here, once.
    pub fn new(sample_rate: f32) -> Self {
        let mut voices = Vec::with_capacity(MAX_VOICES);
        voices.resize(MAX_VOICES, Voice::off());
        let symp = (0..MAX_TRACKS).map(|_| SympBank::new(sample_rate)).collect();
        Self {
            sample_rate,
            voices,
            tracks: [TrackBus {
                instrument: Instrument::Marimba,
                gain: 0.8,
                pan: 0.0,
                pedal: false,
                drive: 0.0,
                tone_c: 0.0,
                amp_x1: 0.0,
                amp_f1: 0.0,
                tone_lp: 0.0,
                ride_thr: 0.0,
                ride_p: 0.0,
                ride_cap: 1.0,
                ride_env: 0.0,
                ride_g: 1.0,
                ride_env_a: 0.0,
                ride_env_r: 0.0,
                ride_up: 0.0,
                ride_dn: 0.0,
                pe_on: false,
                pe_b: [[0.0; 3]; 2],
                pe_a: [[0.0; 2]; 2],
                pe_z: [[0.0; 2]; 2],
                wdf_on: false,
                wdf_voiced: false,
                wdf_amp: WdfAmp::new(),
                body_n: 0,
                body_dry: 1.0,
                body_a1: [0.0; MAX_BODY_MODES],
                body_r2: [0.0; MAX_BODY_MODES],
                body_g: [0.0; MAX_BODY_MODES],
                body_y1: [[0.0; MAX_BODY_MODES]; 2],
                body_y2: [[0.0; MAX_BODY_MODES]; 2],
                pk_on: false,
                pk_b0: 0.0,
                pk_b1: 0.0,
                pk_b2: 0.0,
                pk_a1: 0.0,
                pk_a2: 0.0,
                pk_z1: 0.0,
                pk_z2: 0.0,
                room_send: kernels::room_send(Instrument::Marimba),
                gl: 0.0,
                gr: 0.0,
            }; MAX_TRACKS],
            track_l: [0.0; MAX_BLOCK],
            track_r: [0.0; MAX_BLOCK],
            voice_buf: [0.0; MAX_BLOCK],
            symp,
            reverb: Reverb::new(sample_rate),
            room_in: [0.0; MAX_BLOCK],
            out_l: [0.0; MAX_BLOCK],
            out_r: [0.0; MAX_BLOCK],
            seed: 0x1234_5678,
        }
    }

    pub fn set_track(&mut self, track: usize, instrument: Instrument, gain: f32, pan: f32) {
        if track < MAX_TRACKS {
            let t = &mut self.tracks[track];
            t.instrument = instrument;
            t.gain = gain.clamp(0.0, 2.0);
            t.pan = pan.clamp(-1.0, 1.0);
            t.room_send = kernels::room_send(instrument);
            let (drive, tone_hz) = amp_defaults(instrument);
            t.drive = drive;
            t.tone_c = if tone_hz > 0.0 {
                1.0 - (-core::f32::consts::TAU * tone_hz / self.sample_rate).exp()
            } else {
                0.0
            };
            // amp gain-ride (see TrackBus fields / kernels::amp_ride_defaults)
            let (rthr, rp, rcap, rrec) = kernels::amp_ride_defaults(instrument);
            t.ride_thr = rthr;
            t.ride_p = rp;
            t.ride_cap = rcap;
            t.ride_env = 0.0;
            t.ride_g = 1.0;
            // envelope: 2 ms attack (catch the pick immediately), 120 ms release
            // (tracks string decays without ripple at the fundamental)
            t.ride_env_a = 1.0 - (-1.0 / (0.002 * self.sample_rate)).exp();
            t.ride_env_r = 1.0 - (-1.0 / (0.120 * self.sample_rate)).exp();
            // gain slew: rail recovery up (per-family seconds), sag down ~3 ms
            t.ride_up = 1.0 - (-1.0 / (rrec * self.sample_rate)).exp();
            t.ride_dn = 1.0 - (-1.0 / (0.003 * self.sample_rate)).exp();
            // post-drive cab/presence peaking EQ (see TrackBus fields)
            let sections = kernels::amp_post_eq_defaults(instrument);
            t.pe_on = sections.0 .0 > 0.0;
            t.pe_z = [[0.0; 2]; 2];
            for (k, &(f, q, gdb)) in [sections.0, sections.1].iter().enumerate() {
                if f <= 0.0 {
                    // identity section
                    t.pe_b[k] = [1.0, 0.0, 0.0];
                    t.pe_a[k] = [0.0, 0.0];
                    continue;
                }
                let a = 10f32.powf(gdb / 40.0);
                let w = core::f32::consts::TAU * f / self.sample_rate;
                let (sw, cw) = w.sin_cos();
                let alpha = sw / (2.0 * q);
                let a0 = 1.0 + alpha / a;
                t.pe_b[k] = [(1.0 + alpha * a) / a0, (-2.0 * cw) / a0, (1.0 - alpha * a) / a0];
                t.pe_a[k] = [(-2.0 * cw) / a0, (1.0 - alpha / a) / a0];
            }
            // WDF circuit-sim amp (alternative path — behavioral stays default).
            // Prepare the 12AX7 stage + TMB tone stack + supply sag for this
            // instrument's voicing; `wdf_on` gates whether it's used at runtime.
            match kernels::amp_wdf_voicing(instrument) {
                Some(lead) => {
                    t.wdf_voiced = true;
                    let cfg = if lead {
                        wdf::lead_config()
                    } else {
                        wdf::clean_config()
                    };
                    t.wdf_amp.prepare(cfg, self.sample_rate);
                }
                None => t.wdf_voiced = false,
            }
            t.wdf_on = kernels::WDF_AMP_DEFAULT && t.wdf_voiced;
            // body resonator bank (acoustics)
            let sr = self.sample_rate;
            let (dry, modes) = body_defaults(instrument);
            t.body_dry = dry;
            t.body_n = modes.len().min(MAX_BODY_MODES);
            for (i, &(f, t60, g)) in modes.iter().take(t.body_n).enumerate() {
                let r = (-6.907755 / (t60 * sr)).exp();
                let w = core::f32::consts::TAU * f / sr;
                t.body_a1[i] = 2.0 * r * w.cos();
                t.body_r2[i] = r * r;
                t.body_g[i] = g * (1.0 - r);
                t.body_y1[0][i] = 0.0;
                t.body_y1[1][i] = 0.0;
                t.body_y2[0][i] = 0.0;
                t.body_y2[1][i] = 0.0;
            }
            // sympathetic resonance: piano pedal bloom, and the acoustic
            // guitars' six open strings (guitar r3 — kernels::SympBank owns
            // the guitar retune; feedforward = unconditionally stable)
            match instrument {
                Instrument::Piano => {
                    if self.symp[track].guitar {
                        self.symp[track] = SympBank::new(self.sample_rate);
                    }
                    self.symp[track].enabled = true;
                }
                Instrument::Guitar | Instrument::GuitarSteel => {
                    if !self.symp[track].guitar {
                        self.symp[track].retune_guitar(self.sample_rate);
                    }
                    self.symp[track].enabled = true;
                }
                _ => self.symp[track].enabled = false,
            }
            // magnetic-pickup resonance (electrics): RBJ resonant lowpass
            let (pf, pq) = pickup_defaults(instrument);
            t.pk_on = pf > 0.0;
            if t.pk_on {
                let w = core::f32::consts::TAU * pf / sr;
                let (sw, cw) = w.sin_cos();
                let alpha = sw / (2.0 * pq);
                let a0 = 1.0 + alpha;
                t.pk_b0 = ((1.0 - cw) / 2.0) / a0;
                t.pk_b1 = (1.0 - cw) / a0;
                t.pk_b2 = t.pk_b0;
                t.pk_a1 = (-2.0 * cw) / a0;
                t.pk_a2 = (1.0 - alpha) / a0;
                t.pk_z1 = 0.0;
                t.pk_z2 = 0.0;
            }
        }
    }

    pub fn note_on(&mut self, track: usize, midi: u32, vel: f32) {
        if track >= MAX_TRACKS {
            return;
        }
        let inst = self.tracks[track].instrument;
        self.seed = self.seed.wrapping_mul(747796405).wrapping_add(2891336453);
        // hi-hat choke: a closed hat (42/44) physically clamps a ringing open hat (46)
        if inst.is_drum_kit() && (midi == 42 || midi == 44) {
            let sr = self.sample_rate;
            for v in self.voices.iter_mut() {
                if v.active() && v.track as usize == track && v.midi == 46 {
                    if let Kernel::Drum(d) = &mut v.kernel {
                        d.choke(sr);
                    }
                }
            }
        }
        // voice choice: retrigger same (track,pitch) > free slot > oldest (steal)
        let mut slot = None;
        for (i, v) in self.voices.iter().enumerate() {
            if v.active() && v.track as usize == track && v.midi as u32 == midi && !v.releasing {
                slot = Some(i);
                break;
            }
        }
        if slot.is_none() {
            slot = self.voices.iter().position(|v| !v.active());
        }
        // steal preference: an already-releasing (quiet, fading) voice before a
        // still-ringing one — cutting a live tail is the audible failure mode
        let slot = slot.unwrap_or_else(|| {
            self.voices
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| (v.releasing, v.age))
                .map(|(i, _)| i)
                .unwrap_or(0)
        });
        self.voices[slot] = Voice {
            kernel: start_voice(inst, midi, vel, self.sample_rate, self.seed),
            track: track as u8,
            midi: midi as u8,
            releasing: false,
            pedal_held: false,
            pan: voice_pan(inst, midi, self.seed),
            age: 0,
        };
    }

    pub fn note_off(&mut self, track: usize, midi: u32) {
        let inst = if track < MAX_TRACKS { self.tracks[track].instrument } else { return };
        // one-shot percussion ignores note-off; sustained/damped families release
        let damps = matches!(
            inst,
            Instrument::Vibraphone
                | Instrument::EPiano
                | Instrument::Guitar
                // steel was missing from this list — its notes NEVER released
                // (round-2 finding: refs choke/ring at note-off, renders sailed
                // through it; the release-transient work was inaudible)
                | Instrument::GuitarSteel
                | Instrument::Bass
                | Instrument::SynthPad
                | Instrument::Piano
                // electrics: fretted strings damp on release (NSynth refs decay at
                // t60 ≈ 0.3 s after note-off; see PluckVoice::damp electric branch)
                | Instrument::GuitarElectric
                | Instrument::GuitarDistorted
        );
        if !damps {
            return;
        }
        let pedal = self.tracks[track].pedal;
        let sr = self.sample_rate;
        for v in self.voices.iter_mut() {
            if v.active() && v.track as usize == track && v.midi as u32 == midi && !v.releasing {
                if pedal {
                    v.pedal_held = true; // defer the release until pedal-up
                } else {
                    v.releasing = true;
                    match &mut v.kernel {
                        Kernel::Modal(m) => m.damp(sr),
                        Kernel::Pluck(p) => p.damp(),
                        Kernel::EPluck(p) => p.damp(),
                        Kernel::Synth(s) => s.release(),
                        Kernel::Piano(p) => p.damp(),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Sustain pedal (CC64). Pedal-up releases every note whose note-off was deferred.
    pub fn set_reverb(&mut self, kind: u32) {
        self.reverb.configure(kind.min(REVERB_SPRING));
    }

    pub fn set_room(&mut self, track: usize, send: f32) {
        if track < MAX_TRACKS {
            self.tracks[track].room_send = send.clamp(0.0, 1.0);
        }
    }

    pub fn set_pedal(&mut self, track: usize, on: bool) {
        if track >= MAX_TRACKS {
            return;
        }
        self.tracks[track].pedal = on;
        if self.symp[track].enabled {
            self.symp[track].set_pedal(on);
        }
        if on {
            return;
        }
        let sr = self.sample_rate;
        for v in self.voices.iter_mut() {
            if v.active() && v.track as usize == track && v.pedal_held && !v.releasing {
                v.pedal_held = false;
                v.releasing = true;
                match &mut v.kernel {
                    Kernel::Modal(m) => m.damp(sr),
                    Kernel::Pluck(p) => p.damp(),
                    Kernel::EPluck(p) => p.damp(),
                    Kernel::Synth(s) => s.release(),
                    Kernel::Piano(p) => p.damp(),
                    _ => {}
                }
            }
        }
    }

    pub fn all_off(&mut self) {
        let sr = self.sample_rate;
        for v in self.voices.iter_mut() {
            if v.active() && !v.releasing {
                v.releasing = true;
                match &mut v.kernel {
                    Kernel::Modal(m) => m.damp(sr),
                    Kernel::Pluck(p) => p.damp(),
                    Kernel::EPluck(p) => p.damp(),
                    Kernel::Synth(s) => s.release(),
                    Kernel::Piano(p) => p.damp(),
                    Kernel::Drum(_) => {} // short one-shots; let them ring out
                    Kernel::Off => {}
                }
            }
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    /// Render one quantum for the whole arrangement into `out_l`/`out_r`.
    pub fn process(&mut self, frames: usize) {
        let frames = frames.min(MAX_BLOCK);
        self.out_l[..frames].fill(0.0);
        self.out_r[..frames].fill(0.0);
        self.room_in[..frames].fill(0.0);
        let sr = self.sample_rate;

        for t in 0..MAX_TRACKS {
            // render every voice on this track into the STEREO track bus:
            // each voice renders mono, then lands at its own equal-power position
            let mut any = false;
            self.track_l[..frames].fill(0.0);
            self.track_r[..frames].fill(0.0);
            for v in self.voices.iter_mut() {
                if !v.active() || v.track as usize != t {
                    continue;
                }
                any = true;
                self.voice_buf[..frames].fill(0.0);
                let alive = match &mut v.kernel {
                    Kernel::Modal(m) => m.render(&mut self.voice_buf[..frames]),
                    Kernel::Pluck(p) => p.render(&mut self.voice_buf[..frames]),
                    Kernel::EPluck(p) => p.render(&mut self.voice_buf[..frames]),
                    Kernel::Drum(d) => d.render(&mut self.voice_buf[..frames], sr),
                    Kernel::Synth(s) => s.render(&mut self.voice_buf[..frames]),
                    Kernel::Piano(pn) => pn.render(&mut self.voice_buf[..frames]),
                    Kernel::Off => false,
                };
                let th = (v.pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
                let (vgl, vgr) = (th.cos(), th.sin());
                for i in 0..frames {
                    let s = self.voice_buf[i];
                    self.track_l[i] += s * vgl;
                    self.track_r[i] += s * vgr;
                }
                v.age += frames as u64;
                if !alive {
                    v.kernel = Kernel::Off;
                }
            }
            // sympathetic resonance (pedal bloom): fed by the track's own sound,
            // returned to both channels; keeps ringing after the source damps
            if self.symp[t].enabled && (any || self.symp[t].ringing()) {
                let bank = &mut self.symp[t];
                // guitar open strings ring while the hand is playing and get
                // damped by the flesh when the last note dies (piano banks
                // keep their pedal-driven state)
                if bank.guitar {
                    bank.set_pedal(any);
                }
                for i in 0..frames {
                    let m = (self.track_l[i] + self.track_r[i]) * 0.5;
                    let res = bank.tick(m) * core::f32::consts::FRAC_1_SQRT_2;
                    self.track_l[i] += res;
                    self.track_r[i] += res;
                }
                bank.flush();
                if bank.ringing() {
                    any = true;
                }
            }
            let (tl, tr) = self.tracks[t].targets();
            let bus = &mut self.tracks[t];
            if !any && bus.gl.abs() < 1e-6 && bus.gr.abs() < 1e-6 {
                bus.gl = tl;
                bus.gr = tr;
                continue;
            }
            // body resonator bank (acoustics): parallel modes + dry path, true
            // stereo (independent state per channel — a body radiates in space)
            if bus.body_n > 0 {
                for ch in 0..2 {
                    let buf: &mut [f32] =
                        if ch == 0 { &mut self.track_l } else { &mut self.track_r };
                    for i in 0..frames {
                        let x = buf[i];
                        let mut acc = bus.body_dry * x;
                        for m in 0..bus.body_n {
                            let y = bus.body_a1[m] * bus.body_y1[ch][m]
                                - bus.body_r2[m] * bus.body_y2[ch][m]
                                + bus.body_g[m] * x;
                            bus.body_y2[ch][m] = bus.body_y1[ch][m];
                            bus.body_y1[ch][m] = y;
                            acc += y;
                        }
                        buf[i] = acc;
                    }
                    for m in 0..bus.body_n {
                        bus.body_y1[ch][m] = flush_denormal(bus.body_y1[ch][m]);
                        bus.body_y2[ch][m] = flush_denormal(bus.body_y2[ch][m]);
                    }
                }
            }
            // Electrics are genuinely mono instruments (one pickup, one amp):
            // collapse the spread (their voices pan 0 anyway), run the electrical
            // chain once, and mirror the result to both channels. The 0.7071
            // collapse is exact for center voices: (s/√2 + s/√2)·(1/√2) = s.
            if bus.pk_on || bus.drive > 0.0 {
                for i in 0..frames {
                    let m = (self.track_l[i] + self.track_r[i]) * core::f32::consts::FRAC_1_SQRT_2;
                    self.track_l[i] = m;
                }
                // magnetic-pickup resonance, before the amp
                if bus.pk_on {
                    for i in 0..frames {
                        let x = self.track_l[i];
                        let y = bus.pk_b0 * x + bus.pk_z1;
                        bus.pk_z1 = bus.pk_b1 * x - bus.pk_a1 * y + bus.pk_z2;
                        bus.pk_z2 = bus.pk_b2 * x - bus.pk_a2 * y;
                        self.track_l[i] = y;
                    }
                    bus.pk_z1 = flush_denormal(bus.pk_z1);
                    bus.pk_z2 = flush_denormal(bus.pk_z2);
                }
                if bus.wdf_on {
                    // WDF circuit-sim amp: 12AX7 triode root (bounded Newton) +
                    // Fender TMB tone stack + supply-rail sag. Replaces the
                    // gain-ride + ADAA-tanh + tone lowpass below; the post cab
                    // EQ is still applied. Sag/bias-shift emerge from the
                    // rectifier/RC supply and cathode self-bias, not an envelope
                    // follower (design doc 2026-07-13).
                    bus.wdf_amp.process(&mut self.track_l[..frames]);
                } else {
                    // --- behavioral chain (DEFAULT) ---
                    // amp gain-ride: supply-rail recovery / bias-shift compression
                    // (see TrackBus fields). Pre-drive so the rising gain re-feeds
                    // the tanh limiter — drive-sustain, the "amplifier factor".
                    if bus.ride_thr > 0.0 {
                        for i in 0..frames {
                            let x = self.track_l[i];
                            let a = x.abs();
                            let c = if a > bus.ride_env { bus.ride_env_a } else { bus.ride_env_r };
                            bus.ride_env += c * (a - bus.ride_env);
                            let t = bus.ride_thr / bus.ride_env.max(1e-5);
                            let gt = if t > 1.0 { t.powf(bus.ride_p).min(bus.ride_cap) } else { 1.0 };
                            let cg = if gt < bus.ride_g { bus.ride_dn } else { bus.ride_up };
                            bus.ride_g += cg * (gt - bus.ride_g);
                            self.track_l[i] = x * bus.ride_g;
                        }
                        bus.ride_env = flush_denormal(bus.ride_env);
                        // ride_g rests at cap (>= 1), never denormal
                    }
                    // amp stage: ADAA tanh + tone/cab lowpass
                    if bus.drive > 0.0 {
                        let d = bus.drive;
                        let inv_d = 1.0 / d;
                        for i in 0..frames {
                            let x = self.track_l[i] * d;
                            let dx = x - bus.amp_x1;
                            let fx = ln_cosh(x);
                            let y = if dx.abs() > 1e-4 {
                                (fx - bus.amp_f1) / dx
                            } else {
                                (0.5 * (x + bus.amp_x1)).tanh()
                            };
                            bus.amp_x1 = x;
                            bus.amp_f1 = fx;
                            bus.tone_lp += bus.tone_c * (y - bus.tone_lp);
                            self.track_l[i] = bus.tone_lp * inv_d.max(0.35);
                        }
                        bus.amp_x1 = flush_denormal(bus.amp_x1);
                        bus.tone_lp = flush_denormal(bus.tone_lp);
                    }
                }
                // post-drive cab/presence EQ (see TrackBus fields): transposed
                // DF2 peaking sections — clip-generated presence + LF cab bump
                if bus.pe_on {
                    for k in 0..2 {
                        let b = bus.pe_b[k];
                        let a = bus.pe_a[k];
                        for i in 0..frames {
                            let x = self.track_l[i];
                            let y = b[0] * x + bus.pe_z[k][0];
                            bus.pe_z[k][0] = b[1] * x - a[0] * y + bus.pe_z[k][1];
                            bus.pe_z[k][1] = b[2] * x - a[1] * y;
                            self.track_l[i] = y;
                        }
                        bus.pe_z[k][0] = flush_denormal(bus.pe_z[k][0]);
                        bus.pe_z[k][1] = flush_denormal(bus.pe_z[k][1]);
                    }
                }
                // mirror mono chain to both channels at equal power
                for i in 0..frames {
                    let m = self.track_l[i] * core::f32::consts::FRAC_1_SQRT_2;
                    self.track_l[i] = m;
                    self.track_r[i] = m;
                }
            }
            // ~1 ms gain smoothing against zipper noise
            let c = 1.0 - (-1.0 / (0.001 * sr)).exp();
            let send = bus.room_send;
            for i in 0..frames {
                bus.gl += c * (tl - bus.gl);
                bus.gr += c * (tr - bus.gr);
                let l = self.track_l[i] * bus.gl;
                let r = self.track_r[i] * bus.gr;
                self.out_l[i] += l;
                self.out_r[i] += r;
                self.room_in[i] += (l + r) * 0.5 * send;
            }
            bus.gl = flush_denormal(bus.gl);
            bus.gr = flush_denormal(bus.gr);
        }

        // shared room stage: wet added on top of the dry buses (see Room)
        self.reverb
            .process(&self.room_in, &mut self.out_l, &mut self.out_r, frames);

        // master safety: transparent below ~-12 dBFS, soft-limits above
        for i in 0..frames {
            self.out_l[i] = soft_clip(self.out_l[i]);
            self.out_r[i] = soft_clip(self.out_r[i]);
        }
    }
}

/// Master safety limiter: exactly linear below the knee, then a tanh that is
/// value- AND slope-continuous at the knee (no step, no grit), asymptote 1.0.
#[inline(always)]
fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.6;
    const REST: f32 = 1.0 - KNEE;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let y = KNEE + REST * ((a - KNEE) / REST).tanh();
        y.copysign(x)
    }
}

// ---------------------------------------------------------------------------
// WASM ABI — hand-rolled minimal C ABI (no bindgen glue; see architecture doc).
// The only unsafe in the crate lives here, at the FFI boundary.
// ---------------------------------------------------------------------------
#[allow(unsafe_code)]
pub mod ffi {
    use super::*;

    fn engine<'a>(p: *mut Engine) -> Option<&'a mut Engine> {
        #[allow(unsafe_code)]
        unsafe {
            p.as_mut()
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_engine_new(sample_rate: f32) -> *mut Engine {
        Box::into_raw(Box::new(Engine::new(sample_rate)))
    }

    #[no_mangle]
    pub extern "C" fn ij_engine_free(p: *mut Engine) {
        if !p.is_null() {
            #[allow(unsafe_code)]
            unsafe {
                drop(Box::from_raw(p));
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_set_track(p: *mut Engine, track: u32, inst: u32, gain: f32, pan: f32) {
        if let Some(e) = engine(p) {
            e.set_track(track as usize, Instrument::from_u32(inst), gain, pan);
        }
    }

    /// Select the shared reverb voicing: 0 off, 1 room (default), 2 hall,
    /// 3 plate, 4 spring. Control-path only; allocation-free.
    #[no_mangle]
    pub extern "C" fn ij_set_reverb(p: *mut Engine, kind: u32) {
        if let Some(e) = engine(p) {
            e.set_reverb(kind);
        }
    }

    /// Override the track's room send (default = kernels::room_send for the
    /// family; set after ij_set_track, which resets it). send is clamped 0..1.
    #[no_mangle]
    pub extern "C" fn ij_set_room(p: *mut Engine, track: u32, send: f32) {
        if let Some(e) = engine(p) {
            e.set_room(track as usize, send);
        }
    }

    /// Engine-internal selector for the WDF circuit-sim amp path (design doc
    /// 2026-07-13). Default is the behavioral chain (kernels::WDF_AMP_DEFAULT =
    /// false); this override lets the audition/A-B renders flip a track to the
    /// WDF path without changing the default. No-op on non-voiced instruments.
    /// Call after ij_set_track (which resets the flag).
    #[no_mangle]
    pub extern "C" fn ij_set_amp_wdf(p: *mut Engine, track: u32, on: u32) {
        if let Some(e) = engine(p) {
            let t = track as usize;
            if t < MAX_TRACKS {
                e.tracks[t].wdf_on = on != 0 && e.tracks[t].wdf_voiced;
            }
        }
    }

    /// WDF amp tuning FFI (scratchpad loop only — lets the render harness sweep
    /// the voicing without recompiling). Patches the current track's WDF config
    /// and re-prepares. Not used by the shipped presets.
    #[no_mangle]
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn ij_wdf_tune(
        p: *mut Engine,
        track: u32,
        drive: f32,
        out_scale: f32,
        load_k: f32,
        rsup: f32,
        csup_uf: f32,
        tone_t: f32,
        tone_l: f32,
        tone_m: f32,
    ) {
        if let Some(e) = engine(p) {
            let t = track as usize;
            if t < MAX_TRACKS && e.tracks[t].wdf_voiced {
                let sr = e.sample_rate;
                let mut cfg = e.tracks[t].wdf_amp.config();
                cfg.drive_v = drive as f64;
                cfg.out_scale = out_scale as f64;
                cfg.load_k = load_k as f64;
                cfg.supply_rc = (rsup as f64, csup_uf as f64 * 1e-6);
                cfg.tone = (tone_t as f64, tone_l as f64, tone_m as f64);
                e.tracks[t].wdf_amp.set_config(cfg, sr);
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_note_on(p: *mut Engine, track: u32, midi: u32, vel: f32) {
        if let Some(e) = engine(p) {
            e.note_on(track as usize, midi, vel);
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_note_off(p: *mut Engine, track: u32, midi: u32) {
        if let Some(e) = engine(p) {
            e.note_off(track as usize, midi);
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_pedal(p: *mut Engine, track: u32, on: u32) {
        if let Some(e) = engine(p) {
            e.set_pedal(track as usize, on != 0);
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_all_off(p: *mut Engine) {
        if let Some(e) = engine(p) {
            e.all_off();
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_process(p: *mut Engine, frames: u32) {
        if let Some(e) = engine(p) {
            e.process(frames as usize);
        }
    }

    #[no_mangle]
    pub extern "C" fn ij_out_l(p: *mut Engine) -> *const f32 {
        engine(p).map(|e| e.out_l.as_ptr()).unwrap_or(core::ptr::null())
    }

    #[no_mangle]
    pub extern "C" fn ij_out_r(p: *mut Engine) -> *const f32 {
        engine(p).map(|e| e.out_r.as_ptr()).unwrap_or(core::ptr::null())
    }

    #[no_mangle]
    pub extern "C" fn ij_active_voices(p: *mut Engine) -> u32 {
        engine(p).map(|e| e.active_voices() as u32).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_seconds(e: &mut Engine, secs: f32) -> Vec<f32> {
        let total = (secs * e.sample_rate) as usize;
        let mut out = Vec::with_capacity(total);
        let mut done = 0;
        while done < total {
            let n = QUANTUM_FRAMES.min(total - done);
            e.process(n);
            out.extend_from_slice(&e.out_l[..n]);
            done += n;
        }
        out
    }

    fn zero_crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0)).count()
    }

    /// Autocorrelation pitch estimate (panel finding: zero-crossing counting is
    /// fooled by partial-rich tails). Searches lags for `lo..hi` Hz, parabolic
    /// refinement around the peak.
    fn estimate_pitch(x: &[f32], sr: f32, lo: f32, hi: f32) -> f32 {
        let min_lag = (sr / hi) as usize;
        let max_lag = ((sr / lo) as usize).min(x.len() / 2);
        let mut best = f32::NEG_INFINITY;
        let n = x.len() - max_lag;
        let energy: f32 = x[..n].iter().map(|s| s * s).sum::<f32>().max(1e-12);
        let mut scores = Vec::with_capacity(max_lag - min_lag + 1);
        for lag in min_lag..=max_lag {
            let mut acc = 0.0f32;
            for i in 0..n {
                acc += x[i] * x[i + lag];
            }
            let score = acc / energy;
            scores.push(score);
            if score > best {
                best = score;
            }
        }
        // Octave-error guard: a periodic signal scores ~equally at T and 2T, and
        // slow beating (detuned string pairs) can nudge 2T above T. Take the
        // smallest-lag LOCAL MAXIMUM within 7% of the global best (standard ACF
        // practice); fall back to the global max if none qualifies.
        let mut best_lag = min_lag
            + scores
                .iter()
                .position(|&s| s >= best)
                .unwrap_or(0);
        for (i, &s) in scores.iter().enumerate().skip(1).take(scores.len().saturating_sub(2)) {
            if s >= 0.93 * best && s >= scores[i - 1] && s >= scores[i + 1] {
                best_lag = min_lag + i;
                break;
            }
        }
        // parabolic interpolation for sub-sample lag
        let corr = |lag: usize| -> f32 { (0..n).map(|i| x[i] * x[i + lag]).sum() };
        let (a, b, c) = (corr(best_lag - 1), corr(best_lag), corr(best_lag + 1));
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
        sr / (best_lag as f32 + delta.clamp(-0.5, 0.5))
    }

    /// Spectral-peak pitch estimate (Goertzel scan + parabolic refinement).
    /// The autocorrelation estimator collapses on near-sinusoidal tails (its peak
    /// region goes flat and noise picks the lag — observed 2026-07-11 when the piano's
    /// restored fundamental left a ~pure 440 Hz tail and ACF reported 450 Hz while the
    /// FFT peak sat at 440.08). A windowed spectral peak has no such failure mode.
    fn spectral_pitch(x: &[f32], sr: f32, lo: f32, hi: f32) -> f32 {
        let n = x.len();
        let hann = |i: usize| 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n as f32).cos();
        let power = |f: f32| -> f32 {
            // Goertzel with Hann window
            let w = core::f32::consts::TAU * f / sr;
            let c = 2.0 * w.cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for (i, &v) in x.iter().enumerate() {
                let s0 = v * hann(i) + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            s1 * s1 + s2 * s2 - c * s1 * s2
        };
        let step = 1.0f32;
        let mut best_f = lo;
        let mut best = f32::NEG_INFINITY;
        let mut f = lo;
        while f <= hi {
            let p = power(f);
            if p > best {
                best = p;
                best_f = f;
            }
            f += step;
        }
        let (a, b, c) = (power(best_f - step), best, power(best_f + step));
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
        best_f + delta.clamp(-0.5, 0.5) * step
    }

    #[test]
    fn denormals_are_flushed() {
        assert_eq!(flush_denormal(1.0e-30), 0.0);
        assert_eq!(flush_denormal(-1.0e-30), 0.0);
        assert_eq!(flush_denormal(0.5), 0.5);
    }

    #[test]
    fn silence_without_notes() {
        let mut e = Engine::new(48_000.0);
        let out = render_seconds(&mut e, 0.05);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn note_produces_finite_bounded_audio() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Marimba, 0.8, 0.0);
        e.note_on(0, 69, 1.0);
        let out = render_seconds(&mut e, 0.5);
        let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(out.iter().all(|s| s.is_finite()), "NaN/inf in output");
        assert!(peak > 0.01, "inaudibly quiet: peak={peak}");
        assert!(peak <= 1.0, "clipping escape: peak={peak}");
    }

    #[test]
    fn vibraphone_a4_is_in_tune_at_both_sample_rates() {
        for sr in [44_100.0f32, 48_000.0f32] {
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::Vibraphone, 0.8, 0.0);
            e.note_on(0, 69, 0.9); // A4 = 440 Hz
            let out = render_seconds(&mut e, 0.6);
            // after the attack, the fast-decaying upper partials are gone
            let tail = &out[(0.2 * sr) as usize..(0.5 * sr) as usize];
            let f_est = zero_crossings(tail) as f32 / 2.0 / 0.3;
            assert!(
                (f_est - 440.0).abs() < 440.0 * 0.03,
                "sr={sr}: estimated {f_est} Hz, want 440"
            );
        }
    }

    #[test]
    fn guitar_a3_is_in_tune_at_both_sample_rates() {
        // Pluck tuning is SR-dependent (delay length + fractional allpass) and iOS
        // locks contexts to 44.1 kHz — both rates must be verified.
        for sr in [44_100.0f32, 48_000.0f32] {
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::Guitar, 0.8, 0.0);
            e.note_on(0, 57, 0.8); // A3 = 220 Hz
            let out = render_seconds(&mut e, 0.6);
            let tail = &out[(0.25 * sr) as usize..(0.55 * sr) as usize];
            // autocorrelation, not zero crossings: the reference-matched guitar
            // has h2 ≥ h1 in the tail (as the NSynth references do), which
            // doubles a zero-crossing count (same panel finding as the piano)
            let f_est = estimate_pitch(tail, sr, 100.0, 500.0);
            assert!((f_est - 220.0).abs() < 220.0 * 0.02, "sr={sr}: estimated {f_est} Hz, want 220");
        }
    }

    #[test]
    fn pluck_tuning_is_velocity_independent() {
        // The loop-lowpass phase delay depends on velocity→brightness; without
        // compensation the string detunes with velocity (Juhan panel finding).
        let mut ests = Vec::new();
        for vel in [0.3f32, 0.9f32] {
            let mut e = Engine::new(48_000.0);
            e.set_track(0, Instrument::Guitar, 0.8, 0.0);
            e.note_on(0, 69, vel); // A4 = 440 Hz — short period exposes the error
            let out = render_seconds(&mut e, 0.6);
            let tail = &out[(0.25 * 48_000.0) as usize..(0.55 * 48_000.0) as usize];
            let f_est = estimate_pitch(tail, 48_000.0, 200.0, 900.0);
            assert!(
                (f_est - 440.0).abs() < 440.0 * 0.02,
                "vel={vel}: estimated {f_est} Hz, want 440"
            );
            ests.push(f_est);
        }
        let spread = (ests[0] - ests[1]).abs() / 440.0;
        assert!(spread < 0.01, "tuning drifts {:.2}% between velocities", spread * 100.0);
    }

    #[test]
    fn note_off_damps_guitar() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Guitar, 0.8, 0.0);
        e.note_on(0, 57, 0.9);
        let _ = render_seconds(&mut e, 0.2);
        e.note_off(0, 57);
        let out = render_seconds(&mut e, 0.4);
        let late = &out[(0.3 * 48_000.0) as usize..];
        let peak = late.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 0.01, "string still ringing after damp: {peak}");
    }

    #[test]
    fn piano_is_in_tune_at_both_sample_rates_and_velocities() {
        // dispersion + loop-lowpass delays are both compensated in the string length;
        // any regression shows up as pitch drift across sr or velocity
        for sr in [44_100.0f32, 48_000.0f32] {
            for vel in [0.3f32, 0.9f32] {
                let mut e = Engine::new(sr);
                e.set_track(0, Instrument::Piano, 0.8, 0.0);
                e.note_on(0, 69, vel); // A4
                let out = render_seconds(&mut e, 0.7);
                let tail = &out[(0.3 * sr) as usize..(0.6 * sr) as usize];
                let f_est = spectral_pitch(tail, sr, 400.0, 480.0);
                assert!(
                    (f_est - 440.0).abs() < 440.0 * 0.006,
                    "sr={sr} vel={vel}: estimated {f_est} Hz, want 440"
                );
            }
            // Bass register: the in-loop DC blocker's phase lead grows as f0 falls
            // (uncompensated it left A1 audibly sharp); A2 = 110 Hz guards the fix.
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::Piano, 0.8, 0.0);
            e.note_on(0, 45, 0.7);
            let out = render_seconds(&mut e, 0.7);
            let tail = &out[(0.3 * sr) as usize..(0.6 * sr) as usize];
            let f_est = spectral_pitch(tail, sr, 95.0, 125.0);
            assert!(
                (f_est - 110.0).abs() < 110.0 * 0.006,
                "sr={sr} A2: estimated {f_est} Hz, want 110"
            );
        }
    }

    #[test]
    fn piano_sings_then_damps() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Piano, 0.8, 0.0);
        e.note_on(0, 48, 0.8); // C3 — long-decay register
        let held = render_seconds(&mut e, 2.0);
        let late_held = &held[(1.8 * 48_000.0) as usize..];
        let sing = late_held.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(sing > 0.003, "piano C3 died too fast while held: {sing}");
        e.note_off(0, 48);
        let out = render_seconds(&mut e, 0.8);
        let late = &out[(0.7 * 48_000.0) as usize..];
        let peak = late.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 0.01, "damper failed: {peak}");
    }

    #[test]
    fn piano_bass_and_treble_render_clean() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Piano, 0.8, 0.0);
        e.note_on(0, 21, 1.0); // A0 — longest string, most dispersion
        e.note_on(0, 105, 1.0); // A7 — shortest string
        let out = render_seconds(&mut e, 1.0);
        let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(peak > 0.01 && peak <= 1.0, "extremes peak={peak}");
    }

    #[test]
    fn electric_guitars_stay_in_tune_through_the_amp() {
        // the amp stage is an odd monotonic shaper — pitch must survive it.
        // Both sample rates: the electric voice has its own delay math (two
        // polarization loops + loop-lowpass compensation) and iOS locks 44.1 kHz.
        for sr in [44_100.0f32, 48_000.0f32] {
            for inst in
                [Instrument::GuitarSteel, Instrument::GuitarElectric, Instrument::GuitarDistorted]
            {
                let mut e = Engine::new(sr);
                e.set_track(0, inst, 0.8, 0.0);
                e.note_on(0, 57, 0.8); // A3
                let out = render_seconds(&mut e, 0.7);
                let tail = &out[(0.3 * sr) as usize..(0.6 * sr) as usize];
                // Focus the estimator on the fundamental region: steel's
                // stiffness-stretched upper partials (round 2) ring on the HF
                // loss floor and bias a raw ACF sharp by tens of cents while
                // the true f0 stays within ±3 cents (verified by DFT peak fit
                // 2026-07-11; same estimator-vs-partials lesson as the piano).
                let k = 1.0 - (-core::f32::consts::TAU * 550.0 / sr).exp();
                let mut lp = 0.0f32;
                let filtered: Vec<f32> = tail
                    .iter()
                    .map(|&s| {
                        lp += k * (s - lp);
                        lp
                    })
                    .collect();
                let f_est = estimate_pitch(&filtered, sr, 100.0, 500.0);
                assert!(out.iter().all(|s| s.is_finite()));
                assert!(
                    (f_est - 220.0).abs() < 220.0 * 0.02,
                    "{inst:?} sr={sr}: estimated {f_est} Hz, want 220"
                );
            }
        }
    }

    #[test]
    fn distorted_chord_is_bounded_and_alive() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::GuitarDistorted, 0.9, 0.0);
        // power chord — the intermodulation-in-the-amp case
        e.note_on(0, 40, 1.0);
        e.note_on(0, 47, 1.0);
        e.note_on(0, 52, 1.0);
        let out = render_seconds(&mut e, 1.0);
        let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(out.iter().all(|s| s.is_finite()), "NaN through the amp");
        // 0.035 floor: since the 2026-07-11 LUFS recalibration the distorted bus
        // sits at marimba loudness (-26 LUFS); a sustained saturated signal at
        // that loudness peaks near 0.05 at gain 0.8 — the old 0.05 floor encoded
        // the earlier, 8-LU-hot calibration.
        assert!(peak > 0.035 && peak <= 1.0, "distorted chord peak={peak}");
        let late = &out[(0.9 * 48_000.0) as usize..];
        let sustain = late.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(sustain > 0.02, "distorted guitar should sing: {sustain}");
    }

    #[test]
    fn electric_note_blooms_two_slope_decay() {
        // Round-2 amp life: supply-rail sag + slow polarization must produce the
        // refs' two-slope envelope — fast early decay, near-flat late plateau.
        let sr = 48_000.0;
        let mut e = Engine::new(sr);
        e.set_track(0, Instrument::GuitarElectric, 0.9, 0.0);
        e.note_on(0, 40, 0.8); // E2
        let out = render_seconds(&mut e, 3.0);
        let rms = |a: f32, b: f32| {
            let s = &out[(a * sr) as usize..(b * sr) as usize];
            (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt().max(1e-9)
        };
        // dB/s over an early window vs a late window
        let early = 20.0 * (rms(0.2, 0.45) / rms(0.7, 0.95)).log10() / 0.5;
        let late = 20.0 * (rms(1.6, 1.85) / rms(2.6, 2.85)).log10() / 1.0;
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            early > late * 1.3 && late < 12.0,
            "no bloom: early {early:.1} dB/s late {late:.1} dB/s"
        );
    }

    #[test]
    fn electric_amp_ride_flattens_clean_sustain() {
        // Amp round (owner 2026-07-12: "notes must remain longer"): the bus
        // gain-ride (rail-recovery compression) must hold the singing region.
        // Pre-ride the 0.5→2.5 s envelope dropped ~16 dB (string rate); with
        // the 3:1 ride it must stay under 12 dB. Both rates: the ride is
        // coefficient-computed from sr.
        for sr in [44_100.0f32, 48_000.0f32] {
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::GuitarElectric, 0.9, 0.0);
            e.note_on(0, 45, 0.6);
            let out = render_seconds(&mut e, 2.8);
            let rms = |a: f32, b: f32| {
                let s = &out[(a * sr) as usize..(b * sr) as usize];
                (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt().max(1e-9)
            };
            let drop = 20.0 * (rms(0.5, 0.7) / rms(2.5, 2.7)).log10();
            assert!(out.iter().all(|s| s.is_finite()));
            assert!(drop < 12.0, "sr={sr}: clean sustain fell {drop:.1} dB over 0.5-2.5 s");
        }
    }

    #[test]
    fn distorted_amp_holds_output_for_seconds() {
        // Drive-sustain: the ride re-feeds the tanh limiter, so a single held
        // note stays within ~3 dB of its sustained level for seconds (FreePats
        // refs hold 3.6-22.6 s; pre-ride we fell out after ~1.1 s at mp).
        for sr in [44_100.0f32, 48_000.0f32] {
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::GuitarDistorted, 0.9, 0.0);
            e.note_on(0, 45, 0.6);
            let out = render_seconds(&mut e, 4.0);
            let rms = |a: f32, b: f32| {
                let s = &out[(a * sr) as usize..(b * sr) as usize];
                (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt().max(1e-9)
            };
            let drop = 20.0 * (rms(0.3, 0.5) / rms(3.7, 3.9)).log10();
            assert!(out.iter().all(|s| s.is_finite()));
            assert!(drop.abs() < 3.0, "sr={sr}: distorted hold broke: {drop:.1} dB 0.3->3.7 s");
        }
    }

    #[test]
    fn electric_release_squeak_bounded_and_terminates() {
        // Fret-release noise: bursts at note-off on the drive channel, stays
        // bounded, and the voice still terminates promptly at both rates.
        for sr in [44_100.0f32, 48_000.0f32] {
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::GuitarDistorted, 0.9, 0.0);
            e.note_on(0, 45, 1.0);
            let _ = render_seconds(&mut e, 1.0);
            e.note_off(0, 45);
            let out = render_seconds(&mut e, 1.2);
            assert!(out.iter().all(|s| s.is_finite()), "sr={sr}: NaN in release");
            let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            assert!(peak <= 1.0, "sr={sr}: release burst clipped: {peak}");
            let tail = &out[(1.0 * sr) as usize..];
            let tail_peak = tail.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            assert!(tail_peak < 1e-3, "sr={sr}: voice failed to die: {tail_peak}");
        }
    }

    #[test]
    fn sustain_pedal_defers_release_until_pedal_up() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Guitar, 0.8, 0.0);
        e.set_pedal(0, true);
        e.note_on(0, 57, 0.9);
        let _ = render_seconds(&mut e, 0.1);
        e.note_off(0, 57); // pedal is down — must keep ringing
        let held = render_seconds(&mut e, 0.4);
        let held_peak = held[(0.3 * 48_000.0) as usize..]
            .iter()
            .fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(held_peak > 0.005, "pedal failed to hold the note: {held_peak}");
        e.set_pedal(0, false); // pedal-up releases the deferred note-off
        let out = render_seconds(&mut e, 0.4);
        let late = &out[(0.3 * 48_000.0) as usize..];
        let peak = late.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 0.01, "string still ringing after pedal-up: {peak}");
    }

    #[test]
    fn synth_pad_sustains_and_releases() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::SynthPad, 0.8, 0.0);
        e.note_on(0, 60, 0.8);
        let held = render_seconds(&mut e, 1.0);
        let sustain = &held[(0.8 * 48_000.0) as usize..];
        let sus_peak = sustain.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(sus_peak > 0.005, "pad died while held: {sus_peak}");
        e.note_off(0, 60);
        let out = render_seconds(&mut e, 3.5);
        let late = &out[(3.3 * 48_000.0) as usize..];
        let peak = late.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 1e-3, "pad still sounding 3.3 s after release: {peak}");
        assert_eq!(e.active_voices(), 0, "released pad voice not reclaimed");
    }

    #[test]
    fn guitar_symp_halo_rings_then_hand_damps_it() {
        // The open-string bank rings while the track has live voices and is
        // choked (palm damp) once the last voice dies; by 2.5 s after that,
        // the track must be essentially silent (this is exactly what the
        // isolated-note references show).
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Guitar, 0.9, 0.0);
        e.note_on(0, 52, 0.9); // E3: partials coincide with open E2/E4
        let mut render = |secs: f32, e: &mut Engine| {
            let mut peak = 0.0f32;
            let total = (secs * 48_000.0) as usize;
            let mut done = 0;
            while done < total {
                let n = QUANTUM_FRAMES.min(total - done);
                e.process(n);
                for i in 0..n {
                    peak = peak.max(e.out_l[i].abs());
                }
                done += n;
            }
            peak
        };
        render(1.0, &mut e);
        e.note_off(0, 52);
        // during the release the voice is alive, the bank open: halo present
        let halo = render(0.4, &mut e);
        assert!(halo > 1e-6, "no sympathetic halo during release: {halo}");
        render(2.5, &mut e);
        let damped = render(0.5, &mut e);
        assert!(
            damped < halo * 0.05,
            "open strings not hand-damped after the voice died: {halo} -> {damped}"
        );
    }

    #[test]
    fn piano_symp_bank_restored_after_guitar_switch() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Guitar, 0.9, 0.0);
        e.set_track(0, Instrument::Piano, 0.9, 0.0);
        // the bank must be back on piano tuning/behavior: pedal bloom works
        e.set_pedal(0, true);
        e.note_on(0, 57, 0.9);
        for _ in 0..400 {
            e.process(QUANTUM_FRAMES);
        }
        e.note_off(0, 57);
        let mut peak = 0.0f32;
        for _ in 0..40 {
            e.process(QUANTUM_FRAMES);
            for i in 0..QUANTUM_FRAMES {
                peak = peak.max(e.out_l[i].abs());
            }
        }
        assert!(peak > 1e-4, "pedal bloom lost after guitar->piano switch: {peak}");
    }

    #[test]
    fn sympathetic_bank_blooms_with_pedal_and_dies_without() {
        use kernels::SympBank;
        let sr = 48_000.0;
        let mut b = SympBank::new(sr);
        b.enabled = true;
        b.set_pedal(true);
        // excite with a decaying burst (stands in for a struck chord)
        let mut env = 1.0f32;
        let mut rng = 0x1234u32;
        for _ in 0..(0.1 * sr) as usize {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (rng >> 9) as f32 * (2.0 / 8388608.0) - 1.0;
            b.tick(n * env * 0.3);
            env *= 0.9995;
        }
        // source silent now: the bank must keep singing (this IS the bloom)
        let mut e_ring = 0.0f64;
        for _ in 0..(0.5 * sr) as usize {
            let y = b.tick(0.0);
            e_ring += (y as f64) * (y as f64);
        }
        assert!(e_ring > 1e-4, "no sympathetic ring after excitation: {e_ring}");
        // pedal up: dampers fall, ring collapses fast
        b.set_pedal(false);
        for _ in 0..(0.5 * sr) as usize {
            b.tick(0.0);
        }
        let mut e_dead = 0.0f64;
        for _ in 0..(0.2 * sr) as usize {
            let y = b.tick(0.0);
            e_dead += (y as f64) * (y as f64);
        }
        assert!(e_dead < e_ring * 1e-3, "dampers failed: ring={e_ring} dead={e_dead}");
    }

    #[test]
    fn stereo_placement_spreads_piano_and_kit() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Piano, 0.8, 0.0);
        e.set_track(1, Instrument::Drums, 0.8, 0.0);
        e.note_on(0, 26, 0.9); // low D1 — audience left
        e.note_on(1, 42, 0.9); // closed hat — audience right
        let (mut el, mut er) = (0.0f64, 0.0f64);
        for _ in 0..150 {
            e.process(QUANTUM_FRAMES);
            for i in 0..QUANTUM_FRAMES {
                el += (e.out_l[i] as f64) * (e.out_l[i] as f64);
                er += (e.out_r[i] as f64) * (e.out_r[i] as f64);
            }
        }
        // both channels carry energy (nothing collapsed to mono-silence)…
        assert!(el > 1e-6 && er > 1e-6, "dead channel: L={el} R={er}");
        // …and a lone low piano note creates measurable left asymmetry:
        let mut e2 = Engine::new(48_000.0);
        e2.set_track(0, Instrument::Piano, 0.8, 0.0);
        e2.note_on(0, 26, 0.9);
        let (mut l2, mut r2) = (0.0f64, 0.0f64);
        for _ in 0..150 {
            e2.process(QUANTUM_FRAMES);
            for i in 0..QUANTUM_FRAMES {
                l2 += (e2.out_l[i] as f64) * (e2.out_l[i] as f64);
                r2 += (e2.out_r[i] as f64) * (e2.out_r[i] as f64);
            }
        }
        assert!(l2 > r2 * 1.3, "low piano note should sit audience-left: L={l2} R={r2}");
    }

    /// Choke regression (round-2 mandate): a closed hat (GM 42) must kill a
    /// still-ringing open hat (GM 46) on the SAME track — on every kit. The
    /// choked engine is compared against a control whose open hat rings on,
    /// in a window late enough that the closed hat's own tail has died.
    #[test]
    fn closed_hat_chokes_ringing_open_hat_on_every_kit() {
        let rms = |x: &[f32]| {
            (x.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / x.len() as f64).sqrt()
        };
        for inst in [Instrument::Drums, Instrument::DrumsRock, Instrument::DrumsJazz] {
            let mut e = Engine::new(48_000.0);
            e.set_track(0, inst, 0.8, 0.0);
            e.note_on(0, 46, 0.9);
            render_seconds(&mut e, 0.4);
            e.note_on(0, 42, 0.6); // pedal comes down
            render_seconds(&mut e, 0.8); // closed-hat tail dies too
            let choked = rms(&render_seconds(&mut e, 0.3));

            let mut c = Engine::new(48_000.0);
            c.set_track(0, inst, 0.8, 0.0);
            c.note_on(0, 46, 0.9);
            render_seconds(&mut c, 1.2);
            let ringing = rms(&render_seconds(&mut c, 0.3));
            assert!(
                ringing > 3.0 * choked.max(1e-9),
                "{inst:?}: choke failed — ringing {ringing} vs choked {choked}"
            );
        }
    }

    #[test]
    fn voice_stealing_never_exceeds_pool() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Marimba, 0.8, 0.0);
        for i in 0..(MAX_VOICES as u32 + 40) {
            e.note_on(0, 40 + (i % 60), 0.7);
        }
        assert!(e.active_voices() <= MAX_VOICES);
        let out = render_seconds(&mut e, 0.1);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn multitrack_arrangement_renders() {
        let mut e = Engine::new(48_000.0);
        e.set_track(0, Instrument::Marimba, 0.7, -0.3);
        e.set_track(1, Instrument::Bass, 0.8, 0.0);
        e.set_track(2, Instrument::Drums, 0.8, 0.2);
        e.set_track(3, Instrument::EPiano, 0.6, 0.3);
        e.note_on(0, 72, 0.8);
        e.note_on(1, 36, 0.9);
        e.note_on(2, 36, 1.0);
        e.note_on(2, 42, 0.6);
        e.note_on(3, 60, 0.7);
        e.note_on(3, 64, 0.7);
        e.note_on(3, 67, 0.7);
        let out = render_seconds(&mut e, 0.3);
        let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(peak > 0.05 && peak <= 1.0, "arrangement peak={peak}");
    }
    #[test]
    fn room_tail_exists_and_decays() {
        for sr in [48000.0f32, 44100.0] {
            let mut e = Engine::new(sr);
            e.set_track(0, Instrument::Drums, 0.8, 0.0);
            e.note_on(0, 38, 1.0); // snare: broadband, excites the room
            let win = (0.1 * sr) as usize;
            let mut rms = |e: &mut Engine, upto: usize| {
                let mut acc = 0.0f64;
                let mut n = 0usize;
                let mut done = 0usize;
                while done < upto {
                    let f = 128.min(upto - done);
                    e.process(f);
                    for i in 0..f {
                        acc += (e.out_l[i] as f64).powi(2) + (e.out_r[i] as f64).powi(2);
                        n += 2;
                        assert!(e.out_l[i].is_finite() && e.out_r[i].is_finite());
                    }
                    done += f;
                }
                (acc / n as f64).sqrt()
            };
            let hit = rms(&mut e, win); // 0-0.1 s: the hit itself
            let mid = rms(&mut e, 3 * win); // 0.1-0.4 s: room tail territory
            let late = rms(&mut e, 6 * win); // 0.4-1.0 s: tail must be dying
            assert!(hit > 1e-3, "snare inaudible at {sr}");
            // a tail exists (not anechoic): mid keeps meaningful energy
            assert!(mid > hit * 1e-3, "no room tail at {sr}: mid {mid} vs hit {hit}");
            // and it DECAYS (RT60 0.38 s: 0.4-1.0 s window well below 0.1-0.4 s)
            assert!(late < mid * 0.5, "room tail not decaying at {sr}: {late} vs {mid}");
        }
    }

    #[test]
    fn room_send_zero_restores_dry_engine() {
        let mut wet = Engine::new(48000.0);
        let mut dry = Engine::new(48000.0);
        for e in [&mut wet, &mut dry] {
            e.set_track(0, Instrument::Marimba, 0.8, 0.0);
        }
        dry.set_room(0, 0.0);
        wet.note_on(0, 60, 0.8);
        dry.note_on(0, 60, 0.8);
        wet.process(128);
        dry.process(128);
        // pre-delay (22 ms) means the first block is identical wet vs dry…
        for i in 0..128 {
            assert_eq!(wet.out_l[i], dry.out_l[i]);
        }
        // …after pre-delay + the earliest tap the wet engine must differ (the
        // room is real). Window covers room pre 22 ms + tap 13 ms ≈ 35 ms with
        // margin (the 2026-07-13 pre-delay bump pushed the wet onset past the
        // old 13-block window — that was the whole point of the change).
        let mut differs = false;
        for _ in 0..40 {
            wet.process(128);
            dry.process(128);
            for i in 0..128 {
                if wet.out_l[i] != dry.out_l[i] {
                    differs = true;
                }
            }
        }
        assert!(differs, "room stage is inaudible even with default sends");
    }

    #[test]
    fn reverb_menu_voicings_behave() {
        // hall rings longer than room; off is bone dry; all finite
        let tail_rms = |kind: u32| -> f64 {
            let mut e = Engine::new(48000.0);
            e.set_reverb(kind);
            e.set_track(0, Instrument::Drums, 0.8, 0.0);
            e.note_on(0, 38, 1.0);
            let mut acc = 0.0f64;
            let mut n = 0usize;
            let mut done = 0usize;
            // measure 0.5-1.0 s (snare itself is long gone; the tail remains)
            while done < 48_000 {
                e.process(128);
                if done >= 24_000 {
                    for i in 0..128 {
                        assert!(e.out_l[i].is_finite());
                        acc += (e.out_l[i] as f64).powi(2);
                        n += 1;
                    }
                }
                done += 128;
            }
            (acc / n as f64).sqrt()
        };
        let off = tail_rms(REVERB_OFF);
        let room = tail_rms(REVERB_ROOM);
        let hall = tail_rms(REVERB_HALL);
        let plate = tail_rms(REVERB_PLATE);
        let spring = tail_rms(REVERB_SPRING);
        assert!(hall > room * 2.0, "hall {hall} not longer than room {room}");
        assert!(off < room * 0.8, "off {off} vs room {room} — sends leaking?");
        assert!(plate > off && spring > off, "plate/spring inaudible");
    }

}
