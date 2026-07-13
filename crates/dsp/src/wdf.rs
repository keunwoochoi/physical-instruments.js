//! Wave Digital Filter (WDF) circuit-simulation guitar amplifier.
//!
//! Owner directive (Keunwoo 2026-07-13): "amp: it should be circuit simulation.
//! that's what an amplifier is in the real world." Design doc:
//! `agentic-docs/design/2026-07-13-wdf-amp-circuit-sim.md` (P1: WDF primitive
//! port + single 12AX7 triode stage + Fender TMB tone stack + supply-rail sag,
//! behind an engine-internal selector; the behavioral ADAA chain stays default).
//!
//! ## Ported code (BSD-3, see `agentic-docs/licensing.md` port ledger)
//! The WDF one-port and adaptor primitives below are a Rust port of
//! `chowdsp_wdf` (Chowdhury-DSP, BSD-3-Clause) — specifically the `wdft`
//! (templated, compile-time) headers `wdft_base.h`, `wdft_one_ports.h`,
//! `wdft_adaptors.h`, `wdft_sources.h`. The C++ uses runtime polymorphism +
//! templates; this port is allocation-free monomorphized Rust: one-ports are
//! plain structs, adaptors are generic over their child `Port`s (no `Box`/`dyn`
//! on the audio path). Wave-scattering equations are faithful to the origin.
//!
//! ## Numerics
//! The triode stage runs in `f64`. Justified exception to the repo's f32-default
//! (port-audit checklist): the plate rail (~320 V) and plate current (~1e-3 A)
//! span five orders of magnitude, and the Newton root's Jacobian conditioning
//! needs the headroom. The stage converts to/from f32 only at its bus boundary.
//!
//! ## Anti-aliasing
//! The 12AX7 hard-clips (lead voicing) → a Newton-root nonlinearity aliases at
//! 48 kHz. The stage runs at 2× via a linear-phase half-band FIR pair (design
//! doc: "2× max with a half-band pair"). ADAA-at-the-root is impractical for a
//! Newton solve; oversampling is the sanctioned fallback.

#![allow(clippy::excessive_precision)]
// The fast-exp minimax coefficient equals ln(2) by construction (d/dx 2^x at 0);
// it is a polynomial coefficient, not the mathematical constant.
#![allow(clippy::approximate_constant)]

use crate::flush_denormal;

// ===========================================================================
// Ported WDF primitives — chowdsp_wdf `wdft` (BSD-3-Clause, Chowdhury-DSP 2022)
// ===========================================================================

/// Common WDF port state: impedance `r`, admittance `g`, incident `a`,
/// reflected `b`. (chowdsp `WDFMembers`.)
#[derive(Clone, Copy, Default)]
pub struct Wdf {
    pub r: f64,
    pub g: f64,
    pub a: f64,
    pub b: f64,
}

/// One-port / adaptor interface (chowdsp `BaseWDF`, reduced to the methods the
/// amp needs). Impedance is fixed after `prepare` (component + fs), so there is
/// no runtime `propagateImpedanceChange`; roots read `imp()` once per note.
pub trait Port {
    /// Port impedance (Ω), the up-facing `wdf.R`.
    fn imp(&self) -> f64;
    /// Compute + return the reflected wave from internal state only
    /// (independent of the incident wave — the Norton/Thévenin companion).
    fn reflected(&mut self) -> f64;
    /// Accept an incident wave, advancing internal (capacitor) state.
    fn incident(&mut self, a: f64);
    /// Last reflected wave `b` (read by parent adaptors).
    fn b(&self) -> f64;
}

/// WDF resistor: `Z_R = R`, reflects nothing (chowdsp `ResistorT`).
#[derive(Clone, Copy)]
pub struct Resistor {
    r: f64,
}
impl Resistor {
    pub fn new(r: f64) -> Self {
        Self { r }
    }
}
impl Port for Resistor {
    #[inline]
    fn imp(&self) -> f64 {
        self.r
    }
    #[inline]
    fn reflected(&mut self) -> f64 {
        0.0
    }
    #[inline]
    fn incident(&mut self, _a: f64) {}
    #[inline]
    fn b(&self) -> f64 {
        0.0
    }
}

/// WDF capacitor, bilinear (`Z_C = 1/(2·C·fs)`), unit-delay realization
/// (chowdsp `CapacitorT`). `reflected() = z` (previous incident); `incident`
/// stores `z = a`.
#[derive(Clone, Copy)]
pub struct Capacitor {
    r: f64,
    z: f64,
    b_last: f64,
}
impl Capacitor {
    pub fn new(c_farads: f64, fs: f64) -> Self {
        Self {
            r: 1.0 / (2.0 * c_farads * fs),
            z: 0.0,
            b_last: 0.0,
        }
    }
    pub fn reset(&mut self) {
        self.z = 0.0;
        self.b_last = 0.0;
    }
}
impl Port for Capacitor {
    #[inline]
    fn imp(&self) -> f64 {
        self.r
    }
    #[inline]
    fn reflected(&mut self) -> f64 {
        self.b_last = self.z;
        self.b_last
    }
    #[inline]
    fn incident(&mut self, a: f64) {
        self.z = flush_denormal(a as f32) as f64;
    }
    #[inline]
    fn b(&self) -> f64 {
        self.b_last
    }
}

