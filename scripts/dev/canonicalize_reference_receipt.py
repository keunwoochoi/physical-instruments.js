#!/usr/bin/env python3
"""Rebuild exact private reference WAVs from a committed canonicalization receipt."""

import argparse
import hashlib
import importlib.metadata
import json
import os
import struct
import tempfile
from pathlib import Path

import jsonschema
import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

import reference_contracts


ROOT = Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "evals" / "reference-receipt-schema-v1.json"
REQUIREMENTS = ROOT / "scripts" / "dev" / "requirements-loop.txt"
CANONICAL_PACKAGES = ("numpy", "scipy", "soundfile")


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify_toolchain():
    pinned = {}
    for line in REQUIREMENTS.read_text(encoding="utf-8").splitlines():
        if "==" in line:
            name, value = line.split("==", 1)
            pinned[name.strip().lower()] = value.strip()
    for name in CANONICAL_PACKAGES:
        expected = pinned.get(name)
        actual = importlib.metadata.version(name)
        if expected is None or actual != expected:
            raise ValueError(f"canonicalizer toolchain mismatch for {name}: expected {expected}, got {actual}")


def reject_duplicate_keys(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_receipt(path):
    path = Path(path).resolve()
    receipt = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_keys)
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_keys)
    jsonschema.validate(receipt, schema)
    contract_ids = [entry["contract_id"] for entry in receipt["entries"]]
    reference_paths = [entry["reference_path"] for entry in receipt["entries"]]
    if len(contract_ids) != len(set(contract_ids)) or len(reference_paths) != len(set(reference_paths)):
        raise ValueError("receipt contract IDs and reference paths must be unique")
    bind_registry(receipt, path)
    return receipt


def bind_registry(receipt, receipt_path):
    try:
        owner = receipt_path.relative_to(ROOT).as_posix()
    except ValueError as exc:
        raise ValueError("receipt must live inside the repository") from exc
    registry = reference_contracts.load_registry()
    corpus = registry["corpora"].get(receipt["corpus_id"])
    if corpus is None or corpus["status"] != "verified":
        raise ValueError(f"receipt corpus is not verified: {receipt['corpus_id']}")
    if corpus["provenance_owner"] != owner:
        raise ValueError(f"receipt does not own corpus provenance: expected {corpus['provenance_owner']}, got {owner}")
    if corpus["license"] != receipt["source"]["license"]:
        raise ValueError("receipt source license does not match its registry corpus")
    expected_ids = {contract["id"] for contract in registry["contracts"].values() if contract["corpus_id"] == receipt["corpus_id"]}
    actual_ids = {entry["contract_id"] for entry in receipt["entries"]}
    if actual_ids != expected_ids:
        raise ValueError(f"receipt contract set does not match registry corpus: expected {sorted(expected_ids)}, got {sorted(actual_ids)}")
    for entry in receipt["entries"]:
        contract = registry["contracts"][entry["contract_id"]]
        expected = ("verified", entry["reference_path"], entry["canonical_sha256"], receipt["canonical_format"]["sample_rate"], False)
        actual = (contract["status"], contract["reference_path"], contract["canonical_sha256"], contract["sample_rate"], contract["level_normalized"])
        if actual != expected:
            raise ValueError(f"receipt entry does not match registry contract: {entry['contract_id']}")


def safe_join(root, relative):
    root = Path(root).resolve()
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError as exc:
        raise ValueError(f"receipt path escapes root: {relative}") from exc
    return path


def pin_peak_timestamp(path, timestamp):
    data = bytearray(Path(path).read_bytes())
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError(f"canonical writer did not emit RIFF/WAVE: {path}")
    offset = 12
    while offset + 8 <= len(data):
        chunk_id = bytes(data[offset:offset + 4])
        size = struct.unpack_from("<I", data, offset + 4)[0]
        if chunk_id == b"PEAK":
            if size < 8 or offset + 16 > len(data):
                break
            struct.pack_into("<I", data, offset + 12, timestamp)
            Path(path).write_bytes(data)
            return
        offset += 8 + size + (size & 1)
    raise ValueError(f"canonical writer did not emit a valid PEAK chunk: {path}")


def rebuild_entry(entry, source_root, output_root, canonical_format):
    source = safe_join(source_root, entry["source_path"])
    if not source.is_file() or sha256(source) != entry["source_sha256"]:
        raise ValueError(f"source identity mismatch: {source}")
    audio, sample_rate = sf.read(source, dtype="float64", always_2d=True)
    if sample_rate != entry["source_sample_rate"] or audio.shape[0] == 0:
        raise ValueError(f"source format mismatch: {source}")
    mono = audio.mean(axis=1)
    transform = entry["resample_poly"]
    if transform is not None:
        mono = resample_poly(mono, transform["up"], transform["down"], padtype=transform["padtype"])
        exact_length = round(audio.shape[0] * transform["up"] / transform["down"])
        mono = np.asarray(mono[:exact_length], dtype=np.float64)
    lead = entry["canonical_onset_frame"]
    onset = entry["working_onset_frame"]
    if onset >= lead:
        mono = mono[onset - lead:]
    else:
        mono = np.pad(mono, (lead - onset, 0))
    frames = entry["frames"]
    mono = np.pad(mono[:frames], (0, max(0, frames - len(mono))))
    destination = safe_join(output_root, entry["reference_path"])
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(prefix=f".{destination.name}.", suffix=".wav", dir=destination.parent, delete=False) as handle:
            temporary = Path(handle.name)
        sf.write(temporary, mono, canonical_format["sample_rate"], subtype="FLOAT", format="WAV")
        pin_peak_timestamp(temporary, entry["peak_timestamp"])
        actual = sha256(temporary)
        if actual != entry["canonical_sha256"]:
            raise ValueError(f"canonical digest mismatch for {entry['contract_id']}: expected {entry['canonical_sha256']}, got {actual}")
        os.replace(temporary, destination)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return {"contract_id": entry["contract_id"], "path": str(destination), "sha256": actual}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt")
    parser.add_argument("--source-root", required=True)
    parser.add_argument("--output-root", required=True)
    args = parser.parse_args(argv)
    verify_toolchain()
    receipt = load_receipt(args.receipt)
    results = [rebuild_entry(entry, args.source_root, args.output_root, receipt["canonical_format"]) for entry in receipt["entries"]]
    print(json.dumps({"receipt": receipt["id"], "entries": results}, sort_keys=True))


if __name__ == "__main__":
    main()
