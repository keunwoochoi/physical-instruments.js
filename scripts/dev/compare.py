#!/usr/bin/env python3
"""Multi-axis render↔reference comparison for the match-reference loop.

Usage: python3 scripts/dev/compare.py <render.wav> <reference.wav> [--json]

Axes (never optimize a single number — see skills/match-reference):
  - log-mel spectrogram distance (overall + attack/mid/tail thirds)
  - spectral-centroid trajectory (attack, 0.3 s, 0.8 s)
  - envelope: onset time-to-peak, early/late decay t60s
  - partial structure: top peaks' frequency deviation (cents) and level deltas
  - integrated loudness delta (pyloudnorm, BS.1770)

Both files are mono-ized, resampled to the lower rate, and loudness-normalized
before spectral comparison (loudness reported separately, pre-normalization).
Self-contained: numpy + soundfile + pyloudnorm only.
"""
import json
import sys

import numpy as np
import pyloudnorm
import soundfile as sf


def load_mono(path):
    x, sr = sf.read(path, always_2d=True)
    return x.mean(axis=1).astype(np.float64), sr


def resample_to(x, sr, target_sr):
    if sr == target_sr:
        return x
    n_out = int(round(len(x) * target_sr / sr))
    t_in = np.arange(len(x)) / sr
    t_out = np.arange(n_out) / target_sr
    return np.interp(t_out, t_in, x)


def lufs(x, sr):
    if len(x) < sr // 2:
        return float("-inf")
    return pyloudnorm.Meter(sr).integrated_loudness(x)


