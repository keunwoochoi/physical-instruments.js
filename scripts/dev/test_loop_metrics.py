#!/usr/bin/env python3
"""Synthetic and metamorphic validation for the loop metric kernel."""

import json
import os
import sys
import tempfile
import unittest

import numpy as np
import jsonschema
import soundfile as sf

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import loop_metrics as compare


SR = 48_000


def tone(freq=440.0, seconds=0.6, amp=0.25, lead=0.0):
    n = int(seconds * SR)
    t = np.arange(n) / SR
    x = amp * np.sin(2 * np.pi * freq * t)
    if lead:
        x = np.concatenate([np.zeros(int(lead * SR)), x])
    return x.astype(np.float64)


class ResamplingTests(unittest.TestCase):
    def test_in_band_tone_level_is_preserved(self):
        sr = 96_000
        t = np.arange(sr) / sr
        x = 0.25 * np.sin(2 * np.pi * 1000 * t)
        y = compare.resample_to(x, sr, 48_000)
        interior = y[1000:-1000]
        rms = np.sqrt(np.mean(interior ** 2))
        self.assertAlmostEqual(rms, 0.25 / np.sqrt(2), delta=3e-4)

    def test_above_nyquist_tone_is_rejected_not_aliased(self):
        sr = 96_000
        t = np.arange(sr) / sr
        x = np.sin(2 * np.pi * 30_000 * t)
        y = compare.resample_to(x, sr, 48_000)
        self.assertLess(np.sqrt(np.mean(y[1000:-1000] ** 2)), 0.01)


class TrustGateTests(unittest.TestCase):
    def test_clean_tone_passes_required_gates(self):
        x = tone()
        gates = compare.artifact_gates(x, x, SR)
        self.assertTrue(gates["all_pass"])
        self.assertIsNone(gates["pre_onset_energy"]["pass"])
        self.assertIsNone(gates["release_discontinuity"]["pass"])

    def test_single_sample_flip_fails_jump_gate(self):
        x = tone()
        x[1000] = 1.0
        x[1001] = -1.0
        gates = compare.artifact_gates(x, tone(), SR)
        self.assertFalse(gates["max_sample_jump"]["pass"])
        self.assertFalse(gates["trusted"])

    def test_dc_is_peak_relative(self):
        x = tone() + 0.02
        gates = compare.artifact_gates(x, tone(), SR)
        self.assertFalse(gates["dc_offset"]["pass"])

    def test_nonfinite_render_fails_closed(self):
        x = tone()
        x[100] = np.nan
        gates = compare.artifact_gates(x, tone(), SR)
        self.assertFalse(gates["finite"]["pass"])
        self.assertFalse(gates["trusted"])

    def test_hard_clipping_occupancy_fails(self):
        x = np.clip(tone(amp=2.0), -1.0, 1.0)
        gates = compare.artifact_gates(x, tone(), SR)
        self.assertFalse(gates["clipping"]["pass"])

    def test_ultrasonic_injection_fails(self):
        x = tone(440, amp=0.05)
        t = np.arange(len(x)) / SR
        x += 0.3 * np.sin(2 * np.pi * 19_000 * t)
        gates = compare.artifact_gates(x, tone(), SR)
        self.assertFalse(gates["ultrasonic_ratio"]["pass"])

    def test_declared_pre_onset_noise_fails(self):
        x = tone(lead=0.05)
        x[: int(0.045 * SR)] = 0.03
        gates = compare.artifact_gates(x, x, SR, expected_onset_s=0.05)
        self.assertFalse(gates["pre_onset_energy"]["pass"])

    def test_release_discontinuity_fails(self):
        x = tone(seconds=0.6)
        at = int(0.3 * SR)
        x[at:] += 0.5
        gates = compare.artifact_gates(x, tone(), SR, note_off_s=0.3)
        self.assertFalse(gates["release_discontinuity"]["pass"])

    def test_single_sample_release_window_does_not_crash(self):
        x = np.asarray([0.25], dtype=np.float64)
        gates = compare.artifact_gates(x, x, SR, note_off_s=0.0)
        self.assertEqual(gates["max_sample_jump"]["value"], 0.0)
        self.assertEqual(gates["release_discontinuity"]["peak_ratio"], 0.0)

    def test_silent_signal_fails_energy_gate(self):
        x = np.zeros(SR, dtype=np.float64)
        gates = compare.artifact_gates(x, x, SR)
        self.assertFalse(gates["signal_energy"]["pass"])
        self.assertFalse(gates["trusted"])


