"""Blender main-thread JSON worker. No user source files are modified."""
import json
import hashlib
import math
import os
import shutil
from pathlib import Path
import sys
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parent))
from scene import (build_scene, load_scene_spec, probe_devices, turntable_angles, buildup_schedule,
                   set_visible_layer, validate_camera, layer_for_fraction, layer_for_count, prepare_buildup_section,
                   set_camera_spec, apply_layer_filter)

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


def identical_frame_sources(angles, states, visible_layers):
    """First frame with exactly the same rotation, camera and deposited mesh."""
    first = {}
    sources = []
    for index, (angle, layer) in enumerate(zip(angles, visible_layers)):
        camera = None if states is None else json.dumps(states[index]["camera"], sort_keys=True, allow_nan=False)
        key = (angle % 360, camera, layer)
        sources.append(first.setdefault(key, index))
    return sources


def copy_identical_frame(source, target, width, height, decode_size=decode_png_size):
    temporary = target.with_name(target.stem + ".partial.png")
    try:
        shutil.copyfile(source, temporary)
        if tuple(decode_size(temporary)) != (width, height):
            raise ValueError("identical source PNG dimensions did not match request")
        os.replace(temporary, target)
    finally:
        if temporary.exists():
            temporary.unlink()


def render_spec_for_mode(spec, mode):
    """Derived camera/visibility flags must not change the source scene identity."""
    result = dict(spec)
    if mode != "still":
        result["fit_turntable"] = True
    if mode == "buildup":
        result["build_up"] = True
    return result


def validate_frame_states(states, count):
    if not isinstance(states,list) or len(states)!=count or count>10000:
        raise ValueError("timeline states must match the bounded frame count")
    previous=-1.0
    building=None
    for state in states:
        if not isinstance(state,dict): raise ValueError("invalid timeline frame")
        validate_camera(state.get("camera"))
        angle=state.get("angle_degrees")
        if isinstance(angle,bool) or not isinstance(angle,(float,int)) or not math.isfinite(angle) or abs(angle)>1e9:
            raise ValueError("invalid timeline angle")
        fraction=state.get("buildup_fraction")
        active=fraction is not None
        if building is not None and building!=active:
            raise ValueError("build-up must be consistently enabled across timeline")
        building=active
        if active:
            if isinstance(fraction,bool) or not isinstance(fraction,(float,int)) or not math.isfinite(fraction) or not 0<=fraction<=1 or fraction<previous:
                raise ValueError("build fractions must be finite, monotone and within 0..1")
            previous=fraction
    return states


def load_frame_states(request, count):
    states=request.get("frame_states")
    if request.get("frame_states_path") is not None:
        if states is not None: raise ValueError("supply inline states or one state file")
        path=Path(request["frame_states_path"])
        if not path.is_absolute(): raise ValueError("timeline state path must be absolute")
        with path.open("rb") as stream:
            data=stream.read(8*1024*1024+1)
        if len(data)>8*1024*1024: raise ValueError("timeline state file exceeds 8 MiB")
        states=json.loads(data)
    return None if states is None else validate_frame_states(states,count)


def completion_event(payload, details_path):
    result=dict(payload)
    if len(json.dumps(result,allow_nan=False).encode())>900000:
        for field in ("paths","frame_seconds","frame_analysis","build_layers"):
            result.pop(field,None)
        result["details_path"]=str(details_path)
        if len(json.dumps(result,allow_nan=False).encode())>900000:
            result["warnings_in_details"]=len(result.pop("warnings",[]))
    return result


