#!/usr/bin/env python3
"""Testable metric kernel for render↔reference comparison.

Axes (never optimize a single number — see skills/match-reference):
  - log-mel spectrogram distance (overall + attack/mid/tail thirds)
  - spectral-centroid trajectory (attack, 0.3 s, 0.8 s)
  - envelope: onset time-to-peak, early/late decay t60s
  - partial structure: top peaks' frequency deviation (cents) and level deltas
  - integrated loudness delta (pyloudnorm, BS.1770)

Both files are mono-ized, resampled to the lower rate, and loudness-normalized
before spectral comparison (loudness reported separately, pre-normalization).
Dependencies are owned by scripts/dev/requirements-loop.txt. Reports are
versioned and content-addressed so an iteration can be reproduced exactly.
"""
import hashlib
import json
import os
import platform
from importlib.metadata import version

import numpy as np
import jsonschema
import pyloudnorm
from scipy.signal import resample_poly
import soundfile as sf


REPORT_SCHEMA_VERSION = "1.2.0"
METRIC_VERSION = "2026.07.12-l3.3"
SCHEMA_PATH = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "evals", "metrics", "report-schema-v1.json"))
_REPORT_SCHEMA = None


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def validate_report(report):
    """Validate the public metric-report contract before any caller sees it."""
    global _REPORT_SCHEMA
    if _REPORT_SCHEMA is None:
        with open(SCHEMA_PATH) as f:
            _REPORT_SCHEMA = json.load(f)
        jsonschema.Draft202012Validator.check_schema(_REPORT_SCHEMA)
    jsonschema.validate(instance=report, schema=_REPORT_SCHEMA)


def load_audio(path):
    x, sr = sf.read(path, always_2d=True)
    if x.shape[0] == 0:
        raise ValueError(f"audio file has zero frames: {path}")
    return x.astype(np.float64), sr


def load_mono(path):
    x, sr = load_audio(path)
    return x.mean(axis=1), sr