# Per-instrument-class analysis profiles (owner 2026-07-12: "for some insts we
# need special STFT params, e.g. kick"). A kick fundamental (~50-70 Hz) sits in
# bins 1-2 of the default 1024-pt STFT and its beater attack is smeared by 21 ms
# windows — so the kick profile analyzes TWICE: a fine-time view for the attack
# and a long-window low-frequency view for the fundamental/tail.
PROFILES = {
    "default": {"n": 1024, "hop": 256, "mels": 64, "fmin": 30.0},
    "kick":    {"n": 256,  "hop": 64,  "mels": 48, "fmin": 25.0,
                "lf": {"n": 4096, "hop": 512, "mels": 40, "fmin": 20.0, "fmax": 400.0}},
    "cymbal":  {"n": 2048, "hop": 512, "mels": 72, "fmin": 100.0},
    "pitched": {"n": 1024, "hop": 256, "mels": 64, "fmin": 30.0},
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


def mr_stft_dist(xr, xf, sr, perceptual=True):
    """Multi-resolution STFT distance (256/1024/4096, K-weighted, onset-aligned):
    single-window metrics miss transient-vs-tonal trades. Additive axis for now
    (agents mid-round keep logmel continuity); becomes the headline next round."""
    # onset-align: cross-correlate first 50 ms envelopes so micro-timing
    # differences don't pollute timbre distance
    n50 = int(0.05 * sr)
    er, ef = np.abs(xr[:n50]), np.abs(xf[:n50])
    if len(er) == len(ef) and len(er) > 64:
        xc = np.correlate(er - er.mean(), ef - ef.mean(), mode="full")
        lag = int(np.argmax(xc)) - (len(er) - 1)
        if 0 < lag < n50:
            xr = xr[lag:]
        elif -n50 < lag < 0:
            xf = xf[-lag:]
    out = {}
    for n in (256, 1024, 4096):
        d = logmel_dist(xr, xf, sr, perceptual=perceptual,
                        n=n, hop=n // 4, mels=min(64, n // 8), fmin=25.0)
        out[f"w{n}"] = d["overall"]
    out["mean"] = round(float(np.mean([out["w256"], out["w1024"], out["w4096"]])), 4)
    return out


def artifact_gates(xr, xf, sr, sr_ref=None):
    """Adversarial sanity gates: a red gate means the spectral distances are
    NOT to be trusted for this render (the loop twice optimized FOR artifacts
    the refs could not see — 2026-07-12 audit). All computed on the render,
    crest compared against the reference."""
    gates = {}
    # onset crest: attack (first 3 ms after onset) vs body (10 ms..60%) —
    # render must not exceed the reference's crest by >6 dB (impulse/click
    # artifacts). Onset = first sample above 2% of peak (refs have lead-in
    # silence); both signals peak-referenced so loudness scaling cancels.
    def crest_db(x):
        pk = float(np.abs(x).max()) + 1e-12
        on = int(np.argmax(np.abs(x) > 0.02 * pk))
        seg = x[on:]
        a = float(np.abs(seg[: int(sr * 0.003)]).max()) + 1e-9
        b = float(np.abs(seg[int(sr * 0.01): max(int(len(seg) * 0.6), int(sr * 0.02))]).max()) + 1e-9
        return 20.0 * np.log10(a / b)
    cr, cf = crest_db(xr), crest_db(xf)
    gates["onset_crest_db"] = {"render": round(cr, 1), "reference": round(cf, 1),
                               "pass": bool(cr <= cf + 6.0)}
    # adjacent-sample jump relative to own peak: a near-full-scale single-sample
    # flip is always an artifact regardless of level (scale-invariant)
    pk = float(np.abs(xr).max()) + 1e-12
    j = float(np.abs(np.diff(xr)).max()) / pk
    gates["max_sample_jump"] = {"value": round(j, 3), "pass": bool(j < 1.6)}
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
        gates["ultrasonic_ratio"] = {"value": round(r, 4), "pass": bool(r < 0.05)}
    # DC offset
    dc = float(np.abs(np.mean(xr)))
    gates["dc_offset"] = {"value": round(dc, 5), "pass": bool(dc < 0.01)}
    gates["all_pass"] = bool(all(v["pass"] for v in gates.values() if isinstance(v, dict)))
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


def manifest_lookup(ref_path):
    """Match ref_path against evals/reference-manifest.json `match` substrings.
    Returns the corpus entry (known limitations: sr ceiling, level normalization,
    release-gate mask) or None. Fit with eyes open."""
    import os
    here = os.path.dirname(os.path.abspath(__file__))
    mpath = os.path.join(here, "..", "..", "evals", "reference-manifest.json")
    try:
        with open(mpath) as f:
            man = json.load(f)
    except OSError:
        return None
    p = str(ref_path)
    for c in man.get("corpora", []):
        if c.get("match") and c["match"] in p:
            return c
    return None


def main():
    render_path, ref_path = sys.argv[1], sys.argv[2]
    profile = "default"
    if "--profile" in sys.argv:
        profile = sys.argv[sys.argv.index("--profile") + 1]
    prof = PROFILES.get(profile, PROFILES["default"])
    xr, sr_r = load_mono(render_path)
    xf, sr_f = load_mono(ref_path)
    gates = artifact_gates(xr, xf, sr_r, sr_ref=sr_f)  # native rate, pre-resample/pre-normalize
    ref_meta = manifest_lookup(ref_path)
    if ref_meta and ref_meta.get("mask_after_s"):
        # hard release gates in the corpus tax physically-correct tails: truncate
        m = ref_meta["mask_after_s"]
        xr, xf = xr[: int(m * sr_r)], xf[: int(m * sr_f)]
    sr = min(sr_r, sr_f)
    xr, xf = resample_to(xr, sr_r, sr), resample_to(xf, sr_f, sr)
    l_r, l_f = lufs(xr, sr), lufs(xf, sr)
    # loudness-normalize render to reference for spectral comparison
    if np.isfinite(l_r) and np.isfinite(l_f):
        xr = xr * (10 ** ((l_f - l_r) / 20))
    n = min(len(xr), len(xf))
    xr, xf = xr[:n], xf[:n]

    flat = "--flat-weighting" in sys.argv
    lm = logmel_dist(xr, xf, sr, perceptual=not flat,
                     n=prof["n"], hop=prof["hop"], mels=prof["mels"], fmin=prof["fmin"])
    report = {
        "profile": profile,
        "partial_decay_dbps": {
            "render": partial_decay(xr, sr),
            "reference": partial_decay(xf, sr),
        },
        "sr": sr,
        "seconds": round(n / sr, 2),
        "lufs": {"render": round(l_r, 1), "reference": round(l_f, 1), "delta": round(l_r - l_f, 1)},
        "logmel_dist": lm,
        "weighting": "flat" if flat else "K (BS.1770)",
        "ref_corpus": ({k: ref_meta[k] for k in ("corpus", "sr", "level_normalized", "mask_after_s", "notes") if k in ref_meta} if ref_meta else None),
        "centroid": {"render": centroid_traj(xr, sr), "reference": centroid_traj(xf, sr)},
        "envelope": {"render": envelope_stats(xr, sr), "reference": envelope_stats(xf, sr)},
        "partials": {"render": partials(xr, sr), "reference": partials(xf, sr)},
        "crest": {"render": crest_factor(xr), "reference": crest_factor(xf)},
        "mr_stft": mr_stft_dist(xr, xf, sr, perceptual=not flat),
        "gates": gates,
    }
    if "lf" in prof:
        lf = prof["lf"]
        report["logmel_lf"] = logmel_dist(xr, xf, sr, perceptual=False, n=lf["n"], hop=lf["hop"],
                                          mels=lf["mels"], fmin=lf["fmin"], fmax=lf["fmax"])
        report["glide_hz"] = {"render": glide_track(xr, sr), "reference": glide_track(xf, sr)}
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