def render(request):
    import bpy
    started = time.perf_counter()
    engine = request.get("engine", "eevee")
    mode = request.get("mode", "still")
    if engine not in ("eevee", "cycles") or mode not in ("still", "turntable", "buildup"):
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
    seconds_per_turn = request.get("seconds_per_turn")
    if seconds_per_turn is not None and (mode != "turntable" or request.get("frame_states") is not None or request.get("frame_states_path") is not None):
        raise ValueError("basic rotation speed requires a turntable without camera frame states")
    angles = turntable_angles(request.get("frame_count", 80) if mode != "still" else 1,
                              request.get("start_angle_degrees", 0.0), seconds_per_turn)
    if mode == "buildup":
        angles = [angles[0]]*len(angles)
    states=load_frame_states(request,len(angles))
    if states is not None:
        angles=[state["angle_degrees"] for state in states]
    progressive=mode=="buildup" or states is not None and states[0]["buildup_fraction"] is not None
    stage=request.get("build_stage")
    if stage is not None and (not isinstance(stage,dict) or type(stage.get("visible_layers")) is not int or not 0<=stage["visible_layers"]<=50000):
        raise ValueError("invalid fixed build stage")
    if stage is not None and progressive:
        raise ValueError("choose fixed build stage or progressive build-up")
    building=progressive or stage is not None
    spec = load_scene_spec(request["scene_path"])
    output_dir.mkdir(parents=True, exist_ok=True)
    paths = [output_dir / (f"frame_{i:04d}.png" if mode != "still" else "still.png") for i in range(len(angles))]
    # Initial worker refuses overwrites; the controller owns unique job directories.
    if not reuse and any(path.exists() for path in paths):
        raise ValueError("output frame already exists; use a fresh output directory")
    recipe = {"schema_version": 1, "scene": spec,
              "geometry_hash": file_hash(spec["mesh_path"]),
              "worker_hash": file_hash(__file__), "scene_script_hash": file_hash(Path(__file__).with_name("scene.py")),
              "blender_version": bpy.app.version_string, "blender_build_hash": str(bpy.app.build_hash),
              "engine": engine, "size": size, "width": width, "height": height, "samples": samples, "mode": mode,
              "angles": angles, "transparent": transparent,
              "seconds_per_turn": seconds_per_turn,
              "frame_states_hash":hashlib.sha256(json.dumps(states,sort_keys=True,allow_nan=False).encode()).hexdigest() if states is not None else None,
              "build_stage":stage}
    prepare_manifest(output_dir, recipe, reuse)
    render_spec=render_spec_for_mode(spec, mode)
    render_spec["build_up"]=building
    if states is not None:
        render_spec["fit_turntable"]=False
        render_spec["camera"]=states[0]["camera"]
    obj, info = build_scene(render_spec, engine, size, samples, transparent, width, height)
    if info["geometry_hash"] != recipe["geometry_hash"]:
        raise ValueError("geometry changed during scene preparation")
    prepare_seconds = time.perf_counter() - started
    visible_layers = ([layer_for_count(info["build_layers"],stage["visible_layers"])]*len(angles) if stage is not None
                      else [layer_for_fraction(info["build_layers"],state["buildup_fraction"]) for state in states] if states is not None and building
                      else buildup_schedule(info["build_layers"],len(angles)) if building else [None]*len(angles))
    source_mesh=obj.data.copy() if building and spec.get("section") is not None else None
    sources = identical_frame_sources(angles, states, visible_layers)
    frame_analysis=[]
    completed = []
    durations = []
    rendered_count = 0
    copied_count = 0
    verified_count = 0
    for index, (angle, output_path) in enumerate(zip(angles, paths)):
        frame_start = time.perf_counter()
        if sources[index] < index:
            if not should_reuse_frame(output_path, size, reuse, lambda path: (size, size) if decode_png_size(path) == (width, height) else (0, 0)):
                copy_identical_frame(paths[sources[index]], output_path, width, height)
                copied_count += 1
            else:
                verified_count += 1
            frame_analysis.append(frame_analysis[sources[index]])
            elapsed = time.perf_counter() - frame_start
            durations.append(elapsed)
            completed.append(str(output_path))
            emit(request, "frame", {"path": str(output_path), "index": index, "angle_degrees": angle,
                                    "elapsed_seconds": elapsed, "size": size, "reused": True,
                                    "reuse_kind": "identical", "source_index": sources[index],
                                    "visible_layer": visible_layers[index], "section_analysis": frame_analysis[-1]})
            continue
        if visible_layers[index] is not None:
            if source_mesh is not None:
                info["section_analysis"]=prepare_buildup_section(obj,source_mesh,spec,info["bounds_m"],visible_layers[index])
            else:
                set_visible_layer(obj,visible_layers[index])
        frame_analysis.append(info.get("section_analysis"))
        if states is not None:
            set_camera_spec(bpy.context.scene.camera,states[index]["camera"])
            if spec.get("layer_appearance",False) and spec.get("layer_filtering",True):
                for material in obj.data.materials:
                    apply_layer_filter(material,bpy.context.scene.camera,width,height,min(spec["layer_height_mm"],spec.get("first_layer_height_mm",spec["layer_height_mm"])))
        if should_reuse_frame(output_path, size, reuse, lambda path: (size, size) if decode_png_size(path) == (width, height) else (0, 0)):
            verified_count += 1
            elapsed = time.perf_counter() - frame_start
            durations.append(elapsed)
            completed.append(str(output_path))
            emit(request, "frame", {"path": str(output_path), "index": index, "angle_degrees": angle,
                                    "elapsed_seconds": elapsed, "size": size, "reused": True,"visible_layer":visible_layers[index],"section_analysis":info.get("section_analysis")})
            continue
        obj.rotation_euler.z = math.radians(angle)
        temporary = output_path.with_name(output_path.stem + ".partial.png")
        try:
            bpy.context.scene.render.filepath = str(temporary)
            bpy.ops.render.render(write_still=True)
            rendered_count += 1
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
                                "elapsed_seconds": elapsed, "size": size, "reused": False,"visible_layer":visible_layers[index],"section_analysis":info.get("section_analysis")})
    analysis_path=output_dir/"frame-analysis.json"
    analysis_temporary=analysis_path.with_suffix(".partial.json")
    analysis_temporary.write_text(json.dumps(frame_analysis,allow_nan=False),encoding="utf-8")
    os.replace(analysis_temporary,analysis_path)
    payload={"paths": completed, "engine": engine, "size": size, "samples": samples,
             "frame_count":len(angles),"build_layer_count":len(info.get("build_layers") or []),
             "unique_frame_count": len(set(sources)), "rendered_frame_count": rendered_count,
             "copied_frame_count": copied_count, "verified_frame_count": verified_count,
             "prepare_seconds": prepare_seconds, "frame_seconds": durations,
             "total_seconds": time.perf_counter() - started,"frame_analysis":frame_analysis,
             "frame_analysis_path":str(analysis_path),**info}
    details_path=output_dir/"render-result.json"
    details_temporary=details_path.with_suffix(".partial.json")
    details_temporary.write_text(json.dumps(payload,allow_nan=False),encoding="utf-8")
    os.replace(details_temporary,details_path)
    emit(request, "complete", completion_event(payload,details_path))


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
