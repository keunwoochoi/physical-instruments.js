#!/usr/bin/env python3
"""Stage deterministic equation-owned references for runner plumbing tests only."""

import argparse
import json
import struct
from pathlib import Path

import numpy as np

import loop_campaign
import loop_metrics


FAMILIES = ("piano", "drums", "guitars", "bass")
SOURCE = loop_campaign.ROOT / "evals" / "metrics" / "loop-v1" / "reference.wav"


def write_float_wav(path, samples, sr):
    data = np.asarray(samples, dtype="<f4").tobytes()
    header = (
        b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE"
        + b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, sr, sr * 4, 4, 32)
        + b"data" + struct.pack("<I", len(data))
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(header + data)


def stage(out):
    out = out.resolve()
    if out.exists() and any(out.iterdir()):
        raise FileExistsError(f"pilot reference directory is not empty: {out}")
    source, source_sr = loop_metrics.load_mono(SOURCE)
    entries = []
    for family in FAMILIES:
        manifest_path = loop_campaign.ROOT / "evals" / "cases" / f"{family}.json"
        manifest = loop_campaign.validate_manifest(manifest_path)
        for case in manifest["cases"]:
            target = out / case["reference"]
            corpus = loop_metrics.manifest_lookup(str(target)) or {}
            sr = corpus.get("sr", source_sr)
            audio = loop_metrics.resample_to(source, source_sr, sr)
            write_float_wav(target, audio, sr)
            entries.append({"case": case["id"], "family": family, "path": str(target.relative_to(out)), "sample_rate": sr, "sha256": loop_campaign.sha256(target)})
    provenance = {
        "schema_version": "1.0.0",
        "warning": "Synthetic equation-owned plumbing fixtures only; never use these files as instrument-quality references.",
        "source": str(SOURCE.relative_to(loop_campaign.ROOT)),
        "source_sha256": loop_campaign.sha256(SOURCE),
        "entries": entries,
    }
    loop_campaign.write_json(out / "sources.json", provenance)
    return provenance


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    result = stage(Path(args.out))
    print(json.dumps({"out": str(Path(args.out).resolve()), "references": len(result["entries"]), "source_sha256": result["source_sha256"]}))


if __name__ == "__main__":
    main()
