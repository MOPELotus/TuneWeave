import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import release_manifest as manifest


class ReleaseManifestTest(unittest.TestCase):
    def test_current_manifest_matches_source_contract(self):
        manifest.check_manifest(manifest.load_manifest(Path(__file__).resolve().parents[1] / "release-manifest.json"))

    def test_generation_preserves_release_and_artifact_identity(self):
        value = {"schema_version": 1, "version": "1.2.0", "artifacts": [{"file": "binary", "verification": {"algorithm": "sha256"}}]}
        original = copy.deepcopy(value)
        generated = manifest.generated_manifest(value)
        self.assertEqual(value, original)
        self.assertEqual(generated.pop("compatibility_profiles"), list(manifest.COMPATIBILITY_PROFILES))
        self.assertEqual(generated, original)

    def test_missing_and_malformed_profiles_are_rejected(self):
        for profiles in (None, [], "tuneweave-http-v1", [1], [None], [""], ["UPPER"], ["bad profile"], ["bad\n"], ["a" * 65], ["é"], ["/path"], ["same", "same"], [str(i) for i in range(17)]):
            with self.subTest(profiles=profiles), self.assertRaises(ValueError):
                manifest.validate_profiles({"compatibility_profiles": profiles})

    def test_unknown_profile_is_syntactically_valid_but_not_current_source_declaration(self):
        value = {"schema_version": 1, "compatibility_profiles": ["future-http-v2"]}
        self.assertEqual(manifest.validate_profiles(value), ["future-http-v2"])
        with self.assertRaises(ValueError):
            manifest.check_manifest(value)

    def test_duplicate_json_fields_and_unknown_schema_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            for text in ('{"schema_version":1,"schema_version":1}', '{"schema_version":2}', '{"schema_version":true}', '{"schema_version":1,"extra":NaN}', '[]', '{'):
                path.write_text(text)
                with self.subTest(text=text), self.assertRaises(ValueError):
                    manifest.load_manifest(path)

    def test_manifest_size_is_bounded(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_bytes(b" " * (manifest.MAX_MANIFEST_BYTES + 1))
            with self.assertRaises(ValueError):
                manifest.load_manifest(path)

    def test_cli_generates_staged_manifest_and_rejects_old_missing_declaration(self):
        with tempfile.TemporaryDirectory() as directory:
            source, output = Path(directory) / "source.json", Path(directory) / "staged.json"
            source.write_text(json.dumps({"schema_version": 1, "version": "1.0.0-beta.1"}))
            command = [sys.executable, str(Path(manifest.__file__)), "--manifest", str(source)]
            self.assertNotEqual(subprocess.run(command + ["--check"], capture_output=True).returncode, 0)
            subprocess.run(command + ["--output", str(output)], check=True)
            self.assertNotIn("compatibility_profiles", json.loads(source.read_text()))
            manifest.check_manifest(manifest.load_manifest(output))
            subprocess.run(command + ["--write"], check=True)
            subprocess.run(command + ["--check"], check=True)


if __name__ == "__main__":
    unittest.main()