class AlignmentAndDistanceTests(unittest.TestCase):
    def test_bounded_alignment_reports_shift_and_restores_identity(self):
        x = tone(lead=0.005)
        y = np.concatenate([np.zeros(120), x])
        xr, yf, meta = compare.onset_align(y, x, SR)
        self.assertEqual(meta["status"], "applied")
        self.assertEqual(meta["lag_samples"], 120)
        n = min(len(xr), len(yf))
        np.testing.assert_allclose(xr[:n], yf[:n], atol=1e-12)

    def test_excessive_alignment_is_rejected(self):
        x = tone(lead=0.002)
        y = np.concatenate([np.zeros(int(0.02 * SR)), x])
        _, _, meta = compare.onset_align(y, x, SR, max_lag_s=0.01)
        self.assertEqual(meta["status"], "rejected")

    def test_silent_alignment_is_not_arbitrarily_applied(self):
        x = np.zeros(SR, dtype=np.float64)
        _, _, meta = compare.onset_align(x, x, SR)
        self.assertEqual(meta["status"], "not_evaluated")
        self.assertEqual(meta["reason"], "silent_or_constant_onset_window")

    def test_constant_alignment_is_not_arbitrarily_applied(self):
        x = np.full(SR, 0.25, dtype=np.float64)
        _, _, meta = compare.onset_align(x, x, SR)
        self.assertEqual(meta["status"], "not_evaluated")

    def test_identical_mr_stft_is_zero(self):
        x = tone()
        result = compare.mr_stft_dist(x, x, SR)
        self.assertEqual(result["mean"], 0.0)


