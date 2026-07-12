#!/usr/bin/env python3
"""Validate and run a reproducible instruments.js reference-matching campaign."""

import argparse
import datetime as dt
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import jsonschema
import soundfile as sf

import loop_metrics


ROOT = Path(__file__).resolve().parents[2]
CASE_SCHEMA = ROOT / "evals" / "cases" / "schema-v1.json"
ITERATION_SCHEMA = ROOT / "evals" / "cases" / "iteration-schema-v1.json"
SHIPPED_WASM = ROOT / "packages" / "core" / "wasm" / "instruments_dsp.wasm"
BUILT_WASM = ROOT / "target" / "wasm32-unknown-unknown" / "release" / "instruments_dsp.wasm"
KNOWN_AXES = {"mr_stft", "attack", "decay", "partials", "lufs", "tail", "release", "velocity_loudness"}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read_json(path):
    with open(path) as f:
        return json.load(f)


def write_json(path, value):
    with open(path, "w") as f:
        json.dump(value, f, indent=2, sort_keys=True)
        f.write("\n")


def validate_iteration(iteration):
    schema = read_json(ITERATION_SCHEMA)
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.validate(iteration, schema)


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def validate_manifest(path):
    manifest = read_json(path)
    schema = read_json(CASE_SCHEMA)
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.validate(manifest, schema)
    ids = [case["id"] for case in manifest["cases"]]
    if len(ids) != len(set(ids)):
        raise ValueError("case ids must be unique")
    roles = {case["role"] for case in manifest["cases"]}
    if "tune" not in roles or "held_out" not in roles:
        raise ValueError("campaign requires at least one tune and one held_out case")
    for case in manifest["cases"]:
        ref = Path(case["reference"])
        if ref.is_absolute() or ".." in ref.parts:
            raise ValueError(f"{case['id']}: reference must be a safe path relative to --reference-root")
        render = case["render"]
        if render["total_seconds"] <= render["lead_seconds"] + render["note_seconds"]:
            raise ValueError(f"{case['id']}: total_seconds must include a post-note tail")
        unknown = set(case["analysis"]["required_axes"]) - KNOWN_AXES
        if unknown:
            raise ValueError(f"{case['id']}: unknown required axes: {sorted(unknown)}")
    return manifest


def verify_clean_source(allow_dirty):
    dirty = git("status", "--porcelain", "--untracked-files=no")
    if dirty and not allow_dirty:
        raise RuntimeError("campaign source is dirty; commit the hypothesis first or pass --allow-dirty for a diagnostic-only run")
    return dirty.splitlines()


def verify_wasm(skip):
    if not SHIPPED_WASM.exists():
        raise FileNotFoundError(f"missing shipped WASM: {SHIPPED_WASM}")
    shipped = sha256(SHIPPED_WASM)
    if skip:
        return {"status": "skipped", "shipped_sha256": shipped}
    subprocess.run(
        ["cargo", "build", "-p", "instruments-dsp", "--target", "wasm32-unknown-unknown", "--release"],
        cwd=ROOT,
        check=True,
    )
    built = sha256(BUILT_WASM)
    if shipped != built:
        raise RuntimeError(f"stale shipped WASM: packages/core={shipped}, fresh release build={built}; copy the fresh binary and commit it")
    return {"status": "verified", "shipped_sha256": shipped, "built_sha256": built}


def resolve_cases(manifest, reference_root):
    reference_root = reference_root.resolve()
    resolved = []
    missing = []
    contradictions = []
    for case in manifest["cases"]:
        ref = (reference_root / case["reference"]).resolve()
        try:
            ref.relative_to(reference_root)
        except ValueError as exc:
            raise ValueError(f"{case['id']}: resolved reference escapes --reference-root: {ref}") from exc
        if not ref.is_file():
            missing.append(f"{case['id']}: {ref}")
            continue
        corpus = loop_metrics.manifest_lookup(str(ref)) or {}
        info = sf.info(ref)
        if info.frames <= 0 or info.channels not in (1, 2):
            raise ValueError(f"{case['id']}: reference must decode as non-empty mono/stereo audio: {ref}")
        if corpus.get("sr") and info.samplerate != corpus["sr"]:
            raise ValueError(f"{case['id']}: reference sample rate {info.samplerate} contradicts corpus contract {corpus['sr']}: {ref}")
        invalid = corpus.get("invalid_axes", {})
        conflict = sorted(set(case["analysis"]["required_axes"]) & set(invalid))
        if conflict:
            contradictions.append(f"{case['id']}: requires invalid corpus axes {conflict}")
        resolved.append((case, ref, corpus))
    if missing:
        raise FileNotFoundError("missing campaign references:\n" + "\n".join(missing))
    if contradictions:
        raise ValueError("contradictory corpus trust contracts:\n" + "\n".join(contradictions))
    return resolved


