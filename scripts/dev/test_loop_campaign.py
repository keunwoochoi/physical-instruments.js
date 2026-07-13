#!/usr/bin/env python3
"""Corpus-free validation for the declarative loop campaign runner."""

import copy
import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import loop_campaign
import stage_loop_pilot_refs


ROOT = Path(__file__).resolve().parents[2]
GOLDEN_REFERENCE = ROOT / "evals" / "metrics" / "loop-v1" / "reference.wav"


FIXTURE_PATH = "references/equation-loop-pilot-v1/reference-48000.wav"
FIXTURE_CONTRACT = "ref.equation.loop-pilot-v1.48000"
FIXTURE_SHA256 = "0a3664c121555f0c9e55d85ed0fca97ab72194b9525e65f5a82b71f95a322059"


def case(case_id, role, reference=FIXTURE_PATH, required_axes=None):
    return {
        "id": case_id,
        "role": role,
        "reference": reference,
        "reference_contract_id": FIXTURE_CONTRACT,
        "reference_sha256": FIXTURE_SHA256,
        "render": {
            "family": "marimba",
            "midi": 69,
            "velocity": 90,
            "note_seconds": 0.2,
            "total_seconds": 0.6,
            "sample_rate": 48000,
            "lead_seconds": 0.05,
        },
        "analysis": {"profile": "pitched", "required_axes": required_axes or ["mr_stft", "attack"]},
    }


def manifest():
    return {
        "schema_version": "1.2.0",
        "family": "test",
        "description": "Equation-fixture runner test.",
        "cases": [case("tune-a", "tune"), case("hold-b", "held_out")],
    }


class ManifestTests(unittest.TestCase):
    def write_manifest(self, directory, value):
        path = Path(directory) / "cases.json"
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def test_all_committed_family_manifests_validate(self):
        for path in sorted((ROOT / "evals" / "cases").glob("*.json")):
            if "schema-v1" not in path.name:
                loaded = loop_campaign.validate_manifest(path)
                self.assertGreaterEqual(len(loaded["cases"]), 2, path)

    def test_held_out_case_is_mandatory(self):
        value = manifest()
        value["cases"][1]["role"] = "tune"
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "held_out"):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_duplicate_case_id_is_rejected(self):
        value = manifest()
        value["cases"][1]["id"] = value["cases"][0]["id"]
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "unique"):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_duplicate_manifest_key_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "cases.json"
            path.write_text('{"schema_version":"1.1.0","family":"a","family":"b","cases":[]}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
                loop_campaign.validate_manifest(path)

    def test_reference_path_cannot_escape_root(self):
        value = manifest()
        value["cases"][0]["reference"] = "../secret.wav"
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "safe path"):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_partials_axis_requires_explicit_pairing_model(self):
        value = manifest()
        value["cases"][0]["analysis"]["required_axes"].append("partials")
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "partial_model"):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_partial_model_without_axis_is_rejected(self):
        value = manifest()
        value["cases"][0]["analysis"]["partial_model"] = {
            "type": "proximity_harmonic", "search_cents": 90.0,
        }
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "not required"):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_invalid_partial_model_is_rejected_by_schema(self):
        value = manifest()
        value["cases"][0]["analysis"]["required_axes"].append("partials")
        value["cases"][0]["analysis"]["partial_model"] = {
            "type": "stiff_string", "inharmonicity_b": -1.0, "search_cents": 90.0,
        }
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(loop_campaign.jsonschema.ValidationError):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_unverified_contract_fails_before_render(self):
        value = manifest()
        for item in value["cases"]:
            item["reference"] = "references/guitar-acoustic/canonical/nylon-A3-mf.wav"
            item["reference_contract_id"] = "ref.legacy.guitars.nylon-a3-mf.v1"
            item["reference_sha256"] = None
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            path = self.write_manifest(d, value)
            loaded = loop_campaign.validate_manifest(path)
            registry = loop_campaign.reference_contracts.load_registry()
            with self.assertRaisesRegex(ValueError, "unverified reference contract"):
                loop_campaign.resolve_cases(loaded, root, registry)


