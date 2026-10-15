"""Portable distribution notices and inventory tests; no GPU needed."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import subprocess
import hashlib

ROOT = Path(__file__).resolve().parents[1]


def reject_package(*args):
    # Any regression in early argument validation must never start real Cargo.
    values = [str(ROOT / "tools/package.ps1"), *args]
    command = "function cargo { throw 'UNEXPECTED_CARGO_SENTINEL' }; & " + " ".join(
        "'" + value.replace("'", "''") + "'" if not value.startswith("-") else value for value in values)
    return subprocess.run(["powershell", "-NoProfile", "-Command", command], capture_output=True,
                          text=True, creationflags=subprocess.CREATE_NO_WINDOW)


class PackagingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("notices", ROOT / "tools/generate-notices.py")
        cls.notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.notices)

    def metadata(self, directory):
        root = Path(directory)
        packages = []
        for name in ["turninator", "dependency", "unused"]:
            folder = root / name
            folder.mkdir()
            (folder / "Cargo.toml").write_text("", encoding="utf-8")
            packages.append({"id": name, "name": name, "version": "1.2.3", "license": "MIT", "license_file": None,
                             "manifest_path": str(folder / "Cargo.toml"), "repository": "https://example.invalid/upstream",
                             "source": None if name == "turninator" else "registry+example"})
        return {"packages": packages, "resolve": {"root": "turninator", "nodes": [
            {"id": "turninator", "deps": [{"pkg": "dependency"}]},
            {"id": "dependency", "deps": []}, {"id": "unused", "deps": []}]}}

    def test_exact_dependency_closure_and_font_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            metadata = self.metadata(directory)
            dep = Path(directory) / "dependency"
            (dep / "LICENSE.MIT").write_text("Distinct software attribution", encoding="utf-8")
            (dep / "fonts").mkdir()
            (dep / "fonts/OFL.txt").write_text("Distinct embedded font attribution", encoding="utf-8")
            text = self.notices.render_notices(metadata, {})
            self.assertIn("dependency 1.2.3", text)
            self.assertIn("Distinct software attribution", text)
            self.assertIn("Distinct embedded font attribution", text)
            self.assertNotIn("unused 1.2.3", text)
            self.assertNotIn(directory, text)

    def test_missing_license_fails_instead_of_claiming_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "dependency.*no license"):
                self.notices.render_notices(self.metadata(directory), {})

    def test_notice_output_normalizes_windows_line_endings_without_losing_attribution(self):
        with tempfile.TemporaryDirectory() as directory:
            metadata = self.metadata(directory)
            data = b"Copyright Example 2026  \r\nPermission remains unchanged.\r\n"
            (Path(directory) / "dependency/LICENSE").write_bytes(data)
            text = self.notices.render_notices(metadata, {})
            self.assertNotIn("\r", text)
            self.assertTrue(all(line == line.rstrip() for line in text.splitlines()))
            self.assertIn("Copyright Example 2026", text)
            self.assertIn(self.notices.digest(data), text)

    def test_supplement_hash_is_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            supplemental = {"dependency@1.2.3": [{"path": "LICENSE", "url": "https://example.invalid/exact-commit/LICENSE",
                "sha256": "incorrect", "text": "Attribution"}]}
            with self.assertRaisesRegex(ValueError, "hash"):
                self.notices.render_notices(self.metadata(directory), supplemental)

    def test_inventory_hashes_all_payload_files(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory)
            (package / "renderer").mkdir()
            (package / "renderer/worker.py").write_bytes(b"worker")
            (package / "turninator.exe").write_bytes(b"binary")
            result = self.notices.file_inventory(package)
            self.assertEqual(set(result), {"renderer/worker.py", "turninator.exe"})
            self.assertEqual(result["turninator.exe"]["size_bytes"], 6)
            self.assertEqual(len(result["renderer/worker.py"]["sha256"]), 64)

    def test_native_notices_keep_aggregate_text_and_source_availability(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            notice = b"OIDN aggregate\nCopyright bundled component\nAll license conditions."
            (root / "aggregate.txt").write_bytes(notice)
            manifest = {"package_variant": "full", "files": [{"path": "aggregate.txt", "role": "notice",
                "size_bytes": len(notice), "sha256": hashlib.sha256(notice).hexdigest()}], "components": [
                {"id": "eigen", "version": "5.0.1", "license": "MPL-2.0", "notices": ["aggregate.txt"],
                 "corresponding_source": {"url": "https://example.invalid/exact-source.tar.bz2", "sha256": "a" * 64,
                                          "instructions": "Obtain corresponding source from this exact archive."}}]}
            text = self.notices.render_native_notices(manifest, root)
            self.assertIn(notice.decode(), text)
            self.assertIn("Obtain corresponding source", text)
            self.assertIn("https://example.invalid/exact-source.tar.bz2", text)
            self.assertNotIn(directory, text)
            (root / "aggregate.txt").write_bytes(b"truncated")
            with self.assertRaisesRegex(ValueError, "identity"):
                self.notices.render_native_notices(manifest, root)

    @unittest.skipUnless(os.name == "nt", "Windows packaging uses PowerShell")
    def test_full_package_requires_explicit_native_inputs_before_cargo(self):
        result = reject_package("-SkipBuild")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Full packaging requires", result.stderr)

    @unittest.skipUnless(os.name == "nt", "Windows packaging uses PowerShell")
    def test_explicit_binary_requires_skip_build(self):
        result = reject_package("-ExecutablePath", str(ROOT / "target/release/deps/turninator.exe"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ExecutablePath requires -SkipBuild", result.stderr)

    @unittest.skipUnless(os.name == "nt", "Windows packaging uses PowerShell")
    def test_package_rejects_path_traversal_before_build(self):
        result = reject_package("-SkipBuild", "-PackageName", "../outside")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("PackageName must be a single safe directory name", result.stderr)


if __name__ == "__main__":
    unittest.main()
