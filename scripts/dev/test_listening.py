#!/usr/bin/env python3
"""Deterministic tests for blind-listening evidence and analysis."""

import copy
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import jsonschema

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import listening


ROOT = Path(__file__).resolve().parents[2]
PILOT = ROOT / "evals" / "listening" / "pilot"
EXPERIMENT_PATH = PILOT / "experiment.json"
ANALYSIS_PATH = PILOT / "analysis-manifest.json"
RESULTS_PATH = PILOT / "synthetic-results.json"


def playback(slots, starts=1, completed=1, listened_ms=800):
    return {slot: {"starts": starts, "completed": completed, "listened_ms": listened_ms} for slot in slots}


class ManifestTests(unittest.TestCase):
    def test_pilot_analysis_key_and_every_raw_session_validate(self):
        experiment, analysis = listening.validate_analysis_manifest(ANALYSIS_PATH)
        self.assertEqual(analysis["experiment_digest"], listening.manifest_digest(experiment))
        digest = listening.manifest_digest(experiment)
        schema = listening.load_json(ROOT / "evals" / "listening" / "session-schema-v1.json")
        for session in listening.load_json(RESULTS_PATH):
            jsonschema.validate(session, schema)
            listening.validate_session(session, experiment, digest)

    def test_hidden_reference_role_contract_and_digest_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            copied = Path(directory) / "pilot"
            shutil.copytree(PILOT, copied)
            private = listening.load_json(copied / "analysis-manifest.json")
            private["trials"][0]["stimuli"][0]["role"] = "candidate"
            (copied / "analysis-manifest.json").write_text(json.dumps(private))
            with self.assertRaisesRegex(ValueError, "hidden reference"):
                listening.validate_analysis_manifest(copied / "analysis-manifest.json")

        with tempfile.TemporaryDirectory() as directory:
            copied = Path(directory) / "pilot"
            shutil.copytree(PILOT, copied)
            private = listening.load_json(copied / "analysis-manifest.json")
            private["experiment_digest"] = "f" * 64
            (copied / "analysis-manifest.json").write_text(json.dumps(private))
            with self.assertRaisesRegex(ValueError, "experiment digest"):
                listening.validate_analysis_manifest(copied / "analysis-manifest.json")

    def test_path_escape_fails_closed(self):
        value = listening.load_json(EXPERIMENT_PATH)
        value["trials"][0]["stimuli"][1]["path"] = "../secret.wav"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "experiment.json"
            path.write_text(json.dumps(value))
            with self.assertRaises((ValueError, jsonschema.ValidationError)):
                listening.validate_experiment(path, verify_files=False)

    def test_stimulus_ids_must_be_unique_across_trials(self):
        value = listening.load_json(EXPERIMENT_PATH)
        duplicate = copy.deepcopy(value["trials"][0])
        duplicate["id"] = "duplicate-trial"
        value["trials"].append(duplicate)
        value["exclusion_policy"]["min_completed_trials"] = 2
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "experiment.json"
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "globally unique"):
                listening.validate_experiment(path, verify_files=False)


