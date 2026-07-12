#!/usr/bin/env python3
"""Regenerate equation-owned loop metric fixtures after an intentional version change."""

import json
import os
import struct

import numpy as np

import loop_metrics


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "evals", "metrics", "loop-v1")
SR = 48_000


def synth(freq, harmonic_gain):
    seconds = 0.8
    lead = 0.05
    t = np.arange(int(seconds * SR)) / SR
    active = np.maximum(0.0, t - lead)
    env = np.where(t >= lead, np.exp(-3.0 * active), 0.0)
    return env * (
        0.28 * np.sin(2 * np.pi * freq * active)
        + harmonic_gain * np.sin(2 * np.pi * 2 * freq * active)
        + 0.03 * np.sin(2 * np.pi * 3 * freq * active)
    )


def report_subset(report):
    return {
        "schema_version": report["schema_version"],
        "metric_version": report["metric_version"],
        "render_sha256": report["inputs"]["render"]["sha256"],
        "reference_sha256": report["inputs"]["reference"]["sha256"],
        "interpretation": report["interpretation"],
        "mr_stft": report["mr_stft"],
        "logmel_dist": report["logmel_dist"],
        "gates": report["gates"],
    }


def write_float_wav(path, samples):
    """Write canonical mono IEEE-float WAV without libsndfile's dated PEAK chunk."""
    data = np.asarray(samples, dtype="<f4").tobytes()
    header = (
        b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE"
        + b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, SR, SR * 4, 4, 32)
        + b"data" + struct.pack("<I", len(data))
    )
    with open(path, "wb") as f:
        f.write(header)
        f.write(data)


def main():
    os.makedirs(OUT, exist_ok=True)
    reference = synth(440.0, 0.07)
    candidate = synth(442.0, 0.09)
    t = np.arange(len(candidate)) / SR
    candidate += 0.12 * np.sin(2 * np.pi * 19_000 * t) * (t >= 0.05)
    spike = int(0.052 * SR)
    candidate[spike] = 0.9
    candidate[spike + 1] = -0.9

    ref_path = os.path.join(OUT, "reference.wav")
    candidate_path = os.path.join(OUT, "candidate-artifact.wav")
    write_float_wav(ref_path, reference)
    write_float_wav(candidate_path, candidate)

    identity = loop_metrics.compare_files(ref_path, ref_path, expected_onset_s=0.05)
    mutation = loop_metrics.compare_files(candidate_path, ref_path, expected_onset_s=0.05)
    expected = {
        "fixture_version": 1,
        "generator": "scripts/dev/generate_loop_goldens.py",
        "identity": report_subset(identity),
        "artifact_mutation": report_subset(mutation),
    }
    with open(os.path.join(OUT, "expected.json"), "w") as f:
        json.dump(expected, f, indent=2, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
