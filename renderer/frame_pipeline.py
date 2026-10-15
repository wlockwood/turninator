"""Bounded CPU validation and publication for Blender's rendered PNG frames."""

import binascii
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import struct
import threading
import time
import zlib


BLOCK_BYTES = 1024 * 1024
MAX_FILE_BYTES = 512 * 1024 * 1024


def _dimensions(width, height):
    if (type(width) is not int or type(height) is not int
            or not 1 <= width <= 8000 or not 1 <= height <= 8000
            or width * height > 64_000_000):
        raise ValueError("PNG dimensions exceed pixel budget")


def decode_png_size(path):
    """Validate complete RGB/RGBA8 PNG data with bounded inflate allocations.

    CRC and zlib checks cover the complete file, including pixels. Scanline
    filters are validated without allocating or reconstructing pixel arrays.
    No Blender state or image datablocks are accessed, so threads may call this.
    """
    with Path(path).open("rb") as stream:
        if os.fstat(stream.fileno()).st_size > MAX_FILE_BYTES:
            raise ValueError("PNG exceeds file byte budget")
        consumed = 0
        def read_exact(count):
            nonlocal consumed
            consumed += count
            if consumed > MAX_FILE_BYTES:
                raise ValueError("PNG exceeds file byte budget")
            data = stream.read(count)
            if len(data) != count:
                raise ValueError("truncated PNG")
            return data

        if read_exact(8) != b"\x89PNG\r\n\x1a\n":
            raise ValueError("rendered frame is not PNG")
        width = height = row_bytes = expected = decoded = 0
        inflater = None
        seen_data = ended_data = seen_palette = False
        while True:
            length, kind = struct.unpack(">I4s", read_exact(8))
            if length > MAX_FILE_BYTES - consumed - 4:
                raise ValueError("PNG chunk exceeds file byte budget")
            if (any(not (65 <= byte <= 90 or 97 <= byte <= 122) for byte in kind)
                    or kind[2] & 32):
                raise ValueError("invalid PNG chunk type")
            if not width and kind != b"IHDR":
                raise ValueError("PNG must start with IHDR")
            if kind == b"IHDR":
                if width or length != 13:
                    raise ValueError("invalid or repeated PNG IHDR")
                header = read_exact(length)
                width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", header)
                _dimensions(width, height)
                if depth != 8 or color not in (2, 6) or (compression, filtering, interlace) != (0, 0, 0):
                    raise ValueError("PNG must be noninterlaced RGB/RGBA8")
                row_bytes = 1 + width * (3 if color == 2 else 4)
                expected = row_bytes * height
                inflater = zlib.decompressobj()
                checksum = binascii.crc32(header, binascii.crc32(kind))
            else:
                if kind == b"IDAT":
                    if ended_data:
                        raise ValueError("PNG IDAT chunks must be contiguous")
                    seen_data = True
                else:
                    ended_data |= seen_data
                    if kind == b"PLTE":
                        if seen_palette or seen_data or not 0 < length <= 768 or length % 3:
                            raise ValueError("invalid PNG palette order or size")
                        seen_palette = True
                    elif kind == b"IEND":
                        if length or not seen_data:
                            raise ValueError("invalid PNG IEND")
                    elif not kind[0] & 32:
                        raise ValueError("unsupported critical PNG chunk")
                checksum = binascii.crc32(kind)
                remaining = length
                while remaining:
                    block = read_exact(min(BLOCK_BYTES, remaining))
                    remaining -= len(block)
                    checksum = binascii.crc32(block, checksum)
                    if kind != b"IDAT":
                        continue
                    while block:
                        try:
                            pixels = inflater.decompress(block, min(BLOCK_BYTES, expected - decoded + 1))
                        except zlib.error as error:
                            raise ValueError("invalid PNG zlib stream") from error
                        if inflater.unused_data:
                            raise ValueError("trailing PNG zlib data")
                        if decoded + len(pixels) > expected:
                            raise ValueError("PNG decoded pixels exceed declared dimensions")
                        # Every scanline begins with exactly one filter byte.
                        first_filter = (-decoded) % row_bytes
                        if any(pixels[offset] > 4 for offset in range(first_filter, len(pixels), row_bytes)):
                            raise ValueError("invalid PNG scanline filter")
                        decoded += len(pixels)
                        block = inflater.unconsumed_tail
            if struct.unpack(">I", read_exact(4))[0] != checksum & 0xffffffff:
                raise ValueError("PNG chunk CRC mismatch")
            if kind == b"IEND":
                if not inflater.eof or decoded != expected:
                    raise ValueError("PNG pixel stream is truncated or has wrong decoded length")
                if stream.read(1):
                    raise ValueError("trailing bytes after PNG IEND")
                return width, height


