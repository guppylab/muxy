import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("compatibility", ROOT / "scripts/beta_compatibility.py")
compatibility = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compatibility)


class CompatibilityTests(unittest.TestCase):
    def test_changed_wire_requires_a_different_identifier(self):
        before = {"identifier": 1, "wire": {"hello": "old"}}
        with self.assertRaisesRegex(ValueError, "bump COMPATIBILITY"):
            compatibility.validate(before, {"identifier": 1, "wire": {"hello": "new"}})
        compatibility.validate(before, {"identifier": 2, "wire": {"hello": "new"}})
        compatibility.validate(before, before)

    def test_declaration_matches_actual_wire(self):
        import json
        self.assertEqual(json.loads((ROOT / compatibility.RECORD).read_text()), compatibility.current())