def render_case(case, out_path):
    r = case["render"]
    cmd = [
        "node", str(ROOT / "scripts" / "dev" / "render-note.mjs"), r["family"], str(r["midi"]),
        str(r["velocity"]), str(r["note_seconds"]), str(out_path), str(r["total_seconds"]),
        str(r["sample_rate"]), "--float32", "--lead-seconds", str(r["lead_seconds"]),
    ]
    proc = subprocess.run(cmd, cwd=ROOT, check=True, text=True, capture_output=True)
    try:
        return json.loads(proc.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"render-note returned invalid metadata for {case['id']}: {proc.stdout}") from exc


def expected_f0(case):
    """Return a harmonic-analysis anchor only when MIDI pitch means pitch.

    GM percussion note numbers select voices; they do not encode the voice's
    fundamental. Treating note 70 as a 466 Hz maraca fundamental can also make
    sparse/noise references divide by an empty harmonic match. Drum-specific
    structure belongs in the profile and reference, not a fabricated f0.
    """
    family = case["render"]["family"]
    if family.startswith("drums") or family == "percussion":
        return None
    midi = case["render"]["midi"]
    return 440.0 * (2.0 ** ((midi - 69) / 12.0))


def objective_vector(report):
    out = {
        "mr_stft.mean": report["mr_stft"]["mean"],
        "logmel.overall": report["logmel_dist"]["overall"],
    }
    for key in ("attack", "tail"):
        value = report["logmel_dist"].get(key)
        if value is not None:
            out[f"logmel.{key}"] = value
    if report["lufs"].get("valid", True) and report["lufs"].get("delta") is not None:
        out["abs_lufs_delta"] = abs(report["lufs"]["delta"])
    return out


def compare_baseline(report, baseline_report, epsilon=0.005):
    if baseline_report["metric_version"] != report["metric_version"]:
        raise ValueError("baseline metric_version differs; migrate or regenerate the baseline explicitly")
    current = objective_vector(report)
    previous = objective_vector(baseline_report)
    shared = sorted(set(current) & set(previous))
    deltas = {key: round(current[key] - previous[key], 6) for key in shared}
    improved = [key for key, delta in deltas.items() if delta < -epsilon]
    regressed = [key for key, delta in deltas.items() if delta > epsilon]
    if improved and not regressed:
        classification = "candidate"
    elif regressed and not improved:
        classification = "regressed"
    else:
        classification = "listening_required"
    return {"classification": classification, "deltas": deltas, "improved": improved, "regressed": regressed}


def markdown_summary(iteration):
    lines = [
        f"# {iteration['family']} loop iteration",
        "",
        f"Hypothesis: {iteration['hypothesis']}",
        "",
        f"Changed component: `{iteration['changed_component']}`",
        "",
        f"Classification: **{iteration['classification']}**",
        "",
        "| Case | Role | Trust | MR-STFT | Delta | Classification |",
        "|---|---|---:|---:|---:|---|",
    ]
    for case in iteration["cases"]:
        delta = (case.get("baseline") or {}).get("deltas", {}).get("mr_stft.mean")
        delta_text = "—" if delta is None else f"{delta:+.4f}"
        lines.append(f"| {case['id']} | {case['role']} | {'pass' if case['trusted'] else 'RED'} | {case['mr_stft_mean']:.4f} | {delta_text} | {case['classification']} |")
    lines += ["", "Objective metrics diagnose and reject artifacts; this report does not claim that the candidate sounds better.", ""]
    return "\n".join(lines)


def seal_iteration(out):
    excluded = {".complete", "evidence-digests.json"}
    files = sorted(path for path in out.rglob("*") if path.is_file() and path.name not in excluded)
    digests = {str(path.relative_to(out)): sha256(path) for path in files}
    write_json(out / "evidence-digests.json", {"schema_version": "1.0.0", "files": digests})
    (out / ".complete").write_text(sha256(out / "evidence-digests.json") + "\n")


