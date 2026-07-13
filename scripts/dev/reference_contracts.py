#!/usr/bin/env python3
"""Load and bind exact, fail-closed reference-audio contracts."""

import hashlib
import json
from pathlib import Path, PurePosixPath

import jsonschema
import soundfile as sf


ROOT = Path(__file__).resolve().parents[2]
REGISTRY_PATH = ROOT / "evals" / "reference-manifest.json"
SCHEMA_PATH = ROOT / "evals" / "reference-manifest-schema-v2.json"
KNOWN_AXES = {"mr_stft", "attack", "decay", "partials", "lufs", "tail", "release", "velocity_loudness"}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _reject_duplicates(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def _read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f, object_pairs_hook=_reject_duplicates)


def _safe_relative_path(value):
    path = PurePosixPath(value)
    return bool(value) and "\\" not in value and path.as_posix() == value and not path.is_absolute() and all(part not in ("", ".", "..") for part in path.parts)


def contract_sha256(contract):
    payload = json.dumps(contract, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def load_registry(path=REGISTRY_PATH, schema_path=SCHEMA_PATH):
    path = Path(path).resolve()
    schema_path = Path(schema_path).resolve()
    registry = _read_json(path)
    schema = _read_json(schema_path)
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.validate(registry, schema)

    corpora = {}
    for corpus in registry["corpora"]:
        if corpus["id"] in corpora:
            raise ValueError(f"duplicate corpus id: {corpus['id']}")
        if corpus["status"] == "unverified" and not corpus.get("reason"):
            raise ValueError(f"{corpus['id']}: unverified corpus requires a reason")
        if corpus["status"] == "verified" and (not isinstance(corpus.get("license"), str) or not corpus["license"].strip()):
            raise ValueError(f"{corpus['id']}: verified corpus requires a non-empty license")
        corpora[corpus["id"]] = corpus

    contracts = {}
    paths = {}
    for contract in registry["contracts"]:
        contract_id = contract["id"]
        if contract_id in contracts:
            raise ValueError(f"duplicate reference contract id: {contract_id}")
        path_value = contract["reference_path"]
        if not _safe_relative_path(path_value):
            raise ValueError(f"{contract_id}: reference_path must be a normalized safe POSIX-relative path")
        if path_value in paths:
            raise ValueError(f"duplicate reference_path: {path_value} ({paths[path_value]}, {contract_id})")
        if contract["corpus_id"] not in corpora:
            raise ValueError(f"{contract_id}: unknown corpus_id {contract['corpus_id']}")
        unknown_axes = set(contract["invalid_axes"]) - KNOWN_AXES
        if unknown_axes:
            raise ValueError(f"{contract_id}: unknown invalid_axes: {sorted(unknown_axes)}")
        if contract["status"] == "verified":
            if corpora[contract["corpus_id"]]["status"] != "verified":
                raise ValueError(f"{contract_id}: verified contract belongs to an unverified corpus")
            if not isinstance(contract["canonical_sha256"], str):
                raise ValueError(f"{contract_id}: verified contract requires canonical_sha256")
            if not isinstance(contract["sample_rate"], int) or isinstance(contract["sample_rate"], bool):
                raise ValueError(f"{contract_id}: verified contract requires sample_rate")
            if not isinstance(contract["level_normalized"], bool):
                raise ValueError(f"{contract_id}: verified contract requires level_normalized")
        else:
            if not contract.get("reason"):
                raise ValueError(f"{contract_id}: unverified contract requires a reason")
            if any(contract[field] is not None for field in ("canonical_sha256", "sample_rate", "level_normalized")):
                raise ValueError(f"{contract_id}: unverified identity fields must be null")
        contracts[contract_id] = contract
        paths[path_value] = contract_id

    return {
        "data": registry,
        "corpora": corpora,
        "contracts": contracts,
        "registry_path": path,
        "schema_path": schema_path,
        "registry_sha256": sha256(path),
        "schema_sha256": sha256(schema_path),
    }


def bind_reference(registry, case, reference_root):
    case_id = case.get("id", "<unknown>")
    contract_id = case.get("reference_contract_id")
    if not contract_id:
        raise ValueError(f"{case_id}: reference_contract_id is required")
    contract = registry["contracts"].get(contract_id)
    if contract is None:
        raise ValueError(f"{case_id}: unknown reference contract id: {contract_id}")
    declared_path = case.get("reference")
    if declared_path != contract["reference_path"]:
        raise ValueError(f"{case_id}: reference path does not match contract {contract_id}: {declared_path!r} != {contract['reference_path']!r}")
    declared_sha = case.get("reference_sha256")
    if contract["status"] != "verified":
        if declared_sha is not None:
            raise ValueError(f"{case_id}: unverified reference contract must declare reference_sha256 as null")
        raise ValueError(f"{case_id}: unverified reference contract {contract_id}: {contract['reason']}")
    if declared_sha != contract["canonical_sha256"]:
        raise ValueError(f"{case_id}: reference_sha256 does not match contract {contract_id}")

    root = Path(reference_root).resolve()
    path = (root / declared_path).resolve()
    try:
        path.relative_to(root)
    except ValueError as exc:
        raise ValueError(f"{case_id}: resolved reference escapes --reference-root: {path}") from exc
    if not path.is_file():
        raise FileNotFoundError(f"{case_id}: missing campaign reference: {path}")
    actual_sha = sha256(path)
    if actual_sha != declared_sha:
        raise ValueError(f"{case_id}: reference digest mismatch for {contract_id}: expected {declared_sha}, got {actual_sha}")
    try:
        info = sf.info(path)
    except Exception as exc:
        raise ValueError(f"{case_id}: reference audio could not be decoded: {path}: {exc}") from exc
    if info.frames <= 0 or info.channels not in (1, 2):
        raise ValueError(f"{case_id}: reference must decode as non-empty mono/stereo audio: {path}")
    if info.samplerate != contract["sample_rate"]:
        raise ValueError(f"{case_id}: reference sample rate {info.samplerate} contradicts contract {contract['sample_rate']}: {path}")
    invalid = contract.get("invalid_axes", {})
    conflict = sorted(set(case["analysis"]["required_axes"]) & set(invalid))
    if conflict:
        raise ValueError(f"{case_id}: requires invalid reference axes {conflict} under {contract_id}")

    return {
        "path": path,
        "contract": contract,
        "evidence": {
            "id": contract_id,
            "corpus_id": contract["corpus_id"],
            "status": contract["status"],
            "declared_path": declared_path,
            "reference_sha256": actual_sha,
            "contract_sha256": contract_sha256(contract),
            "registry_sha256": registry["registry_sha256"],
            "registry_schema_sha256": registry["schema_sha256"],
        },
    }
