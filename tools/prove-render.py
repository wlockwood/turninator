"""Run the Rust -> Blender -> verified PNG/GIF development proof.

Requires Python 3.10+ and Pillow. Processes use argument arrays; the future
desktop controller will own these operations in Rust.
"""
import argparse
import ctypes
import hashlib
import json
import os
import pathlib
import queue
import subprocess
import tempfile
import threading
import time

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[1]
PREFIX = "TURNINATOR_EVENT "
GIF_PALETTE = "full,opaque,sierra2_4a"


def validate_frames(directory, frame_count, size):
    if not isinstance(frame_count, int) or not 1 <= frame_count <= 10000:
        raise ValueError("Frame count must be 1..10000")
    if not isinstance(size, int) or not 1 <= size <= 8192:
        raise ValueError("Image size must be 1..8192")
    frames = []
    for index in range(frame_count):
        path = pathlib.Path(directory) / f"frame_{index:04d}.png"
        try:
            with Image.open(path) as frame:
                frame.load()
                if frame.format != "PNG" or frame.size != (size, size):
                    raise ValueError(f"Unexpected frame dimensions/format: {path}")
        except (OSError, SyntaxError) as error:
            raise ValueError(f"Unreadable frame: {path}") from error
        frames.append(path)
    return frames


def inspect_gif(path):
    delays = []
    with Image.open(path) as animation:
        if animation.format != "GIF":
            raise ValueError("Encoder did not produce a GIF")
        dimensions = list(animation.size)
        loop = animation.info.get("loop")
        for index in range(animation.n_frames):
            animation.seek(index)
            animation.load()
            delays.append(animation.info.get("duration", 0))
    return {"dimensions": dimensions, "frame_count": len(delays),
            "delays_ms": delays, "loop": loop, "duration_ms": sum(delays),
            "bytes": pathlib.Path(path).stat().st_size}


def run_encoder(args, cwd=None):
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=600,
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    if result.returncode:
        raise RuntimeError(f"FFmpeg failed ({result.returncode}): {result.stderr[-6000:]}")


def encode_gif(frames_dir, destination, ffmpeg, frame_count, size):
    validate_frames(frames_dir, frame_count, size)
    destination = pathlib.Path(destination).resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    with tempfile.TemporaryDirectory(prefix="turninator-encode-", dir=destination.parent) as temp:
        temp = pathlib.Path(temp)
        palette = temp / "palette.png"
        encoded = temp / "output.gif"
        frames_dir = pathlib.Path(frames_dir).resolve()
        pattern = "frame_%04d.png"
        common = [str(ffmpeg), "-hide_banner", "-loglevel", "error", "-y",
                  "-filter_threads", "1", "-filter_complex_threads", "1"]
        run_encoder(common + ["-framerate", "20", "-start_number", "0", "-i", pattern,
                              "-vf", f"trim=end_frame={frame_count},palettegen=stats_mode=full:reserve_transparent=0",
                              "-frames:v", "1", "-update", "1", str(palette)], cwd=frames_dir)
        run_encoder(common + ["-framerate", "20", "-start_number", "0", "-i", pattern,
                              "-f", "image2", "-pattern_type", "none", "-i", str(palette), "-lavfi", "paletteuse=dither=sierra2_4a",
                              "-frames:v", str(frame_count), "-r", "20", "-loop", "0",
                              "-final_delay", "5", str(encoded)], cwd=frames_dir)
        info = inspect_gif(encoded)
        if (info["frame_count"] != frame_count or info["dimensions"] != [size, size]
                or info["delays_ms"] != [50] * frame_count or info["loop"] != 0):
            raise ValueError(f"Encoded animation failed validation: {info}")
        os.replace(encoded, destination)
    info["encode_seconds"] = time.perf_counter() - started
    info["path"] = str(destination)
    info["palette"] = GIF_PALETTE
    return info