class RandomizationTests(unittest.TestCase):
    def test_python_and_browser_javascript_vectors_match(self):
        experiment = listening.load_json(EXPERIMENT_PATH)
        seeds = [0, 1, 0x10010001, 0xFFFFFFFF]
        expected = {
            str(seed): {
                "presentations": listening.expected_presentations(experiment, seed),
                "trial_order": listening.expected_trial_order(experiment, seed),
            }
            for seed in seeds
        }
        script = f"""
          import {{ presentations, trialOrder }} from './evals/listening/randomization.js';
          const experiment = {json.dumps(experiment)};
          const seeds = {json.dumps(seeds)};
          console.log(JSON.stringify(Object.fromEntries(seeds.map((seed) => [String(seed), {{ presentations: presentations(experiment, seed), trial_order: trialOrder(experiment, seed) }}]))));
        """
        actual = json.loads(subprocess.check_output(["node", "--input-type=module", "-e", script], cwd=ROOT, text=True))
        self.assertEqual(actual, expected)

    def test_python_and_browser_manifest_digests_match_numeric_edges(self):
        experiment = listening.load_json(EXPERIMENT_PATH)
        script = f"""
          import {{ manifestDigest }} from './evals/listening/randomization.js';
          console.log(await manifestDigest({json.dumps(experiment)}));
        """
        browser_digest = subprocess.check_output(["node", "--input-type=module", "-e", script], cwd=ROOT, text=True).strip()
        self.assertEqual(browser_digest, listening.manifest_digest(experiment))
        fixture = {"small": -0.000039, "positive": 0.000004, "decimal": -22.999961, "integer_float": -23.0}
        script = f"""
          import {{ manifestDigest }} from './evals/listening/randomization.js';
          console.log(await manifestDigest({json.dumps(fixture)}));
        """
        browser_digest = subprocess.check_output(["node", "--input-type=module", "-e", script], cwd=ROOT, text=True).strip()
        self.assertEqual(browser_digest, listening.manifest_digest(fixture))
        with self.assertRaisesRegex(ValueError, "non-finite"):
            listening.canonical_json({"bad": float("nan")})
        self.assertNotIn("crypto.subtle", (ROOT / "evals" / "listening" / "randomization.js").read_text())

    def test_randomization_is_deterministic_and_position_balanced(self):
        ids = ["a", "b", "c"]
        self.assertEqual(listening.shuffled_ids(ids, 12345), listening.shuffled_ids(ids, 12345))
        counts = {item: [0, 0, 0] for item in ids}
        orders = set()
        seed = 0x12345678
        for _ in range(600):
            seed = listening.xorshift32(seed ^ 0x9E3779B9)
            order = listening.shuffled_ids(ids, seed)
            orders.add(tuple(order))
            for position, item in enumerate(order):
                counts[item][position] += 1
        self.assertEqual(len(orders), 6)
        for positions in counts.values():
            self.assertLess(max(positions) - min(positions), 55)

    def test_presentation_and_trial_order_tamper_are_rejected(self):
        experiment = listening.load_json(EXPERIMENT_PATH)
        digest = listening.manifest_digest(experiment)
        session = copy.deepcopy(listening.load_json(RESULTS_PATH)[0])
        session["trials"][0]["presentation"].reverse()
        with self.assertRaisesRegex(ValueError, "randomization mismatch"):
            listening.validate_session(session, experiment, digest)
        session = copy.deepcopy(listening.load_json(RESULTS_PATH)[0])
        session["trial_order"] = ["not-a-trial"]
        with self.assertRaisesRegex(ValueError, "trial order"):
            listening.validate_session(session, experiment, digest)


