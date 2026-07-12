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

use kernels::{
    amp_defaults, body_defaults, makeup_gain, pickup_defaults, start_voice, voice_pan, Instrument,
    Kernel, SympBank, Voice, MAX_BLOCK, MAX_BODY_MODES,
};

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
    pe_on: bool,
    pe_b: [[f32; 3]; 2],
    pe_a: [[f32; 2]; 2],
    pe_z: [[f32; 2]; 2],
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

pub struct Engine {
    pub sample_rate: f32,
    voices: Vec<Voice>,
    tracks: [TrackBus; MAX_TRACKS],
    track_l: [f32; MAX_BLOCK],
    track_r: [f32; MAX_BLOCK],
    voice_buf: [f32; MAX_BLOCK],
    symp: Vec<SympBank>,
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
                gl: 0.0,
                gr: 0.0,
            }; MAX_TRACKS],
            track_l: [0.0; MAX_BLOCK],
            track_r: [0.0; MAX_BLOCK],
            voice_buf: [0.0; MAX_BLOCK],
            symp,
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
            for i in 0..frames {
                bus.gl += c * (tl - bus.gl);
                bus.gr += c * (tr - bus.gr);
                self.out_l[i] += self.track_l[i] * bus.gl;
                self.out_r[i] += self.track_r[i] * bus.gr;
            }
            bus.gl = flush_denormal(bus.gl);
            bus.gr = flush_denormal(bus.gr);
        }

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
}