def resample_to(x, sr, target_sr):
    """Band-limited deterministic resampling with an exact output length.

    Linear interpolation aliases energy above the destination Nyquist into the
    comparison band, which can reward an artifact. scipy's polyphase FIR path
    applies the anti-alias filter before decimation.
    """
    if sr == target_sr:
        return x
    from math import gcd
    g = gcd(int(sr), int(target_sr))
    y = resample_poly(x, int(target_sr) // g, int(sr) // g, padtype="constant")
    n_out = int(round(len(x) * target_sr / sr))
    if len(y) < n_out:
        y = np.pad(y, (0, n_out - len(y)))
    return np.asarray(y[:n_out], dtype=np.float64)


def lufs(x, sr):
    if len(x) < sr // 2:
        return float("-inf")
    return pyloudnorm.Meter(sr).integrated_loudness(x)


# Per-instrument-class analysis profiles (owner 2026-07-12: "for some insts we
# need special STFT params, e.g. kick"). A kick fundamental (~50-70 Hz) sits in
# bins 1-2 of the default 1024-pt STFT and its beater attack is smeared by 21 ms
# windows — so the kick profile analyzes TWICE: a fine-time view for the attack
# and a long-window low-frequency view for the fundamental/tail.
COMMON_THRESHOLDS = {"max_sample_jump": 1.6, "max_clipping_occupancy": 1e-4, "max_peak": 1.05, "max_ultrasonic_ratio": 0.05, "max_dc_ratio": 0.01, "max_release_jump": 0.6, "max_warp_ms": 30.0, "trajectory_attack_end_ms": 50.0, "trajectory_body_end_ms": 500.0}
PROFILES = {
    "default": {"n": 1024, "hop": 256, "mels": 64, "fmin": 30.0, "thresholds": COMMON_THRESHOLDS},
    "kick":    {"n": 256,  "hop": 64,  "mels": 48, "fmin": 25.0,
                "thresholds": {**COMMON_THRESHOLDS, "max_warp_ms": 10.0, "trajectory_attack_end_ms": 30.0, "trajectory_body_end_ms": 300.0},
                "lf": {"n": 4096, "hop": 512, "mels": 40, "fmin": 20.0, "fmax": 400.0}},
    "cymbal":  {"n": 2048, "hop": 512, "mels": 72, "fmin": 100.0, "thresholds": {**COMMON_THRESHOLDS, "max_ultrasonic_ratio": 0.12}},
    "pitched": {"n": 1024, "hop": 256, "mels": 64, "fmin": 30.0, "thresholds": COMMON_THRESHOLDS},
}


def frames(x, n=1024, hop=256):
    if len(x) < n:
        x = np.pad(x, (0, n - len(x)))
    idx = np.arange(0, len(x) - n + 1, hop)
    w = np.hanning(n)
    return np.stack([x[i : i + n] * w for i in idx]), idx


def mel_bank(sr, n_fft, n_mels=64, fmin=30.0, fmax=None):
    fmax = fmax or 0.47 * sr
    def hz2mel(f):
        return 2595.0 * np.log10(1.0 + f / 700.0)
    def mel2hz(m):
        return 700.0 * (10 ** (m / 2595.0) - 1.0)
    pts = mel2hz(np.linspace(hz2mel(fmin), hz2mel(fmax), n_mels + 2))
    bins = np.floor((n_fft + 1) * pts / sr).astype(int)
    fb = np.zeros((n_mels, n_fft // 2 + 1))
    for m in range(n_mels):
        a, b, c = bins[m], bins[m + 1], bins[m + 2]
        if b > a:
            fb[m, a:b] = (np.arange(a, b) - a) / (b - a)
        if c > b:
            fb[m, b:c] = (c - np.arange(b, c)) / (c - b)
    return fb


def log_mel(x, sr, n=1024, hop=256, mels=64, fmin=30.0, fmax=None):
    F, _ = frames(x, n=n, hop=hop)
    spec = np.abs(np.fft.rfft(F, axis=1)) ** 2
    mel = spec @ mel_bank(sr, F.shape[1], n_mels=mels, fmin=fmin, fmax=fmax).T
    return np.log10(mel + 1e-10)


def k_weight(freqs):
    """BS.1770 K-weighting magnitude at `freqs` (Hz) — the same perceptual
    curve the loudness pipeline uses (pyloudnorm), evaluated from the
    standard's 48 kHz biquads. Returned as linear weights, floored at 0.2 so
    no band drops below 1/5 influence (deep-LF distances still count), then
    normalized to mean 1 so weighted distances stay scale-comparable."""
    w = 2.0 * np.pi * np.asarray(freqs, dtype=float) / 48000.0
    def mag(b, a):
        z = np.exp(-1j * w)
        return np.abs((b[0] + b[1] * z + b[2] * z * z) / (a[0] + a[1] * z + a[2] * z * z))
    shelf = mag([1.53512485958697, -2.69169618940638, 1.19839281085285],
                [1.0, -1.69065929318241, 0.73248077421585])
    rlb = mag([1.0, -2.0, 1.0], [1.0, -1.99004745483398, 0.99007225036621])
    k = np.maximum(shelf * rlb, 0.2)
    return k / k.mean()


def mel_centers(sr, n_mels, fmin, fmax=None):
    fmax = fmax or 0.47 * sr
    def hz2mel(f):
        return 2595.0 * np.log10(1.0 + f / 700.0)
    def mel2hz(m):
        return 700.0 * (10 ** (m / 2595.0) - 1.0)
    return mel2hz(np.linspace(hz2mel(fmin), hz2mel(fmax), n_mels + 2))[1:-1]


def logmel_dist(xr, xf, sr, perceptual=True, **kw):
    """Log-mel distance. `perceptual=True` (default, Keunwoo 2026-07-12)
    weights each mel band by K-weighting at its center frequency — bands the
    ear barely hears no longer count as much as the presence region. The LF
    zoom views (e.g. kick 20-400 Hz) are called with perceptual=False: their
    whole purpose is inspecting a band the weighted main view de-emphasizes."""
    mr, mf = log_mel(xr, sr, **kw), log_mel(xf, sr, **kw)
    t = min(len(mr), len(mf))
    d = np.abs(mr[:t] - mf[:t])
    if perceptual:
        cw = k_weight(mel_centers(sr, kw.get("mels", 64), kw.get("fmin", 30.0), kw.get("fmax")))
        d = d * cw[None, :]
    thirds = np.array_split(d, 3)
    return {
        "overall": round(float(d.mean()), 4),
        "attack": round(float(thirds[0].mean()), 4),
        "mid": round(float(thirds[1].mean()), 4),
        "tail": round(float(thirds[2].mean()), 4),
    }


def onset_align(xr, xf, sr, max_lag_s=0.012, search_s=0.1):
    """Bounded onset-envelope alignment; report rather than hide the shift.

    Alignment may remove capture latency, but it must not warp an attack. A lag
    outside max_lag_s fails closed and leaves both signals untouched.
    """
    n = min(len(xr), len(xf), int(search_s * sr))
    max_lag = max(1, int(max_lag_s * sr))
    if n <= 64:
        return xr, xf, {"status": "not_evaluated", "lag_samples": 0, "lag_ms": 0.0,
                        "max_lag_ms": round(max_lag_s * 1000, 3),
                        "reason": "insufficient_samples"}
    # Threshold a short moving-RMS envelope instead of raw periodic waveform
    # correlation. Raw correlation can choose a one-period or window-edge peak
    # even for identical pitched notes with leading silence.
    def onset(x):
        seg = x[:n]
        if float(np.std(seg)) <= 1e-12:
            return None
        width = max(4, int(0.0005 * sr))
        energy = np.convolve(seg * seg, np.ones(width) / width, mode="same")
        if float(energy.max()) <= 1e-18 or float(np.ptp(energy)) <= 1e-18:
            return None
        threshold = max(1e-16, 0.0004 * float(energy.max()))  # (2% amplitude)^2
        hit = np.flatnonzero(energy >= threshold)
        return int(hit[0]) if len(hit) else None
    onset_r, onset_f = onset(xr), onset(xf)
    if onset_r is None or onset_f is None:
        return xr, xf, {"status": "not_evaluated", "lag_samples": 0, "lag_ms": 0.0,
                        "max_lag_ms": round(max_lag_s * 1000, 3),
                        "reason": "silent_or_constant_onset_window"}
    lag = onset_r - onset_f
    if abs(lag) > max_lag:
        return xr, xf, {"status": "rejected", "method": "moving_rms_threshold",
                        "lag_samples": lag,
                        "lag_ms": round(1000 * lag / sr, 3),
                        "max_lag_ms": round(max_lag_s * 1000, 3)}
    if lag > 0:
        xr = xr[lag:]
    elif lag < 0:
        xf = xf[-lag:]
    return xr, xf, {"status": "applied", "method": "moving_rms_threshold",
                    "lag_samples": lag,
                    "lag_ms": round(1000 * lag / sr, 3),
                    "max_lag_ms": round(max_lag_s * 1000, 3)}


def mr_stft_dist(xr, xf, sr, perceptual=True, alignment=None):
    """Multi-resolution STFT distance (256/1024/4096, K-weighted).

    The caller may provide a precomputed bounded alignment so its operation is
    visible in report provenance. Direct callers get the same bounded default.
    """
    if alignment is None:
        xr, xf, alignment = onset_align(xr, xf, sr)
    out = {}
    for n in (256, 1024, 4096):
        d = logmel_dist(xr, xf, sr, perceptual=perceptual,
                        n=n, hop=n // 4, mels=min(64, n // 8), fmin=25.0)
        out[f"w{n}"] = d["overall"]
    out["mean"] = round(float(np.mean([out["w256"], out["w1024"], out["w4096"]])), 4)
    out["alignment"] = alignment
    return out


def artifact_gates(xr, xf, sr, sr_ref=None, expected_onset_s=None, note_off_s=None,
                   thresholds=None,
                   max_post_note_off_db=None):
    """Adversarial sanity gates: a red gate means the spectral distances are
    NOT to be trusted for this render (the loop twice optimized FOR artifacts
    the refs could not see — 2026-07-12 audit). All computed on the render,
    crest compared against the reference."""
    thresholds = thresholds or COMMON_THRESHOLDS
    if len(xr) == 0 or len(xf) == 0:
        raise ValueError(f"artifact gates require non-empty signals: render_frames={len(xr)}, reference_frames={len(xf)}")
    gates = {}
    finite = bool(np.isfinite(xr).all())
    gates["finite"] = {"value": finite, "pass": finite}
    if not finite:
        gates["all_pass"] = False
        gates["trusted"] = False
        return gates
    peak_render = float(np.abs(xr).max())
    peak_reference = float(np.abs(xf).max())
    has_signal = peak_render > 1e-9 and peak_reference > 1e-9
    gates["signal_energy"] = {"render_peak": round(peak_render, 12),
                              "reference_peak": round(peak_reference, 12),
                              "pass": has_signal}
    # onset crest: attack (first 3 ms after onset) vs body (10 ms..60%) —
    # render must not exceed the reference's crest by >6 dB (impulse/click
    # artifacts). Onset = first sample above 2% of peak (refs have lead-in
    # silence); both signals peak-referenced so loudness scaling cancels.
    def crest_db(x, rate):
        pk = float(np.abs(x).max()) + 1e-12
        on = int(np.argmax(np.abs(x) > 0.02 * pk))
        seg = x[on:]
        attack = seg[: max(1, int(rate * 0.003))]
        body = seg[int(rate * 0.01): max(int(len(seg) * 0.6), int(rate * 0.02))]
        a = float(np.abs(attack).max()) + 1e-9 if len(attack) else 1e-9
        b = float(np.abs(body).max()) + 1e-9 if len(body) else a
        return 20.0 * np.log10(a / b)
    cr, cf = crest_db(xr, sr), crest_db(xf, sr_ref or sr)
    gates["onset_crest_db"] = {"render": round(cr, 1), "reference": round(cf, 1),
                               "pass": bool(cr <= cf + 6.0)}
    # adjacent-sample jump relative to own peak: a near-full-scale single-sample
    # flip is always an artifact regardless of level (scale-invariant)
    pk = peak_render + 1e-12
    sample_diff = np.abs(np.diff(xr))
    j = float(sample_diff.max()) / pk if len(sample_diff) else 0.0
    gates["max_sample_jump"] = {"value": round(j, 3), "limit": thresholds["max_sample_jump"], "pass": bool(j < thresholds["max_sample_jump"])}
    # Hard clipping occupancy. A single true peak near 1 can be legitimate; a
    # run of samples pinned there is not. Render floats may exceed 1 in debug,
    # which also fails this gate.
    clipped = float(np.mean(np.abs(xr) >= 0.999))
    gates["clipping"] = {"occupancy": round(clipped, 6), "peak": round(pk, 5),
                         "occupancy_limit": thresholds["max_clipping_occupancy"], "peak_limit": thresholds["max_peak"],
                         "pass": bool(clipped <= thresholds["max_clipping_occupancy"] and pk <= thresholds["max_peak"])}
    # ultrasonic ratio: energy above 16 kHz vs 1-8 kHz band (needs sr > 32k);
    # references recorded at <=16 kHz cannot police this region
    if sr > 33000:  # native render rate — never gate on the resampled signal
        sp = np.abs(np.fft.rfft(xr * np.hanning(len(xr)))) ** 2
        fr = np.fft.rfftfreq(len(xr), 1 / sr)
        hi = float(sp[fr > 16000].sum())
        # denominator = TOTAL audible energy, not a mid band: LF-dominant
        # instruments (kick) have near-empty mids, which inflates a band ratio
        total = float(sp[fr <= 16000].sum()) + 1e-12
        r = hi / total
        gates["ultrasonic_ratio"] = {"value": round(r, 4), "limit": thresholds["max_ultrasonic_ratio"], "pass": bool(r < thresholds["max_ultrasonic_ratio"])}
    # DC offset
    dc = float(np.abs(np.mean(xr))) / pk
    gates["dc_offset"] = {"peak_ratio": round(dc, 5), "limit": thresholds["max_dc_ratio"], "pass": bool(dc < thresholds["max_dc_ratio"])}
    # Optional case-aware gates. L2 case manifests will make these required for
    # families where the reference declares a stable onset/note-off contract.
    if expected_onset_s is None:
        gates["pre_onset_energy"] = {"status": "not_evaluated", "pass": None}
    else:
        cut = max(0, min(len(xr), int(expected_onset_s * sr)))
        pre = xr[: max(0, cut - int(0.002 * sr))]
        body = xr[cut: min(len(xr), cut + int(0.1 * sr))]
        pre_rms = float(np.sqrt(np.mean(pre ** 2))) if len(pre) else 0.0
        body_rms = float(np.sqrt(np.mean(body ** 2))) + 1e-12 if len(body) else 1e-12
        pre_db = 20 * np.log10(pre_rms / body_rms + 1e-12)
        gates["pre_onset_energy"] = {"relative_db": round(float(pre_db), 1),
                                     "pass": bool(pre_db <= -35.0)}
    if note_off_s is None:
        gates["release_discontinuity"] = {"status": "not_evaluated", "pass": None}
        gates["post_note_off_energy"] = {"status": "not_evaluated", "pass": None}
    else:
        at = max(1, min(len(xr) - 2, int(note_off_s * sr)))
        radius = max(2, int(0.002 * sr))
        lo, hi = max(1, at - radius), min(len(xr) - 1, at + radius)
        release_diff = np.abs(np.diff(xr[lo:hi]))
        release_jump = float(release_diff.max()) / pk if len(release_diff) else 0.0
        gates["release_discontinuity"] = {"peak_ratio": round(release_jump, 4),
                                          "limit": thresholds["max_release_jump"],
                                          "pass": bool(release_jump < thresholds["max_release_jump"])}
        post = xr[at:]
        post_rms = float(np.sqrt(np.mean(post ** 2))) / pk if len(post) else 0.0
        post_db = 20 * np.log10(post_rms + 1e-12)
        gates["post_note_off_energy"] = {
            "relative_db": round(float(post_db), 1),
            "limit_db": max_post_note_off_db,
            "status": "not_evaluated" if max_post_note_off_db is None else "evaluated",
            "pass": None if max_post_note_off_db is None else bool(post_db <= max_post_note_off_db),
        }
    # Ignore explicitly unavailable optional gates in the aggregate; any real
    # failure invalidates interpretation of every downstream distance.
    evaluated = [v["pass"] for v in gates.values()
                 if isinstance(v, dict) and v.get("pass") is not None]
    gates["all_pass"] = bool(all(evaluated))
    gates["trusted"] = gates["all_pass"]
    return gates


def crest_factor(x):
    rms = float(np.sqrt(np.mean(x ** 2)) + 1e-12)
    return round(float(np.max(np.abs(x))) / rms, 2)


def glide_track(x, sr, ms=150, fmax=250.0):
    """Kick pitch track over the first `ms`: LP the signal, autocorr per 10 ms hop."""
    seg = x[: int(ms * 1e-3 * sr)]
    # crude one-pole LP at fmax*2 to isolate the fundamental
    c = 1.0 - np.exp(-2 * np.pi * fmax * 2 / sr)
    lp = np.zeros_like(seg)
    acc = 0.0
    for i, v in enumerate(seg):
        acc += c * (v - acc)
        lp[i] = acc
    hop = int(0.010 * sr)
    out = []
    for i in range(0, len(lp) - hop * 3, hop):
        w = lp[i : i + hop * 3]
        if np.max(np.abs(w)) < 1e-4:
            out.append(None)
            continue
        min_lag, max_lag = int(sr / fmax), int(sr / 25.0)
        max_lag = min(max_lag, len(w) - 1)
        if max_lag <= min_lag:
            out.append(None)
            continue
        ac = [float(np.dot(w[:-lag], w[lag:])) for lag in range(min_lag, max_lag)]
        lag = int(np.argmax(ac)) + min_lag
        out.append(round(sr / lag, 1))
    return out


def centroid_traj(x, sr):
    F, idx = frames(x)
    spec = np.abs(np.fft.rfft(F, axis=1))
    freqs = np.fft.rfftfreq(F.shape[1], 1 / sr)
    keep = freqs <= 0.47 * sr
    c = (spec[:, keep] ** 2 @ freqs[keep]) / (np.sum(spec[:, keep] ** 2, axis=1) + 1e-12)
    times = idx / sr
    def at(t):
        i = np.argmin(np.abs(times - t))
        return float(c[i])
    return {"attack": at(0.03), "t03": at(0.3), "t08": at(0.8)}


def envelope_stats(x, sr):
    hop = int(0.05 * sr)
    n = len(x) // hop
    env = np.sqrt(np.mean(x[: n * hop].reshape(n, hop) ** 2, axis=1) + 1e-12)
    peak_i = int(np.argmax(env))
    db = 20 * np.log10(env + 1e-9)
    def t60(t0, t1):
        i0, i1 = int(t0 / 0.05), int(t1 / 0.05)
        if i0 >= len(db) or i1 >= len(db) or db[i0] <= db[i1] + 0.5:
            return None
        rate = (db[i0] - db[i1]) / (t1 - t0)
        return round(60 / rate, 2) if rate > 0.1 else None
    return {
        "time_to_peak_ms": round(peak_i * 50.0, 1),
        "t60_early": t60(0.1, 0.4),
        "t60_late": t60(0.8, min(1.8, (len(db) - 1) * 0.05)),
    }


def rms_trajectory(x, sr, window_s=0.02, hop_s=0.01):
    window = max(8, int(window_s * sr))
    hop = max(1, int(hop_s * sr))
    if len(x) < window:
        x = np.pad(x, (0, window - len(x)))
    starts = range(0, len(x) - window + 1, hop)
    rms = np.array([np.sqrt(np.mean(x[i:i + window] ** 2) + 1e-18) for i in starts])
    db = 20 * np.log10(rms / (rms.max() + 1e-18) + 1e-9)
    return [round(float(v), 3) for v in np.clip(db, -80, 0)]


def centroid_trajectory(x, sr, n=1024, hop_s=0.01):
    hop = max(1, int(hop_s * sr))
    F, _ = frames(x, n=n, hop=hop)
    power = np.abs(np.fft.rfft(F, axis=1)) ** 2
    freqs = np.fft.rfftfreq(n, 1 / sr)
    energy = power.sum(axis=1)
    centroid = (power @ freqs) / (energy + 1e-18)
    centroid[energy < max(1e-18, energy.max() * 1e-8)] = 0.0
    return [round(float(v), 2) for v in centroid]


def bounded_dtw(a, b, max_warp_frames):
    """Banded DTW with explicit path displacement; no unconstrained hiding of defects."""
    a = np.asarray(a, dtype=float)
    b = np.asarray(b, dtype=float)
    if not len(a) or not len(b):
        return {"cost": None, "max_displacement_frames": None, "path_length": 0}
    width = max(int(max_warp_frames), abs(len(a) - len(b)))
    dp = np.full((len(a) + 1, len(b) + 1), np.inf)
    prev = np.zeros((len(a) + 1, len(b) + 1), dtype=np.int8)
    dp[0, 0] = 0.0
    for i in range(1, len(a) + 1):
        for j in range(max(1, i - width), min(len(b), i + width) + 1):
            choices = (dp[i - 1, j - 1], dp[i - 1, j], dp[i, j - 1])
            move = int(np.argmin(choices))
            dp[i, j] = choices[move] + abs(a[i - 1] - b[j - 1])
            prev[i, j] = move
    if not np.isfinite(dp[-1, -1]):
        return {"cost": None, "max_displacement_frames": None, "path_length": 0}
    i, j, length, displacement = len(a), len(b), 0, 0
    while i > 0 or j > 0:
        displacement = max(displacement, abs(i - j))
        move = prev[i, j]
        if move == 0:
            i, j = i - 1, j - 1
        elif move == 1:
            i -= 1
        else:
            j -= 1
        length += 1
    return {"cost": round(float(dp[-1, -1] / max(1, length)), 4),
            "max_displacement_frames": int(displacement), "path_length": length}


def partial_targets(f0, count=16, model=None):
    """Return declared modal targets without assuming every source is harmonic.

    `proximity_harmonic` finds a local peak around integer multiples, `stiff_string`
    applies the standard inharmonic string coefficient B, and `modal_ratios`
    accepts measured/profile-owned ratios for membranes or bars.
    """
    if not f0 or f0 <= 0:
        return []
    model = model or {"type": "proximity_harmonic", "search_cents": 70.0}
    kind = model.get("type", "proximity_harmonic")
    if kind == "modal_ratios":
        ratios = [float(value) for value in model.get("ratios", [])]
    elif kind == "stiff_string":
        coefficient = float(model.get("inharmonicity_b", 0.0))
        ratios = [n * np.sqrt((1.0 + coefficient * n * n) / (1.0 + coefficient))
                  for n in range(1, count + 1)]
    elif kind == "proximity_harmonic":
        ratios = [float(n) for n in range(1, count + 1)]
    else:
        raise ValueError(f"unknown partial model: {kind}")
    return [{"partial": index, "ratio": ratio, "frequency_hz": float(f0 * ratio)}
            for index, ratio in enumerate(ratios[:count], start=1)]


def harmonic_decay_trajectories(x, sr, f0, count=6, window_s=0.08, hop_s=0.05,
                                partial_model=None):
    if not f0 or f0 <= 0:
        return {}
    window = max(32, int(window_s * sr))
    hop = max(1, int(hop_s * sr))
    starts = range(0, max(1, len(x) - window + 1), hop)
    local_t = np.arange(window) / sr
    result = {}
    for target in partial_targets(f0, count, partial_model):
        partial = target["partial"]
        freq = target["frequency_hz"]
        if freq >= 0.47 * sr:
            break
        osc = np.exp(-2j * np.pi * freq * local_t) * np.hanning(window)
        values = []
        for start in starts:
            segment = x[start:start + window]
            if len(segment) < window:
                segment = np.pad(segment, (0, window - len(segment)))
            values.append(abs(np.dot(segment, osc)))
        values = np.asarray(values)
        db = 20 * np.log10(values / (values.max() + 1e-18) + 1e-9)
        result[str(partial)] = [round(float(v), 3) for v in np.clip(db, -80, 0)]
    return result


def trajectory_region_distances(render, reference, width, attack_end_ms, body_end_ms, hop_ms=10.0):
    attack_end = max(1, int(round(attack_end_ms / hop_ms)))
    body_end = max(attack_end + 1, int(round(body_end_ms / hop_ms)))
    regions = {
        "attack": (0, attack_end),
        "body": (attack_end, body_end),
        "tail": (body_end, max(len(render), len(reference))),
    }
    return {name: bounded_dtw(render[start:end], reference[start:end], width)
            for name, (start, end) in regions.items()}


def trajectory_diagnostics(xr, xf, sr, max_warp_ms, expected_f0=None, partial_model=None,
                           attack_end_ms=50.0, body_end_ms=500.0):
    hop_ms = 10.0
    width = max(0, int(round(max_warp_ms / hop_ms)))
    er, ef = rms_trajectory(xr, sr), rms_trajectory(xf, sr)
    cr, cf = centroid_trajectory(xr, sr), centroid_trajectory(xf, sr)
    # Centroid error is perceptually closer to pitch intervals than raw Hz.
    def log_centroid(values):
        return [12 * np.log2(max(v, 20.0) / 20.0) for v in values]
    result = {
        "hop_ms": hop_ms,
        "max_warp_ms": max_warp_ms,
        "envelope_db": {"render": er, "reference": ef, "distance": bounded_dtw(er, ef, width),
                        "regions": trajectory_region_distances(er, ef, width, attack_end_ms, body_end_ms)},
        "centroid_hz": {"render": cr, "reference": cf,
                         "distance_semitones": bounded_dtw(log_centroid(cr), log_centroid(cf), width),
                         "regions_semitones": trajectory_region_distances(log_centroid(cr), log_centroid(cf), width, attack_end_ms, body_end_ms)},
    }
    if expected_f0:
        pr = harmonic_decay_trajectories(xr, sr, expected_f0, partial_model=partial_model)
        pf = harmonic_decay_trajectories(xf, sr, expected_f0, partial_model=partial_model)
        partial_width = max(0, int(round(max_warp_ms / 50.0)))
        result["partial_decay_db"] = {
            "hop_ms": 50.0,
            "render": pr,
            "reference": pf,
            "distances": {harmonic: bounded_dtw(pr[harmonic], pf[harmonic], partial_width)
                          for harmonic in sorted(set(pr) & set(pf), key=int)},
        }
    else:
        result["partial_decay_db"] = None
    return result


def harmonic_partials(x, sr, f0, t0=0.08, dur=0.5, count=16, partial_model=None):
    if not f0 or f0 <= 0:
        return []
    seg = x[int(t0 * sr):int((t0 + dur) * sr)]
    if len(seg) < 512:
        return []
    n = 1 << int(np.ceil(np.log2(len(seg))))
    spec = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), n=n))
    freqs = np.fft.rfftfreq(n, 1 / sr)
    out = []
    model = partial_model or {"type": "proximity_harmonic", "search_cents": 70.0}
    search_cents = float(model.get("search_cents", 70.0))
    ratio = 2.0 ** (search_cents / 1200.0)
    for target_info in partial_targets(f0, count, model):
        partial = target_info["partial"]
        target = target_info["frequency_hz"]
        if target >= 0.47 * sr:
            break
        lo, hi = target / ratio, target * ratio
        idx = np.flatnonzero((freqs >= lo) & (freqs <= hi))
        if not len(idx):
            continue
        peak = int(idx[np.argmax(spec[idx])])
        out.append({"harmonic": partial, "target_hz": round(target, 3),
                    "target_ratio": round(target_info["ratio"], 8),
                    "frequency_hz": round(float(freqs[peak]), 2), "magnitude": float(spec[peak])})
    ref = max((item["magnitude"] for item in out), default=1.0) or 1.0
    for item in out:
        item["level_db"] = round(float(20 * np.log10(item.pop("magnitude") / ref + 1e-12)), 2)
    return [item for item in out if item["level_db"] >= -80.0]


def match_harmonic_partials(xr, xf, sr, f0, partial_model=None):
    model = partial_model or {"type": "proximity_harmonic", "search_cents": 70.0}
    render = harmonic_partials(xr, sr, f0, partial_model=model)
    reference = harmonic_partials(xf, sr, f0, partial_model=model)
    rr = {item["harmonic"]: item for item in render}
    rf = {item["harmonic"]: item for item in reference}
    pairs = []
    for harmonic in sorted(set(rr) & set(rf)):
        a, b = rr[harmonic], rf[harmonic]
        pairs.append({"harmonic": harmonic, "render_hz": a["frequency_hz"], "reference_hz": b["frequency_hz"],
                      "cents_delta": round(float(1200 * np.log2(a["frequency_hz"] / b["frequency_hz"])), 2),
                      "level_delta_db": round(a["level_db"] - b["level_db"], 2)})
    return {"expected_f0_hz": f0, "partial_model": model, "render": render, "reference": reference, "pairs": pairs,
            "mean_abs_cents": round(float(np.mean([abs(p["cents_delta"]) for p in pairs])), 2) if pairs else None,
            "mean_abs_level_db": round(float(np.mean([abs(p["level_delta_db"]) for p in pairs])), 2) if pairs else None}


def stereo_stats(audio):
    if audio.shape[1] < 2:
        return {"channels": 1, "width_db": None, "correlation": None}
    left, right = audio[:, 0], audio[:, 1]
    if float(np.max(np.abs(audio))) <= 1e-9:
        return {"channels": audio.shape[1], "width_db": None, "correlation": None}
    mid = (left + right) * 0.5
    side = (left - right) * 0.5
    width = 20 * np.log10((np.sqrt(np.mean(side ** 2)) + 1e-12) / (np.sqrt(np.mean(mid ** 2)) + 1e-12))
    corr = np.corrcoef(left, right)[0, 1] if np.std(left) > 1e-12 and np.std(right) > 1e-12 else 1.0
    return {"channels": 2, "width_db": round(float(width), 3), "correlation": round(float(corr), 5)}


def partials(x, sr, t0=0.25, dur=0.5, top=12):
    seg = x[int(t0 * sr) : int((t0 + dur) * sr)]
    if len(seg) < 2048:
        return []
    n = 1 << int(np.ceil(np.log2(len(seg))))
    spec = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), n=n))
    freqs = np.fft.rfftfreq(n, 1 / sr)
    peaks = []
    for i in range(2, len(spec) - 2):
        if spec[i] > spec[i - 1] and spec[i] > spec[i + 1] and freqs[i] > 25:
            peaks.append((float(freqs[i]), float(spec[i])))
    peaks.sort(key=lambda p: -p[1])
    out, taken = [], []
    for f, m in peaks:
        if all(abs(f - g) > 20 for g in taken):
            out.append((round(f, 1), m))
            taken.append(f)
        if len(out) >= top:
            break
    ref = max(m for _, m in out) if out else 1.0
    return [(f, round(20 * np.log10(m / ref + 1e-12), 1)) for f, m in sorted(out)]