class OwnedProcess:
    """Kill-on-close Windows Job Object; never touch other processes."""
    def __init__(self, process):
        self.process = process
        self.handle = None
        if os.name != "nt":
            return
        from ctypes import wintypes

        class BasicLimits(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                        ("flags", wintypes.DWORD), ("min_working", ctypes.c_size_t),
                        ("max_working", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]

        class IoCounters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_uint64) for name in
                        ("read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [("basic", BasicLimits), ("io", IoCounters),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]

        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = wintypes.HANDLE
        self.kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                       ctypes.c_void_p, wintypes.DWORD]
        self.kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.kernel.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = self.kernel.CreateJobObjectW(None, None)
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000
        try:
            if not self.kernel.SetInformationJobObject(handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
                raise ctypes.WinError(ctypes.get_last_error())
            if not self.kernel.AssignProcessToJobObject(handle, int(process._handle)):
                raise ctypes.WinError(ctypes.get_last_error())
        except Exception:
            self.kernel.CloseHandle(handle)
            raise
        self.handle = handle

    def close(self):
        if self.process.poll() is None:
            if self.handle:
                self.kernel.TerminateJobObject(self.handle, 1)
            else:
                self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


class Worker:
    def __init__(self, blender, log_path):
        self.log = open(log_path, "a", encoding="utf-8")
        self.events = queue.Queue()
        try:
            self.process = subprocess.Popen(
                [str(blender), "--background", "--factory-startup", "--disable-autoexec",
                 "--python", str(ROOT / "renderer" / "worker.py")],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, encoding="utf-8", errors="replace", bufsize=1,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        except Exception:
            self.log.close()
            raise
        try:
            self.owner = OwnedProcess(self.process)
        except Exception:
            self.process.kill()
            self.process.wait(timeout=10)
            self.log.close()
            raise
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        try:
            for line in self.process.stdout:
                self.log.write(line)
                self.log.flush()
                if line.startswith(PREFIX):
                    try:
                        self.events.put(decode_event(line[len(PREFIX):]))
                    except ValueError as error:
                        self.events.put({"kind": "transport_error", "payload": {"message": str(error)}})
        finally:
            self.events.put({"kind": "eof", "payload": {"returncode": self.process.poll()}})

    def request(self, kind, request_id, timeout_seconds=None, **options):
        request = {"protocol_version": 1, "request_id": request_id,
                   "scene_revision": 1, "kind": kind, **options}
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        collected = []
        if timeout_seconds is None:
            timeout_seconds = 60 if kind == "probe" else 20 if kind == "shutdown" else 1800
        deadline = time.monotonic() + timeout_seconds
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"Worker request {request_id} timed out; inspect {self.log.name}")
            try:
                event = self.events.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError(f"Worker request {request_id} timed out; inspect {self.log.name}") from error
            if event["kind"] == "transport_error":
                raise RuntimeError(f"Malformed worker event: {event['payload']}")
            if event["kind"] == "eof":
                raise RuntimeError(f"Worker exited unexpectedly; inspect {self.log.name}")
            if event.get("request_id") != request_id:
                continue
            if event.get("scene_revision") != request["scene_revision"]:
                raise RuntimeError(f"Worker event has wrong revision for {request_id}")
            if event["kind"] == "error":
                raise RuntimeError(f"Worker error: {event.get('payload')}; inspect {self.log.name}")
            collected.append(event)
            if event["kind"] == "frame":
                payload = event.get("payload", {})
                print(f"Rendered frame {payload.get('index', '?')}", flush=True)
            if event["kind"] in ("complete", "probe", "shutdown"):
                return collected

    def close(self):
        if self.log.closed:
            return
        self.owner.close()
        self.reader.join(timeout=10)
        self.process.stdin.close()
        self.process.stdout.close()
        self.log.close()


def decode_event(raw):
    event = json.loads(raw)
    if (not isinstance(event, dict) or event.get("protocol_version") != 1
            or event.get("kind") not in ("ready", "probe", "frame", "complete", "error", "shutdown")
            or type(event.get("scene_revision")) is not int
            or not isinstance(event.get("payload"), dict)
            or (event.get("request_id") is not None and not isinstance(event.get("request_id"), str))):
        raise ValueError("Invalid worker event envelope")
    return event


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path, value):
    path = pathlib.Path(path)
    temporary = path.with_name(path.name + ".partial")
    temporary.write_text(json.dumps(value, indent=2), encoding="utf-8")
    os.replace(temporary, path)