class BaselineTests(unittest.TestCase):
    def test_pareto_classification_distinguishes_improvement_and_regression(self):
        base = {"metric_version": "v", "mr_stft": {"mean": 1.0}, "logmel_dist": {"overall": 1.0, "attack": 1.0, "tail": 1.0}, "lufs": {"valid": True, "delta": 1.0}}
        better = copy.deepcopy(base)
        better["mr_stft"]["mean"] = 0.8
        better["logmel_dist"] = {"overall": 0.8, "attack": 0.8, "tail": 0.8}
        better["lufs"]["delta"] = 0.5
        result = loop_campaign.compare_baseline(better, base)
        self.assertEqual(result["classification"], "candidate")
        worse = copy.deepcopy(base)
        worse["mr_stft"]["mean"] = 1.2
        worse["logmel_dist"] = {"overall": 1.2, "attack": 1.2, "tail": 1.2}
        worse["lufs"]["delta"] = 1.5
        self.assertEqual(loop_campaign.compare_baseline(worse, base)["classification"], "regressed")

    def test_metric_version_mismatch_is_rejected(self):
        a = {"metric_version": "new"}
        b = {"metric_version": "old"}
        with self.assertRaisesRegex(ValueError, "metric_version"):
            loop_campaign.compare_baseline(a, b)

    def test_baseline_contract_manifest_family_and_case_set_must_match(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ref = root / FIXTURE_PATH
            ref.parent.mkdir(parents=True)
            shutil.copyfile(GOLDEN_REFERENCE, ref)
            manifest_path = root / "cases.json"
            manifest_path.write_text(json.dumps(manifest()), encoding="utf-8")
            loaded_manifest = loop_campaign.validate_manifest(manifest_path)
            registry = loop_campaign.reference_contracts.load_registry()
            resolved = loop_campaign.resolve_cases(loaded_manifest, root, registry)
            baseline = {
                "family": "test",
                "reference_registry": {"path": "reference-registry.json", "sha256": registry["registry_sha256"], "schema_path": "reference-registry-schema.json", "schema_sha256": registry["schema_sha256"]},
                "manifest": {"path": "case-manifest.json", "sha256": loop_campaign.sha256(manifest_path), "schema_path": "case-schema.json", "schema_sha256": loop_campaign.sha256(loop_campaign.CASE_SCHEMA)},
                "cases": [{"id": case["id"], "reference_contract": bound["evidence"]} for case, bound in resolved],
            }
            mutations = []
            family = copy.deepcopy(baseline); family["family"] = "other"; mutations.append((family, "family differs"))
            registry_bad = copy.deepcopy(baseline); registry_bad["reference_registry"]["sha256"] = "0" * 64; mutations.append((registry_bad, "registry identity differs"))
            manifest_bad = copy.deepcopy(baseline); manifest_bad["manifest"]["sha256"] = "0" * 64; mutations.append((manifest_bad, "manifest identity differs"))
            cases_bad = copy.deepcopy(baseline); cases_bad["cases"].pop(); mutations.append((cases_bad, "case set differs"))
            contract_bad = copy.deepcopy(baseline); contract_bad["cases"][0]["reference_contract"]["contract_sha256"] = "0" * 64; mutations.append((contract_bad, "contract identity differs"))
            for value, message in mutations:
                with self.subTest(message=message), mock.patch.object(loop_campaign, "verify_iteration", return_value=value):
                    with self.assertRaisesRegex(ValueError, message):
                        loop_campaign.verify_baseline_compatibility(root / "baseline", manifest_path, loaded_manifest, resolved, registry)


class RunnerFailureTests(unittest.TestCase):
    def test_json_helpers_round_trip_non_ascii_as_utf8(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "evidence.json"
            value = {"hypothesis": "diffuse snare — écoute"}
            loop_campaign.write_json(path, value)
            self.assertEqual(loop_campaign.read_json(path), value)
            self.assertIn("écoute".encode("utf-8"), path.read_bytes())

    def test_renderer_failure_surfaces_stderr(self):
        failure = subprocess.CalledProcessError(3, ["node"], output="", stderr="WASM rejected the note")
        with mock.patch.object(loop_campaign.subprocess, "run", side_effect=failure):
            with self.assertRaisesRegex(RuntimeError, "WASM rejected the note"):
                loop_campaign.render_case(case("broken", "tune"), Path("render.wav"))

    def test_renderer_invalid_metadata_surfaces_both_streams(self):
        completed = subprocess.CompletedProcess(["node"], 0, stdout="not-json\n", stderr="renderer warning\n")
        with mock.patch.object(loop_campaign.subprocess, "run", return_value=completed):
            with self.assertRaisesRegex(RuntimeError, "not-json"):
                loop_campaign.render_case(case("broken", "tune"), Path("render.wav"))

    def test_output_file_is_rejected_cleanly(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "iteration"
            out.write_text("not a directory", encoding="utf-8")
            with self.assertRaisesRegex(FileExistsError, "not an empty directory"):
                loop_campaign.prepare_output_dir(out)

    def test_nonempty_output_directory_is_rejected_cleanly(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "iteration"
            out.mkdir()
            (out / "existing").write_text("evidence", encoding="utf-8")
            with self.assertRaisesRegex(FileExistsError, "not an empty directory"):
                loop_campaign.prepare_output_dir(out)

    def test_invalid_output_fails_before_clean_check_or_wasm_build(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ref = root / FIXTURE_PATH
            ref.parent.mkdir(parents=True)
            shutil.copyfile(GOLDEN_REFERENCE, ref)
            manifest_path = root / "cases.json"
            manifest_path.write_text(json.dumps(manifest()), encoding="utf-8")
            out = root / "iteration"
            out.write_text("not a directory", encoding="utf-8")
            args = SimpleNamespace(
                manifest=str(manifest_path), reference_root=str(root), out=str(out), hypothesis="Reject invalid output first.",
                changed_component="test", baseline_dir=None, drift_baseline=None, allow_dirty=False, skip_wasm_verify=False,
            )
            with mock.patch.object(loop_campaign, "verify_clean_source") as clean, mock.patch.object(loop_campaign, "verify_wasm") as wasm:
                with self.assertRaisesRegex(FileExistsError, "not an empty directory"):
                    loop_campaign.run_campaign(args)
                clean.assert_not_called()
                wasm.assert_not_called()

    def test_reference_change_after_binding_stops_before_classification_or_audition(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ref = root / FIXTURE_PATH
            ref.parent.mkdir(parents=True)
            shutil.copyfile(GOLDEN_REFERENCE, ref)
            manifest_path = root / "cases.json"
            manifest_path.write_text(json.dumps(manifest()), encoding="utf-8")
            args = SimpleNamespace(
                manifest=str(manifest_path), reference_root=str(root), out=str(root / "iteration"), hypothesis="Detect TOCTOU.",
                changed_component="test", baseline_dir=None, drift_baseline=str(root / "drift"), allow_dirty=True, skip_wasm_verify=True,
            )
            changed_report = {"inputs": {"reference": {"sha256": "f" * 64}}}
            with mock.patch.object(loop_campaign, "verify_clean_source", return_value=[]), mock.patch.object(loop_campaign, "verify_wasm", return_value={"status": "skipped", "shipped_sha256": "0" * 64}), mock.patch.object(loop_campaign, "render_case", return_value={}), mock.patch.object(loop_campaign.loop_metrics, "compare_files", return_value=changed_report), mock.patch.object(loop_campaign, "run_drift") as drift, mock.patch.object(loop_campaign, "seal_iteration") as seal, mock.patch.object(loop_campaign.subprocess, "run") as proc:
                with self.assertRaisesRegex(ValueError, "changed after contract binding"):
                    loop_campaign.run_campaign(args)
                drift.assert_not_called()
                seal.assert_not_called()
                proc.assert_not_called()


class EndToEndTests(unittest.TestCase):
    def test_four_family_pilot_staging_is_complete_and_corpus_rate_correct(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "refs"
            result = stage_loop_pilot_refs.stage(out)
            self.assertEqual(len(result["entries"]), 16)
            self.assertEqual(len(result["assets"]), 3)
            rates = {entry["sample_rate"] for entry in result["entries"]}
            self.assertEqual(rates, {16000, 44100, 48000})
            self.assertTrue((out / "sources.json").is_file())
            for family in stage_loop_pilot_refs.FAMILY_SAMPLE_RATES:
                loop_campaign.validate_manifest(out / "manifests" / f"{family}.json")
            with self.assertRaisesRegex(FileExistsError, "not empty"):
                stage_loop_pilot_refs.stage(out)

    def test_equation_reference_runs_through_shipped_wasm_and_writes_immutable_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ref = root / FIXTURE_PATH
            ref.parent.mkdir(parents=True)
            shutil.copyfile(GOLDEN_REFERENCE, ref)
            path = root / "cases.json"
            path.write_text(json.dumps(manifest()))
            out = root / "iteration"
            args = SimpleNamespace(
                manifest=str(path), reference_root=str(root), out=str(out),
                hypothesis="The shipped-WASM path produces complete evidence.",
                changed_component="test-fixture", baseline_dir=None, drift_baseline=None,
                allow_dirty=True, skip_wasm_verify=True,
            )
            with contextlib.redirect_stdout(io.StringIO()):
                result = loop_campaign.run_campaign(args)
            self.assertIn(result, (0, 1))
            iteration = json.loads((out / "iteration.json").read_text())
            self.assertEqual(iteration["source"]["commit"], loop_campaign.git("rev-parse", "HEAD"))
            self.assertEqual(len(iteration["cases"]), 2)
            self.assertTrue((out / ".complete").is_file())
            self.assertTrue((out / "summary.md").is_file())
            self.assertEqual(loop_campaign.verify_iteration(out)["family"], "test")
            for item in iteration["cases"]:
                report = json.loads((out / item["report"]).read_text())
                self.assertEqual(report["metric_version"], loop_campaign.loop_metrics.METRIC_VERSION)
                self.assertEqual(report["inputs"]["render"]["sha256"], item["render_sha256"])
                self.assertEqual(report["configuration"]["expected_onset_s"], 0.05)
                self.assertIsNone(report["configuration"]["expected_f0"])
                self.assertIsNone(report["harmonic_partials"])
            with self.assertRaisesRegex(FileExistsError, "not an empty directory"):
                with contextlib.redirect_stdout(io.StringIO()):
                    loop_campaign.run_campaign(args)

            out2 = root / "iteration-2"
            args2 = copy.copy(args)
            args2.out = str(out2)
            args2.baseline_dir = str(out)
            args2.drift_baseline = str(root / "accepted-drift")
            def fake_drift(_baseline, drift_out):
                (drift_out / "drift.log").write_text("synthetic pass\n")
                return {"status": "pass", "log": "drift.log"}
            with mock.patch.object(loop_campaign, "run_drift", side_effect=fake_drift):
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(loop_campaign.run_campaign(args2), 0)
            second = loop_campaign.verify_iteration(out2)
            self.assertEqual(second["classification"], "listening_required")
            self.assertEqual(second["drift"]["status"], "pass")
            self.assertEqual(second["audition"], "listening/index.html")
            self.assertEqual(second["listening"]["protocol"], "ab")
            self.assertEqual(second["listening"]["trials"], 2)
            self.assertTrue((out2 / "listening" / "index.html").is_file())
            self.assertTrue((out2 / "listening" / "experiment.json").is_file())
            self.assertTrue((out2 / "listening-analysis.json").is_file())
            participant = json.loads((out2 / "listening" / "experiment.json").read_text())
            self.assertNotIn("candidate", json.dumps(participant))
            self.assertNotIn("incumbent", json.dumps(participant))
            self.assertTrue((out2 / "reference-registry-schema.json").is_file())
            self.assertTrue((out2 / "case-manifest.json").is_file())
            self.assertTrue((out2 / "case-schema.json").is_file())

            nested = out2 / "reports" / "evidence-digests.json"
            nested.write_text("must be sealed\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "file set changed"):
                loop_campaign.verify_iteration(out2)
            nested.unlink()

            registry_tamper = root / "registry-tamper"
            shutil.copytree(out2, registry_tamper)
            registry = loop_campaign.read_json(registry_tamper / "reference-registry.json")
            registry["description"] += " tampered"
            loop_campaign.write_json(registry_tamper / "reference-registry.json", registry)
            loop_campaign.seal_iteration(registry_tamper)
            with self.assertRaisesRegex(ValueError, "registry snapshot digest"):
                loop_campaign.verify_iteration(registry_tamper)

            identity_tamper = root / "identity-tamper"
            shutil.copytree(out2, identity_tamper)
            changed = loop_campaign.read_json(identity_tamper / "iteration.json")
            changed["cases"][0]["render_sha256"] = "0" * 64
            loop_campaign.write_json(identity_tamper / "iteration.json", changed)
            loop_campaign.seal_iteration(identity_tamper)
            with self.assertRaisesRegex(ValueError, "rendered audio identity"):
                loop_campaign.verify_iteration(identity_tamper)

            metadata_tamper = root / "metadata-tamper"
            shutil.copytree(out2, metadata_tamper)
            changed = loop_campaign.read_json(metadata_tamper / "iteration.json")
            changed["cases"][0]["render_metadata"]["midi"] += 1
            loop_campaign.write_json(metadata_tamper / "iteration.json", changed)
            loop_campaign.seal_iteration(metadata_tamper)
            with self.assertRaisesRegex(ValueError, "renderer metadata"):
                loop_campaign.verify_iteration(metadata_tamper)

            analysis_tamper = root / "analysis-tamper"
            shutil.copytree(out2, analysis_tamper)
            changed_manifest = loop_campaign.read_json(analysis_tamper / "case-manifest.json")
            changed_manifest["cases"][0]["analysis"]["profile"] = "default"
            loop_campaign.write_json(analysis_tamper / "case-manifest.json", changed_manifest)
            changed_iteration = loop_campaign.read_json(analysis_tamper / "iteration.json")
            changed_iteration["manifest"]["sha256"] = loop_campaign.sha256(analysis_tamper / "case-manifest.json")
            loop_campaign.write_json(analysis_tamper / "iteration.json", changed_iteration)
            loop_campaign.seal_iteration(analysis_tamper)
            with self.assertRaisesRegex(ValueError, "metric configuration"):
                loop_campaign.verify_iteration(analysis_tamper)

            with open(out2 / "summary.md", "a") as f:
                f.write("tamper\n")
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                loop_campaign.verify_iteration(out2)


if __name__ == "__main__":
    unittest.main()
