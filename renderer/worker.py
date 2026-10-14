"""Blender main-thread JSON worker. No user source files are modified."""
import json
import hashlib
import math
import os
from pathlib import Path
import sys
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parent))
from scene import build_scene, load_scene_spec, probe_devices, turntable_angles

PREFIX = "TURNINATOR_EVENT "


def emit(request, kind, payload):
    event = {"protocol_version": 1, "request_id": request.get("request_id"),
             "scene_revision": request.get("scene_revision", 0), "kind": kind, "payload": payload}
    print(PREFIX + json.dumps(event, allow_nan=False), flush=True)


def validate_request(request):
    if not isinstance(request, dict) or request.get("protocol_version") != 1:
        raise ValueError("unsupported protocol version")
    if not isinstance(request.get("request_id"), str) or not request["request_id"]:
        raise ValueError("request_id must be a nonempty string")
    if type(request.get("scene_revision")) is not int or request["scene_revision"] < 0:
        raise ValueError("scene_revision must be a nonnegative integer")
    if request.get("kind") not in ("probe", "render", "shutdown"):
        raise ValueError("unsupported request kind")


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def prepare_manifest(directory, recipe, reuse):
    path = directory / "render-manifest.json"
    if reuse:
        if not path.is_file():
            raise ValueError("frame reuse requires an existing render manifest")
        if path.stat().st_size > 2_097_152:
            raise ValueError("render manifest exceeds budget")
        if json.loads(path.read_text(encoding="utf-8")) != recipe:
            raise ValueError("render manifest differs; refuse mismatched frame reuse")
        return
    temporary = directory / "render-manifest.partial.json"
    temporary.write_text(json.dumps(recipe, sort_keys=True, indent=2, allow_nan=False), encoding="utf-8")
    os.replace(temporary, path)


def should_reuse_frame(path, size, reuse, decode_size):
    if not path.exists():
        return False
    if not reuse:
        raise ValueError("output frame already exists; use fresh output or verified reuse")
    try:
        return tuple(decode_size(path)) == (size, size)
    except Exception:
        return False


def decode_png_size(path):
    import bpy
    # Read signature as well: bpy otherwise accepts arbitrary image formats.
    with Path(path).open("rb") as stream:
        if stream.read(8) != b"\x89PNG\r\n\x1a\n":
            raise ValueError("rendered frame is not PNG")
    check = bpy.data.images.load(str(path), check_existing=False)
    try:
        if not check.pixels or not math.isfinite(check.pixels[0]):
            raise RuntimeError("rendered PNG pixels are unavailable")
        return tuple(check.size)
    finally:
        bpy.data.images.remove(check)