class TrajectoryAndStructureTests(unittest.TestCase):
    def test_decay_mutation_increases_envelope_trajectory_distance(self):
        t = np.arange(SR) / SR
        slow = np.sin(2 * np.pi * 440 * t) * np.exp(-2 * t)
        fast = np.sin(2 * np.pi * 440 * t) * np.exp(-6 * t)
        identity = compare.trajectory_diagnostics(slow, slow, SR, 30)
        mutation = compare.trajectory_diagnostics(fast, slow, SR, 30)
        self.assertEqual(identity["envelope_db"]["distance"]["cost"], 0.0)
        self.assertGreater(mutation["envelope_db"]["distance"]["cost"], 1.0)

    def test_decay_mutation_increases_partial_decay_trajectory_distance(self):
        t = np.arange(SR) / SR
        slow = np.sin(2 * np.pi * 440 * t) * np.exp(-2 * t)
        fast = np.sin(2 * np.pi * 440 * t) * np.exp(-6 * t)
        identity = compare.trajectory_diagnostics(slow, slow, SR, 30, 440.0)
        mutation = compare.trajectory_diagnostics(fast, slow, SR, 30, 440.0)
        self.assertEqual(identity["partial_decay_db"]["distances"]["1"]["cost"], 0.0)
        self.assertGreater(mutation["partial_decay_db"]["distances"]["1"]["cost"], 1.0)

    def test_bounded_warp_reports_and_limits_displacement(self):
        a = [0, 1, 2, 3, 4, 5]
        b = [0, 0, 1, 2, 3, 4]
        rigid = compare.bounded_dtw(a, b, 0)
        warped = compare.bounded_dtw(a, b, 2)
        self.assertLess(warped["cost"], rigid["cost"])
        self.assertLessEqual(warped["max_displacement_frames"], 2)

    def test_detune_increases_fundamental_aware_partial_residual(self):
        reference = tone(440, seconds=0.8)
        identity = compare.match_harmonic_partials(reference, reference, SR, 440.0)
        detuned = compare.match_harmonic_partials(tone(445, seconds=0.8), reference, SR, 440.0)
        self.assertEqual(identity["mean_abs_cents"], 0.0)
        self.assertGreater(detuned["mean_abs_cents"], 2.0)

    def test_stiff_string_targets_move_upper_partials_without_moving_fundamental(self):
        harmonic = compare.partial_targets(110.0, count=4)
        stiff = compare.partial_targets(110.0, count=4, model={
            "type": "stiff_string", "inharmonicity_b": 0.001, "search_cents": 90.0,
        })
        self.assertAlmostEqual(stiff[0]["frequency_hz"], harmonic[0]["frequency_hz"])
        self.assertGreater(stiff[3]["frequency_hz"], harmonic[3]["frequency_hz"])

    def test_modal_ratio_targets_do_not_assume_integer_harmonics(self):
        targets = compare.partial_targets(100.0, model={
            "type": "modal_ratios", "ratios": [1.0, 1.59, 2.14], "search_cents": 60.0,
        })
        self.assertEqual([round(item["frequency_hz"], 1) for item in targets], [100.0, 159.0, 214.0])

    def test_silent_partial_analysis_is_empty_not_exceptional(self):
        silent = np.zeros(SR, dtype=np.float64)
        self.assertEqual(compare.harmonic_partials(silent, SR, 440.0), [])

    def test_stronger_beating_moves_envelope_axis_monotonically(self):
        t = np.arange(SR) / SR
        carrier = np.sin(2 * np.pi * 440 * t) * np.exp(-1.5 * t)
        mild = carrier * (1.0 + 0.08 * np.sin(2 * np.pi * 4 * t))
        strong = carrier * (1.0 + 0.30 * np.sin(2 * np.pi * 4 * t))
        identity = compare.trajectory_diagnostics(carrier, carrier, SR, 30)["envelope_db"]["distance"]["cost"]
        mild_cost = compare.trajectory_diagnostics(mild, carrier, SR, 30)["envelope_db"]["distance"]["cost"]
        strong_cost = compare.trajectory_diagnostics(strong, carrier, SR, 30)["envelope_db"]["distance"]["cost"]
        self.assertEqual(identity, 0.0)
        self.assertGreater(mild_cost, identity)
        self.assertGreater(strong_cost, mild_cost)

    def test_stronger_transient_moves_centroid_axis_monotonically(self):
        t = np.arange(SR) / SR
        base = np.sin(2 * np.pi * 440 * t) * np.exp(-2.0 * t)
        click = np.zeros_like(base)
        burst_frames = int(0.01 * SR)
        click[:burst_frames] = np.sin(2 * np.pi * 8000 * t[:burst_frames]) * np.hanning(burst_frames)
        mild = base + 0.05 * click
        strong = base + 0.30 * click
        identity = compare.trajectory_diagnostics(base, base, SR, 30)["centroid_hz"]["regions_semitones"]["attack"]["cost"]
        mild_cost = compare.trajectory_diagnostics(mild, base, SR, 30)["centroid_hz"]["regions_semitones"]["attack"]["cost"]
        strong_cost = compare.trajectory_diagnostics(strong, base, SR, 30)["centroid_hz"]["regions_semitones"]["attack"]["cost"]
        self.assertEqual(identity, 0.0)
        self.assertGreater(mild_cost, identity)
        self.assertGreater(strong_cost, mild_cost)

    def test_stereo_collapse_moves_width_and_correlation_axes(self):
        t = np.arange(SR) / SR
        left = np.sin(2 * np.pi * 440 * t)
        right = np.sin(2 * np.pi * 440 * t + np.pi / 2)
        wide = compare.stereo_stats(np.column_stack([left, right]))
        collapsed = compare.stereo_stats(np.column_stack([left, left]))
        self.assertGreater(wide["width_db"], collapsed["width_db"])
        self.assertLess(wide["correlation"], collapsed["correlation"])

    def test_silent_stereo_diagnostics_are_explicitly_unavailable(self):
        stats = compare.stereo_stats(np.zeros((SR, 2), dtype=np.float64))
        self.assertIsNone(stats["width_db"])
        self.assertIsNone(stats["correlation"])

    def test_profiles_own_thresholds(self):
        self.assertEqual(compare.PROFILES["kick"]["thresholds"]["max_warp_ms"], 10.0)
        self.assertGreater(compare.PROFILES["cymbal"]["thresholds"]["max_ultrasonic_ratio"], compare.PROFILES["default"]["thresholds"]["max_ultrasonic_ratio"])


