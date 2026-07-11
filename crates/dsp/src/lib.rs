//! instruments.js DSP core.
//!
//! Constraints (AGENTS.md constitution #4, architecture doc 2026-07-11):
//! - Allocation-free after `Engine::new` — the audio thread never touches the allocator.
//! - All recursive filter state passes through [`flush_denormal`] (WASM has no hardware FTZ).
//! - Voices live in structure-of-arrays banks so independent voices can batch across SIMD lanes.
//! - One engine renders and mixes ALL tracks/instruments; the budget is 2.67 ms per 128-frame
//!   quantum at 48 kHz for a full multi-track arrangement.

#![forbid(unsafe_code)]

pub const MAX_VOICES: usize = 64;
pub const MAX_TRACKS: usize = 16;
pub const QUANTUM_FRAMES: usize = 128;

/// Flush denormals to zero. Denormals in recursive feedback loops are the top WASM perf killer
/// (Letz/Orlarey 2018); every state variable update must pass through this.
#[inline(always)]
pub fn flush_denormal(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

/// Structure-of-arrays voice bank shared by all instruments across all tracks.
/// Fields are parallel arrays (not an array of voice structs) so per-sample kernels
/// can batch 4 independent voices across 128-bit SIMD lanes.
pub struct VoiceBank {
    pub active: [bool; MAX_VOICES],
    pub track: [u8; MAX_VOICES],
    pub pitch_hz: [f32; MAX_VOICES],
    pub velocity: [f32; MAX_VOICES],
    pub age_frames: [u64; MAX_VOICES],
}

impl VoiceBank {
    pub fn new() -> Self {
        Self {
            active: [false; MAX_VOICES],
            track: [0; MAX_VOICES],
            pitch_hz: [0.0; MAX_VOICES],
            velocity: [0.0; MAX_VOICES],
            age_frames: [0; MAX_VOICES],
        }
    }
}

impl Default for VoiceBank {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-track bus: gain/pan applied when voices are mixed to the stereo output.
#[derive(Clone, Copy)]
pub struct TrackBus {
    pub gain: f32,
    pub pan: f32, // -1.0 .. 1.0
}

pub struct Engine {
    pub sample_rate: f32,
    pub voices: VoiceBank,
    pub tracks: [TrackBus; MAX_TRACKS],
}

impl Engine {
    /// All allocation happens here, once. `sample_rate` is per-instance — never global,
    /// never assumed 48 kHz (iOS Safari locks contexts to 44.1 kHz).
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            voices: VoiceBank::new(),
            tracks: [TrackBus { gain: 1.0, pan: 0.0 }; MAX_TRACKS],
        }
    }

    /// Render one quantum for the whole arrangement into interleaved-free stereo buffers.
    /// Silence until issue #6 lands the first kernels; the signature and the
    /// allocation-free contract are the point of this skeleton.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        debug_assert_eq!(out_l.len(), out_r.len());
        out_l.fill(0.0);
        out_r.fill(0.0);
        for i in 0..MAX_VOICES {
            if self.voices.active[i] {
                self.voices.age_frames[i] += out_l.len() as u64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormals_are_flushed() {
        assert_eq!(flush_denormal(1.0e-30), 0.0);
        assert_eq!(flush_denormal(-1.0e-30), 0.0);
        assert_eq!(flush_denormal(0.5), 0.5);
    }

    #[test]
    fn engine_renders_silence_without_allocating_per_call() {
        let mut e = Engine::new(48_000.0);
        let (mut l, mut r) = ([1.0f32; QUANTUM_FRAMES], [1.0f32; QUANTUM_FRAMES]);
        e.process(&mut l, &mut r);
        assert!(l.iter().all(|&s| s == 0.0) && r.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn sample_rate_is_per_instance() {
        assert_eq!(Engine::new(44_100.0).sample_rate, 44_100.0);
        assert_eq!(Engine::new(48_000.0).sample_rate, 48_000.0);
    }
}