/// WDF voltage source with series resistance (chowdsp `ResistiveVoltageSourceT`):
/// `Z = R`, `reflected() = Vs`. Drives the plate rail (sagging supply).
#[derive(Clone, Copy)]
pub struct ResVSource {
    r: f64,
    vs: f64,
    b_last: f64,
}
impl ResVSource {
    pub fn new(r: f64) -> Self {
        Self {
            r,
            vs: 0.0,
            b_last: 0.0,
        }
    }
    #[inline]
    pub fn set_voltage(&mut self, v: f64) {
        self.vs = v;
    }
}
impl Port for ResVSource {
    #[inline]
    fn imp(&self) -> f64 {
        self.r
    }
    #[inline]
    fn reflected(&mut self) -> f64 {
        self.b_last = self.vs;
        self.b_last
    }
    #[inline]
    fn incident(&mut self, _a: f64) {}
    #[inline]
    fn b(&self) -> f64 {
        self.b_last
    }
}

/// WDF 3-port parallel adaptor (chowdsp `WDFParallelT`). Impedance
/// `1/R = 1/R1 + 1/R2`. Used for the cathode network `Rk ‖ Ck`.
#[derive(Clone, Copy)]
pub struct Parallel<P1: Port, P2: Port> {
    pub p1: P1,
    pub p2: P2,
    r: f64,
    g: f64,
    p1_reflect: f64,
    b_diff: f64,
    b_last: f64,
}
impl<P1: Port, P2: Port> Parallel<P1, P2> {
    pub fn new(p1: P1, p2: P2) -> Self {
        let mut s = Self {
            p1,
            p2,
            r: 0.0,
            g: 0.0,
            p1_reflect: 1.0,
            b_diff: 0.0,
            b_last: 0.0,
        };
        s.calc_impedance();
        s
    }
    fn calc_impedance(&mut self) {
        self.g = 1.0 / self.p1.imp() + 1.0 / self.p2.imp();
        self.r = 1.0 / self.g;
        self.p1_reflect = (1.0 / self.p1.imp()) / self.g;
    }
}
impl<P1: Port, P2: Port> Port for Parallel<P1, P2> {
    #[inline]
    fn imp(&self) -> f64 {
        self.r
    }
    #[inline]
    fn reflected(&mut self) -> f64 {
        let b1 = self.p1.reflected();
        let b2 = self.p2.reflected();
        self.b_diff = b2 - b1;
        self.b_last = b2 - self.p1_reflect * self.b_diff;
        self.b_last
    }
    #[inline]
    fn incident(&mut self, x: f64) {
        let b2 = self.b_last - self.p2.b() + x;
        self.p1.incident(b2 + self.b_diff);
        self.p2.incident(b2);
    }
    #[inline]
    fn b(&self) -> f64 {
        self.b_last
    }
}

/// WDF 3-port series adaptor (chowdsp `WDFSeriesT`). Impedance `R = R1 + R2`.
/// Used for the plate network `Rp + supply source`.
#[derive(Clone, Copy)]
pub struct Series<P1: Port, P2: Port> {
    pub p1: P1,
    pub p2: P2,
    r: f64,
    p1_reflect: f64,
    b_last: f64,
}
impl<P1: Port, P2: Port> Series<P1, P2> {
    pub fn new(p1: P1, p2: P2) -> Self {
        let mut s = Self {
            p1,
            p2,
            r: 0.0,
            p1_reflect: 1.0,
            b_last: 0.0,
        };
        s.calc_impedance();
        s
    }
    fn calc_impedance(&mut self) {
        self.r = self.p1.imp() + self.p2.imp();
        self.p1_reflect = self.p1.imp() / self.r;
    }
}
impl<P1: Port, P2: Port> Port for Series<P1, P2> {
    #[inline]
    fn imp(&self) -> f64 {
        self.r
    }
    #[inline]
    fn reflected(&mut self) -> f64 {
        self.b_last = -(self.p1.reflected() + self.p2.reflected());
        self.b_last
    }
    #[inline]
    fn incident(&mut self, x: f64) {
        let b1 = self.p1.b() - self.p1_reflect * (x + self.p1.b() + self.p2.b());
        self.p1.incident(b1);
        self.p2.incident(-(x + b1));
    }
    #[inline]
    fn b(&self) -> f64 {
        self.b_last
    }
}

// ===========================================================================
// 12AX7 triode (Koren 1996) — the WDF root nonlinearity
// ===========================================================================

/// Koren 12AX7 parameters (Koren 1996; Duncan/common values).
const KOREN_MU: f64 = 100.0;
const KOREN_EX: f64 = 1.4;
const KOREN_KG1: f64 = 1060.0;
const KOREN_KP: f64 = 600.0;
const KOREN_KVB: f64 = 300.0;

/// Newton iteration cap on the audio path. Warm-started, the smooth triode
/// converges in ~2 steps; the ≤8 design-doc hard cap is never approached.
const HARD_ITERS: usize = 2;