def partial_decay(x, sr, top=6):
    """Per-partial decay rates (dB/s) via heterodyne envelopes of the strongest
    early partials — the sharpest decay diagnostic (fleet round 1 lesson)."""
    peaks = partials(x, sr, t0=0.08, dur=0.25, top=top)
    out = []
    for f, _ in peaks:
        t_axis = np.arange(len(x)) / sr
        osc = np.exp(-2j * np.pi * f * t_axis)
        prod = x * osc
        hop = int(0.05 * sr)
        n = len(prod) // hop
        if n < 8:
            return out
        env = np.abs(prod[: n * hop].reshape(n, hop).mean(axis=1))
        db = 20 * np.log10(env + 1e-9)
        i0, i1 = 3, min(n - 1, 20)
        rate = (db[i0] - db[i1]) / ((i1 - i0) * 0.05)
        out.append((round(f, 1), round(float(rate), 1)))
    return out


def disable_invalid_axes(report, invalid_axes):
    """Fail closed: remove values a corpus contract says cannot be trusted."""
    if not invalid_axes:
        report["axis_validity"] = {}
        return
    report["axis_validity"] = {
        axis: {"valid": False, "reason": reason}
        for axis, reason in sorted(invalid_axes.items())
    }
    if "lufs" in invalid_axes:
        report["lufs"] = {"valid": False, "reason": invalid_axes["lufs"],
                          "render": None, "reference": None, "delta": None}
    if "decay" in invalid_axes:
        report["partial_decay_dbps"] = {"valid": False,
                                         "reason": invalid_axes["decay"],
                                         "render": None, "reference": None}
        for side in ("render", "reference"):
            report["envelope"][side]["t60_early"] = None
            report["envelope"][side]["t60_late"] = None
        report["trajectories"]["envelope_db"]["distance"] = None
        if report["trajectories"].get("partial_decay_db"):
            report["trajectories"]["partial_decay_db"]["distances"] = None
    if "attack" in invalid_axes:
        report["logmel_dist"]["attack"] = None
        for side in ("render", "reference"):
            report["centroid"][side]["attack"] = None
            report["envelope"][side]["time_to_peak_ms"] = None
        report["trajectories"]["centroid_hz"]["distance_semitones"] = None
    if "tail" in invalid_axes:
        report["logmel_dist"]["tail"] = None