def verify_iteration(out):
    out = Path(out).resolve()
    complete = out / ".complete"
    digest_path = out / "evidence-digests.json"
    if not complete.is_file() or not digest_path.is_file():
        raise ValueError("iteration is not sealed")
    if complete.read_text().strip() != sha256(digest_path):
        raise ValueError("completion digest does not match evidence-digests.json")
    evidence = read_json(digest_path)
    expected = evidence.get("files", {})
    actual_paths = sorted(path for path in out.rglob("*") if path.is_file() and path.name not in {".complete", "evidence-digests.json"})
    actual = {str(path.relative_to(out)) for path in actual_paths}
    if actual != set(expected):
        raise ValueError(f"iteration file set changed: expected={sorted(expected)}, actual={sorted(actual)}")
    for rel, digest in expected.items():
        path = (out / rel).resolve()
        try:
            path.relative_to(out)
        except ValueError as exc:
            raise ValueError(f"unsafe evidence path: {rel}") from exc
        if sha256(path) != digest:
            raise ValueError(f"evidence digest mismatch: {rel}")
    iteration = read_json(out / "iteration.json")
    validate_iteration(iteration)
    for case in iteration["cases"]:
        loop_metrics.validate_report(read_json(out / case["report"]))
    return iteration


def run_drift(baseline, out):
    log_path = out / "drift.log"
    proc = subprocess.run(
        [str(ROOT / "scripts" / "dev" / "drift-check.sh"), str(Path(baseline).resolve()), str(out / "drift-renders")],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    log_path.write_text(proc.stdout + proc.stderr)
    return {"status": "pass" if proc.returncode == 0 else "fail", "log": "drift.log"}


def run_campaign(args):
    manifest_path = Path(args.manifest).resolve()
    manifest = validate_manifest(manifest_path)
    reference_root = Path(args.reference_root).resolve()
    resolved = resolve_cases(manifest, reference_root)
    dirty = verify_clean_source(args.allow_dirty)
    wasm = verify_wasm(args.skip_wasm_verify)
    out = Path(args.out).resolve()
    if out.exists() and any(out.iterdir()):
        raise FileExistsError(f"iteration directory is not empty: {out}")
    (out / "renders").mkdir(parents=True, exist_ok=True)
    (out / "reports").mkdir()

    baseline_dir = Path(args.baseline_dir).resolve() if args.baseline_dir else None
    cases = []
    for case, reference, corpus in resolved:
        render_path = out / "renders" / f"{case['id']}.wav"
        render_meta = render_case(case, render_path)
        r = case["render"]
        is_drum = r["family"].startswith("drums") or r["family"] == "percussion"
        report = loop_metrics.compare_files(
            render_path,
            reference,
            profile=case["analysis"]["profile"],
            expected_onset_s=r["lead_seconds"],
            note_off_s=None if is_drum else r["lead_seconds"] + r["note_seconds"],
            max_post_note_off_db=case["analysis"].get("max_post_note_off_db"),
            expected_f0=expected_f0(case),
        )
        report_path = out / "reports" / f"{case['id']}.json"
        write_json(report_path, report)
        baseline = None
        classification = "untrusted" if not report["gates"]["trusted"] else "listening_required"
        if baseline_dir:
            old_path = baseline_dir / "reports" / f"{case['id']}.json"
            if not old_path.is_file():
                raise FileNotFoundError(f"baseline missing report for {case['id']}: {old_path}")
            baseline = compare_baseline(report, read_json(old_path))
            if classification != "untrusted":
                classification = baseline["classification"]
        cases.append({
            "id": case["id"], "role": case["role"], "reference": str(reference),
            "reference_sha256": report["inputs"]["reference"]["sha256"],
            "render_sha256": report["inputs"]["render"]["sha256"], "render_metadata": render_meta,
            "report": f"reports/{case['id']}.json", "trusted": report["gates"]["trusted"],
            "mr_stft_mean": report["mr_stft"]["mean"], "classification": classification,
            "baseline": baseline, "corpus": corpus.get("corpus"),
        })

    drift = {"status": "not_evaluated"}
    if args.drift_baseline:
        drift = run_drift(args.drift_baseline, out)

    classes = {case["classification"] for case in cases}
    if "untrusted" in classes:
        classification = "untrusted"
    elif "regressed" in classes:
        classification = "regressed"
    elif classes == {"candidate"}:
        classification = "candidate"
    else:
        classification = "listening_required"
    if args.drift_baseline is None or drift["status"] != "pass":
        classification = "incomplete" if classification not in {"untrusted", "regressed"} else classification

    iteration = {
        "schema_version": "1.0.0", "created_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "family": manifest["family"], "hypothesis": args.hypothesis,
        "changed_component": args.changed_component, "classification": classification,
        "source": {"commit": git("rev-parse", "HEAD"), "dirty": dirty},
        "wasm": wasm,
        "manifest": {"path": str(manifest_path), "sha256": sha256(manifest_path),
                     "schema_sha256": sha256(CASE_SCHEMA)},
        "metric_version": loop_metrics.METRIC_VERSION, "drift": drift, "cases": cases,
        "baseline_dir": str(baseline_dir) if baseline_dir else None,
    }
    validate_iteration(iteration)
    write_json(out / "iteration.json", iteration)
    (out / "summary.md").write_text(markdown_summary(iteration))

    audition = None
    if baseline_dir and classification in {"candidate", "listening_required"} and (baseline_dir / "renders").is_dir():
        audition = out / "audition.html"
        subprocess.run(["node", str(ROOT / "scripts" / "dev" / "ab-page.mjs"), str(baseline_dir / "renders"), str(out / "renders"), str(audition)], cwd=ROOT, check=True, text=True, capture_output=True)
    if audition:
        iteration["audition"] = "audition.html"
        validate_iteration(iteration)
        write_json(out / "iteration.json", iteration)
    seal_iteration(out)
    print(json.dumps({"out": str(out), "family": manifest["family"], "classification": classification, "cases": len(cases), "audition": str(audition) if audition else None}))
    return 0 if classification not in {"untrusted", "regressed"} else 1


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    validate = sub.add_parser("validate")
    validate.add_argument("manifest")
    verify = sub.add_parser("verify")
    verify.add_argument("iteration")
    def add_run_options(command):
        command.add_argument("--reference-root", required=True)
        command.add_argument("--out", required=True)
        command.add_argument("--hypothesis", required=True)
        command.add_argument("--changed-component", required=True)
        command.add_argument("--baseline-dir")
        command.add_argument("--drift-baseline")
        command.add_argument("--allow-dirty", action="store_true")
        command.add_argument("--skip-wasm-verify", action="store_true")
    run = sub.add_parser("run")
    run.add_argument("manifest")
    add_run_options(run)
    pilot = sub.add_parser("pilot")
    pilot.add_argument("--manifest-dir", default=str(ROOT / "evals" / "cases"))
    pilot.add_argument("--families", nargs="+", default=["piano", "drums", "guitars", "bass"])
    add_run_options(pilot)
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    if args.command == "validate":
        manifest = validate_manifest(Path(args.manifest))
        print(json.dumps({"valid": True, "family": manifest["family"], "cases": len(manifest["cases"])}))
        return 0
    if args.command == "verify":
        iteration = verify_iteration(args.iteration)
        print(json.dumps({"valid": True, "family": iteration["family"], "classification": iteration["classification"], "cases": len(iteration["cases"])}))
        return 0
    if args.command == "pilot":
        root_out = Path(args.out).resolve()
        results = []
        for family in args.families:
            child = argparse.Namespace(**vars(args))
            child.command = "run"
            child.manifest = str(Path(args.manifest_dir).resolve() / f"{family}.json")
            child.out = str(root_out / family)
            child.baseline_dir = str(Path(args.baseline_dir).resolve() / family) if args.baseline_dir else None
            code = run_campaign(child)
            iteration = verify_iteration(child.out)
            results.append({"family": family, "classification": iteration["classification"], "exit_code": code})
        write_json(root_out / "pilot.json", {"schema_version": "1.0.0", "results": results})
        print(json.dumps({"out": str(root_out), "results": results}))
        return 1 if any(result["exit_code"] for result in results) else 0
    return run_campaign(args)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"loop_campaign: {exc}", file=sys.stderr)
        sys.exit(2)
