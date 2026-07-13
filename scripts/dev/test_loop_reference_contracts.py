#!/usr/bin/env python3
"""Fail-closed reference-contract registry and binding tests."""

import copy
import json
import os
import shutil
import struct
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import soundfile as sf

import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import loop_campaign
import reference_contracts
import canonicalize_reference_receipt


ROOT = Path(__file__).resolve().parents[2]
GOLDEN = ROOT / "evals" / "metrics" / "loop-v1" / "reference.wav"
FIXTURE_ID = "ref.equation.loop-pilot-v1.48000"
FIXTURE_PATH = "references/equation-loop-pilot-v1/reference-48000.wav"
FIXTURE_SHA = "0a3664c121555f0c9e55d85ed0fca97ab72194b9525e65f5a82b71f95a322059"


def fixture_case(**updates):
    value = {
        "id": "fixture",
        "reference": FIXTURE_PATH,
        "reference_contract_id": FIXTURE_ID,
        "reference_sha256": FIXTURE_SHA,
        "analysis": {"required_axes": ["mr_stft", "attack"]},
    }
    value.update(updates)
    return value


class RegistryTests(unittest.TestCase):
    def setUp(self):
        self.registry = json.loads(reference_contracts.REGISTRY_PATH.read_text(encoding="utf-8"))

    def write(self, directory, value):
        path = Path(directory) / "registry.json"
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def test_committed_registry_is_valid_and_exact(self):
        loaded = reference_contracts.load_registry()
        self.assertEqual(len(loaded["contracts"]), 28)
        self.assertEqual(sum(c["status"] == "verified" for c in loaded["contracts"].values()), 16)

    def test_exact_source_receipts_bind_every_promoted_contract(self):
        expected = {
            "piano-salamander-attack-v1.json": {
                "ref.salamander.grand-v3.a1-v12.attack-canon-v1": "bba308985d65e792212bf3f01aaf2766276a05f0b85fe1474f04920db919f7f8",
                "ref.salamander.grand-v3.c4-v2.attack-canon-v1": "3884d4936969f254a9756618cd0e8bb2b795ce62183a2ea465f5199fa6614b3c",
                "ref.salamander.grand-v3.c4-v16.attack-canon-v1": "14ac208246645106af9eb5cc10a3d3eb39c1ac577d5e05c9425b4de6e1509d7b",
                "ref.salamander.grand-v3.c5-v12.cs5-attack-canon-v1": "c572f0601cfd4114a430b5b21079bcdcdecbf4b372c2d1cbc08cb07f2db98e83",
            },
            "drums-jazz-virtuosity-kick-v1.json": {
                "ref.virtuosity-drums.kick-close-snoff-vl1-rr1.canon-v1": "6713a4823bb778957a04b72ee0b580beb2b08ca7c1f08660d2d8323af4754c17",
                "ref.virtuosity-drums.kick-close-snoff-vl3-rr1.canon-v1": "001c0b203d04bbc1c21654fe5e95e526d039a25bcbb5ee9b6bc7aaa2932e891d",
                "ref.virtuosity-drums.kick-close-snoff-vl4-rr1.canon-v1": "1a21facffa9659c768f78c8aac83813c2f4d32fbd948bbdc9f42a3e657c01797",
            },
        }
        for filename, identities in expected.items():
            with self.subTest(receipt=filename):
                receipt = canonicalize_reference_receipt.load_receipt(ROOT / "evals" / "reference-receipts" / filename)
                self.assertEqual({entry["contract_id"]: entry["canonical_sha256"] for entry in receipt["entries"]}, identities)

    def test_canonicalizer_pins_toolchain_and_peak_identity(self):
        canonicalize_reference_receipt.verify_toolchain("1.0.31")
        with tempfile.TemporaryDirectory() as d:
            license_path = Path(d) / "LICENSE"
            license_path.write_text("fixture license", encoding="utf-8")
            source = {"license_path": "LICENSE", "license_sha256": canonicalize_reference_receipt.sha256(license_path)}
            canonicalize_reference_receipt.verify_license(source, d)
            source["license_sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "license identity mismatch"):
                canonicalize_reference_receipt.verify_license(source, d)
            path = Path(d) / "fixture.wav"
            payload = b"\x00" * 4
            path.write_bytes(
                b"RIFF" + struct.pack("<I", 52 + len(payload)) + b"WAVE"
                + b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, 48000, 192000, 4, 32)
                + b"PEAK" + struct.pack("<IIIfI", 16, 1, 123, 0.0, 0)
                + b"data" + struct.pack("<I", len(payload)) + payload
            )
            canonicalize_reference_receipt.pin_peak_timestamp(path, 1783887255)
            data = path.read_bytes()
            offset = data.index(b"PEAK")
            self.assertEqual(struct.unpack_from("<I", data, offset + 12)[0], 1783887255)

    def test_a43_contract_table_matches_owner_evidence(self):
        expected = {
            "ref.a43.freesound-9698.kick-short.canon-v2": ("references/drums-808-original/a43/canonical/kick-short-9698.wav", "6e3d0f4cec53f431141d3aa3a0b9deb9f016ede4a316a01c64edd675a8aeb32f", {"lufs", "velocity_loudness", "decay", "release"}),
            "ref.a43.freesound-11378.snare.canon-v2": ("references/drums-808-original/a43/canonical/snare-11378.wav", "0b5d938cb1822c07d046f483720f8d748a166952d93dac90ab834516d06b69a4", {"lufs", "velocity_loudness", "decay", "release"}),
            "ref.a43.freesound-9877.closed-hat.canon-v2": ("references/drums-808-original/a43/canonical/closed-hat-9877.wav", "fcd3e19a8a38f765f30349c2afa0cc8312c07d5bdd6dfda08c57490d324d30cc", {"lufs", "velocity_loudness"}),
            "ref.a43.freesound-9697.maraca.canon-v2": ("references/drums-808-original/a43/canonical/maraca-9697.wav", "a7ab479cb41d562d1001f2151324567aad25157a1cd34ce2158c6372708e7ae3", {"lufs", "velocity_loudness", "decay", "release"}),
            "ref.a43.freesound-9878.open-hat.canon-v2": ("references/drums-808-original/a43/canonical/open-hat-9878.wav", "a85e47ccb63fd5cbff51f7e3d9db20c26e29a928e916d92a186abf9e4b25988a", {"lufs", "velocity_loudness"}),
            "ref.a43.freesound-9780.cowbell.canon-v2": ("references/drums-808-original/a43/canonical/cowbell-9780.wav", "05d8766d09088a0358102399af216d1d36c6ddc6c6e308012ffed086e77f5ced", {"lufs", "velocity_loudness"}),
        }
        loaded = reference_contracts.load_registry()["contracts"]
        for contract_id, (path, digest, axes) in expected.items():
            contract = loaded[contract_id]
            self.assertEqual((contract["reference_path"], contract["canonical_sha256"], contract["sample_rate"], contract["level_normalized"]), (path, digest, 48000, False))
            self.assertEqual(set(contract["invalid_axes"]), axes)

    def test_duplicate_json_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "registry.json"
            path.write_text('{"schema_version":"2.0.0","schema_version":"2.0.0","corpora":[],"contracts":[]}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
                reference_contracts.load_registry(path)

    def test_duplicate_ids_and_paths_are_rejected(self):
        for field in ("id", "reference_path"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as d:
                value = copy.deepcopy(self.registry)
                duplicate = copy.deepcopy(value["contracts"][0])
                if field == "id":
                    duplicate["reference_path"] = "references/equation-loop-pilot-v1/duplicate.wav"
                else:
                    duplicate["id"] = "ref.equation.loop-pilot-v1.duplicate"
                value["contracts"].append(duplicate)
                with self.assertRaisesRegex(ValueError, "duplicate"):
                    reference_contracts.load_registry(self.write(d, value))

    def test_foreign_key_and_unsafe_path_are_rejected(self):
        mutations = [
            ("corpus_id", "corpus.missing", "unknown corpus_id"),
            ("reference_path", "references/../escape.wav", "safe POSIX-relative"),
        ]
        for field, replacement, message in mutations:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as d:
                value = copy.deepcopy(self.registry)
                value["contracts"][0][field] = replacement
                with self.assertRaisesRegex(ValueError, message):
                    reference_contracts.load_registry(self.write(d, value))

    def test_verification_semantics_are_enforced(self):
        with tempfile.TemporaryDirectory() as d:
            value = copy.deepcopy(self.registry)
            value["contracts"][0]["corpus_id"] = "corpus.legacy.acoustic-drums.v1"
            with self.assertRaisesRegex(ValueError, "unverified corpus"):
                reference_contracts.load_registry(self.write(d, value))
        for license_value in (None, "", "   "):
            with self.subTest(license=license_value), tempfile.TemporaryDirectory() as d:
                value = copy.deepcopy(self.registry)
                value["corpora"][0]["license"] = license_value
                with self.assertRaisesRegex(ValueError, "requires a non-empty license"):
                    reference_contracts.load_registry(self.write(d, value))
        with tempfile.TemporaryDirectory() as d:
            value = copy.deepcopy(self.registry)
            value["contracts"][0]["invalid_axes"] = {"attak": "typo"}
            with self.assertRaisesRegex(Exception, "attak"):
                reference_contracts.load_registry(self.write(d, value))
        with tempfile.TemporaryDirectory() as d:
            value = copy.deepcopy(self.registry)
            contract = next(c for c in value["contracts"] if c["status"] == "unverified")
            contract["canonical_sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "identity fields must be null"):
                reference_contracts.load_registry(self.write(d, value))


class BindingTests(unittest.TestCase):
    def setUp(self):
        self.registry = reference_contracts.load_registry()

    def stage(self, root):
        path = Path(root) / FIXTURE_PATH
        path.parent.mkdir(parents=True)
        shutil.copyfile(GOLDEN, path)
        return path

    def test_verified_fixture_binds_exact_identity(self):
        with tempfile.TemporaryDirectory() as d:
            path = self.stage(d)
            bound = reference_contracts.bind_reference(self.registry, fixture_case(), Path(d))
            self.assertEqual(bound["path"], path.resolve())
            self.assertEqual(bound["evidence"]["reference_sha256"], FIXTURE_SHA)
            self.assertEqual(bound["evidence"]["status"], "verified")

    def test_unknown_id_path_mismatch_and_declared_digest_mismatch_fail(self):
        cases = [
            (fixture_case(reference_contract_id="ref.missing"), "unknown reference contract"),
            (fixture_case(reference="references/wrong.wav"), "reference path does not match"),
            (fixture_case(reference_sha256="1" * 64), "reference_sha256 does not match"),
        ]
        with tempfile.TemporaryDirectory() as d:
            for value, message in cases:
                with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                    reference_contracts.bind_reference(self.registry, value, Path(d))

    def test_unverified_contract_fails_before_filesystem_or_decode(self):
        value = fixture_case(
            reference="references/drumkit/canonical/pop-kick-ff.wav",
            reference_contract_id="ref.legacy.drums.pop-kick-ff.v1",
            reference_sha256=None,
        )
        with mock.patch.object(reference_contracts, "sha256") as digest, mock.patch.object(reference_contracts.sf, "info") as info:
            with self.assertRaisesRegex(ValueError, "unverified reference contract"):
                reference_contracts.bind_reference(self.registry, value, Path("/does/not/exist"))
            digest.assert_not_called()
            info.assert_not_called()

    def test_digest_substitution_symlink_escape_rate_and_axis_conflict_fail(self):
        with tempfile.TemporaryDirectory() as d:
            path = self.stage(d)
            path.write_bytes(path.read_bytes() + b"tamper")
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                reference_contracts.bind_reference(self.registry, fixture_case(), Path(d))
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as outside:
            root = Path(d)
            target = Path(outside) / "reference.wav"
            shutil.copyfile(GOLDEN, target)
            link = root / FIXTURE_PATH
            link.parent.mkdir(parents=True)
            link.symlink_to(target)
            with self.assertRaisesRegex(ValueError, "escapes"):
                reference_contracts.bind_reference(self.registry, fixture_case(), root)
        with tempfile.TemporaryDirectory() as d:
            self.stage(d)
            with mock.patch.object(reference_contracts.sf, "info", return_value=SimpleNamespace(frames=10, channels=1, samplerate=44100)):
                with self.assertRaisesRegex(ValueError, "44100.*48000"):
                    reference_contracts.bind_reference(self.registry, fixture_case(), Path(d))
        with tempfile.TemporaryDirectory() as d:
            self.stage(d)
            registry = copy.deepcopy(self.registry)
            registry["contracts"][FIXTURE_ID] = copy.deepcopy(registry["contracts"][FIXTURE_ID])
            registry["contracts"][FIXTURE_ID]["invalid_axes"] = {"attack": "fixture does not own attack"}
            with self.assertRaisesRegex(ValueError, "invalid reference axes"):
                reference_contracts.bind_reference(registry, fixture_case(), Path(d))

    def test_missing_and_decode_boundaries_fail_closed(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(FileNotFoundError, "missing campaign reference"):
                reference_contracts.bind_reference(self.registry, fixture_case(), Path(d))
        with tempfile.TemporaryDirectory() as d:
            self.stage(d)
            with mock.patch.object(reference_contracts.sf, "info", side_effect=RuntimeError("decoder failed")):
                with self.assertRaisesRegex(ValueError, "could not be decoded.*decoder failed"):
                    reference_contracts.bind_reference(self.registry, fixture_case(), Path(d))
        for frames, channels in ((0, 1), (10, 3)):
            with self.subTest(frames=frames, channels=channels), tempfile.TemporaryDirectory() as d:
                self.stage(d)
                with mock.patch.object(reference_contracts.sf, "info", return_value=SimpleNamespace(frames=frames, channels=channels, samplerate=48000)):
                    with self.assertRaisesRegex(ValueError, "non-empty mono/stereo"):
                        reference_contracts.bind_reference(self.registry, fixture_case(), Path(d))


class RunOrderTests(unittest.TestCase):
    def test_unverified_production_manifest_fails_before_all_campaign_side_effects(self):
        with tempfile.TemporaryDirectory() as d:
            for family in ("drums", "guitars", "bass"):
                with self.subTest(family=family):
                    out = Path(d) / family
                    args = SimpleNamespace(
                        manifest=str(ROOT / "evals" / "cases" / f"{family}.json"), reference_root=d, out=str(out),
                        hypothesis="Must stop before side effects.", changed_component="test", baseline_dir=str(Path(d) / "baseline"),
                        drift_baseline=str(Path(d) / "drift"), allow_dirty=False, skip_wasm_verify=False,
                    )
                    names = ("verify_clean_source", "verify_wasm", "prepare_output_dir", "render_case", "run_drift", "seal_iteration", "write_json")
                    patches = [mock.patch.object(loop_campaign, name) for name in names]
                    mocks = [patch.start() for patch in patches]
                    try:
                        with mock.patch.object(loop_campaign.loop_metrics, "compare_files") as compare, mock.patch.object(loop_campaign.subprocess, "run") as proc:
                            with self.assertRaisesRegex(ValueError, "unverified reference contract"):
                                loop_campaign.run_campaign(args)
                            compare.assert_not_called()
                            proc.assert_not_called()
                        for item in mocks:
                            item.assert_not_called()
                        self.assertFalse(out.exists())
                    finally:
                        for patch in reversed(patches):
                            patch.stop()


if __name__ == "__main__":
    unittest.main()