class AnalysisTests(unittest.TestCase):
    def setUp(self):
        self.experiment, self.analysis = listening.validate_analysis_manifest(ANALYSIS_PATH)
        self.sessions = listening.load_json(RESULTS_PATH)

    def test_synthetic_pilot_proves_exclusion_uncertainty_and_raw_retention(self):
        report = listening.analyze(self.experiment, self.sessions, self.analysis)
        self.assertEqual(report["n_submitted"], 6)
        self.assertEqual(report["n_included"], 5)
        self.assertEqual(report["exclusions"], [{"session_id": "synthetic-pilot-6", "reason": "hidden_reference_below_threshold:synthetic-tone-mushra"}])
        self.assertEqual(report["stimuli"]["condition-01"]["role"], "hidden_reference")
        self.assertEqual(report["stimuli"]["condition-03"]["role"], "anchor")
        self.assertEqual(report["stimuli"]["condition-02"]["mean"], 80.6)
        self.assertEqual(len(report["stimuli"]["condition-02"]["mean_ci95_bootstrap"]), 2)
        self.assertEqual(report["raw_sessions"], self.sessions)
        self.assertIsNone(report["quality_verdict"])
        self.assertEqual(report["evidence_kind_counts"]["human"], 0)

    def test_analysis_is_byte_deterministic(self):
        first = listening.canonical_json(listening.analyze(self.experiment, self.sessions, self.analysis))
        second = listening.canonical_json(listening.analyze(self.experiment, self.sessions, self.analysis))
        self.assertEqual(first, second)

    def test_incomplete_playback_is_declared_and_excluded(self):
        session = copy.deepcopy(self.sessions[0])
        session["session_id"] = "no-playback"
        session["listener"]["id"] = "no-playback-listener"
        session["trials"][0]["playback"]["condition-02"]["completed"] = 0
        report = listening.analyze(self.experiment, [session], self.analysis)
        self.assertEqual(report["n_included"], 0)
        self.assertEqual(report["exclusions"], [{"session_id": "no-playback", "reason": "insufficient_completed_playback:synthetic-tone-mushra"}])

        session = copy.deepcopy(self.sessions[0])
        session["session_id"] = "short-coverage"
        session["listener"]["id"] = "short-coverage-listener"
        session["trials"][0]["playback"]["condition-02"]["listened_ms"] = 40
        report = listening.analyze(self.experiment, [session], self.analysis)
        self.assertEqual(report["exclusions"], [{"session_id": "short-coverage", "reason": "insufficient_playback_coverage:synthetic-tone-mushra"}])

    def test_duplicate_sessions_listeners_and_mixed_evidence_fail_closed(self):
        first = copy.deepcopy(self.sessions[0])
        second = copy.deepcopy(self.sessions[1])
        second["session_id"] = first["session_id"]
        with self.assertRaisesRegex(ValueError, "duplicate session"):
            listening.analyze(self.experiment, [first, second], self.analysis)
        second = copy.deepcopy(self.sessions[1])
        second["listener"]["id"] = first["listener"]["id"]
        with self.assertRaisesRegex(ValueError, "duplicate listener"):
            listening.analyze(self.experiment, [first, second], self.analysis)
        second = copy.deepcopy(self.sessions[1])
        second["evidence_kind"] = "human"
        with self.assertRaisesRegex(ValueError, "cannot be pooled"):
            listening.analyze(self.experiment, [first, second], self.analysis)

    def test_abx_uncertainty_ab_ties_and_preferences_are_listener_level(self):
        experiment = {
            "id": "protocol-fixture",
            "purpose": "iteration",
            "trials": [
                {"id": "ab", "protocol": "ab", "stimuli": [{"id": "a"}, {"id": "b"}]},
                {"id": "abx", "protocol": "abx", "x": {"id": "x", "path": "opaque-x.wav"}, "stimuli": [{"id": "x-a"}, {"id": "x-b"}]},
            ],
            "exclusion_policy": {"min_completed_trials": 2, "hidden_reference_min_score": 90, "min_completed_plays_per_stimulus": 1, "unique_listener_ids_required": True},
        }
        digest = listening.manifest_digest(experiment)
        analysis = {
            "experiment_digest": digest,
            "trials": [
                {"id": "ab", "case_id": "ab-case", "stimuli": [{"id": "a", "role": "candidate", "duration_ms": 800}, {"id": "b", "role": "incumbent", "duration_ms": 800}]},
                {"id": "abx", "case_id": "abx-case", "x_source": "x-a", "stimuli": [{"id": "x-a", "role": "candidate", "duration_ms": 800}, {"id": "x-b", "role": "incumbent", "duration_ms": 800}]},
            ],
        }
        sessions = []
        choices = ["a", "a", "a", "tie"]
        for index, seed in enumerate([11, 22, 33, 44]):
            order = listening.expected_presentations(experiment, seed)
            trial_order = listening.expected_trial_order(experiment, seed)
            ab_slots = order["ab"]
            abx_slots = order["abx"] + ["x"]
            responses = {
                "ab": {"trial_id": "ab", "protocol": "ab", "presentation": order["ab"], "response": {"choice": choices[index]}, "play_counts": {slot: 1 for slot in ab_slots}, "playback": playback(ab_slots)},
                "abx": {"trial_id": "abx", "protocol": "abx", "presentation": order["abx"], "response": {"choice": "x-a" if index != 3 else "x-b"}, "play_counts": {slot: 1 for slot in abx_slots}, "playback": playback(abx_slots)},
            }
            sessions.append({
                "schema_version": "1.0.0", "experiment_id": experiment["id"], "experiment_digest": digest,
                "session_id": f"p-{index}", "evidence_kind": "human",
                "listener": {"id": f"p-{index}", "experience": "test", "hearing_notes": "none"},
                "setup": {"transducer": "headphones", "environment": "test", "device": "test", "volume_check": True},
                "randomization": {"algorithm": listening.RANDOMIZATION_ALGORITHM, "seed": seed},
                "trial_order": trial_order, "started_at": "x", "submitted_at": "y",
                "trials": [responses[trial_id] for trial_id in trial_order],
            })
        report = listening.analyze(experiment, sessions, analysis)
        self.assertEqual(report["stimuli"]["a"]["preference_count"], 3)
        self.assertEqual(report["stimuli"]["a"]["preference_total"], 4)
        self.assertEqual(report["stimuli"]["a"]["preference_decisive_total"], 3)
        self.assertEqual(report["stimuli"]["a"]["tie_count"], 1)
        self.assertEqual(report["abx"]["correct"], 3)
        self.assertEqual(report["abx"]["total"], 4)


