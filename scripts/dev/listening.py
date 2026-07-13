#!/usr/bin/env python3
"""Validate, randomize, and analyze local blind-listening evidence.

This module deliberately emits diagnostics and uncertainty, never a release
verdict. Human listeners remain the authority for sound quality.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import secrets
import shutil
import statistics
import sys
from pathlib import Path
from typing import Any, Iterable

import jsonschema
import numpy as np
import pyloudnorm
import soundfile as sf


ROOT = Path(__file__).resolve().parents[2]
LISTENING_ROOT = ROOT / "evals" / "listening"
ANALYSIS_SCHEMA = LISTENING_ROOT / "analysis-manifest-schema-v1.json"
SCHEMA_VERSION = "1.0.0"
RANDOMIZATION_ALGORITHM = "xorshift32-fisher-yates-v1"
CAMPAIGN_BUNDLE_VERSION = "campaign-ab-v2"
CAMPAIGN_TARGET_LUFS = -23.0
CAMPAIGN_TOLERANCE_LU = 0.1


def canonical_json(value: Any) -> str:
    def encode(item: Any) -> str:
        if isinstance(item, dict):
            return "{" + ",".join(
                f"{json.dumps(key, ensure_ascii=False)}:{encode(item[key])}" for key in sorted(item)
            ) + "}"
        if isinstance(item, list):
            return "[" + ",".join(encode(child) for child in item) + "]"
        if item is None or isinstance(item, (str, bool)):
            return json.dumps(item, ensure_ascii=False, separators=(",", ":"))
        if isinstance(item, int):
            if abs(item) > 9_007_199_254_740_991:
                raise ValueError("canonical JSON integer exceeds the browser-safe range")
            return str(item)
        if isinstance(item, float):
            if not math.isfinite(item):
                raise ValueError("canonical JSON does not permit non-finite numbers")
            if item.is_integer():
                integer = int(item)
                if abs(integer) > 9_007_199_254_740_991:
                    raise ValueError("canonical JSON integer exceeds the browser-safe range")
                return str(integer)
            mantissa, exponent = format(item, ".16e").split("e")
            return f"{mantissa}e{int(exponent)}"
        raise TypeError(f"unsupported canonical JSON value: {type(item).__name__}")

    return encode(value)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def manifest_digest(manifest: dict[str, Any]) -> str:
    return sha256_bytes(canonical_json(manifest).encode())


def load_json(path: Path | str) -> Any:
    return json.loads(Path(path).read_text())


def _require_keys(value: dict[str, Any], required: set[str], allowed: set[str], context: str) -> None:
    missing = required - value.keys()
    extra = value.keys() - allowed
    if missing:
        raise ValueError(f"{context}: missing keys {sorted(missing)}")
    if extra:
        raise ValueError(f"{context}: unknown keys {sorted(extra)}")


def _safe_audio_path(base: Path, relative: str) -> Path:
    candidate = (base / relative).resolve()
    try:
        candidate.relative_to(base.resolve())
    except ValueError as exc:
        raise ValueError(f"unsafe stimulus path: {relative}") from exc
    if candidate.suffix.lower() != ".wav":
        raise ValueError(f"stimulus must be WAV: {relative}")
    return candidate


def _prepare_level_matched_audio(source: Path, destination: Path, target_lufs: float) -> dict[str, Any]:
    audio, sample_rate = sf.read(source, dtype="float64", always_2d=True)
    if audio.size == 0 or not np.isfinite(audio).all():
        raise ValueError(f"invalid campaign listening source: {source}")
    meter = pyloudnorm.Meter(sample_rate)
    before = float(meter.integrated_loudness(audio))
    if not math.isfinite(before):
        raise ValueError(f"campaign listening source has no measurable loudness: {source}")
    prepared = np.asarray(pyloudnorm.normalize.loudness(audio, before, target_lufs), dtype=np.float64)
    peak = float(np.max(np.abs(prepared)))
    if not np.isfinite(prepared).all() or peak >= 1.0:
        raise ValueError(f"campaign listening normalization clips {source}: peak={peak}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    sf.write(destination, prepared, sample_rate, subtype="PCM_24")
    round_trip, written_rate = sf.read(destination, dtype="float64", always_2d=True)
    after = float(pyloudnorm.Meter(written_rate).integrated_loudness(round_trip))
    if abs(after - target_lufs) > CAMPAIGN_TOLERANCE_LU:
        raise ValueError(f"campaign listening loudness missed target for {source}: {after} LUFS")
    return {
        "source_sha256": sha256_bytes(source.read_bytes()),
        "gain_db": round(target_lufs - before, 6),
        "integrated_lufs_before": round(before, 6),
        "integrated_lufs_after": round(after, 6),
        "duration_ms": round(len(round_trip) * 1000 / written_rate),
    }


def prepare_campaign_bundle(iteration_dir: Path | str, baseline_dir: Path | str, out: Path | str) -> dict[str, Any]:
    iteration_dir = Path(iteration_dir).resolve()
    baseline_dir = Path(baseline_dir).resolve()
    out = Path(out).resolve()
    if out.exists() and any(out.iterdir()):
        raise FileExistsError(f"listening bundle directory is not empty: {out}")
    candidate = load_json(iteration_dir / "iteration.json")
    baseline = load_json(baseline_dir / "iteration.json")
    if candidate["family"] != baseline["family"]:
        raise ValueError("candidate and baseline listening families differ")
    if candidate["manifest"] != baseline["manifest"]:
        raise ValueError("candidate and baseline case manifests differ")
    if candidate.get("reference_registry") != baseline.get("reference_registry"):
        raise ValueError("candidate and baseline reference registries differ")
    if candidate["metric_version"] != baseline["metric_version"]:
        raise ValueError("candidate and baseline metric versions differ")
    baseline_cases = {item["id"]: item for item in baseline["cases"]}
    candidate_ids = [item["id"] for item in candidate["cases"]]
    if set(candidate_ids) != set(baseline_cases):
        raise ValueError("candidate and baseline listening case matrices differ")

    for case in candidate["cases"]:
        previous = baseline_cases[case["id"]]
        if (case.get("role") != previous.get("role")
                or case.get("reference_sha256") != previous.get("reference_sha256")
                or case.get("reference_contract") != previous.get("reference_contract")):
            raise ValueError(f"{case['id']}: baseline role/reference contract differs")
        ignored_metadata = {"out", "peak"}
        current_protocol = {key: value for key, value in case.get("render_metadata", {}).items() if key not in ignored_metadata}
        previous_protocol = {key: value for key, value in previous.get("render_metadata", {}).items() if key not in ignored_metadata}
        if not current_protocol or current_protocol != previous_protocol:
            raise ValueError(f"{case['id']}: baseline render protocol differs")

    out.mkdir(parents=True, exist_ok=True)
    for name in ("app.js", "randomization.js", "style.css"):
        shutil.copyfile(LISTENING_ROOT / name, out / name)
    index = (LISTENING_ROOT / "index.html").read_text()
    index = index.replace(
        '<meta name="ij-listening-experiment" content="pilot/experiment.json">',
        '<meta name="ij-listening-experiment" content="experiment.json">',
    )
    (out / "index.html").write_text(index)

    sample_rates: set[int] = set()
    blinding_nonce = secrets.token_hex(32)
    public_trials: list[dict[str, Any]] = []
    private_trials: list[dict[str, Any]] = []
    for case_index, case in enumerate(candidate["cases"], 1):
        case_id = case["id"]
        if not case_id or any(character not in "abcdefghijklmnopqrstuvwxyz0123456789-" for character in case_id):
            raise ValueError(f"unsafe listening case id: {case_id}")
        sources = {
            "incumbent": baseline_dir / "renders" / f"{case_id}.wav",
            "candidate": iteration_dir / "renders" / f"{case_id}.wav",
        }
        public_stimuli = []
        private_stimuli = []
        source_shapes: set[tuple[int, int, int]] = set()
        for role, source in sources.items():
            if not source.is_file():
                raise FileNotFoundError(f"missing campaign listening source: {source}")
            info = sf.info(source)
            sample_rates.add(info.samplerate)
            source_shapes.add((info.samplerate, info.channels, info.frames))
            opaque = hashlib.sha256(
                f"{blinding_nonce}:{case_id}:{role}".encode()
            ).hexdigest()[:16]
            stimulus_id = f"condition-{opaque}"
            relative = Path("audio") / f"{opaque}.wav"
            provenance = _prepare_level_matched_audio(source, out / relative, CAMPAIGN_TARGET_LUFS)
            public_stimuli.append({"id": stimulus_id, "path": relative.as_posix()})
            private_stimuli.append({
                "id": stimulus_id,
                "role": role,
                "sha256": sha256_bytes((out / relative).read_bytes()),
                **provenance,
            })
        if len(source_shapes) != 1:
            raise ValueError(f"{case_id}: baseline/candidate sample rate, channels, frames, or duration differ")
        trial_id = f"trial-{case_index:03d}"
        public_trials.append({
            "id": trial_id,
            "protocol": "ab",
            "prompt": f"Which rendering better matches the intended {candidate['family']} sound for case {case_index}?",
            "stimuli": public_stimuli,
        })
        private_trials.append({"id": trial_id, "case_id": case_id, "reference_sha256": case["reference_sha256"], "reference_contract": case["reference_contract"], "stimuli": private_stimuli})
    if len(sample_rates) != 1 or next(iter(sample_rates)) not in {44100, 48000}:
        raise ValueError(f"campaign listening bundle requires one browser sample rate, got {sorted(sample_rates)}")

    experiment_token = hashlib.sha256(
        f"{candidate['source']['commit']}:{baseline['source']['commit']}:{candidate['manifest']['sha256']}".encode()
    ).hexdigest()[:16]
    experiment = {
        "schema_version": SCHEMA_VERSION,
        "id": f"{candidate['family']}-{experiment_token}-iteration",
        "title": f"Blind {candidate['family']} iteration",
        "purpose": "iteration",
        "instructions": "Keep playback volume fixed. Compare realism, articulation, and artifacts; use no preference when neither rendering is clearly better.",
        "sample_rate": next(iter(sample_rates)),
        "level_matching": {
            "method": "bs1770_integrated",
            "target_lufs": CAMPAIGN_TARGET_LUFS,
            "tolerance_lu": CAMPAIGN_TOLERANCE_LU,
            "window": "full_file",
        },
        "randomization": {"algorithm": RANDOMIZATION_ALGORITHM, "seed_policy": "per_listener"},
        "exclusion_policy": {
            "min_completed_trials": len(public_trials),
            "hidden_reference_min_score": 90,
            "min_completed_plays_per_stimulus": 1,
            "unique_listener_ids_required": True,
        },
        "trials": public_trials,
    }
    experiment_path = out / "experiment.json"
    experiment_path.write_text(json.dumps(experiment, indent=2, sort_keys=True) + "\n")
    validated = validate_experiment(experiment_path)
    analysis = {
        "schema_version": SCHEMA_VERSION,
        "experiment": str(experiment_path.relative_to(iteration_dir)),
        "experiment_digest": manifest_digest(validated),
        "provenance": {
            "generator": CAMPAIGN_BUNDLE_VERSION,
            "candidate_commit": candidate["source"]["commit"],
            "baseline_commit": baseline["source"]["commit"],
            "metric_version": candidate["metric_version"],
            "case_manifest_sha256": candidate["manifest"]["sha256"],
            "case_schema_sha256": candidate["manifest"]["schema_sha256"],
            "reference_registry_sha256": candidate["reference_registry"]["sha256"],
            "reference_registry_schema_sha256": candidate["reference_registry"]["schema_sha256"],
            "blinding_nonce": blinding_nonce,
        },
        "trials": private_trials,
    }
    analysis_path = iteration_dir / "listening-analysis.json"
    analysis_path.write_text(json.dumps(analysis, indent=2, sort_keys=True) + "\n")
    validate_analysis_manifest(analysis_path)
    return {
        "experiment": str(experiment_path.relative_to(iteration_dir)),
        "experiment_digest": manifest_digest(validated),
        "analysis_manifest": str(analysis_path.relative_to(iteration_dir)),
        "analysis_manifest_sha256": sha256_bytes(analysis_path.read_bytes()),
        "protocol": "ab",
        "trials": len(public_trials),
    }


def validate_experiment(path: Path | str, verify_files: bool = True) -> dict[str, Any]:
    path = Path(path)
    value = load_json(path)
    jsonschema.validate(value, load_json(LISTENING_ROOT / "experiment-schema-v1.json"))
    _require_keys(
        value,
        {"schema_version", "id", "title", "purpose", "instructions", "sample_rate", "level_matching", "randomization", "exclusion_policy", "trials"},
        {"schema_version", "id", "title", "purpose", "instructions", "sample_rate", "level_matching", "randomization", "exclusion_policy", "trials"},
        "experiment",
    )
    if value["schema_version"] != SCHEMA_VERSION:
        raise ValueError(f"unsupported experiment schema: {value['schema_version']}")
    if value["purpose"] not in {"harness_validation", "iteration", "release_gate"}:
        raise ValueError("experiment purpose is invalid")
    if value["sample_rate"] not in {44100, 48000}:
        raise ValueError("sample_rate must be 44100 or 48000")
    _require_keys(value["randomization"], {"algorithm", "seed_policy"}, {"algorithm", "seed_policy"}, "randomization")
    if value["randomization"]["algorithm"] != RANDOMIZATION_ALGORITHM:
        raise ValueError("unsupported randomization algorithm")
    if value["randomization"]["seed_policy"] not in {"per_listener", "fixed_pilot"}:
        raise ValueError("seed_policy is invalid")
    _require_keys(
        value["level_matching"],
        {"method", "target_lufs", "tolerance_lu", "window"},
        {"method", "target_lufs", "tolerance_lu", "window"},
        "level_matching",
    )
    if value["level_matching"]["method"] not in {"bs1770_integrated", "declared_prematched"} or value["level_matching"]["window"] != "full_file":
        raise ValueError("level matching contract is invalid")
    _require_keys(
        value["exclusion_policy"],
        {"min_completed_trials", "hidden_reference_min_score", "min_completed_plays_per_stimulus", "unique_listener_ids_required"},
        {"min_completed_trials", "hidden_reference_min_score", "min_completed_plays_per_stimulus", "unique_listener_ids_required"},
        "exclusion_policy",
    )
    if value["exclusion_policy"]["unique_listener_ids_required"] is not True:
        raise ValueError("listener IDs must be unique")
    if not isinstance(value["trials"], list) or not value["trials"]:
        raise ValueError("experiment must contain trials")

    trial_ids: set[str] = set()
    experiment_stimulus_ids: set[str] = set()
    base = path.parent
    for trial in value["trials"]:
        _require_keys(trial, {"id", "protocol", "prompt", "stimuli"}, {"id", "protocol", "prompt", "stimuli", "reference", "x"}, f"trial {trial.get('id', '?')}")
        trial_id = trial["id"]
        if trial_id in trial_ids:
            raise ValueError(f"duplicate trial id: {trial_id}")
        trial_ids.add(trial_id)
        protocol = trial["protocol"]
        if protocol not in {"ab", "abx", "mushra"}:
            raise ValueError(f"{trial_id}: unsupported protocol {protocol}")
        stimuli = trial["stimuli"]
        if protocol in {"ab", "abx"} and len(stimuli) != 2:
            raise ValueError(f"{trial_id}: {protocol} requires exactly two stimuli")
        if protocol == "mushra" and len(stimuli) < 3:
            raise ValueError(f"{trial_id}: MUSHRA requires at least three conditions")
        stimulus_ids: set[str] = set()
        for stimulus in stimuli:
            _require_keys(stimulus, {"id", "path"}, {"id", "path"}, f"{trial_id} stimulus")
            if stimulus["id"] in stimulus_ids:
                raise ValueError(f"{trial_id}: duplicate stimulus id {stimulus['id']}")
            if stimulus["id"] in experiment_stimulus_ids:
                raise ValueError(f"stimulus id must be globally unique: {stimulus['id']}")
            stimulus_ids.add(stimulus["id"])
            experiment_stimulus_ids.add(stimulus["id"])
            audio = _safe_audio_path(base, stimulus["path"])
            if verify_files:
                if not audio.is_file():
                    raise ValueError(f"missing stimulus: {audio}")
                info = sf.info(audio)
                if info.frames <= 0 or info.channels not in {1, 2} or info.samplerate != value["sample_rate"]:
                    raise ValueError(f"{trial_id}: stimulus format violates the experiment contract")
        if protocol == "abx":
            if "x" not in trial:
                raise ValueError(f"{trial_id}: ABX requires an opaque X asset")
            x_audio = trial["x"]
            _require_keys(x_audio, {"id", "path"}, {"id", "path"}, f"{trial_id} X")
            if x_audio["id"] != "x":
                raise ValueError(f"{trial_id}: opaque X asset id must be x")
            x_path = _safe_audio_path(base, x_audio["path"])
            if verify_files:
                if not x_path.is_file():
                    raise ValueError(f"{trial_id}: X asset is missing")
                info = sf.info(x_path)
                if info.frames <= 0 or info.channels not in {1, 2} or info.samplerate != value["sample_rate"]:
                    raise ValueError(f"{trial_id}: X asset format violates the experiment contract")
        elif "x" in trial:
            raise ValueError(f"{trial_id}: only ABX may declare an X asset")
        if protocol == "mushra":
            if "reference" not in trial:
                raise ValueError(f"{trial_id}: MUSHRA requires an explicit reference")
            reference = trial["reference"]
            _require_keys(reference, {"id", "path"}, {"id", "path"}, f"{trial_id} reference")
            reference_path = _safe_audio_path(base, reference["path"])
            if verify_files:
                if not reference_path.is_file():
                    raise ValueError(f"{trial_id}: explicit reference is missing")
                info = sf.info(reference_path)
                if info.frames <= 0 or info.channels not in {1, 2} or info.samplerate != value["sample_rate"]:
                    raise ValueError(f"{trial_id}: explicit reference format violates the experiment contract")
        elif "reference" in trial:
            raise ValueError(f"{trial_id}: only MUSHRA may declare an explicit reference")
    return value


def validate_analysis_manifest(path: Path | str, verify_files: bool = True) -> tuple[dict[str, Any], dict[str, Any]]:
    path = Path(path).resolve()
    value = load_json(path)
    schema = load_json(ANALYSIS_SCHEMA)
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.validate(value, schema)
    experiment_path = (path.parent / value["experiment"]).resolve()
    try:
        experiment_path.relative_to(path.parent)
    except ValueError as exc:
        raise ValueError("unsafe participant experiment path") from exc
    experiment = validate_experiment(experiment_path, verify_files=verify_files)
    if manifest_digest(experiment) != value["experiment_digest"]:
        raise ValueError("participant experiment digest mismatch")
    public_trials = {trial["id"]: trial for trial in experiment["trials"]}
    if value["provenance"]["generator"] == "campaign-ab-v2":
        _require_keys(
            value["provenance"],
            {"generator", "candidate_commit", "baseline_commit", "metric_version", "case_manifest_sha256", "case_schema_sha256", "reference_registry_sha256", "reference_registry_schema_sha256", "blinding_nonce"},
            {"generator", "candidate_commit", "baseline_commit", "metric_version", "case_manifest_sha256", "case_schema_sha256", "reference_registry_sha256", "reference_registry_schema_sha256", "blinding_nonce"},
            "campaign provenance",
        )
    private_trials = {trial["id"]: trial for trial in value["trials"]}
    if len(private_trials) != len(value["trials"]) or set(private_trials) != set(public_trials):
        raise ValueError("analysis key trial matrix differs from participant experiment")
    case_ids: set[str] = set()
    for trial_id, trial in public_trials.items():
        private = private_trials[trial_id]
        if value["provenance"]["generator"] == "campaign-ab-v2":
            _require_keys(private, {"id", "case_id", "reference_sha256", "reference_contract", "stimuli"}, {"id", "case_id", "reference_sha256", "reference_contract", "stimuli", "x_source"}, f"{trial_id} private trial")
            if private["reference_sha256"] != private["reference_contract"]["reference_sha256"]:
                raise ValueError(f"{trial_id}: reference contract digest mismatch")
            for stimulus in private["stimuli"]:
                expected_id = "condition-" + hashlib.sha256(f"{value['provenance']['blinding_nonce']}:{private['case_id']}:{stimulus['role']}".encode()).hexdigest()[:16]
                if stimulus["id"] != expected_id:
                    raise ValueError(f"{trial_id}: private blinding mapping does not match its sealed nonce")
        if private["case_id"] in case_ids:
            raise ValueError(f"duplicate analysis case id: {private['case_id']}")
        case_ids.add(private["case_id"])
        public_stimuli = {item["id"]: item for item in trial["stimuli"]}
        private_stimuli = {item["id"]: item for item in private["stimuli"]}
        if len(private_stimuli) != len(private["stimuli"]) or set(private_stimuli) != set(public_stimuli):
            raise ValueError(f"{trial_id}: analysis condition matrix differs")
        roles = sorted(item["role"] for item in private["stimuli"])
        if trial["protocol"] in {"ab", "abx"} and roles != ["candidate", "incumbent"]:
            raise ValueError(f"{trial_id}: analysis key requires one candidate and one incumbent")
        if trial["protocol"] == "mushra" and (roles.count("hidden_reference") != 1 or "anchor" not in roles):
            raise ValueError(f"{trial_id}: analysis key requires one hidden reference and at least one anchor")
        for stimulus_id, stimulus in public_stimuli.items():
            private_stimulus = private_stimuli[stimulus_id]
            audio = _safe_audio_path(experiment_path.parent, stimulus["path"])
            if verify_files:
                if sha256_bytes(audio.read_bytes()) != private_stimulus["sha256"]:
                    raise ValueError(f"{trial_id}: prepared stimulus digest mismatch")
                samples, rate = sf.read(audio, dtype="float64", always_2d=True)
                loudness = float(pyloudnorm.Meter(rate).integrated_loudness(samples))
                target = experiment["level_matching"]["target_lufs"]
                tolerance = experiment["level_matching"]["tolerance_lu"]
                if not math.isfinite(loudness) or abs(loudness - target) > tolerance:
                    raise ValueError(f"{trial_id}: stimulus loudness outside declared tolerance: {loudness}")
                if abs(private_stimulus["integrated_lufs_after"] - loudness) > 0.01:
                    raise ValueError(f"{trial_id}: prepared loudness provenance mismatch")
        if trial["protocol"] == "abx":
            if private.get("x_source") not in private_stimuli:
                raise ValueError(f"{trial_id}: private X source must name an A/B stimulus")
            x_audio = _safe_audio_path(experiment_path.parent, trial["x"]["path"])
            if verify_files and sha256_bytes(x_audio.read_bytes()) != private_stimuli[private["x_source"]]["sha256"]:
                raise ValueError(f"{trial_id}: opaque X asset does not match its private source")
        elif "x_source" in private:
            raise ValueError(f"{trial_id}: only ABX may have a private X source")
        if trial["protocol"] == "mushra":
            reference = _safe_audio_path(experiment_path.parent, trial["reference"]["path"])
            reference_digest = sha256_bytes(reference.read_bytes())
            if private.get("reference_sha256") != reference_digest:
                raise ValueError(f"{trial_id}: explicit reference digest mismatch")
            hidden = next(item for item in private["stimuli"] if item["role"] == "hidden_reference")
            if hidden["sha256"] != reference_digest:
                raise ValueError(f"{trial_id}: hidden reference must be bit-identical to explicit reference")
    return experiment, value

def xorshift32(state: int) -> int:
    state &= 0xFFFFFFFF
    if state == 0:
        state = 0x6D2B79F5
    state ^= (state << 13) & 0xFFFFFFFF
    state ^= state >> 17
    state ^= (state << 5) & 0xFFFFFFFF
    return state & 0xFFFFFFFF


def shuffled_ids(ids: Iterable[str], seed: int) -> list[str]:
    out = list(ids)
    state = seed & 0xFFFFFFFF
    for index in range(len(out) - 1, 0, -1):
        state = xorshift32(state)
        swap = state % (index + 1)
        out[index], out[swap] = out[swap], out[index]
    return out


def trial_seed(session_seed: int, trial_index: int) -> int:
    state = session_seed & 0xFFFFFFFF
    for _ in range(trial_index + 1):
        state = xorshift32(state ^ 0x9E3779B9)
    return state


def expected_presentations(experiment: dict[str, Any], seed: int) -> dict[str, list[str]]:
    return {
        trial["id"]: shuffled_ids((item["id"] for item in trial["stimuli"]), trial_seed(seed, index))
        for index, trial in enumerate(experiment["trials"])
    }


def expected_trial_order(experiment: dict[str, Any], seed: int) -> list[str]:
    return shuffled_ids((trial["id"] for trial in experiment["trials"]), trial_seed(seed, len(experiment["trials"])))


def validate_session(session: dict[str, Any], experiment: dict[str, Any], digest: str) -> None:
    jsonschema.validate(session, load_json(LISTENING_ROOT / "session-schema-v1.json"))
    _require_keys(
        session,
        {"schema_version", "experiment_id", "experiment_digest", "session_id", "evidence_kind", "listener", "setup", "randomization", "trial_order", "started_at", "submitted_at", "trials"},
        {"schema_version", "experiment_id", "experiment_digest", "session_id", "evidence_kind", "listener", "setup", "randomization", "trial_order", "started_at", "submitted_at", "trials"},
        "session",
    )
    if session["schema_version"] != SCHEMA_VERSION or session["experiment_id"] != experiment["id"] or session["experiment_digest"] != digest:
        raise ValueError(f"{session.get('session_id', '?')}: experiment identity/digest mismatch")
    if session["evidence_kind"] not in {"human", "synthetic_harness_pilot"}:
        raise ValueError("invalid evidence_kind")
    _require_keys(session["listener"], {"id", "experience", "hearing_notes"}, {"id", "experience", "hearing_notes"}, "listener")
    _require_keys(session["setup"], {"transducer", "environment", "device", "volume_check"}, {"transducer", "environment", "device", "volume_check"}, "setup")
    if session["setup"]["transducer"] not in {"headphones", "studio_monitors", "speakers", "other"}:
        raise ValueError("invalid transducer")
    randomization = session["randomization"]
    _require_keys(randomization, {"algorithm", "seed"}, {"algorithm", "seed"}, "session randomization")
    if randomization["algorithm"] != RANDOMIZATION_ALGORITHM or not isinstance(randomization["seed"], int):
        raise ValueError("session randomization is invalid")
    expected = expected_presentations(experiment, randomization["seed"])
    trial_order = expected_trial_order(experiment, randomization["seed"])
    if session["trial_order"] != trial_order:
        raise ValueError("session trial order/randomization mismatch")
    if [response["trial_id"] for response in session["trials"]] != trial_order[:len(session["trials"])]:
        raise ValueError("response order does not match randomized trial order")
    trials_by_id = {trial["id"]: trial for trial in experiment["trials"]}
    seen: set[str] = set()
    for response in session["trials"]:
        _require_keys(response, {"trial_id", "protocol", "presentation", "response", "play_counts", "playback"}, {"trial_id", "protocol", "presentation", "response", "play_counts", "playback"}, "trial response")
        trial_id = response["trial_id"]
        if trial_id in seen or trial_id not in trials_by_id:
            raise ValueError(f"invalid or duplicate response trial: {trial_id}")
        seen.add(trial_id)
        trial = trials_by_id[trial_id]
        if response["protocol"] != trial["protocol"] or response["presentation"] != expected[trial_id]:
            raise ValueError(f"{trial_id}: presentation/randomization mismatch")
        slots = set(response["presentation"])
        expected_play_slots = slots | ({"reference"} if trial["protocol"] == "mushra" else set()) | ({"x"} if trial["protocol"] == "abx" else set())
        if set(response["play_counts"]) != expected_play_slots or any(not isinstance(count, int) or count < 0 for count in response["play_counts"].values()):
            raise ValueError(f"{trial_id}: play_counts invalid")
        if set(response["playback"]) != expected_play_slots:
            raise ValueError(f"{trial_id}: playback evidence slots invalid")
        for slot, playback in response["playback"].items():
            _require_keys(playback, {"starts", "completed", "listened_ms"}, {"starts", "completed", "listened_ms"}, f"{trial_id} playback")
            if any(not isinstance(playback[key], int) or playback[key] < 0 for key in playback) or playback["completed"] > playback["starts"]:
                raise ValueError(f"{trial_id}: playback evidence invalid")
            if response["play_counts"][slot] != playback["starts"]:
                raise ValueError(f"{trial_id}: play count/playback start mismatch")
        answer = response["response"]
        if trial["protocol"] == "mushra":
            if set(answer.get("ratings", {})) != slots or any(not isinstance(score, int) or not 0 <= score <= 100 for score in answer["ratings"].values()):
                raise ValueError(f"{trial_id}: MUSHRA ratings invalid")
        elif trial["protocol"] == "ab":
            if answer.get("choice") not in slots | {"tie"}:
                raise ValueError(f"{trial_id}: A/B choice invalid")
        elif answer.get("choice") not in slots:
            raise ValueError(f"{trial_id}: ABX choice invalid")


def _percentile(ordered: list[float], q: float) -> float:
    if not ordered:
        return math.nan
    position = (len(ordered) - 1) * q
    lo = math.floor(position)
    hi = math.ceil(position)
    if lo == hi:
        return ordered[lo]
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (position - lo)


def bootstrap_mean_ci(values: list[float], seed: int, iterations: int = 2000) -> list[float] | None:
    if not values:
        return None
    rng = random.Random(seed)
    means = [statistics.fmean(rng.choices(values, k=len(values))) for _ in range(iterations)]
    means.sort()
    return [round(_percentile(means, 0.025), 3), round(_percentile(means, 0.975), 3)]


def wilson_interval(successes: int, total: int) -> list[float] | None:
    if total == 0:
        return None
    z = 1.959963984540054
    p = successes / total
    denom = 1 + z * z / total
    center = (p + z * z / (2 * total)) / denom
    radius = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / denom
    return [round(max(0.0, center - radius), 4), round(min(1.0, center + radius), 4)]


def analyze(experiment: dict[str, Any], sessions: list[dict[str, Any]], analysis_manifest: dict[str, Any]) -> dict[str, Any]:
    digest = manifest_digest(experiment)
    if analysis_manifest["experiment_digest"] != digest:
        raise ValueError("analysis key does not bind the participant experiment")
    session_ids = [session.get("session_id") for session in sessions]
    if len(session_ids) != len(set(session_ids)):
        raise ValueError("duplicate session IDs are not allowed")
    listener_ids = [session.get("listener", {}).get("id") for session in sessions]
    if len(listener_ids) != len(set(listener_ids)):
        raise ValueError("duplicate listener IDs are not allowed")
    evidence_kinds = {session.get("evidence_kind") for session in sessions}
    if len(evidence_kinds) > 1:
        raise ValueError("synthetic and human evidence cannot be pooled")
    if evidence_kinds == {"synthetic_harness_pilot"} and experiment["purpose"] != "harness_validation":
        raise ValueError("synthetic evidence is valid only for harness validation")
    for session in sessions:
        validate_session(session, experiment, digest)
    trials_by_id = {trial["id"]: trial for trial in experiment["trials"]}
    private_by_id = {trial["id"]: trial for trial in analysis_manifest["trials"]}
    duration_by_trial = {
        trial_id: {item["id"]: item["duration_ms"] for item in private["stimuli"]}
        for trial_id, private in private_by_id.items()
    }
    policy = experiment["exclusion_policy"]
    included: list[dict[str, Any]] = []
    exclusions: list[dict[str, str]] = []
    for session in sessions:
        reasons: list[str] = []
        if len(session["trials"]) < policy["min_completed_trials"]:
            reasons.append("incomplete")
        for response in session["trials"]:
            trial = trials_by_id[response["trial_id"]]
            if any(
                playback["completed"] < policy["min_completed_plays_per_stimulus"]
                for playback in response["playback"].values()
            ):
                reasons.append(f"insufficient_completed_playback:{trial['id']}")
            for slot, playback in response["playback"].items():
                duration_slot = slot
                if slot == "reference":
                    duration_slot = next(item["id"] for item in private_by_id[trial["id"]]["stimuli"] if item["role"] == "hidden_reference")
                elif slot == "x":
                    duration_slot = private_by_id[trial["id"]]["x_source"]
                expected_ms = duration_by_trial[trial["id"]][duration_slot] * playback["completed"]
                if playback["completed"] and playback["listened_ms"] < 0.95 * expected_ms:
                    reasons.append(f"insufficient_playback_coverage:{trial['id']}")
            if trial["protocol"] == "mushra":
                hidden = next(item["id"] for item in private_by_id[trial["id"]]["stimuli"] if item["role"] == "hidden_reference")
                if response["response"]["ratings"][hidden] < policy["hidden_reference_min_score"]:
                    reasons.append(f"hidden_reference_below_threshold:{trial['id']}")
        if reasons:
            exclusions.append({"session_id": session["session_id"], "reason": ";".join(dict.fromkeys(reasons))})
        else:
            included.append(session)

    stimulus_meta = {
        item["id"]: {"role": item["role"], "trial_id": trial["id"], "case_id": trial["case_id"]}
        for trial in analysis_manifest["trials"]
        for item in trial["stimuli"]
    }
    scores: dict[str, list[float]] = {key: [] for key in stimulus_meta}
    ab_by_trial: dict[str, dict[str, Any]] = {
        trial["id"]: {
            "counts": {item["id"]: 0 for item in private_by_id[trial["id"]]["stimuli"]},
            "ties": 0,
            "total": 0,
        }
        for trial in experiment["trials"] if trial["protocol"] == "ab"
    }
    abx_correct = 0
    abx_total = 0
    for session in included:
        for response in session["trials"]:
            trial = trials_by_id[response["trial_id"]]
            answer = response["response"]
            if trial["protocol"] == "mushra":
                for stimulus_id, score in answer["ratings"].items():
                    scores[stimulus_id].append(float(score))
            elif trial["protocol"] == "ab":
                summary = ab_by_trial[trial["id"]]
                summary["total"] += 1
                if answer["choice"] == "tie":
                    summary["ties"] += 1
                else:
                    summary["counts"][answer["choice"]] += 1
            elif trial["protocol"] == "abx":
                abx_total += 1
                abx_correct += int(answer["choice"] == private_by_id[trial["id"]]["x_source"])

    stimulus_results: dict[str, Any] = {}
    for index, (stimulus_id, meta) in enumerate(sorted(stimulus_meta.items())):
        values = scores[stimulus_id]
        result: dict[str, Any] = {**meta, "n": len(values)}
        if values:
            result.update({"mean": round(statistics.fmean(values), 3), "median": round(statistics.median(values), 3), "mean_ci95_bootstrap": bootstrap_mean_ci(values, 0x4C340000 + index)})
        trial_ab = ab_by_trial.get(meta["trial_id"])
        if trial_ab and trial_ab["total"]:
            decisive = trial_ab["total"] - trial_ab["ties"]
            count = trial_ab["counts"][stimulus_id]
            result.update({
                "preference_count": count,
                "preference_total": trial_ab["total"],
                "preference_decisive_total": decisive,
                "tie_count": trial_ab["ties"],
                "preference_ci95_wilson_decisive": wilson_interval(count, decisive),
            })
        stimulus_results[stimulus_id] = result

    return {
        "schema_version": SCHEMA_VERSION,
        "experiment_id": experiment["id"],
        "experiment_digest": digest,
        "analysis_manifest_digest": sha256_bytes(canonical_json(analysis_manifest).encode()),
        "evidence_kind_counts": {kind: sum(session["evidence_kind"] == kind for session in sessions) for kind in ["human", "synthetic_harness_pilot"]},
        "n_submitted": len(sessions),
        "n_included": len(included),
        "exclusions": exclusions,
        "stimuli": stimulus_results,
        "abx": {"correct": abx_correct, "total": abx_total, "accuracy_ci95_wilson": wilson_interval(abx_correct, abx_total)},
        "raw_sessions": sessions,
        "quality_verdict": None,
        "interpretation": "Listening evidence with uncertainty; a human owner applies the declared gate. Synthetic pilot sessions validate only the harness.",
    }

def render_markdown(report: dict[str, Any]) -> str:
    lines = [
        f"# Listening analysis: {report['experiment_id']}",
        "",
        f"Submitted: {report['n_submitted']}; included: {report['n_included']}; excluded: {len(report['exclusions'])}.",
        "",
        "This report does not contain a release verdict. Synthetic pilot sessions validate only the harness.",
        "",
        "| Stimulus | Case | Role | n | Mean | 95% bootstrap CI | Preference / decisive (ties; total) |",
        "|---|---|---:|---:|---:|---:|---:|",
    ]
    for stimulus_id, row in sorted(report["stimuli"].items()):
        ci = row.get("mean_ci95_bootstrap")
        preference = "—"
        if "preference_count" in row:
            preference = f"{row['preference_count']} / {row['preference_decisive_total']} ({row['tie_count']} ties; {row['preference_total']} total)"
        lines.append(f"| {stimulus_id} | {row['case_id']} | {row['role']} | {row['n']} | {row.get('mean', '—')} | {ci if ci else '—'} | {preference} |")
    if report["abx"]["total"]:
        lines.extend(["", f"ABX: {report['abx']['correct']} / {report['abx']['total']} correct; Wilson 95% CI {report['abx']['accuracy_ci95_wilson']}."])
    if report["exclusions"]:
        lines.extend(["", "## Exclusions", ""] + [f"- `{item['session_id']}`: {item['reason']}" for item in report["exclusions"]])
    lines.extend(["", "Raw listener-level sessions are retained in the JSON report.", ""])
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    validate = sub.add_parser("validate")
    validate.add_argument("manifest")
    validate.add_argument("--no-files", action="store_true")
    analyze_parser = sub.add_parser("analyze")
    analyze_parser.add_argument("analysis_manifest")
    analyze_parser.add_argument("results", nargs="+")
    analyze_parser.add_argument("--out", required=True)
    analyze_parser.add_argument("--markdown")
    prepare = sub.add_parser("prepare-campaign")
    prepare.add_argument("iteration")
    prepare.add_argument("--baseline", required=True)
    prepare.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    if args.command == "prepare-campaign":
        result = prepare_campaign_bundle(args.iteration, args.baseline, args.out)
        print(json.dumps(result, sort_keys=True))
        return 0
    if args.command == "validate":
        value = load_json(args.manifest)
        if {"experiment", "experiment_digest", "provenance", "trials"} <= value.keys():
            experiment, _ = validate_analysis_manifest(args.manifest, verify_files=not args.no_files)
        else:
            experiment = validate_experiment(args.manifest, verify_files=not args.no_files)
        print(json.dumps({"experiment": experiment["id"], "digest": manifest_digest(experiment), "trials": len(experiment["trials"])}))
        return 0
    experiment, analysis_manifest = validate_analysis_manifest(args.analysis_manifest)
    sessions: list[dict[str, Any]] = []
    for result_path in args.results:
        loaded = load_json(result_path)
        sessions.extend(loaded if isinstance(loaded, list) else [loaded])
    report = analyze(experiment, sessions, analysis_manifest)
    Path(args.out).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    if args.markdown:
        Path(args.markdown).write_text(render_markdown(report))
    print(json.dumps({"out": args.out, "submitted": report["n_submitted"], "included": report["n_included"], "quality_verdict": None}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