def render(request):
    import bpy
    started = time.perf_counter()
    engine = request.get("engine", "eevee")
    mode = request.get("mode", "still")
    if engine not in ("eevee", "cycles") or mode not in ("still", "turntable"):
        raise ValueError("unsupported engine or mode")
    size = request.get("size", 960)
    width = request.get("width", size)
    height = request.get("height", size)
    samples = request.get("samples", 32)
    if type(size) is not int or not 16 <= size <= 8000:
        raise ValueError("size must be 16..8000 pixels")
    if type(width) is not int or type(height) is not int or not 16 <= width <= 8000 or not 16 <= height <= 8000 or width * height > 64_000_000:
        raise ValueError("invalid width/height or image budget exceeded")
    if type(samples) is not int or not 1 <= samples <= 4096:
        raise ValueError("samples must be 1..4096")
    transparent = request.get("transparent", False)
    if type(transparent) is not bool:
        raise ValueError("transparent must be boolean")
    reuse = request.get("reuse_verified_frames", False)
    if type(reuse) is not bool:
        raise ValueError("reuse_verified_frames must be boolean")
    raw_directory = request.get("output_dir")
    if not isinstance(raw_directory, str) or not Path(raw_directory).is_absolute():
        raise ValueError("output_dir must be absolute")
    output_dir = Path(raw_directory)
    angles = turntable_angles(request.get("frame_count", 80) if mode == "turntable" else 1,
                              request.get("start_angle_degrees", 0.0))
    spec = load_scene_spec(request["scene_path"])
    if mode == "turntable":
        spec["fit_turntable"] = True
    output_dir.mkdir(parents=True, exist_ok=True)
    paths = [output_dir / (f"frame_{i:04d}.png" if mode == "turntable" else "still.png") for i in range(len(angles))]
    # Initial worker refuses overwrites; the controller owns unique job directories.
    if not reuse and any(path.exists() for path in paths):
        raise ValueError("output frame already exists; use a fresh output directory")
    recipe = {"schema_version": 1, "scene": spec,
              "geometry_hash": file_hash(spec["mesh_path"]),
              "worker_hash": file_hash(__file__), "scene_script_hash": file_hash(Path(__file__).with_name("scene.py")),
              "blender_version": bpy.app.version_string, "blender_build_hash": str(bpy.app.build_hash),
              "engine": engine, "size": size, "width": width, "height": height, "samples": samples, "mode": mode,
              "angles": angles, "transparent": transparent}
    prepare_manifest(output_dir, recipe, reuse)
    obj, info = build_scene(spec, engine, size, samples, transparent, width, height)
    if info["geometry_hash"] != recipe["geometry_hash"]:
        raise ValueError("geometry changed during scene preparation")
    prepare_seconds = time.perf_counter() - started
    completed = []
    durations = []
    for index, (angle, output_path) in enumerate(zip(angles, paths)):
        frame_start = time.perf_counter()
        if should_reuse_frame(output_path, size, reuse, lambda path: (size, size) if decode_png_size(path) == (width, height) else (0, 0)):
            elapsed = time.perf_counter() - frame_start
            durations.append(elapsed)
            completed.append(str(output_path))
            emit(request, "frame", {"path": str(output_path), "index": index, "angle_degrees": angle,
                                    "elapsed_seconds": elapsed, "size": size, "reused": True})
            continue
        obj.rotation_euler.z = math.radians(angle)
        temporary = output_path.with_name(output_path.stem + ".partial.png")
        try:
            bpy.context.scene.render.filepath = str(temporary)
            bpy.ops.render.render(write_still=True)
            if decode_png_size(temporary) != (width, height):
                raise RuntimeError("rendered PNG dimensions did not match request")
            os.replace(temporary, output_path)
        finally:
            if temporary.exists():
                temporary.unlink()
        elapsed = time.perf_counter() - frame_start
        durations.append(elapsed)
        completed.append(str(output_path))
        emit(request, "frame", {"path": str(output_path), "index": index, "angle_degrees": angle,
                                "elapsed_seconds": elapsed, "size": size, "reused": False})
    emit(request, "complete", {"paths": completed, "engine": engine, "size": size, "samples": samples,
                               "prepare_seconds": prepare_seconds, "frame_seconds": durations,
                               "total_seconds": time.perf_counter() - started, **info})


def main():
    emit({}, "ready", {"protocol_version": 1})
    while True:
        raw = sys.stdin.readline(1_048_577)
        if not raw:
            break
        request = {}
        try:
            if len(raw) > 1_048_576:
                # Drain the rest of this rejected line so the next request stays aligned.
                while raw and not raw.endswith("\n"):
                    raw = sys.stdin.readline(1_048_577)
                raise ValueError("request line exceeds 1 MiB")
            parsed = json.loads(raw)
            if isinstance(parsed, dict):
                request = parsed
            validate_request(parsed)
            if request["kind"] == "probe":
                emit(request, "probe", probe_devices())
            elif request["kind"] == "render":
                render(request)
            else:
                emit(request, "shutdown", {})
                break
        except Exception as error:
            traceback.print_exc(file=sys.stderr)
            # Invalid identities may not be JSON-safe; sanitize error envelope fields.
            identity = {"request_id": request.get("request_id") if isinstance(request.get("request_id"), str) else None,
                        "scene_revision": request.get("scene_revision") if type(request.get("scene_revision")) is int else 0}
            emit(identity, "error", {"error_type": type(error).__name__, "message": str(error)})


if __name__ == "__main__":
    main()