class ReportContractTests(unittest.TestCase):
    def test_reports_are_deterministic_and_content_addressed(self):
        x = tone(seconds=0.7)
        with tempfile.TemporaryDirectory() as d:
            a = os.path.join(d, "candidate.wav")
            b = os.path.join(d, "reference.wav")
            sf.write(a, x, SR, subtype="FLOAT")
            sf.write(b, x, SR, subtype="FLOAT")
            first = compare.compare_files(a, b)
            second = compare.compare_files(a, b)
        self.assertEqual(first, second)
        self.assertEqual(first["schema_version"], compare.REPORT_SCHEMA_VERSION)
        self.assertEqual(first["metric_version"], compare.METRIC_VERSION)
        self.assertEqual(first["inputs"]["render"]["sha256"], first["inputs"]["reference"]["sha256"])

    def test_report_schema_rejects_missing_contract_field(self):
        x = tone(seconds=0.7)
        with tempfile.TemporaryDirectory() as d:
            a = os.path.join(d, "candidate.wav")
            b = os.path.join(d, "reference.wav")
            sf.write(a, x, SR, subtype="FLOAT")
            sf.write(b, x, SR, subtype="FLOAT")
            report = compare.compare_files(a, b)
        del report["metric_version"]
        with self.assertRaises(jsonschema.ValidationError):
            compare.validate_report(report)

    def test_nonfinite_file_aborts_before_distances(self):
        x = tone(seconds=0.7)
        broken = x.copy()
        broken[100] = np.nan
        with tempfile.TemporaryDirectory() as d:
            a = os.path.join(d, "candidate.wav")
            b = os.path.join(d, "reference.wav")
            sf.write(a, broken, SR, subtype="FLOAT")
            sf.write(b, x, SR, subtype="FLOAT")
            with self.assertRaisesRegex(ValueError, "NaN or infinite"):
                compare.compare_files(a, b)

    def test_zero_frame_file_fails_with_actionable_error(self):
        with tempfile.TemporaryDirectory() as d:
            a = os.path.join(d, "empty.wav")
            b = os.path.join(d, "reference.wav")
            sf.write(a, np.asarray([], dtype=np.float32), SR, subtype="FLOAT")
            sf.write(b, tone(), SR, subtype="FLOAT")
            with self.assertRaisesRegex(ValueError, "zero frames"):
                compare.compare_files(a, b)

    def test_rejected_alignment_marks_report_untrusted_without_aborting(self):
        reference = tone(lead=0.002)
        delayed = np.concatenate([np.zeros(int(0.02 * SR)), reference])
        with tempfile.TemporaryDirectory() as d:
            a = os.path.join(d, "candidate.wav")
            b = os.path.join(d, "reference.wav")
            sf.write(a, delayed, SR, subtype="FLOAT")
            sf.write(b, reference, SR, subtype="FLOAT")
            report = compare.compare_files(a, b)
        self.assertEqual(report["mr_stft"]["alignment"]["status"], "rejected")
        self.assertFalse(report["gates"]["alignment"]["pass"])
        self.assertEqual(report["interpretation"], "untrusted")

    def test_silent_file_returns_finite_serializable_untrusted_report(self):
        silent = np.zeros(SR, dtype=np.float64)
        with tempfile.TemporaryDirectory() as d:
            a = os.path.join(d, "candidate.wav")
            b = os.path.join(d, "reference.wav")
            sf.write(a, silent, SR, subtype="FLOAT")
            sf.write(b, silent, SR, subtype="FLOAT")
            report = compare.compare_files(a, b)
        self.assertEqual(report["interpretation"], "untrusted")
        self.assertFalse(report["gates"]["signal_energy"]["pass"])
        self.assertIsNone(report["lufs"]["render"])
        json.dumps(report, allow_nan=False)

    def test_invalid_loudness_axis_is_removed(self):
        report = {
            "lufs": {"render": -20.0, "reference": -22.0, "delta": 2.0},
            "partial_decay_dbps": {"render": [], "reference": []},
            "envelope": {"render": {"t60_early": 1, "t60_late": 2, "time_to_peak_ms": 1},
                         "reference": {"t60_early": 1, "t60_late": 2, "time_to_peak_ms": 1}},
            "logmel_dist": {"attack": 1, "tail": 1},
            "centroid": {"render": {"attack": 1}, "reference": {"attack": 1}},
        }
        compare.disable_invalid_axes(report, {"lufs": "normalized"})
        self.assertFalse(report["lufs"]["valid"])
        self.assertIsNone(report["lufs"]["delta"])

    def test_explicit_contract_disables_invalid_axis_in_full_report(self):
        x = tone(seconds=0.7)
        with tempfile.TemporaryDirectory() as d:
            ref_dir = os.path.join(d, "references", "guitar-acoustic")
            os.makedirs(ref_dir)
            a = os.path.join(d, "candidate.wav")
            b = os.path.join(ref_dir, "reference.wav")
            sf.write(a, x, SR, subtype="FLOAT")
            sf.write(b, x, SR, subtype="FLOAT")
            evidence = {
                "id": "ref.test", "corpus_id": "corpus.test", "status": "verified",
                "declared_path": "references/test.wav", "reference_sha256": "0" * 64,
                "contract_sha256": "1" * 64, "registry_sha256": "2" * 64,
                "registry_schema_sha256": "3" * 64,
            }
            bound = {"contract": {"mask_after_s": 3.3, "invalid_axes": {"lufs": "normalized", "release": "hard gate"}}, "evidence": evidence}
            report = compare.compare_files(a, b, reference_contract=bound)
        self.assertFalse(report["axis_validity"]["lufs"]["valid"])
        self.assertIsNone(report["lufs"]["delta"])
        self.assertIn("release", report["axis_validity"])