def tool_identity(path, version_flag):
    path = pathlib.Path(path).resolve()
    result = subprocess.run([str(path), version_flag], capture_output=True, text=True,
                            timeout=30, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    if result.returncode:
        raise RuntimeError(f"Tool version probe failed: {path}: {result.stderr[-2000:]}")
    return {"path": str(path), "sha256": sha256(path), "version": result.stdout.splitlines()[0]}


def encode_only(output, ffmpeg):
    output = pathlib.Path(output).resolve()
    manifest_path = output / "proof-manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    recipe = manifest["recipe"]
    if sha256(recipe["mesh_path"]) != recipe["mesh_sha256"]:
        raise ValueError("Geometry snapshot changed; cannot reuse rendered frames")
    if "scene_path" in recipe and sha256(recipe["scene_path"]) != recipe["scene_sha256"]:
        raise ValueError("Scene snapshot changed; cannot reuse rendered frames")
    frame_manifest = json.loads((output / "frames" / "render-manifest.json").read_text(encoding="utf-8"))
    expected = {"geometry_hash": recipe["mesh_sha256"], "engine": recipe["engine"],
                "size": recipe["size"], "samples": recipe["samples"],
                "worker_hash": recipe["worker_sha256"], "scene_script_hash": recipe["scene_script_sha256"],
                "transparent": recipe.get("transparent", False),
                "angles": [recipe.get("start_angle_degrees", 0) + 360.0 * i / recipe["frames"]
                           for i in range(recipe["frames"])]}
    if "frame_pipeline_sha256" in recipe:
        expected["frame_pipeline_hash"] = recipe["frame_pipeline_sha256"]
    if any(frame_manifest.get(key) != value for key, value in expected.items()):
        raise ValueError("Frame render manifest differs from proof recipe; cannot reuse these images")
    if "scene_path" in recipe:
        scene_snapshot = json.loads(pathlib.Path(recipe["scene_path"]).read_text(encoding="utf-8"))
        if frame_manifest.get("scene") != scene_snapshot:
            raise ValueError("Frame scene differs from requested immutable snapshot")
    if "probe" in manifest:
        probe = manifest["probe"]
        if frame_manifest.get("blender_version") != probe["blender_version"]:
            raise ValueError("Frame Blender version differs from probed renderer")
        build = frame_manifest.get("blender_build_hash", "").removeprefix("b'").removesuffix("'")
        if build != probe["blender_build_hash"]:
            raise ValueError("Frame Blender build differs from probed renderer")
    info = encode_gif(output / "frames", output / "turntable.gif", ffmpeg, recipe["frames"], recipe["size"])
    manifest["encoding"] = {"tool": tool_identity(ffmpeg, "-version"), "controller_sha256": sha256(__file__),
                            "palette": info["palette"], "fps": 20, "final_delay_cs": 5, "result": info}
    manifest["stage"] = "complete"
    write_json(manifest_path, manifest)
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job", required=True, type=pathlib.Path)
    parser.add_argument("--blender", default="C:/Program Files/Blender Foundation/Blender 4.5/blender.exe")
    parser.add_argument("--ffmpeg", default="C:/Program Files/ffmpeg/bin/ffmpeg.exe")
    parser.add_argument("--engine", choices=("eevee", "cycles"), default="eevee")
    parser.add_argument("--size", type=int, default=960)
    parser.add_argument("--frames", type=int, default=80)
    parser.add_argument("--still-size", type=int, default=4096)
    parser.add_argument("--samples", type=int, default=32)
    parser.add_argument("--encode-only", action="store_true", help="--job names a retained proof directory; no Blender launch")
    args = parser.parse_args()
    if args.encode_only:
        print(json.dumps(encode_only(args.job, args.ffmpeg), indent=2))
        return
    if not (1 <= args.frames <= 10000 and 16 <= args.size <= 8000
            and 16 <= args.still_size <= 8000 and 1 <= args.samples <= 4096):
        parser.error("Frame count, resolution or sample count outside proof limits")
    job = args.job.resolve()
    scene_path = job / "scene.json"
    scene = json.loads(scene_path.read_text(encoding="utf-8"))
    blender_identity = tool_identity(args.blender, "--version")
    recipe = {"scene_path": str(scene_path), "scene_sha256": sha256(scene_path),
              "mesh_path": scene["mesh_path"], "mesh_sha256": sha256(scene["mesh_path"]),
              "blender": blender_identity,
              "worker_sha256": sha256(ROOT / "renderer" / "worker.py"),
              "scene_script_sha256": sha256(ROOT / "renderer" / "scene.py"),
              "frame_pipeline_sha256": sha256(ROOT / "renderer" / "frame_pipeline.py"),
              "engine": args.engine, "size": args.size, "frames": args.frames,
              "samples": args.samples, "still_size": args.still_size,
              "start_angle_degrees": 0, "transparent": False}
    fingerprint = hashlib.sha256(json.dumps(recipe, sort_keys=True).encode()).hexdigest()[:12]
    output = job / f"proof-{args.engine}-{fingerprint}"
    output.mkdir(exist_ok=True)
    metrics_path = output / "metrics.json"
    if metrics_path.exists():
        parser.error(f"Completed proof preserved at {output}; use --encode-only --job <that-directory> to re-encode")
    started = time.perf_counter()
    metrics = {"recipe": recipe, "fingerprint": fingerprint, "blender_path": args.blender,
               "ffmpeg_path": args.ffmpeg, "artifacts": {}, "requests": [], "stage": "preparing"}
    manifest_path = output / "proof-manifest.json"
    if manifest_path.exists():
        existing = json.loads(manifest_path.read_text(encoding="utf-8"))
        if existing["recipe"] != recipe:
            raise ValueError("Proof manifest differs from requested recipe")
    write_json(manifest_path, metrics)
    worker = None
    try:
        worker = Worker(args.blender, output / "blender.log")
        metrics["probe"] = worker.request("probe", "proof-probe")[-1]["payload"]
        common = {"scene_path": str(scene_path), "engine": args.engine,
                  "samples": args.samples, "start_angle_degrees": 0,
                  "transparent": False}
        try:
            worker.request("unsupported-proof-command", "proof-invalid")
        except RuntimeError:
            metrics["invalid_request_recovered"] = True
        metrics["stage"] = "rendering"
        write_json(manifest_path, metrics)
        metrics["requests"].append(worker.request("render", "proof-still", **common,
                                                  mode="still", size=args.still_size,
                                                  frame_count=1, output_dir=str(output / "still"),
                                                  reuse_verified_frames=(output / "still" / "render-manifest.json").exists()))
        if metrics["requests"][-1][-1]["payload"]["geometry_hash"] != recipe["mesh_sha256"]:
            raise ValueError("Worker loaded changed geometry")
        with Image.open(output / "still" / "still.png") as still:
            still.load()
            if still.size != (args.still_size, args.still_size):
                raise ValueError("Still dimensions did not match request")
        metrics["artifacts"]["still"] = str(output / "still" / "still.png")
        metrics["requests"].append(worker.request("render", "proof-turntable", **common,
                                                  mode="turntable", size=args.size,
                                                  frame_count=args.frames, output_dir=str(output / "frames"),
                                                  reuse_verified_frames=(output / "frames" / "render-manifest.json").exists()))
        if metrics["requests"][-1][-1]["payload"]["geometry_hash"] != recipe["mesh_sha256"]:
            raise ValueError("Geometry changed between render requests")
        metrics["stage"] = "encoding"
        write_json(manifest_path, metrics)
        gif = encode_only(output, args.ffmpeg)
        metrics["artifacts"]["gif"] = gif
        worker.request("shutdown", "proof-shutdown")
        worker.process.wait(timeout=20)
        if worker.process.returncode != 0:
            raise RuntimeError(f"Blender exited with {worker.process.returncode}")
        metrics["persistent_requests_verified"] = 2
        frame_events = [event for events in metrics["requests"] for event in events if event["kind"] == "frame"]
        metrics["frames_reused"] = sum(bool(event["payload"].get("reused")) for event in frame_events)
        metrics["frames_rendered"] = len(frame_events) - metrics["frames_reused"]
        metrics["fresh_render_measurement"] = metrics["frames_reused"] == 0
        metrics["total_seconds"] = time.perf_counter() - started
        metrics["encoding"] = json.loads(manifest_path.read_text(encoding="utf-8"))["encoding"]
        metrics["stage"] = "complete"
        write_json(metrics_path, metrics)
        write_json(manifest_path, metrics)
        print(json.dumps({"metrics": str(metrics_path), "still": metrics["artifacts"]["still"],
                          "gif": gif["path"], "total_seconds": metrics["total_seconds"]}, indent=2))
    except Exception as error:
        metrics["stage"] = "failed"
        metrics["error"] = str(error)
        write_json(manifest_path, metrics)
        raise
    finally:
        if worker is not None:
            worker.close()


if __name__ == "__main__":
    main()