/// Fast `2^x` (minimax poly on the fractional part + exponent bit-scale).
/// ~1e-6 relative — ample for a Newton fixed point + audio nonlinearity, and
/// several× faster than `std::exp`/`powf` in the per-sample triode loop.
#[inline(always)]
fn fexp2(x: f64) -> f64 {
    let xc = x.clamp(-1020.0, 1020.0);
    let fl = xc.floor();
    let f = xc - fl;
    // minimax 2^f on [0,1)
    let p = 1.0
        + f * (0.6931471805599453
            + f * (0.2402265069591007
                + f * (0.05550410866482158
                    + f * (0.009618129107628477 + f * 0.0013333558146428443))));
    let e = ((fl as i64 + 1023) << 52) as u64;
    f64::from_bits(e) * p
}
#[inline(always)]
fn fexp(x: f64) -> f64 {
    fexp2(x * core::f64::consts::LOG2_E)
}
/// Fast `ln(x)` for `x > 0` (exponent + minimax on the mantissa).
#[inline(always)]
fn fln(x: f64) -> f64 {
    let bits = x.to_bits();
    let e = (((bits >> 52) & 0x7ff) as i64 - 1023) as f64;
    let m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    // ln(m), m in [1,2): minimax around sqrt(2) via t=(m-1)
    let t = m - 1.0;
    let lnm = t
        * (0.9999964239
            + t * (-0.4998741238
                + t * (0.3317990258 + t * (-0.2407338084 + t * (0.1676540711 - t * 0.0953293897)))));
    e * core::f64::consts::LN_2 + lnm
}

/// Koren plate current `I_p(V_pk, V_gk)` and its partials `∂I_p/∂V_pk`,
/// `∂I_p/∂V_gk` (grid current = 0 for P1; blocking distortion is P2).
#[inline]
fn koren(vpk: f64, vgk: f64) -> (f64, f64, f64) {
    let vpk = if vpk < 0.0 { 0.0 } else { vpk };
    let root = (KOREN_KVB + vpk * vpk).sqrt();
    let x = KOREN_KP * (1.0 / KOREN_MU + vgk / root);
    // softplus ln(1+e^x), overflow-guarded; sigmoid = its derivative
    let (sp, sig) = if x > 30.0 {
        (x, 1.0)
    } else if x < -30.0 {
        (0.0, 0.0)
    } else {
        let e = fexp(x);
        (fln(1.0 + e), e / (1.0 + e))
    };
    let e1 = (vpk / KOREN_KP) * sp;
    if e1 <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    // one pow: E1^EX = exp(EX·ln E1) via the fast pair; derivative uses E1^EX/E1
    let p_ex = fexp(KOREN_EX * fln(e1));
    let ip = p_ex / KOREN_KG1;
    let dx_dvpk = -KOREN_KP * vgk * vpk / (root * root * root);
    let dx_dvgk = KOREN_KP / root;
    let de1_dvpk = sp / KOREN_KP + (vpk / KOREN_KP) * sig * dx_dvpk;
    let de1_dvgk = (vpk / KOREN_KP) * sig * dx_dvgk;
    let dip_de1 = (KOREN_EX / KOREN_KG1) * (p_ex / e1);
    (ip, dip_de1 * de1_dvpk, dip_de1 * de1_dvgk)
}

// ===========================================================================
// Fender '59 Bassman TMB tone stack — exact bilinear discretization
// ===========================================================================
//
// Realized as its exact bilinear-transformed transfer function H(z) (Yeh &
// Smith, DAFx-06, "Discretization of the '59 Fender Bassman Tone Stack",
// Eqs. 1-2). Yeh proves this discretization is the response a WDF realization
// of the same tree produces; the bridged tone-stack topology needs an R-type
// adaptor (deferred with the rest of the R-type port), so P1 ships the proven
// H(z). Coefficients verified against the analytic H(s) to <0.07 dB through
// 6 kHz across knob settings (scratchpad tonestack.py), and against the SPICE-
// matched paper curves in the unit tests below.

/// '59 Bassman component values (paper Fig. 1).
const TS_C1: f64 = 0.25e-9;
const TS_C2: f64 = 20e-9;
const TS_C3: f64 = 20e-9;
const TS_R1: f64 = 250e3; // treble pot
const TS_R2: f64 = 1e6; // bass pot
const TS_R3: f64 = 25e3; // mid pot
const TS_R4: f64 = 56e3;