class GoldenFixtureTests(unittest.TestCase):
    def test_committed_equation_fixtures_match_versioned_golden_report(self):
        root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
        fixture_dir = os.path.join(root, "evals", "metrics", "loop-v1")
        with open(os.path.join(fixture_dir, "expected.json")) as f:
            expected = json.load(f)
        ref = os.path.join(fixture_dir, "reference.wav")
        candidate = os.path.join(fixture_dir, "candidate-artifact.wav")
        identity = compare.compare_files(ref, ref, expected_onset_s=0.05, expected_f0=440.0)
        mutation = compare.compare_files(candidate, ref, expected_onset_s=0.05, expected_f0=440.0)

        self.assertEqual(identity["inputs"]["render"]["sha256"], expected["identity"]["render_sha256"])
        self.assertEqual(identity["mr_stft"], expected["identity"]["mr_stft"])
        self.assertEqual(identity["gates"], expected["identity"]["gates"])
        self.assertEqual(mutation["inputs"]["render"]["sha256"], expected["artifact_mutation"]["render_sha256"])
        self.assertEqual(mutation["mr_stft"], expected["artifact_mutation"]["mr_stft"])
        self.assertEqual(mutation["logmel_dist"], expected["artifact_mutation"]["logmel_dist"])
        self.assertEqual(mutation["trajectories"], expected["artifact_mutation"]["trajectories"])
        self.assertEqual(mutation["harmonic_partials"], expected["artifact_mutation"]["harmonic_partials"])
        self.assertEqual(mutation["stereo"], expected["artifact_mutation"]["stereo"])
        self.assertEqual(mutation["gates"], expected["artifact_mutation"]["gates"])
        self.assertEqual(mutation["interpretation"], "untrusted")


if __name__ == "__main__":
    unittest.main()
