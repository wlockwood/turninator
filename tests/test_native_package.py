"""Native package boundary tests. Tiny PE fixtures are test data, never SDK payloads."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def pe(import_name="KERNEL32.dll", delay=False):
    data = bytearray(1024)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<HH", data, 132, 0x8664, 1)
    struct.pack_into("<H", data, 148, 240)
    struct.pack_into("<H", data, 152, 0x20b)
    struct.pack_into("<I", data, 152 + 108, 16)
    struct.pack_into("<II", data, 152 + 112 + (13 if delay else 1) * 8, 4096, 64)
    struct.pack_into("<IIII", data, 392 + 8, 512, 4096, 512, 512)
    if delay:
        struct.pack_into("<II", data, 512, 1, 4224)
    else:
        struct.pack_into("<I", data, 524, 4224)
    data[640:640 + len(import_name)] = import_name.encode("ascii")
    return bytes(data)


class NativePackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("native_package", ROOT / "tools/native_package.py")
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.lock = json.loads((ROOT / "native/sdk-lock.json").read_text())
        self.manifest = {"schema_version": 1, "sdk_lock_sha256": hashlib.sha256((ROOT / "native/sdk-lock.json").read_bytes()).hexdigest(),
                         "components": [{"id": "filament", "version": "1.78.0", "license": "Apache-2.0", "notices": ["notices/filament.txt"]}],
                         "files": [], "backends": {}}
        self.add_file("notices/filament.txt", b"Full license attribution", "notice")
        self.add_file("renderers/filament/tn_renderer_filament.dll", pe(), "bridge")

    def add_file(self, name, data, role):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        self.manifest["files"].append({"path": name, "sha256": hashlib.sha256(data).hexdigest(), "size_bytes": len(data), "component": "filament", "role": role})

    def validate(self):
        return self.module.validate_inventory(self.manifest, self.root, ROOT / "native/sdk-lock.json")

    def test_pe_import_and_delay_import_are_both_closed(self):
        self.assertEqual(self.module.pe_imports(pe()), {"kernel32.dll"})
        self.assertEqual(self.module.pe_imports(pe("missing.dll", True)), {"missing.dll"})

    def test_corrupt_or_wrong_architecture_binary_is_rejected(self):
        for data in (b"not a DLL", pe()[:600], pe().replace(b"d\x86", b"L\x01", 1)):
            with self.subTest(data=data[:8]), self.assertRaises(ValueError):
                self.module.pe_imports(data)

    def test_valid_inventory_contains_only_relative_identity(self):
        result = self.validate()
        self.assertEqual(len(result["files"]), 2)
        self.assertNotIn(str(self.root), json.dumps(result))

    def test_hash_and_size_mismatch_fail(self):
        for field, value in (("sha256", "0" * 64), ("size_bytes", 0)):
            with self.subTest(field=field):
                original = copy.deepcopy(self.manifest)
                self.manifest["files"][1][field] = value
                with self.assertRaisesRegex(ValueError, "identity"):
                    self.validate()
                self.manifest = original

    def test_missing_transitive_crt_is_not_an_os_dependency(self):
        self.manifest["files"].pop()
        self.add_file("renderers/filament/tn_renderer_filament.dll", pe("MSVCP140_2.dll"), "bridge")
        with self.assertRaisesRegex(ValueError, "MSVCP140_2.dll|msvcp140_2.dll"):
            self.validate()

    def test_delay_dependency_must_be_in_same_backend_directory(self):
        self.manifest["files"].pop()
        self.add_file("renderers/filament/tn_renderer_filament.dll", pe("helper.dll", True), "bridge")
        self.add_file("renderers/luxcore/helper.dll", pe(), "runtime")
        with self.assertRaisesRegex(ValueError, "helper.dll"):
            self.validate()

    def test_unlisted_payload_cannot_sneak_into_bundle(self):
        (self.root / "renderers/filament/matc.exe").write_bytes(b"compiler")
        with self.assertRaisesRegex(ValueError, "unlisted"):
            self.validate()

    def test_unsafe_and_duplicate_windows_paths_fail(self):
        for value in ("../escape", "C:/absolute", "notices/file:stream", "notices/CON.txt", "notices/trailing.", "notices\\filament.txt", "NOTICES/FILAMENT.TXT"):
            original = copy.deepcopy(self.manifest)
            self.manifest["files"].append({**self.manifest["files"][0], "path": value})
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.validate()
            self.manifest = original

    def test_missing_component_notice_and_wrong_lock_fail(self):
        for change in (lambda: self.manifest["components"][0].update(notices=[]),
                       lambda: self.manifest.update(sdk_lock_sha256="0" * 64)):
            original = copy.deepcopy(self.manifest)
            change()
            with self.assertRaises(ValueError):
                self.validate()
            self.manifest = original

    def test_manifest_does_not_export_build_machine_paths(self):
        self.manifest["backends"]["filament"] = {"compiler": "D:/private/toolchain/cl.exe"}
        with self.assertRaisesRegex(ValueError, "absolute"):
            self.validate()

    def test_attached_drive_and_unc_paths_cannot_escape_metadata_validation(self):
        for value in ("-ID:/private/build/include", "compiler=D:/private/toolchain/cl.exe",
                      '-I"D:/private/build/include"', r"flags=-IC:\private\include",
                      r"-I\\private-server\build\include", r"compiler=\\private-server\build\cl.exe",
                      "-I//private-server/build/include", "compiler=//private-server/build/cl.exe",
                      "-ID://private/build/include", "see https://example.org then -ID:/private/include"):
            with self.subTest(value=value):
                self.manifest["backends"]["filament"] = {"compiler_flags": [value]}
                with self.assertRaisesRegex(ValueError, "absolute"):
                    self.validate()

    def test_portable_flags_urls_and_relative_paths_remain_accepted(self):
        for value in ("/MD", "/MT", "MSVC /MD /W4 /WX /EHsc", "-Iinclude", "compiler=tools/cl.exe",
                      "https://example.org/source/LICENSE", "source=https://example.org/a//b?version=1.0",
                      "http://example.org/source.tar.gz", "notices/filament.txt"):
            with self.subTest(value=value):
                self.manifest["backends"]["filament"] = {"compiler_flags": [value]}
                self.validate()

    def test_case_varied_notice_reference_survives_inventory_and_notice_generation(self):
        spec = importlib.util.spec_from_file_location("notices", ROOT / "tools/generate-notices.py")
        notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(notices)
        self.manifest["package_variant"] = "full"
        self.manifest["components"][0]["notices"] = ["NOTICES/FILAMENT.TXT"]
        validated = self.validate()
        rendered = notices.render_native_notices(validated, self.root)
        self.assertIn("Full license attribution", rendered)
        self.assertIn("### notices/filament.txt", rendered)

    def test_native_payload_cannot_overwrite_application_files(self):
        self.add_file("README.md", b"unexpected replacement", "notice")
        with self.assertRaisesRegex(ValueError, "namespace"):
            self.validate()

    def test_mpl_component_requires_corresponding_source_information(self):
        self.manifest["components"][0]["license"] = "MPL-2.0"
        with self.assertRaisesRegex(ValueError, "source"):
            self.validate()

    def test_full_package_cannot_be_declared_with_one_backend(self):
        with self.assertRaisesRegex(ValueError, "filament.*luxcore|both"):
            self.module.validate_full(self.manifest, self.root, ROOT / "native/sdk-lock.json")

    def test_core_manifest_is_honest_and_needs_no_sdk(self):
        result = self.module.core_manifest()
        self.assertEqual(result["package_variant"], "core")
        self.assertEqual(result["backends"]["filament"]["status"], "not_bundled")
        self.assertEqual(result["backends"]["luxcore"]["status"], "not_bundled")

    def test_filament_receipt_binds_reviewed_build_and_allows_new_build_hash(self):
        bridge = self.root / "renderers/filament/tn_renderer_filament.dll"
        attestation = {"schema_version": 1, "backend": "filament", "source_revision": "a" * 40,
                       "sdk_lock_sha256": self.manifest["sdk_lock_sha256"], "bridge_sha256": hashlib.sha256(bridge.read_bytes()).hexdigest(),
                       "compiler": self.lock["toolchain"]["compiler"], "source_files": {"native/filament/backend.cpp": "b" * 64},
                       "static_inputs": [{"path": "sdk/library.lib", "sha256": "c" * 64}],
                       "assets": [{"path": "surface.filamat", "sha256": "d" * 64}], "build_commands": [["cmake", "--build", "<owned-build>"]],
                       "tool_versions": {"cmake": self.lock["toolchain"]["cmake"]}, "imports": sorted(self.module.pe_imports(bridge.read_bytes()))}
        receipt = self.module.filament_receipt(bridge, ROOT / "native/sdk-lock.json", attestation)
        self.assertEqual(receipt["native_recipe"], "turninator-filament-native-v1")
        self.assertEqual(receipt["sdk_version"], "1.78.0+c1dc4e444784850d6dc891a1fb38747c30b85463")
        self.assertEqual(receipt["bridge_sha256"], hashlib.sha256(bridge.read_bytes()).hexdigest())
        bridge.write_bytes(pe() + b"new legitimate build")
        with self.assertRaisesRegex(ValueError, "bridge"):
            self.module.filament_receipt(bridge, ROOT / "native/sdk-lock.json", attestation)
        attestation["bridge_sha256"] = hashlib.sha256(bridge.read_bytes()).hexdigest()
        updated = self.module.filament_receipt(bridge, ROOT / "native/sdk-lock.json", attestation)
        self.assertNotEqual(receipt["bridge_sha256"], updated["bridge_sha256"])
        attestation["sdk_lock_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "lock"):
            self.module.filament_receipt(bridge, ROOT / "native/sdk-lock.json", attestation)


if __name__ == "__main__":
    unittest.main()