class FramePipeline:
    def __init__(self, width, height, on_frame, workers=None, capacity=None, decode_size=None):
        """Validate and atomically publish private temporary frames on CPU workers.

        Callbacks run on validation threads in submission order and must not use
        Blender APIs. submit/flush/finish belong to the producer thread. A bounded
        queue applies backpressure while the renderer produces subsequent frames.
        """
        _dimensions(width, height)
        workers = max(1, min(os.cpu_count() or 1, 32)) if workers is None else workers
        if type(workers) is not int or not 1 <= workers <= 32:
            raise ValueError("frame validation workers must be 1..32")
        capacity = 2 * workers if capacity is None else capacity
        if type(capacity) is not int or not 1 <= capacity <= 64:
            raise ValueError("frame validation capacity must be 1..64")
        if not callable(on_frame):
            raise ValueError("frame callback must be callable")
        self.width, self.height = width, height
        self.workers, self.capacity = workers, capacity
        self.peak_in_flight = 0
        self.validation_wait_seconds = 0.0
        self._decode = decode_png_size if decode_size is None else decode_size
        self._on_frame = on_frame
        self._executor = ThreadPoolExecutor(max_workers=workers, thread_name_prefix="frame-validation")
        self._slots = threading.BoundedSemaphore(capacity)
        self._lock = threading.Lock()
        self._futures = []
        self._results = {}
        self._errors = {}
        self._completed = {}
        self._next_callback = self._in_flight = 0
        self._closed = False

    def submit(self, index, temporary, target, render_seconds):
        if self._closed:
            raise RuntimeError("frame pipeline is already finished")
        temporary, target = Path(temporary), Path(target)
        if temporary == target:
            raise ValueError("temporary frame and published destination must differ")
        started = time.perf_counter()
        self._slots.acquire()
        self.validation_wait_seconds += time.perf_counter() - started
        with self._lock:
            self._in_flight += 1
            self.peak_in_flight = max(self.peak_in_flight, self._in_flight)
        sequence = len(self._futures)
        try:
            future = self._executor.submit(self._process, sequence, index, temporary, target, render_seconds)
            self._futures.append(future)
        except BaseException:
            with self._lock:
                self._in_flight -= 1
            self._slots.release()
            raise

    def _process(self, sequence, index, temporary, target, render_seconds):
        result = error = None
        started = time.perf_counter()
        try:
            if tuple(self._decode(temporary)) != (self.width, self.height):
                raise ValueError("rendered PNG dimensions did not match request")
            os.replace(temporary, target)
            result = {"index": index, "render_seconds": render_seconds,
                      "validation_seconds": time.perf_counter() - started}
        except BaseException as failure:
            error = failure
        finally:
            try:
                temporary.unlink(missing_ok=True)
            except OSError as failure:
                if error is None:
                    error = failure
            try:
                with self._lock:
                    self._completed[sequence] = result, error
                    while self._next_callback in self._completed:
                        current = self._next_callback
                        completed, failure = self._completed.pop(current)
                        if failure is not None:
                            self._errors[current] = failure
                        elif not self._errors:
                            try:
                                self._on_frame(completed["index"], completed["render_seconds"],
                                               completed["validation_seconds"])
                            except BaseException as failure:
                                self._errors[current] = failure
                        if completed is not None:
                            self._results[current] = completed
                        self._next_callback += 1
            finally:
                with self._lock:
                    self._in_flight -= 1
                self._slots.release()

    def flush(self):
        """Drain all currently submitted frames without closing the pool."""
        for future in self._futures:
            future.result()
        if self._errors:
            raise self._errors[min(self._errors)]
        return [self._results[i] for i in range(len(self._futures))]

    def finish(self):
        if not self._closed:
            self._closed = True
            self._executor.shutdown(wait=True)
        return self.flush()

    def __enter__(self):
        return self

    def __exit__(self, error_type, error, traceback):
        if error_type is None:
            self.finish()
        else:
            try:
                self.finish()
            except BaseException:
                # A validation failure must not replace a render/main exception.
                pass
        return False
