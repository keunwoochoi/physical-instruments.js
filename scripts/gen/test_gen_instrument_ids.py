#!/usr/bin/env python3
"""Guard: a fake instrument in the catalog without a matching Rust arm fails --check.

Also asserts the generated TS is current and that no second hand-maintained INST
table lives in packages/core/src/index.ts.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GEN = ROOT / "scripts" / "gen" / "gen_instrument_ids.py"
CATALOG = ROOT / "crates" / "dsp" / "instruments.catalog.json"
INDEX = ROOT / "packages" / "core" / "src" / "index.ts"
GENERATED = ROOT / "packages" / "core" / "src" / "instrument-ids.generated.ts"


class TestInstrumentIdGeneration(unittest.TestCase):
    def test_check_passes_on_live_tree(self) -> None:
        result = subprocess.run(
            [sys.executable, str(GEN), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)

    def test_fake_catalog_instrument_fails_check(self) -> None:
        """Adding a catalog row without updating the Rust enum must fail --check."""
        data = json.loads(CATALOG.read_text(encoding="utf-8"))
        data["instruments"].append(
            {
                "id": len(data["instruments"]),
                "key": "fakeophone",
                "rust": "Fakeophone",
                "tsConst": "fakeophone",
            }
        )
        data["groups"].append({"group": "fakeophone", "instrument": "fakeophone"})
        with tempfile.TemporaryDirectory() as tmp:
            # Run generator against a swapped catalog by monkeypatching via cwd copy
            # is heavy; instead write catalog aside and invoke verify by importing.
            sys.path.insert(0, str(GEN.parent))
            import gen_instrument_ids as mod  # type: ignore

            original = mod.CATALOG
            fake = Path(tmp) / "instruments.catalog.json"
            fake.write_text(json.dumps(data), encoding="utf-8")
            try:
                mod.CATALOG = fake
                catalog = mod.load_catalog()
                errors = mod.verify_rust(catalog)
            finally:
                mod.CATALOG = original
            self.assertTrue(errors, "fake instrument was accepted against the Rust enum")

    def test_index_ts_has_no_hand_rolled_inst_table(self) -> None:
        text = INDEX.read_text(encoding="utf-8")
        self.assertNotIn("const INST = {", text)
        self.assertNotIn("const GROUP_TO_INSTRUMENT", text)
        self.assertIn("instrument-ids.generated", text)
        self.assertTrue(GENERATED.is_file())
        self.assertIn("@generated", GENERATED.read_text(encoding="utf-8"))


if __name__ == "__main__":
    raise SystemExit(unittest.main(verbosity=2))