def compare_files(render_path, ref_path, profile="default", flat=False,
                  expected_onset_s=None, note_off_s=None,
                  max_post_note_off_db=None, expected_f0=None, partial_model=None,
                  reference_contract=None):
    prof = PROFILES.get(profile, PROFILES["default"])
    audio_r, sr_r = load_audio(render_path)
    audio_f, sr_f = load_audio(ref_path)
    xr, xf = audio_r.mean(axis=1), audio_f.mean(axis=1)
    native_frames = {"render": len(xr), "reference": len(xf)}
    gates = artifact_gates(xr, xf, sr_r, sr_ref=sr_f,
                           expected_onset_s=expected_onset_s,
                           note_off_s=note_off_s,
                           thresholds=prof["thresholds"],
                           max_post_note_off_db=max_post_note_off_db)
    if not gates["finite"]["pass"]:
        raise ValueError("render contains NaN or infinite samples; comparison aborted")
    ref_meta = reference_contract["contract"] if reference_contract else {}
    operations = []
    if ref_meta and ref_meta.get("mask_after_s"):
        # hard release gates in the corpus tax physically-correct tails: truncate
        m = ref_meta["mask_after_s"]
        xr, xf = xr[: int(m * sr_r)], xf[: int(m * sr_f)]
        operations.append({"operation": "mask_after_s", "value": m})
    sr = min(sr_r, sr_f)
    xr, xf = resample_to(xr, sr_r, sr), resample_to(xf, sr_f, sr)
    if sr_r != sr:
        operations.append({"operation": "resample_render", "from_sr": sr_r, "to_sr": sr,
                           "method": "scipy.signal.resample_poly"})
    if sr_f != sr:
        operations.append({"operation": "resample_reference", "from_sr": sr_f, "to_sr": sr,
                           "method": "scipy.signal.resample_poly"})
    l_r, l_f = lufs(xr, sr), lufs(xf, sr)
    # loudness-normalize render to reference for spectral comparison
    if np.isfinite(l_r) and np.isfinite(l_f):
        gain_db = l_f - l_r
        xr = xr * (10 ** (gain_db / 20))
        operations.append({"operation": "loudness_normalize_render", "gain_db": round(gain_db, 6)})
    n = min(len(xr), len(xf))
    xr, xf = xr[:n], xf[:n]

    ar, af, alignment = onset_align(xr, xf, sr)
    lm = logmel_dist(xr, xf, sr, perceptual=not flat,
                     n=prof["n"], hop=prof["hop"], mels=prof["mels"], fmin=prof["fmin"])
    gates["alignment"] = {"status": alignment["status"],
                          "pass": alignment["status"] == "applied"}
    evaluated = [v["pass"] for v in gates.values()
                 if isinstance(v, dict) and v.get("pass") is not None]
    gates["all_pass"] = bool(all(evaluated))
    gates["trusted"] = gates["all_pass"]
    trajectory_start = int((expected_onset_s or 0.0) * sr)
    trajectory_r = ar[min(len(ar), trajectory_start):]
    trajectory_f = af[min(len(af), trajectory_start):]
    report = {
        "schema_version": REPORT_SCHEMA_VERSION,
        "metric_version": METRIC_VERSION,
        "interpretation": "trusted" if gates["trusted"] else "untrusted",
        "inputs": {
            "render": {"path": str(render_path), "sha256": file_sha256(render_path),
                       "sample_rate": sr_r, "frames": native_frames["render"]},
            "reference": {"path": str(ref_path), "sha256": file_sha256(ref_path),
                          "sample_rate": sr_f, "frames": native_frames["reference"]},
        },
        "runtime": {
            "python": platform.python_version(),
            "numpy": version("numpy"),
            "scipy": version("scipy"),
            "soundfile": version("soundfile"),
            "pyloudnorm": version("pyloudnorm"),
            "jsonschema": version("jsonschema"),
        },
        "configuration": {
            "profile": profile,
            "weighting": "flat" if flat else "K (BS.1770)",
            "expected_onset_s": expected_onset_s,
            "note_off_s": note_off_s,
            "max_post_note_off_db": max_post_note_off_db,
            "expected_f0": expected_f0,
            "partial_model": partial_model,
            "thresholds": prof["thresholds"],
        },
        "operations": operations,
        "profile": profile,
        "partial_decay_dbps": {
            "render": partial_decay(xr, sr),
            "reference": partial_decay(xf, sr),
        },
        "sr": sr,
        "seconds": round(n / sr, 2),
        "lufs": {
            "render": round(l_r, 1) if np.isfinite(l_r) else None,
            "reference": round(l_f, 1) if np.isfinite(l_f) else None,
            "delta": round(l_r - l_f, 1) if np.isfinite(l_r) and np.isfinite(l_f) else None,
        },
        "logmel_dist": lm,
        "weighting": "flat" if flat else "K (BS.1770)",
        "reference_contract": reference_contract["evidence"] if reference_contract else None,
        "centroid": {"render": centroid_traj(xr, sr), "reference": centroid_traj(xf, sr)},
        "envelope": {"render": envelope_stats(xr, sr), "reference": envelope_stats(xf, sr)},
        "partials": {"render": partials(xr, sr), "reference": partials(xf, sr)},
        "crest": {"render": crest_factor(xr), "reference": crest_factor(xf)},
        "mr_stft": mr_stft_dist(ar, af, sr, perceptual=not flat, alignment=alignment),
        "trajectories": trajectory_diagnostics(
            trajectory_r, trajectory_f, sr, prof["thresholds"]["max_warp_ms"], expected_f0, partial_model,
            prof["thresholds"]["trajectory_attack_end_ms"], prof["thresholds"]["trajectory_body_end_ms"],
        ),
        "harmonic_partials": match_harmonic_partials(xr, xf, sr, expected_f0, partial_model) if expected_f0 else None,
        "stereo": {"render": stereo_stats(audio_r), "reference": stereo_stats(audio_f)},
        "gates": gates,
    }
    if "lf" in prof:
        lf = prof["lf"]
        report["logmel_lf"] = logmel_dist(xr, xf, sr, perceptual=False, n=lf["n"], hop=lf["hop"],
                                          mels=lf["mels"], fmin=lf["fmin"], fmax=lf["fmax"])
        report["glide_hz"] = {"render": glide_track(xr, sr), "reference": glide_track(xf, sr)}
    disable_invalid_axes(report, ref_meta.get("invalid_axes", {}))
    validate_report(report)
    return report