class CampaignBundleTests(unittest.TestCase):
    def make_iteration_pair(self, root):
        baseline = root / "baseline"
        candidate = root / "candidate"
        for path in (baseline, candidate):
            (path / "renders").mkdir(parents=True)
        shutil.copyfile(PILOT / "audio" / "reference.wav", baseline / "renders" / "case-a.wav")
        shutil.copyfile(PILOT / "audio" / "condition-02.wav", candidate / "renders" / "case-a.wav")
        metadata = {"family": "piano", "midi": 60, "vel": 90, "onsetSeconds": 0.03, "noteOffSeconds": 1.03, "seconds": 2.0, "sampleRate": 48000, "float32": True}
        reference_contract = {
            "id": "ref.test.case-a", "corpus_id": "corpus.test", "status": "verified", "declared_path": "references/test/case-a.wav",
            "reference_sha256": "b" * 64, "contract_sha256": "c" * 64, "registry_sha256": "d" * 64, "registry_schema_sha256": "e" * 64,
        }
        common = {
            "family": "piano", "metric_version": "test-metric",
            "manifest": {"path": "case-manifest.json", "sha256": "a" * 64, "schema_path": "case-schema.json", "schema_sha256": "f" * 64},
            "reference_registry": {"path": "reference-registry.json", "sha256": "d" * 64, "schema_path": "reference-registry-schema.json", "schema_sha256": "e" * 64},
            "cases": [{"id": "case-a", "role": "tune", "reference_sha256": "b" * 64, "reference_contract": reference_contract, "render_metadata": metadata}],
        }
        (baseline / "iteration.json").write_text(json.dumps({**common, "source": {"commit": "1" * 40}}))
        (candidate / "iteration.json").write_text(json.dumps({**common, "source": {"commit": "2" * 40}}))
        return baseline, candidate

    def test_campaign_bundle_is_opaque_level_matched_self_contained_and_analyzable(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline, candidate = self.make_iteration_pair(Path(directory))
            result = listening.prepare_campaign_bundle(candidate, baseline, candidate / "listening")
            experiment, analysis = listening.validate_analysis_manifest(candidate / result["analysis_manifest"])
            self.assertEqual(result["trials"], 1)
            self.assertEqual(result["experiment_digest"], listening.manifest_digest(experiment))
            participant_json = json.dumps(experiment, sort_keys=True)
            self.assertNotIn("candidate", participant_json)
            self.assertNotIn("incumbent", participant_json)
            self.assertNotIn("222222", participant_json)
            self.assertNotIn(analysis["provenance"]["blinding_nonce"], participant_json)
            self.assertIn('content="experiment.json"', (candidate / "listening" / "index.html").read_text())
            roles = {item["role"] for item in analysis["trials"][0]["stimuli"]}
            self.assertEqual(roles, {"candidate", "incumbent"})
            self.assertEqual(analysis["trials"][0]["reference_contract"]["id"], "ref.test.case-a")
            self.assertEqual(analysis["provenance"]["reference_registry_sha256"], "d" * 64)
            old_role_tokens = {
                "condition-" + hashlib.sha256(f"{'2' * 40}:{'1' * 40}:case-a:{role}".encode()).hexdigest()[:16]
                for role in ("candidate", "incumbent")
            }
            self.assertTrue(old_role_tokens.isdisjoint({item["id"] for item in analysis["trials"][0]["stimuli"]}))
            for stimulus in analysis["trials"][0]["stimuli"]:
                self.assertAlmostEqual(stimulus["integrated_lufs_after"], -23.0, places=2)

            seed = 123
            trial = experiment["trials"][0]
            presentation = listening.expected_presentations(experiment, seed)[trial["id"]]
            candidate_id = next(item["id"] for item in analysis["trials"][0]["stimuli"] if item["role"] == "candidate")
            session = {
                "schema_version": "1.0.0", "experiment_id": experiment["id"], "experiment_digest": result["experiment_digest"],
                "session_id": "campaign-round-trip", "evidence_kind": "human",
                "listener": {"id": "listener", "experience": "test", "hearing_notes": "none"},
                "setup": {"transducer": "headphones", "environment": "test", "device": "test", "volume_check": True},
                "randomization": {"algorithm": listening.RANDOMIZATION_ALGORITHM, "seed": seed},
                "trial_order": listening.expected_trial_order(experiment, seed), "started_at": "x", "submitted_at": "y",
                "trials": [{"trial_id": trial["id"], "protocol": "ab", "presentation": presentation, "response": {"choice": candidate_id}, "play_counts": {item: 1 for item in presentation}, "playback": playback(presentation)}],
            }
            report = listening.analyze(experiment, [session], analysis)
            self.assertEqual(report["n_included"], 1)
            self.assertEqual(report["stimuli"][candidate_id]["preference_count"], 1)
            self.assertIsNone(report["quality_verdict"])

    def test_campaign_pair_protocol_mismatch_fails_before_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline, candidate = self.make_iteration_pair(Path(directory))
            value = json.loads((candidate / "iteration.json").read_text())
            value["cases"][0]["render_metadata"]["midi"] = 61
            (candidate / "iteration.json").write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "render protocol differs"):
                listening.prepare_campaign_bundle(candidate, baseline, candidate / "listening")

    def test_campaign_reference_contract_mismatch_fails_before_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline, candidate = self.make_iteration_pair(Path(directory))
            value = json.loads((candidate / "iteration.json").read_text())
            value["cases"][0]["reference_contract"]["contract_sha256"] = "0" * 64
            (candidate / "iteration.json").write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "reference contract differs"):
                listening.prepare_campaign_bundle(candidate, baseline, candidate / "listening")
            self.assertFalse((candidate / "listening").exists())

    def test_campaign_private_nonce_mapping_tamper_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline, candidate = self.make_iteration_pair(Path(directory))
            result = listening.prepare_campaign_bundle(candidate, baseline, candidate / "listening")
            analysis_path = candidate / result["analysis_manifest"]
            value = listening.load_json(analysis_path)
            value["provenance"]["blinding_nonce"] = "0" * 64
            analysis_path.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "blinding mapping"):
                listening.validate_analysis_manifest(analysis_path)


if __name__ == "__main__":
    unittest.main()
