#!/usr/bin/env python3
"""Generate the deterministic hidden-reference/anchor harness pilot."""

from __future__ import annotations

import argparse
import json
import math
import shutil
import tempfile
from pathlib import Path

import numpy as np
import pyloudnorm
import soundfile as sf

import listening


ROOT = Path(__file__).resolve().parents[2]
DESTINATION = ROOT / "evals" / "listening" / "pilot"
SR = 48000
TARGET_LUFS = -23.0


def envelope(length: int) -> np.ndarray:
    time = np.arange(length) / SR
    attack = np.minimum(1.0, time / 0.025)
    release = np.minimum(1.0, (length / SR - time) / 0.12)
    return np.sin(0.5 * np.pi * np.minimum(attack, release).clip(0, 1)) ** 2


def normalize_loudness(audio: np.ndarray) -> np.ndarray:
    meter = pyloudnorm.Meter(SR)
    loudness = meter.integrated_loudness(audio)
    return np.asarray(pyloudnorm.normalize.loudness(audio, loudness, TARGET_LUFS), dtype=np.float64)


def audio_set() -> dict[str, np.ndarray]:
    length = int(0.8 * SR)
    time = np.arange(length) / SR
    env = envelope(length)
    reference = env * (0.72 * np.sin(2 * np.pi * 220 * time) + 0.22 * np.sin(2 * np.pi * 440 * time) + 0.08 * np.sin(2 * np.pi * 660 * time))
    candidate = env * (0.72 * np.sin(2 * np.pi * 220 * time) + 0.20 * np.sin(2 * np.pi * 441.5 * time) + 0.10 * np.sin(2 * np.pi * 660 * time))
    anchor_source = reference.copy()
    anchor = np.zeros_like(anchor_source)
    coefficient = 1.0 - math.exp(-2 * math.pi * 900 / SR)
    state = 0.0
    for index, sample in enumerate(anchor_source):
        state += coefficient * (sample - state)
        anchor[index] = round(state * 31) / 31
    normalized_reference = normalize_loudness(reference)
    return {
        "audio/reference.wav": normalized_reference,
        "audio/condition-01.wav": normalized_reference,
        "audio/condition-02.wav": normalize_loudness(candidate),
        "audio/condition-03.wav": normalize_loudness(anchor),
    }


def write_audio(out: Path) -> dict[str, dict[str, float | str]]:
    out.mkdir(parents=True, exist_ok=True)
    evidence: dict[str, dict[str, float | str]] = {}
    for name, audio in audio_set().items():
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        sf.write(path, audio, SR, subtype="PCM_16")
        written, rate = sf.read(path, dtype="float64", always_2d=True)
        loudness = float(pyloudnorm.Meter(rate).integrated_loudness(written))
        evidence[name] = {"sha256": listening.sha256_bytes(path.read_bytes()), "loudness": round(loudness, 6)}
    return evidence


def experiment() -> dict:
    return {
        "schema_version": listening.SCHEMA_VERSION,
        "id": "hidden-reference-anchor-pilot",
        "title": "Blind listening harness pilot",
        "purpose": "harness_validation",
        "instructions": "Rate the fidelity of every anonymous condition against the explicit reference. These synthetic tones validate only the listening harness, not an instrument or release.",
        "sample_rate": SR,
        "level_matching": {"method": "bs1770_integrated", "target_lufs": TARGET_LUFS, "tolerance_lu": 0.1, "window": "full_file"},
        "randomization": {"algorithm": listening.RANDOMIZATION_ALGORITHM, "seed_policy": "fixed_pilot"},
        "exclusion_policy": {
            "min_completed_trials": 1,
            "hidden_reference_min_score": 90,
            "min_completed_plays_per_stimulus": 1,
            "unique_listener_ids_required": True,
        },
        "trials": [{
            "id": "synthetic-tone-mushra",
            "protocol": "mushra",
            "prompt": "Rate fidelity to the explicit reference.",
            "reference": {"id": "explicit-reference", "path": "audio/reference.wav"},
            "stimuli": [
                {"id": "condition-01", "path": "audio/condition-01.wav"},
                {"id": "condition-02", "path": "audio/condition-02.wav"},
                {"id": "condition-03", "path": "audio/condition-03.wav"},
            ],
        }],
    }


