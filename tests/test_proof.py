"""Checks the proof controller's actual encoder and integrity boundaries."""
import importlib.util
import pathlib
import json
import tempfile
import unittest

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("proof", ROOT / "tools" / "prove-render.py")


class ProofTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.proof = importlib.util.module_from_spec(SPEC)
        SPEC.loader.exec_module(cls.proof)

    def test_shared_palette_encoding_decodes_uniform_delays_and_infinite_loop(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            frames = root / "frames"
            frames.mkdir()
            for index, color in enumerate(["red", "blue", "green", "yellow"]):
                Image.new("RGB", (24, 24), color).save(frames / f"frame_{index:04d}.png")
            output = root / "test.gif"
            info = self.proof.encode_gif(frames, output, "C:/Program Files/ffmpeg/bin/ffmpeg.exe", 4, 24)
            self.assertEqual(info["frame_count"], 4)
            self.assertEqual(info["delays_ms"], [50, 50, 50, 50])
            self.assertEqual(info["loop"], 0)
            self.assertEqual(info["dimensions"], [24, 24])
            self.assertEqual(info["palette"], "full,opaque,sierra2_4a")

    def test_missing_frame_fails_before_replacing_existing_destination(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            frames = root / "frames"
            frames.mkdir()
            Image.new("RGB", (24, 24)).save(frames / "frame_0000.png")
            output = root / "test.gif"
            output.write_bytes(b"previous output")
            with self.assertRaises(ValueError):
                self.proof.encode_gif(frames, output, "unused", 2, 24)
            self.assertEqual(output.read_bytes(), b"previous output")

    def test_wrong_dimensions_fail_before_encoding(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            Image.new("RGB", (30, 24)).save(root / "frame_0000.png")
            with self.assertRaises(ValueError):
                self.proof.validate_frames(root, 1, 24)

    def test_invalid_frame_counts_and_sizes_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            for count, size in [(0, 24), (1, 0), (-1, 24)]:
                with self.subTest(count=count, size=size), self.assertRaises(ValueError):
                    self.proof.validate_frames(root, count, size)

    def test_percent_sequence_in_parent_path_is_literal(self):
        with tempfile.TemporaryDirectory(prefix="turninator-%04d-") as temp:
            root = pathlib.Path(temp)
            frames = root / "frames"
            frames.mkdir()
            for index, color in enumerate(["red", "blue"]):
                Image.new("RGB", (24, 24), color).save(frames / f"frame_{index:04d}.png")
            info = self.proof.encode_gif(frames, root / "literal.gif",
                                         "C:/Program Files/ffmpeg/bin/ffmpeg.exe", 2, 24)
            self.assertEqual(info["frame_count"], 2)

    def test_encoder_failure_preserves_output_and_rendered_frames(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            Image.new("RGB", (24, 24), "red").save(root / "frame_0000.png")
            output = root / "existing.gif"
            output.write_bytes(b"previous output")
            with self.assertRaises(OSError):
                self.proof.encode_gif(root, output, root / "missing-ffmpeg.exe", 1, 24)
            self.assertEqual(output.read_bytes(), b"previous output")
            self.assertTrue((root / "frame_0000.png").exists())

    def test_encode_only_uses_retained_frames_without_a_blender_executable(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            frames = root / "frames"
            frames.mkdir()
            for index, color in enumerate(["red", "blue"]):
                Image.new("RGB", (24, 24), color).save(frames / f"frame_{index:04d}.png")
            source = root / "mesh.bin"
            source.write_bytes(b"immutable geometry snapshot")
            recipe = {"size": 24, "frames": 2, "mesh_path": str(source),
                      "mesh_sha256": self.proof.sha256(source), "engine": "eevee", "samples": 1,
                      "worker_sha256": "worker-A", "scene_script_sha256": "scene-A",
                      "frame_pipeline_sha256": "pipeline-A"}
            manifest = {"recipe": recipe, "stage": "encoding"}
            (root / "proof-manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
            frame_manifest = {"geometry_hash": recipe["mesh_sha256"], "engine": "eevee", "size": 24,
                              "samples": 1, "angles": [0.0, 180.0], "transparent": False,
                              "worker_hash": "worker-A", "scene_script_hash": "scene-A",
                              "frame_pipeline_hash": "pipeline-A"}
            (frames / "render-manifest.json").write_text(json.dumps(frame_manifest), encoding="utf-8")
            result = self.proof.encode_only(root, "C:/Program Files/ffmpeg/bin/ffmpeg.exe")
            self.assertEqual(result["frame_count"], 2)
            self.assertEqual(result["delays_ms"], [50, 50])
            saved = json.loads((root / "proof-manifest.json").read_text())
            self.assertEqual(saved["stage"], "complete")
            frame_manifest["frame_pipeline_hash"] = "different-pipeline"
            (frames / "render-manifest.json").write_text(json.dumps(frame_manifest), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "manifest differs"):
                self.proof.encode_only(root, "unused-ffmpeg")

    def test_encode_only_rejects_valid_images_from_a_different_render(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            frames = root / "frames"
            frames.mkdir()
            Image.new("RGB", (24, 24), "red").save(frames / "frame_0000.png")
            source = root / "mesh.bin"
            source.write_bytes(b"geometry A")
            manifest = {"recipe": {"size": 24, "frames": 1, "mesh_path": str(source),
                                    "mesh_sha256": self.proof.sha256(source), "engine": "eevee",
                                    "samples": 1, "worker_sha256": "worker-A", "scene_script_sha256": "scene-A"}}
            (root / "proof-manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
            (frames / "render-manifest.json").write_text(json.dumps({"geometry_hash": "geometry B"}), encoding="utf-8")
            with self.assertRaises(ValueError):
                self.proof.encode_only(root, "C:/Program Files/ffmpeg/bin/ffmpeg.exe")

    def test_bad_protocol_events_fail_at_the_transport_boundary(self):
        for raw in ["{", "[]", '{"protocol_version":2}', '{"protocol_version":1,"kind":"complete"}']:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                self.proof.decode_event(raw)


if __name__ == "__main__":
    unittest.main()