/// Tone-stack s-domain coefficients for knob settings `t,l,m ∈ [0,1]`
/// (treble, low/bass, mid). Returns `(b1,b2,b3, a1,a2,a3)` with `a0 = 1`.
fn tonestack_s_coeffs(t: f64, l: f64, m: f64) -> (f64, f64, f64, f64, f64, f64) {
    let (c1, c2, c3, r1, r2, r3, r4) = (TS_C1, TS_C2, TS_C3, TS_R1, TS_R2, TS_R3, TS_R4);
    let b1 = t * c1 * r1 + m * c3 * r3 + l * (c1 * r2 + c2 * r2) + (c1 * r3 + c2 * r3);
    let b2 = t * (c1 * c2 * r1 * r4 + c1 * c3 * r1 * r4) - m * m * (c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + m * (c1 * c3 * r1 * r3 + c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * (c1 * c2 * r1 * r2 + c1 * c2 * r2 * r4 + c1 * c3 * r2 * r4)
        + l * m * (c1 * c3 * r2 * r3 + c2 * c3 * r2 * r3)
        + (c1 * c2 * r1 * r3 + c1 * c2 * r3 * r4 + c1 * c3 * r3 * r4);
    let b3 = l * m * (c1 * c2 * c3 * r1 * r2 * r3 + c1 * c2 * c3 * r2 * r3 * r4)
        - m * m * (c1 * c2 * c3 * r1 * r3 * r3 + c1 * c2 * c3 * r3 * r3 * r4)
        + m * (c1 * c2 * c3 * r1 * r3 * r3 + c1 * c2 * c3 * r3 * r3 * r4)
        + t * c1 * c2 * c3 * r1 * r3 * r4
        - t * m * c1 * c2 * c3 * r1 * r3 * r4
        + t * l * c1 * c2 * c3 * r1 * r2 * r4;
    let a1 = (c1 * r1 + c1 * r3 + c2 * r3 + c2 * r4 + c3 * r4) + m * c3 * r3 + l * (c1 * r2 + c2 * r2);
    let a2 = m * (c1 * c3 * r1 * r3 - c2 * c3 * r3 * r4 + c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * m * (c1 * c3 * r2 * r3 + c2 * c3 * r2 * r3)
        - m * m * (c1 * c3 * r3 * r3 + c2 * c3 * r3 * r3)
        + l * (c1 * c2 * r2 * r4 + c1 * c2 * r1 * r2 + c1 * c3 * r2 * r4 + c2 * c3 * r2 * r4)
        + (c1 * c2 * r1 * r4
            + c1 * c3 * r1 * r4
            + c1 * c2 * r1 * r3
            + c1 * c2 * r3 * r4
            + c1 * c3 * r3 * r4
            + c2 * c3 * r3 * r4);
    let a3 = l * m * (c1 * c2 * c3 * r1 * r2 * r3 + c1 * c2 * c3 * r2 * r3 * r4)
        - m * m * (c1 * c2 * c3 * r1 * r3 * r3 + c1 * c2 * c3 * r3 * r3 * r4)
        + m * (c1 * c2 * c3 * r3 * r3 * r4 + c1 * c2 * c3 * r1 * r3 * r3 - c1 * c2 * c3 * r1 * r3 * r4)
        + l * c1 * c2 * c3 * r1 * r2 * r4
        + t * c1 * c2 * c3 * r1 * r3 * r4;
    (b1, b2, b3, a1, a2, a3)
}

/// Third-order tone-stack filter (Yeh bilinear H(z), Direct-Form I).
#[derive(Clone, Copy, Default)]
struct ToneStack {
    b: [f64; 4],
    a: [f64; 4], // a[0] == 1 after normalization
    x: [f64; 3],
    y: [f64; 3],
}
impl ToneStack {
    fn set(&mut self, t: f64, l: f64, m: f64, fs: f64) {
        let (b1, b2, b3, a1, a2, a3) = tonestack_s_coeffs(t, l, m);
        let c = 2.0 * fs;
        let (c2, c3) = (c * c, c * c * c);
        let mut bb = [
            -b1 * c - b2 * c2 - b3 * c3,
            -b1 * c + b2 * c2 + 3.0 * b3 * c3,
            b1 * c + b2 * c2 - 3.0 * b3 * c3,
            b1 * c - b2 * c2 + b3 * c3,
        ];
        let aa = [
            -1.0 - a1 * c - a2 * c2 - a3 * c3,
            -3.0 - a1 * c + a2 * c2 + 3.0 * a3 * c3,
            -3.0 + a1 * c + a2 * c2 - 3.0 * a3 * c3,
            -1.0 + a1 * c - a2 * c2 + a3 * c3,
        ];
        let a0 = aa[0];
        for v in &mut bb {
            *v /= a0;
        }
        self.b = bb;
        self.a = [1.0, aa[1] / a0, aa[2] / a0, aa[3] / a0];
    }
    fn reset(&mut self) {
        self.x = [0.0; 3];
        self.y = [0.0; 3];
    }
    #[inline]
    fn process(&mut self, xn: f64) -> f64 {
        let yn = self.b[0] * xn + self.b[1] * self.x[0] + self.b[2] * self.x[1] + self.b[3] * self.x[2]
            - self.a[1] * self.y[0]
            - self.a[2] * self.y[1]
            - self.a[3] * self.y[2];
        self.x[2] = self.x[1];
        self.x[1] = self.x[0];
        self.x[0] = xn;
        self.y[2] = self.y[1];
        self.y[1] = self.y[0];
        self.y[0] = flush_denormal(yn as f32) as f64;
        self.y[0]
    }
}

// ===========================================================================
// 2× half-band oversampler (linear-phase FIR, 31-tap, Kaiser β=8)
// ===========================================================================
//
// Half-band symmetric FIR: even-offset taps are zero (except the center 0.5),
// so an N=31 kernel costs ~8 mults/sample. Passband ripple <0.4 dB to 19 kHz,
// stopband ≥27 dB above 29 kHz (scratchpad halfband design).

const HB_LEN: usize = 31;
/// One-sided half-band taps (center + odd offsets); even offsets are 0.
const HB_H: [f64; HB_LEN] = [
    // index 0..30, symmetric around center=15
    -0.0000121018, 0.0, 0.0003016208, 0.0, -0.0016888416, 0.0, 0.0059365893, 0.0, -0.0161606016, 0.0,
    0.0381149467, 0.0, -0.0884970575, 0.0, 0.3120054456, 0.5, 0.3120054456, 0.0, -0.0884970575, 0.0,
    0.0381149467, 0.0, -0.0161606016, 0.0, 0.0059365893, 0.0, -0.0016888416, 0.0, 0.0003016208, 0.0,
    -0.0000121018,
];

/// Polyphase 2× up/down half-band pair with a shared delay line.
#[derive(Clone, Copy)]
struct Halfband {
    up: [f64; HB_LEN],
    down: [f64; HB_LEN],
}
impl Halfband {
    fn new() -> Self {
        Self {
            up: [0.0; HB_LEN],
            down: [0.0; HB_LEN],
        }
    }
    fn reset(&mut self) {
        self.up = [0.0; HB_LEN];
        self.down = [0.0; HB_LEN];
    }
    /// Push one input sample, return the two upsampled (2×) samples.
    /// Interpolation: even phase = delayed center tap; odd phase = FIR of
    /// zero-stuffed history (×2 gain to preserve level).
    #[inline]
    fn upsample(&mut self, x: f64) -> (f64, f64) {
        // shift history
        for i in (1..HB_LEN).rev() {
            self.up[i] = self.up[i - 1];
        }
        self.up[0] = x;
        // even output phase: the center tap (pure delay), ×2·0.5 = ×1
        let even = self.up[HB_LEN / 2];
        // odd output phase: convolution with odd taps, ×2
        let mut odd = 0.0;
        let mut i = 0;
        while i < HB_LEN {
            odd += HB_H[i] * self.up[i];
            i += 2;
        }
        (even, 2.0 * odd)
    }
    /// Decimate two 2× samples to one, applying the half-band anti-alias FIR.
    #[inline]
    fn downsample(&mut self, x0: f64, x1: f64) -> f64 {
        // history holds interleaved 2× stream; push both, filter at the
        // decimated phase (center + odd taps on the appropriate samples).
        for i in (2..HB_LEN).rev() {
            self.down[i] = self.down[i - 2];
        }
        self.down[1] = x0;
        self.down[0] = x1;
        // decimated output = center-tap sample + odd-tap FIR (half-band)
        let mut acc = 0.5 * self.down[HB_LEN / 2];
        let mut i = 0;
        while i < HB_LEN {
            acc += HB_H[i] * self.down[i];
            i += 2;
        }
        acc
    }
}

// ===========================================================================
// Amp stage: input gain → triode (2× OS) → AC couple → tone stack → output
// ===========================================================================

/// Per-voicing WDF amp configuration.
#[derive(Clone, Copy)]
pub struct WdfAmpConfig {
    /// Grid drive: bus-signal → grid volts scale.
    pub drive_v: f64,
    /// Output makeup after the plate (keeps stage ≈ unity into the bus).
    pub out_scale: f64,
    /// Tone-stack knobs (treble, bass, mid) ∈ [0,1].
    pub tone: (f64, f64, f64),
    /// Supply-rail RC: (R_supply Ω, C_supply F) — bigger R = more sag,
    /// R·C = recovery time constant (the "sing" recovery).
    pub supply_rc: (f64, f64),
    /// B+ rail volts.
    pub b_plus: f64,
    /// Power-stage current proxy: the shared B+ rail is loaded by the output
    /// tubes' current, ≈ `load_k · |output|`. P1 models only the preamp triode,
    /// so this reduced term carries the class-AB power-stage draw that makes the
    /// rail sag under a loud attack and recover (→ singing sustain) as the note
    /// decays. 0 = preamp draw only.
    pub load_k: f64,
    /// 2× oversample the triode (lead needs it; clean may not).
    pub oversample: bool,
}

/// The full WDF amplifier stage for one track bus. Fixed-size, allocation-free.
#[derive(Clone, Copy)]
pub struct WdfAmp {
    cfg: WdfAmpConfig,
    fs: f64,
    // circuit component values
    rp: f64,
    // WDF one-port network on the audio path (ported parallel adaptor): the
    // cathode self-bias network Rk ‖ Ck. The plate side (Rp + supply source) is
    // a trivial Thévenin (E=Vs, R=Rp) computed directly — the ported `Series`
    // adaptor is unit-tested as infrastructure but its reflected-wave polarity
    // is for in-tree use, not standalone companion extraction.
    cathode: Parallel<Resistor, Capacitor>,
    // triode / supply state
    ip_prev: f64,
    vp_dc: f64, // plate DC operating point (for AC coupling)
    // supply sag companion (slow reservoir)
    j_sup: f64,
    vs: f64,
    // power-stage rail-load proxy: smoothed |output| (envelope of the current
    // the shared rail delivers). Drives the sag alongside the preamp i_p.
    load_env: f64,
    // AC-coupling highpass (coupling cap, ~5 Hz)
    hp_x1: f64,
    hp_y1: f64,
    hp_c: f64,
    // tone stack + oversampler
    tone: ToneStack,
    hb: Halfband,
    ready: bool,
}

impl WdfAmp {
    pub fn new() -> Self {
        Self {
            cfg: WdfAmpConfig {
                drive_v: 1.0,
                out_scale: 1.0,
                tone: (0.5, 0.5, 0.5),
                supply_rc: (1000.0, 10e-6),
                b_plus: 320.0,
                load_k: 0.0,
                oversample: true,
            },
            fs: 48000.0,
            rp: 100e3,
            cathode: Parallel::new(Resistor::new(1.5e3), Capacitor::new(22e-6, 48000.0)),
            ip_prev: 1.0e-3,
            vp_dc: 0.0,
            j_sup: 0.0,
            vs: 320.0,
            load_env: 0.0,
            hp_x1: 0.0,
            hp_y1: 0.0,
            hp_c: 0.0,
            tone: ToneStack::default(),
            hb: Halfband::new(),
            ready: false,
        }
    }

    /// Configure for a voicing at a sample rate. Rebuilds impedances, solves the
    /// DC operating point, and primes the AC-coupling reference.
    pub fn prepare(&mut self, cfg: WdfAmpConfig, sr: f32) {
        self.cfg = cfg;
        self.fs = sr as f64;
        self.rp = 100e3;
        // cathode: 1.5k ‖ 22µF (self-bias with bypass)
        self.cathode = Parallel::new(Resistor::new(1.5e3), Capacitor::new(22e-6, self.fs));
        // AC-coupling highpass corner ~5 Hz (coupling cap into 1M grid leak)
        let fc = 5.0;
        self.hp_c = (-core::f64::consts::TAU * fc / self.fs).exp();
        self.reset_state();
        // settle DC operating point at grid = 0 (a few supply time constants)
        self.vs = cfg.b_plus;
        self.j_sup = (2.0 * cfg.supply_rc.1 * self.fs) * cfg.b_plus;
        let mut vp = self.vp_dc;
        for _ in 0..8000 {
            vp = self.tick_triode(0.0);
        }
        self.vp_dc = vp;
        // prime AC-coupling to the settled plate DC so t=0 has no thump
        self.hp_x1 = vp;
        self.hp_y1 = 0.0;
        self.tone.set(cfg.tone.0, cfg.tone.1, cfg.tone.2, self.fs);
        self.tone.reset();
        self.hb.reset();
        self.ready = true;
    }

    fn reset_state(&mut self) {
        self.ip_prev = 1.0e-3;
        self.load_env = 0.0;
        self.cathode = Parallel::new(Resistor::new(1.5e3), Capacitor::new(22e-6, self.fs));
    }

    /// One triode sample at the (possibly oversampled) rate. `vg` = grid volts.
    /// Returns the plate voltage. Advances cathode + supply state.
    #[inline]
    fn tick_triode(&mut self, vg: f64) -> f64 {
        // supply sag: reservoir behind R_supply, drawn by the (lagged) preamp
        // plate current plus the power-stage proxy load (≈ load_k·|output|). The
        // reservoir cap integrates the draw → a rail that dips on a loud attack
        // and recovers over R·C as the note decays (singing sustain).
        let g_sup = 1.0 / self.cfg.supply_rc.0;
        let g_csup = 2.0 * self.cfg.supply_rc.1 * self.fs;
        let i_draw = self.ip_prev + self.cfg.load_k * self.load_env;
        // reservoir can sag but never exceed B+ or collapse below ~40% (a real
        // rail is bounded by the rectifier and never reverses) — this bound also
        // keeps the sag feedback unconditionally stable.
        let vb = self.cfg.b_plus;
        self.vs = ((g_sup * vb + self.j_sup - i_draw) / (g_sup + g_csup)).clamp(0.4 * vb, vb);
        // cathode companion from the ported parallel one-port (on the path)
        let e_cath = self.cathode.reflected();
        let r_cath = self.cathode.imp();
        // plate Thévenin: supply Vs behind Rp (open-circuit V = Vs, R = Rp)
        let e_plate = self.vs;
        let r_plate = self.rp;
        // Grid-conduction clamp: a real grid draws current once it swings past
        // the cathode, pinning V_gk near 0 (the grid-stopper + source resistance
        // drop the excess). Full blocking distortion is P2; this reduced clamp
        // keeps the (grid-current-free) Newton well-posed under hard overdrive
        // and gives the physically-correct asymmetric top-side compression.
        let vk_est = e_cath + r_cath * self.ip_prev;
        let vg = vg.min(vk_est + 0.4);
        // hard current bound: plate can't go below ~0 V, so i_p ≤ Vs/Rp.
        let i_max = 1.2 * (e_plate.max(1.0) / r_plate);
        // scalar Newton on plate current i_p (bounded, warm-started, damped).
        // Warm-started from the previous sample (consecutive samples are close,
        // especially at 2× rate), the smooth Koren curve converges in ~2 steps;
        // cap at HARD_ITERS (well under the ≤8 hard cap) to hold the budget.
        // `converged` tracks the tolerance hit; a miss keeps the last good i.
        let mut i = self.ip_prev.clamp(0.0, i_max);
        let mut converged = false;
        for _ in 0..HARD_ITERS {
            let vpk = (e_plate - e_cath) - i * (r_plate + r_cath);
            let vgk = vg - (e_cath + r_cath * i);
            let (ip, dvpk, dvgk) = koren(vpk, vgk);
            let f = ip - i;
            let fp = dvpk * (-(r_plate + r_cath)) + dvgk * (-r_cath) - 1.0;
            let mut step = f / fp;
            // damp the step to at most i_max/2 to prevent overshoot oscillation
            let lim = 0.5 * i_max;
            if step > lim {
                step = lim;
            } else if step < -lim {
                step = -lim;
            }
            let i_new = (i - step).clamp(0.0, i_max);
            if (i_new - i).abs() < 1.0e-10 {
                i = i_new;
                converged = true;
                break;
            }
            i = i_new;
        }
        // safe fallback: if Newton didn't converge/diverged, keep the last good
        if !converged && !i.is_finite() {
            i = self.ip_prev;
        }
        i = i.clamp(0.0, i_max);
        self.ip_prev = i;
        // resolve node voltages
        let v_k = e_cath + r_cath * i;
        let v_p = self.vs - self.rp * i;
        // advance cathode reactive state (WDF incident) + supply companion
        self.cathode.incident(2.0 * v_k - e_cath);
        self.j_sup = 2.0 * g_csup * self.vs - self.j_sup;
        v_p
    }

    /// One bus sample through grid drive → triode → AC couple → tone stack.
    #[inline]
    fn tick_stage(&mut self, x: f64) -> f64 {
        let vg = self.cfg.drive_v * x;
        let vp = self.tick_triode(vg);
        // AC-couple (one-pole highpass = coupling cap + grid leak)
        let y = self.hp_c * (self.hp_y1 + vp - self.hp_x1);
        self.hp_x1 = vp;
        self.hp_y1 = flush_denormal(y as f32) as f64;
        // scale plate swing to bus level, then tone stack
        let s = self.hp_y1 * self.cfg.out_scale;
        let out = self.tone.process(s);
        // power-stage rail-load envelope (fast-ish follower of |output|; the
        // slow recovery lives in the supply R·C, not here)
        let a = out.abs();
        let c = if a > self.load_env { 0.02 } else { 0.0008 };
        self.load_env += c * (a - self.load_env);
        out
    }

    /// Process a mono block in place (bus signal, f32).
    pub fn process(&mut self, buf: &mut [f32]) {
        if !self.ready {
            return;
        }
        if self.cfg.oversample {
            for s in buf.iter_mut() {
                let (a, b) = self.hb.upsample(*s as f64);
                let ya = self.tick_stage(a);
                let yb = self.tick_stage(b);
                *s = self.hb.downsample(ya, yb) as f32;
            }
        } else {
            for s in buf.iter_mut() {
                *s = self.tick_stage(*s as f64) as f32;
            }
        }
    }

    /// Re-prepare with an updated config (tuning FFI; not on the audio path).
    pub fn set_config(&mut self, cfg: WdfAmpConfig, sr: f32) {
        self.prepare(cfg, sr);
    }

    /// Current config (so the tuning FFI can patch individual fields).
    pub fn config(&self) -> WdfAmpConfig {
        self.cfg
    }

    /// DC operating-point probe (for tests): returns (i_p mA, V_P, V_K).
    pub fn dc_operating_point(&mut self) -> (f64, f64, f64) {
        let mut vp = 0.0;
        for _ in 0..8000 {
            vp = self.tick_triode(0.0);
        }
        let v_k = self.cathode.reflected() + self.cathode.imp() * self.ip_prev;
        (self.ip_prev * 1e3, vp, v_k)
    }
}

impl Default for WdfAmp {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Voicing presets (chosen by fit against the same refs the behavioral chain
// was fit to — clean vs NSynth-028 cluster, lead vs FreePats dist2). Tuned in
// the match-reference loop; see the round report.
// ===========================================================================

/// Clean channel voicing (bright Fender-ish; gentle asymmetric compression +
/// slow rail sag for the singing sustain the owner asked for).
pub fn clean_config() -> WdfAmpConfig {
    WdfAmpConfig {
        drive_v: 2.2, // mostly clean with a touch of tube thickening
        out_scale: 0.020,
        tone: (0.52, 0.50, 0.80), // mid up: minimize the Bassman scoop for a
        // flatter, NSynth-cluster-like clean voicing
        supply_rc: (6000.0, 150.0e-6), // R·C ≈ 0.9 s recovery (rail-sag sustain)
        b_plus: 320.0,
        load_k: 0.35,
        // clean runs near-linear (drive 2.2); the single-note fizz measured
        // 0.00 at 1× so no oversampling is needed — saves half the cost.
        oversample: false,
    }
}

/// Lead channel voicing (high-gain: grid driven hard into cutoff AND plate
/// saturation → asymmetric near-square clip; deeper, slower rail sag).
pub fn lead_config() -> WdfAmpConfig {
    WdfAmpConfig {
        drive_v: 50.0,
        out_scale: 0.020,
        // dark voicing — the FreePats bridge-dist2 refs are dark (attack
        // centroid ~884 Hz); low treble + high bass rolls toward that character
        tone: (0.20, 0.65, 0.60),
        supply_rc: (7000.0, 90.0e-6), // R·C ≈ 0.63 s recovery
        b_plus: 320.0,
        load_k: 0.08,
        oversample: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptor_impedance_identities() {
        // series R = R1 + R2
        let s = Series::new(Resistor::new(1000.0), Resistor::new(2200.0));
        assert!((s.imp() - 3200.0).abs() < 1e-6);
        // parallel 1/R = 1/R1 + 1/R2
        let p = Parallel::new(Resistor::new(1000.0), Resistor::new(4000.0));
        assert!((p.imp() - 800.0).abs() < 1e-6);
        // capacitor impedance Z_C = 1/(2 C fs)
        let c = Capacitor::new(22e-6, 48000.0);
        assert!((c.imp() - 1.0 / (2.0 * 22e-6 * 48000.0)).abs() < 1e-9);
        // cathode network R = Rk ‖ Zc
        let cath = Parallel::new(Resistor::new(1.5e3), Capacitor::new(22e-6, 48000.0));
        let zc = 1.0 / (2.0 * 22e-6 * 48000.0);
        let expect = 1.0 / (1.0 / 1.5e3 + 1.0 / zc);
        assert!((cath.imp() - expect).abs() < 1e-9);
    }

    #[test]
    fn triode_dc_operating_point_physical() {
        // A 12AX7 common-cathode stage (B+ 320, Rp 100k, Rk 1.5k) must settle to
        // the textbook bias: i_p ~0.8-1.1 mA, V_P ~200-250 V, V_K ~1-2 V.
        for &sr in &[44100.0f32, 48000.0] {
            let mut amp = WdfAmp::new();
            amp.prepare(clean_config(), sr);
            let (ip_ma, vp, vk) = amp.dc_operating_point();
            assert!(ip_ma > 0.7 && ip_ma < 1.3, "i_p {ip_ma} mA @ {sr}");
            assert!(vp > 190.0 && vp < 260.0, "V_P {vp} @ {sr}");
            assert!(vk > 0.9 && vk < 2.2, "V_K {vk} @ {sr}");
        }
    }

    #[test]
    fn tonestack_magnitude_matches_yeh_analytic() {
        // Sweep the tone stack, measure magnitude via Goertzel, compare to the
        // analytic |H(s)| (Yeh Eq. 1) that the SPICE-verified paper curves match.
        let fs = 48000.0;
        let t = 0.5;
        let l = 0.5;
        let m = 0.5;
        let mut ts = ToneStack::default();
        ts.set(t, l, m, fs);
        let (b1, b2, b3, a1, a2, a3) = tonestack_s_coeffs(t, l, m);
        for &f in &[50.0f64, 200.0, 700.0, 2000.0, 5000.0] {
            // measured magnitude: drive a sine, settle, measure RMS ratio
            let mut ts2 = ts;
            ts2.reset();
            let n = (fs / f * 40.0) as usize;
            let mut num = 0.0;
            let mut den = 0.0;
            for k in 0..n {
                let ph = core::f64::consts::TAU * f * k as f64 / fs;
                let x = ph.sin();
                let y = ts2.process(x);
                if k > n / 2 {
                    num += y * y;
                    den += x * x;
                }
            }
            let meas_db = 10.0 * (num / den).log10();
            // analytic |H(jw)|
            let w = core::f64::consts::TAU * f;
            let jw = num_complex(0.0, w);
            let sn = cmul(b1_scaled(b1), jw)
                .add(cmul(b2, cmul(jw, jw)))
                .add(cmul(b3, cmul(jw, cmul(jw, jw))));
            let sd = C { re: 1.0, im: 0.0 }
                .add(cmul(a1, jw))
                .add(cmul(a2, cmul(jw, jw)))
                .add(cmul(a3, cmul(jw, cmul(jw, jw))));
            let anal_db = 20.0 * (cabs(cdiv(sn, sd))).log10();
            assert!(
                (meas_db - anal_db).abs() < 0.5,
                "f={f}: measured {meas_db:.2} dB vs analytic {anal_db:.2} dB"
            );
        }
    }

    // tiny complex helpers for the analytic tone-stack check
    #[derive(Clone, Copy)]
    struct C {
        re: f64,
        im: f64,
    }
    impl C {
        fn add(self, o: C) -> C {
            C {
                re: self.re + o.re,
                im: self.im + o.im,
            }
        }
    }
    fn num_complex(re: f64, im: f64) -> C {
        C { re, im }
    }
    fn b1_scaled(b1: f64) -> f64 {
        b1
    }
    fn cmul<T: Into<C>>(a: T, b: C) -> C {
        let a = a.into();
        C {
            re: a.re * b.re - a.im * b.im,
            im: a.re * b.im + a.im * b.re,
        }
    }
    impl From<f64> for C {
        fn from(x: f64) -> C {
            C { re: x, im: 0.0 }
        }
    }
    fn cabs(a: C) -> f64 {
        (a.re * a.re + a.im * a.im).sqrt()
    }
    fn cdiv(a: C, b: C) -> C {
        let d = b.re * b.re + b.im * b.im;
        C {
            re: (a.re * b.re + a.im * b.im) / d,
            im: (a.im * b.re - a.re * b.im) / d,
        }
    }

    #[test]
    fn triode_bounded_and_finite_under_hard_drive() {
        let mut amp = WdfAmp::new();
        amp.prepare(lead_config(), 48000.0);
        let mut buf = [0.0f32; 512];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = (i as f32 * 0.05).sin(); // full-scale drive
        }
        amp.process(&mut buf);
        for s in buf {
            assert!(s.is_finite());
            assert!(s.abs() < 8.0, "runaway sample {s}");
        }
    }
}
