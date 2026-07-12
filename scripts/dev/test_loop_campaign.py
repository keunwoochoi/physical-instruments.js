#!/usr/bin/env python3
"""Corpus-free validation for the declarative loop campaign runner."""

import copy
import contextlib
import io
import json
import os
import shutil
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


def case(case_id, role, reference="references/test/reference.wav", required_axes=None):
    return {
        "id": case_id,
        "role": role,
        "reference": reference,
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
        "schema_version": "1.0.0",
        "family": "test",
        "description": "Equation-fixture runner test.",
        "cases": [case("tune-a", "tune"), case("hold-b", "held_out")],
    }


class ManifestTests(unittest.TestCase):
    def write_manifest(self, directory, value):
        path = Path(directory) / "cases.json"
        path.write_text(json.dumps(value))
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

    def test_reference_path_cannot_escape_root(self):
        value = manifest()
        value["cases"][0]["reference"] = "../secret.wav"
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "safe path"):
                loop_campaign.validate_manifest(self.write_manifest(d, value))

    def test_corpus_contract_contradiction_fails_before_render(self):
        value = manifest()
        for item in value["cases"]:
            item["reference"] = "references/guitar-acoustic/canonical/reference.wav"
            item["analysis"]["required_axes"] = ["mr_stft", "lufs"]
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ref = root / "references" / "guitar-acoustic" / "canonical" / "reference.wav"
            ref.parent.mkdir(parents=True)
            audio, sr = loop_campaign.loop_metrics.load_mono(GOLDEN_REFERENCE)
            audio = loop_campaign.loop_metrics.resample_to(audio, sr, 16000)
            loop_campaign.sf.write(ref, audio, 16000, subtype="FLOAT")
            path = self.write_manifest(d, value)
            loaded = loop_campaign.validate_manifest(path)
            with self.assertRaisesRegex(ValueError, "invalid corpus axes"):
                loop_campaign.resolve_cases(loaded, root)


class BaselineTests(unittest.TestCase):
    def test_drum_note_numbers_are_not_misread_as_fundamentals(self):
        drum = case("maraca", "tune")
        drum["render"]["family"] = "drums-808"
        drum["render"]["midi"] = 70
        self.assertIsNone(loop_campaign.expected_f0(drum))
        pitched = case("a4", "tune")
        self.assertAlmostEqual(loop_campaign.expected_f0(pitched), 440.0)

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


class EndToEndTests(unittest.TestCase):
    def test_four_family_pilot_staging_is_complete_and_corpus_rate_correct(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "refs"
            result = stage_loop_pilot_refs.stage(out)
            self.assertEqual(len(result["entries"]), 16)
            rates = {entry["sample_rate"] for entry in result["entries"]}
            self.assertEqual(rates, {16000, 44100, 48000})
            self.assertTrue((out / "sources.json").is_file())
            with self.assertRaisesRegex(FileExistsError, "not empty"):
                stage_loop_pilot_refs.stage(out)

    def test_equation_reference_runs_through_shipped_wasm_and_writes_immutable_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ref = root / "references" / "test" / "reference.wav"
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
            with self.assertRaisesRegex(FileExistsError, "not empty"):
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
            self.assertTrue((out2 / "audition.html").is_file())

            with open(out2 / "summary.md", "a") as f:
                f.write("tamper\n")
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                loop_campaign.verify_iteration(out2)


if __name__ == "__main__":
    unittest.main()