def analysis_manifest(value: dict, evidence: dict[str, dict[str, float | str]]) -> dict:
    def condition(condition_id: str, path: str, role: str) -> dict:
        row = evidence[path]
        return {
            "id": condition_id,
            "role": role,
            "sha256": row["sha256"],
            "source_sha256": row["sha256"],
            "gain_db": 0.0,
            "integrated_lufs_before": row["loudness"],
            "integrated_lufs_after": row["loudness"],
            "duration_ms": 800,
        }

    return {
        "schema_version": listening.SCHEMA_VERSION,
        "experiment": "experiment.json",
        "experiment_digest": listening.manifest_digest(value),
        "provenance": {
            "generator": "campaign-ab-v1",
            "candidate_commit": "0" * 40,
            "baseline_commit": "0" * 40,
            "metric_version": "synthetic-harness-v1",
            "case_manifest_sha256": "0" * 64,
        },
        "trials": [{
            "id": "synthetic-tone-mushra",
            "case_id": "synthetic-tone",
            "reference_sha256": evidence["audio/reference.wav"]["sha256"],
            "stimuli": [
                condition("condition-01", "audio/condition-01.wav", "hidden_reference"),
                condition("condition-02", "audio/condition-02.wav", "candidate"),
                condition("condition-03", "audio/condition-03.wav", "anchor"),
            ],
        }],
    }


def playback(count: int = 2) -> dict:
    return {"starts": count, "completed": count, "listened_ms": 800 * count}


def sessions(value: dict) -> list[dict]:
    digest = listening.manifest_digest(value)
    rows = [
        (0x10010001, 98, 82, 21),
        (0x20020002, 96, 79, 18),
        (0x30030003, 100, 85, 24),
        (0x40040004, 95, 76, 27),
        (0x50050005, 97, 81, 20),
        (0x60060006, 65, 88, 12),
    ]
    out = []
    for index, (seed, hidden, candidate, anchor) in enumerate(rows, 1):
        presentation = listening.expected_presentations(value, seed)["synthetic-tone-mushra"]
        slots = ["condition-01", "condition-02", "condition-03", "reference"]
        out.append({
            "schema_version": listening.SCHEMA_VERSION,
            "experiment_id": value["id"],
            "experiment_digest": digest,
            "session_id": f"synthetic-pilot-{index}",
            "evidence_kind": "synthetic_harness_pilot",
            "listener": {"id": f"simulated-{index}", "experience": "synthetic fixture", "hearing_notes": "not a human listener"},
            "setup": {"transducer": "other", "environment": "deterministic test fixture", "device": "none", "volume_check": True},
            "randomization": {"algorithm": listening.RANDOMIZATION_ALGORITHM, "seed": seed},
            "trial_order": listening.expected_trial_order(value, seed),
            "started_at": f"2026-07-12T12:0{index}:00Z",
            "submitted_at": f"2026-07-12T12:0{index}:30Z",
            "trials": [{
                "trial_id": "synthetic-tone-mushra",
                "protocol": "mushra",
                "presentation": presentation,
                "response": {"ratings": {"condition-01": hidden, "condition-02": candidate, "condition-03": anchor}},
                "play_counts": {slot: 2 for slot in slots},
                "playback": {slot: playback() for slot in slots},
            }],
        })
    return out


def generate(out: Path) -> None:
    evidence = write_audio(out)
    value = experiment()
    private = analysis_manifest(value, evidence)
    (out / "experiment.json").write_text(json.dumps(value, indent=2) + "\n")
    (out / "analysis-manifest.json").write_text(json.dumps(private, indent=2) + "\n")
    result = sessions(value)
    (out / "synthetic-results.json").write_text(json.dumps(result, indent=2) + "\n")
    report = listening.analyze(value, result, private)
    (out / "synthetic-analysis.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    (out / "synthetic-analysis.md").write_text(listening.render_markdown(report))

def check() -> None:
    with tempfile.TemporaryDirectory() as directory:
        generated = Path(directory) / "pilot"
        generate(generated)
        expected = sorted(path.relative_to(generated) for path in generated.rglob("*") if path.is_file())
        actual = sorted(path.relative_to(DESTINATION) for path in DESTINATION.rglob("*") if path.is_file())
        if expected != actual:
            raise SystemExit(f"pilot file set differs: generated={expected}, committed={actual}")
        for relative in expected:
            if (generated / relative).read_bytes() != (DESTINATION / relative).read_bytes():
                raise SystemExit(f"pilot artifact stale: {relative}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check:
        check()
    else:
        if DESTINATION.exists():
            shutil.rmtree(DESTINATION)
        generate(DESTINATION)
        print(f"wrote {DESTINATION}")


if __name__ == "__main__":
    main()
