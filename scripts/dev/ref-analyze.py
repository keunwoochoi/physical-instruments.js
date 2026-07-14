#!/usr/bin/env python3
"""
Measure a set of reference recordings on the axes that actually decide a sustained
instrument, and print them as TARGETS.

This exists because every "real trombone is ~500 -> ~1500 Hz" number in this repo's
recent history was invented from memory. A target you made up is not a target.

Pitch is MEASURED, never taken from the filename: VSCO's octave naming is shifted, and
trusting a label is how you tune an instrument to the wrong note.
"""
import sys, glob, json, os
import numpy as np
import soundfile as sf
import pyloudnorm as pyln

SR = 48000

def load(path):
    x, sr = sf.read(path, always_2d=True)
    x = x.mean(axis=1)                     # to mono
    if sr != SR:                           # resample (polyphase, no decimation ghosts)
        from scipy.signal import resample_poly
        from math import gcd
        g = gcd(int(sr), SR)
        x = resample_poly(x, SR // g, sr // g)
    return x.astype(np.float64)

def f0_autocorr(x, lo=30.0, hi=1200.0):
    """Measured, parabolically refined, and OCTAVE-CORRECTED.

    Raw autocorrelation happily locks onto the sub-octave: a period of 2T correlates
    almost as well as T. On this corpus it did exactly that on 5 of 31 notes, and the
    tell was unmissable once the harmonics were printed - h2 came out +49, +74, +87 dB
    ABOVE h1, which does not mean a weak fundamental, it means the fundamental is not
    there and I had halved the pitch. A trombone does not sustain F1 at 43.6 Hz.

    So: after the raw estimate, test whether 2*f0 (and 3*f0) explains the spectrum
    better, and take the highest candidate that still carries the energy. This is the
    rigor the skill demands of measurement tools - a broken pitch metric would have
    re-tuned the entire instrument to the wrong note.
    """
    seg = x[: min(len(x), 4 * SR)]
    seg = seg - seg.mean()
    n = len(seg)
    # unbiased-ish autocorrelation via FFT
    f = np.fft.rfft(seg, 2 * n)
    ac = np.fft.irfft(f * np.conj(f))[:n]
    lag_lo, lag_hi = int(SR / hi), int(SR / lo)
    lag_hi = min(lag_hi, n - 2)
    band = ac[lag_lo:lag_hi]
    if len(band) < 3:
        return 0.0
    k = int(np.argmax(band)) + lag_lo
    y0, y1, y2 = ac[k - 1], ac[k], ac[k + 1]
    d = 0.5 * (y0 - y2) / (y0 - 2 * y1 + y2 + 1e-30)
    f = SR / (k + d)

    # octave correction: prefer a higher candidate if it carries clearly more energy
    def amp(freq):
        a, b = 0, min(len(x), int(0.5 * SR))
        s2 = x[a:b] * np.hanning(b - a)
        t = np.arange(len(s2))
        return np.abs((s2 * np.exp(-2j * np.pi * freq * t / SR)).sum()) / len(s2)

    # SMALLEST multiple first. Trying 4 first and breaking on the first hit overshoots
    # to the double octave when a single octave was the answer - it did, on 2 notes, and
    # put them above a tenor trombone-s playable range, which is what gave it away.
    for mult in (2.0, 3.0, 4.0):
        cand = f * mult
        if cand > hi:
            continue
        # the candidate wins only if it beats the current fundamental outright
        if amp(cand) > 4.0 * amp(f):
            f = cand
            break
    return f

def env(x, ms=5.0):
    w = int(SR * ms / 1000)
    n = len(x) // w
    return np.sqrt((x[: n * w].reshape(n, w) ** 2).mean(axis=1) + 1e-30)

def harmonics(x, f0, nh=24, t0=0.8, dur=0.35):
    """Amplitude at each harmonic, over a steady window. Goertzel-equivalent via DFT bin."""
    a, b = int(t0 * SR), int((t0 + dur) * SR)
    if b > len(x):
        a, b = 0, min(len(x), int(dur * SR))
    seg = x[a:b] * np.hanning(b - a)
    out = []
    t = np.arange(len(seg))
    for h in range(1, nh + 1):
        f = f0 * h
        if f > SR / 2 - 100:
            out.append(0.0); continue
        c = np.exp(-2j * np.pi * f * t / SR)
        out.append(np.abs((seg * c).sum()) / len(seg))
    return np.array(out)

def centroid_harmonic(H, f0):
    idx = np.arange(1, len(H) + 1)
    return float((H * idx * f0).sum() / (H.sum() + 1e-30))

def analyze(path):
    x = load(path)
    pk = np.abs(x).max()
    if pk < 1e-4:
        return None
    f0 = f0_autocorr(x)
    if not (25 < f0 < 1500):
        return None
    e = env(x)
    steady = float(np.median(e[int(0.6 / 0.005):int(1.2 / 0.005)])) if len(e) > 240 else float(e.max())
    if steady <= 0:
        return None
    # attack: time to reach 90% of the steady level
    hit = np.argmax(e >= 0.9 * steady)
    atk = float(hit * 5.0)
    H = harmonics(x, f0)
    Hn = H / (H[0] + 1e-30)
    cen = centroid_harmonic(H, f0)
    m = pyln.Meter(SR)
    lufs = float(m.integrated_loudness(x)) if len(x) > SR // 2 else float("nan")
    midi = 69 + 12 * np.log2(f0 / 440.0)
    return dict(file=os.path.basename(path), f0=float(f0), midi=float(midi),
                attack_ms=atk, centroid=cen, lufs=lufs,
                h=[float(v) for v in Hn[:12]], peak=float(pk),
                crest=float(20 * np.log10(pk / (np.sqrt((x ** 2).mean()) + 1e-30))))

if __name__ == "__main__":
    files = sorted(glob.glob(sys.argv[1]))
    rows = [r for r in (analyze(f) for f in files) if r]
    print(json.dumps(rows, indent=1))
