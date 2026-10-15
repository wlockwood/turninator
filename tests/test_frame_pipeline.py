import binascii
import importlib.util
from pathlib import Path
import struct
import tempfile
import threading
import tracemalloc
import unittest
from unittest import mock
import zlib


SPEC = importlib.util.spec_from_file_location(
    "frame_pipeline", Path(__file__).resolve().parents[1] / "renderer" / "frame_pipeline.py")
pipeline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pipeline)


def chunk(kind, data):
    return (struct.pack(">I", len(data)) + kind + data
            + struct.pack(">I", binascii.crc32(kind + data) & 0xffffffff))


def png(width=16, height=16, channels=3, raw=None, split=False):
    header = struct.pack(">IIBBBBB", width, height, 8, 2 if channels == 3 else 6, 0, 0, 0)
    raw = raw if raw is not None else (b"\0" + b"\x80" * width * channels) * height
    compressed = zlib.compress(raw)
    data = ([compressed[:len(compressed)//2], compressed[len(compressed)//2:]]
            if split else [compressed])
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + b"".join(chunk(b"IDAT", part) for part in data) + chunk(b"IEND", b""))


class PngTests(unittest.TestCase):
    def decode(self, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "frame.png"
            path.write_bytes(data)
            return pipeline.decode_png_size(path)

    def test_rgb_rgba_and_split_idat_decode_without_pixel_arrays(self):
        for channels in (3, 4):
            for split in (False, True):
                with self.subTest(channels=channels, split=split):
                    self.assertEqual(self.decode(png(channels=channels, split=split)), (16, 16))

    def test_crc_truncation_and_missing_end_are_rejected(self):
        valid = png()
        corrupt = bytearray(valid)
        corrupt[45] ^= 1
        for data in (bytes(corrupt), valid[:-1], valid[:-12], valid + b"extra"):
            with self.subTest(length=len(data)), self.assertRaises(ValueError):
                self.decode(data)

    def test_pixel_filters_lengths_and_zlib_trailers_are_checked(self):
        for raw in ((b"\5" + b"x" * 48) * 16,
                    (b"\0" + b"x" * 48) * 15,
                    (b"\0" + b"x" * 48) * 17):
            with self.subTest(length=len(raw)), self.assertRaises(ValueError):
                self.decode(png(raw=raw))
        valid = png()
        header = valid[8:33]
        for payload in (zlib.compress(b"\0" * 784)[:-2],
                        zlib.compress(b"\0" * 784) + b"trailing"):
            with self.assertRaises(ValueError):
                self.decode(valid[:8] + header + chunk(b"IDAT", payload) + chunk(b"IEND", b""))

    def test_chunk_order_critical_chunks_and_dimensions_are_checked(self):
        valid = png()
        header = valid[8:33]
        data = valid[33:-12]
        for body in (data + header, header + header + data,
                     header + chunk(b"NOPE", b"") + data,
                     header + data + chunk(b"tEXt", b"x") + data,
                     header + data + chunk(b"PLTE", b"abc")):
            with self.assertRaises(ValueError):
                self.decode(valid[:8] + body + valid[-12:])
        with self.assertRaises(ValueError):
            self.decode(png(width=8001, height=1, raw=b""))

    def test_inflate_bomb_is_stopped_at_declared_pixel_budget(self):
        # The small declared image cannot cause a large decoded allocation.
        bomb = zlib.compress(b"\0" * (8 * 1024 * 1024))
        valid = png()
        tracemalloc.start()
        try:
            with self.assertRaises(ValueError):
                self.decode(valid[:33] + chunk(b"IDAT", bomb) + valid[-12:])
            self.assertLess(tracemalloc.get_traced_memory()[1], 2 * 1024 * 1024,
                            "PNG validation inflated an oversized pixel allocation")
        finally:
            tracemalloc.stop()

    def test_scanlines_cross_inflate_blocks_and_all_valid_filters_are_accepted(self):
        raw = b"".join(bytes([row % 5]) + b"\x80" * 129 * 4 for row in range(3000))
        self.assertGreater(len(raw), 1024 * 1024)
        self.assertEqual(self.decode(png(129, 3000, 4, raw, split=True)), (129, 3000))

    def test_file_budget_and_unsupported_pixel_layouts_fail_before_publication(self):
        with mock.patch.object(pipeline, "MAX_FILE_BYTES", 32), self.assertRaises(ValueError):
            self.decode(png())
        for depth, color, interlace in ((16, 2, 0), (8, 3, 0), (8, 6, 1)):
            header = struct.pack(">IIBBBBB", 16, 16, depth, color, 0, 0, interlace)
            with self.assertRaises(ValueError):
                self.decode(png()[:8] + chunk(b"IHDR", header) + png()[33:])


class PipelineTests(unittest.TestCase):
    def test_flush_publishes_before_copies_and_allows_more_submissions(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            callbacks = []
            def checked(path):
                self.assertEqual(Path(path).read_bytes(), b"test checker")
                return (16, 16)
            with pipeline.FramePipeline(16, 16, lambda i, *args: callbacks.append(i),
                                        workers=1, decode_size=checked) as frames:
                for i in range(2):
                    source, target = directory / f"in-{i}.png", directory / f"out-{i}.png"
                    source.write_bytes(b"test checker")
                    frames.submit(i, source, target, 0.5)
                    self.assertEqual(len(frames.flush()), i + 1)
                    self.assertTrue(target.exists())
                    self.assertEqual(callbacks, list(range(i + 1)))
                self.assertEqual(len(frames.finish()), 2)
                self.assertEqual(len(frames.finish()), 2)
                with self.assertRaises(RuntimeError):
                    frames.submit(2, source, target, 0.)

    def test_finish_reports_earliest_submission_failure_not_fastest_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            second_failed = threading.Event()
            def checked(path):
                if Path(path).stem == "first":
                    if not second_failed.wait(5):
                        raise RuntimeError("test timed out")
                    raise ValueError("first submitted failure")
                second_failed.set()
                raise ValueError("second submitted failure")
            frames = pipeline.FramePipeline(16, 16, lambda *args: self.fail("failed frame callback"),
                                            workers=2, decode_size=checked)
            for i, name in enumerate(("first", "second")):
                source = directory / f"{name}.png"
                source.write_bytes(b"bad")
                frames.submit(i, source, directory / f"out-{i}.png", 0.)
            with self.assertRaisesRegex(ValueError, "first submitted failure"):
                frames.finish()
            self.assertFalse(list(directory.iterdir()))

    def test_validation_overlaps_and_callbacks_keep_submission_order(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            first_started = threading.Event()
            second_done = threading.Event()
            release = threading.Event()
            callbacks = []
            real_decode = pipeline.decode_png_size
            def decode(path):
                if Path(path).stem == "first":
                    first_started.set()
                    if not release.wait(5):
                        raise RuntimeError("test release timed out")
                else:
                    second_done.set()
                return real_decode(path)
            with mock.patch.object(pipeline, "decode_png_size", side_effect=decode):
                with pipeline.FramePipeline(16, 16, lambda i, r, v: callbacks.append((i, r)), workers=2) as frames:
                    for index, name in ((7, "first"), (2, "second")):
                        temporary = directory / f"{name}.png"
                        temporary.write_bytes(png())
                        frames.submit(index, temporary, directory / f"out-{index}.png", 0.25)
                    self.assertTrue(first_started.wait(5))
                    self.assertTrue(second_done.wait(5), "second CPU validation did not overlap first")
                    self.assertEqual(callbacks, [])
                    release.set()
                    result = frames.finish()
            self.assertEqual(callbacks, [(7, 0.25), (2, 0.25)])
            self.assertEqual([entry["index"] for entry in result], [7, 2])
            self.assertTrue(all(entry["validation_seconds"] >= 0 for entry in result))
            self.assertEqual(frames.peak_in_flight, 2)

    def test_capacity_blocks_producer_until_validation_finishes(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            started, release, submitted = threading.Event(), threading.Event(), threading.Event()
            real_decode = pipeline.decode_png_size
            def decode(path):
                started.set()
                if not release.wait(5):
                    raise RuntimeError("test release timed out")
                return real_decode(path)
            with mock.patch.object(pipeline, "decode_png_size", side_effect=decode):
                with pipeline.FramePipeline(16, 16, lambda *args: None, workers=2, capacity=1) as frames:
                    for name in ("first", "second"):
                        (directory / f"{name}.png").write_bytes(png())
                    frames.submit(0, directory / "first.png", directory / "out-0.png", 0.)
                    self.assertTrue(started.wait(5))
                    def submit():
                        frames.submit(1, directory / "second.png", directory / "out-1.png", 0.)
                        submitted.set()
                    producer = threading.Thread(target=submit)
                    producer.start()
                    self.assertFalse(submitted.wait(0.05), "capacity did not apply backpressure")
                    release.set()
                    producer.join(5)
                    self.assertFalse(producer.is_alive())
                    self.assertTrue(submitted.is_set())
                    frames.finish()
            self.assertEqual(frames.peak_in_flight, 1)
            self.assertGreater(frames.validation_wait_seconds, 0.04)

    def test_invalid_frame_preserves_existing_destination_and_finish_raises(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            temporary, target = directory / "bad.png", directory / "out.png"
            temporary.write_bytes(b"broken")
            target.write_bytes(b"existing output")
            callbacks = []
            frames = pipeline.FramePipeline(16, 16, lambda *args: callbacks.append(args), workers=1)
            frames.submit(0, temporary, target, 1.)
            with self.assertRaises(ValueError):
                frames.finish()
            self.assertEqual(target.read_bytes(), b"existing output")
            self.assertFalse(temporary.exists())
            self.assertFalse(callbacks)

    def test_dimension_and_callback_failures_propagate_in_submission_order(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            frames = pipeline.FramePipeline(32, 32, lambda *args: None, workers=2)
            for i in range(2):
                source = directory / f"in-{i}.png"
                source.write_bytes(png())
                frames.submit(i, source, directory / f"out-{i}.png", 0.)
            with self.assertRaisesRegex(ValueError, "dimensions"):
                frames.finish()
            self.assertFalse(list(directory.glob("out-*")))
            source = directory / "valid.png"
            source.write_bytes(png())
            def failed_callback(*args):
                raise RuntimeError("callback failed")
            with self.assertRaisesRegex(RuntimeError, "callback failed"):
                with pipeline.FramePipeline(16, 16, failed_callback, workers=1) as frames:
                    frames.submit(0, source, directory / "valid-out.png", 0.)
            self.assertTrue((directory / "valid-out.png").exists())

    def test_context_cleanup_preserves_main_thread_exception(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "bad.png"
            source.write_bytes(b"bad")
            with self.assertRaisesRegex(RuntimeError, "main rendering failed"):
                with pipeline.FramePipeline(16, 16, lambda *args: None, workers=1) as frames:
                    frames.submit(0, source, directory / "out.png", 0.)
                    raise RuntimeError("main rendering failed")
            self.assertFalse(source.exists())


if __name__ == "__main__":
    unittest.main()
