#!/usr/bin/env python3
"""Stage equation-owned references and shadow manifests for plumbing tests only."""

import argparse
import copy
import json
import struct
from pathlib import Path

import numpy as np

import loop_campaign
import loop_metrics
import reference_contracts


FAMILY_SAMPLE_RATES = {"piano": 48000, "drums": 44100, "guitars": 16000, "bass": 16000}
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
    registry = reference_contracts.load_registry()
    source, source_sr = loop_metrics.load_mono(SOURCE)
    assets = []
    for sample_rate in sorted(set(FAMILY_SAMPLE_RATES.values())):
        contract_id = f"ref.equation.loop-pilot-v1.{sample_rate}"
        contract = registry["contracts"][contract_id]
        target = out / contract["reference_path"]
        write_float_wav(target, loop_metrics.resample_to(source, source_sr, sample_rate), sample_rate)
        digest = loop_campaign.sha256(target)
        if digest != contract["canonical_sha256"]:
            raise ValueError(f"generated equation fixture digest differs for {sample_rate}: {digest}")
        assets.append({"contract_id": contract_id, "path": contract["reference_path"], "sample_rate": sample_rate, "sha256": digest})

    entries = []
    manifest_dir = out / "manifests"
    manifest_dir.mkdir(parents=True)
    for family, sample_rate in FAMILY_SAMPLE_RATES.items():
        source_manifest = loop_campaign.validate_manifest(loop_campaign.ROOT / "evals" / "cases" / f"{family}.json")
        shadow = copy.deepcopy(source_manifest)
        contract = registry["contracts"][f"ref.equation.loop-pilot-v1.{sample_rate}"]
        for case in shadow["cases"]:
            case["reference"] = contract["reference_path"]
            case["reference_contract_id"] = contract["id"]
            case["reference_sha256"] = contract["canonical_sha256"]
            entries.append({"case": case["id"], "family": family, "contract_id": contract["id"], "path": contract["reference_path"], "sample_rate": sample_rate, "sha256": contract["canonical_sha256"]})
        loop_campaign.write_json(manifest_dir / f"{family}.json", shadow)

    provenance = {
        "schema_version": "1.1.0",
        "warning": "Synthetic equation-owned plumbing fixtures only; never use these files as instrument-quality references.",
        "source": str(SOURCE.relative_to(loop_campaign.ROOT)),
        "source_sha256": loop_campaign.sha256(SOURCE),
        "registry_sha256": registry["registry_sha256"],
        "assets": assets,
        "entries": entries,
        "manifest_dir": "manifests",
    }
    loop_campaign.write_json(out / "sources.json", provenance)
    return provenance


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    result = stage(Path(args.out))
    print(json.dumps({"out": str(Path(args.out).resolve()), "assets": len(result["assets"]), "cases": len(result["entries"]), "source_sha256": result["source_sha256"]}))


if __name__ == "__main__":
    main()
