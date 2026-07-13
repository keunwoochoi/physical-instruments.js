#!/usr/bin/env python3
"""BS.1770 integrated loudness (LUFS) per instrument family via pyloudnorm.

Usage:
  node scripts/dev/measure-loudness.mjs --wav-dir <dir>   # render family WAVs
  python3 scripts/dev/measure-loudness.py <dir>           # measure + suggest gains

Prints the makeup-gain table for crates/dsp/src/kernels.rs::makeup_gain,
referenced to marimba (gain 1.0 by definition). LUFS is the authoritative
perceptual number; the .mjs RMS table is only a quick smoke check.
"""
import math
import sys
from pathlib import Path

import pyloudnorm
import soundfile as sf

wav_dir = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
files = sorted(wav_dir.glob("family-*.wav"))
if not files:
    sys.exit(f"no family-*.wav files in {wav_dir} — run measure-loudness.mjs --wav-dir first")

rows = []
for f in files:
    data, rate = sf.read(f)
    meter = pyloudnorm.Meter(rate)
    lufs = meter.integrated_loudness(data)
    rows.append((f.stem.removeprefix("family-"), lufs))

ref = dict(rows)["marimba"]
print(f"{'family':<14}{'LUFS':>8}   suggested makeup ×current (marimba ref {ref:.1f} LUFS)")
for name, lufs in rows:
    gain = 10 ** ((ref - lufs) / 20)
    print(f"{name:<14}{lufs:>8.1f}   {gain:>6.2f}")
print("\nMultiply each family's CURRENT makeup_gain by its factor and paste into kernels.rs.")
