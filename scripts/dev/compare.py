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


def log_mel(x, sr):
    F, _ = frames(x)
    spec = np.abs(np.fft.rfft(F, axis=1)) ** 2
    mel = spec @ mel_bank(sr, F.shape[1]).T
    return np.log10(mel + 1e-10)


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
        if i1 >= len(db) or db[i0] <= db[i1] + 0.5:
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


def main():
    render_path, ref_path = sys.argv[1], sys.argv[2]
    xr, sr_r = load_mono(render_path)
    xf, sr_f = load_mono(ref_path)
    sr = min(sr_r, sr_f)
    xr, xf = resample_to(xr, sr_r, sr), resample_to(xf, sr_f, sr)
    l_r, l_f = lufs(xr, sr), lufs(xf, sr)
    # loudness-normalize render to reference for spectral comparison
    if np.isfinite(l_r) and np.isfinite(l_f):
        xr = xr * (10 ** ((l_f - l_r) / 20))
    n = min(len(xr), len(xf))
    xr, xf = xr[:n], xf[:n]

    mr, mf = log_mel(xr, sr), log_mel(xf, sr)
    t = min(len(mr), len(mf))
    mr, mf = mr[:t], mf[:t]
    d = np.abs(mr - mf)
    thirds = np.array_split(d, 3)
    report = {
        "sr": sr,
        "seconds": round(n / sr, 2),
        "lufs": {"render": round(l_r, 1), "reference": round(l_f, 1), "delta": round(l_r - l_f, 1)},
        "logmel_dist": {
            "overall": round(float(d.mean()), 4),
            "attack": round(float(thirds[0].mean()), 4),
            "mid": round(float(thirds[1].mean()), 4),
            "tail": round(float(thirds[2].mean()), 4),
        },
        "centroid": {"render": centroid_traj(xr, sr), "reference": centroid_traj(xf, sr)},
        "envelope": {"render": envelope_stats(xr, sr), "reference": envelope_stats(xf, sr)},
        "partials": {"render": partials(xr, sr), "reference": partials(xf, sr)},
    }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
