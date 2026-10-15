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
import threading

sys.path.insert(0, str(Path(__file__).resolve().parent))
from scene import (build_scene, load_scene_spec, probe_devices, turntable_angles, buildup_schedule,
                   set_visible_layer, validate_camera, layer_for_fraction, layer_for_count, prepare_buildup_section,
                   set_camera_spec, apply_layer_filter)

from frame_pipeline import FramePipeline, decode_png_size

PREFIX = "TURNINATOR_EVENT "
_EVENT_LOCK = threading.Lock()


def emit(request, kind, payload):
    event = {"protocol_version": 1, "request_id": request.get("request_id"),
             "scene_revision": request.get("scene_revision", 0), "kind": kind, "payload": payload}
    with _EVENT_LOCK:
        # Keep the text and newline in one write while Blender logs the next
        # GPU frame; print() would leave a separate text/newline write gap.
        sys.stdout.write(PREFIX + json.dumps(event, allow_nan=False) + '\n')
        sys.stdout.flush()


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


def identical_frame_sources(angles, states, visible_layers):
    """First frame with exactly the same rotation, camera and deposited mesh."""
    first = {}
    sources = []
    for index, (angle, layer) in enumerate(zip(angles, visible_layers)):
        camera = None if states is None else json.dumps(states[index]["camera"], sort_keys=True, allow_nan=False)
        key = (angle % 360, camera, layer)
        sources.append(first.setdefault(key, index))
    return sources


def animation_prefix(mode, states, progressive, reuse, sources):
    """Batch only a fresh static turntable with a contiguous unique prefix."""
    if mode != 'turntable' or states is not None or progressive or reuse:
        return 0
    count = len(set(sources))
    if count < 2 or sources[:count] != list(range(count)) or any(i >= count for i in sources[count:]):
        return 0
    return count


def render_animation(obj, angles, paths, width, height, on_frame, decode_size=decode_png_size, pipeline_stats=None):
    """Keep Blender rendering while CPU threads validate and publish PNGs.

    Only main-thread handlers touch Blender data. The bounded publication pool
    uses plain file bytes and reports ordered progress from its CPU threads.
    """
    import bpy
    scene = bpy.context.scene
    scratch = paths[0].parent / '.render-animation'
    scratch.mkdir(exist_ok=False)
    temporary = [scratch / f'frame_{i:04d}.png' for i in range(len(paths))]
    previous = (scene.frame_start, scene.frame_end, scene.frame_current, scene.render.filepath)
    rendered = []
    errors = []
    last = [time.perf_counter()]
    def report(index, render_seconds, validation_seconds):
        on_frame(index, render_seconds + validation_seconds)
    try:
        with FramePipeline(width,height,report,workers=min(len(paths),os.cpu_count() or 1,32),decode_size=decode_size) as pipeline:
            def before_frame(scene, *_):
                if errors:
                    return
                try:
                    obj.rotation_euler.z = math.radians(angles[scene.frame_current])
                except Exception as error:
                    errors.append(error)
            def after_write(scene, *_):
                if errors:
                    return
                try:
                    index = scene.frame_current
                    if index != len(rendered):
                        raise RuntimeError('animation frames arrived out of order')
                    now = time.perf_counter()
                    rendered.append(now-last[0])
                    last[0] = now
                    pipeline.submit(index,temporary[index],paths[index],rendered[-1])
                except Exception as error:
                    # Blender swallows handler errors; propagate them once its
                    # operator returns instead of completing with wrong frames.
                    errors.append(error)
            bpy.app.handlers.frame_change_pre.append(before_frame)
            bpy.app.handlers.render_write.append(after_write)
            try:
                scene.frame_start = 0
                scene.frame_end = len(paths) - 1
                scene.render.filepath = str(scratch / 'frame_')
                result = bpy.ops.render.render(animation=True)
                if errors:
                    raise errors[0]
                if result != {'FINISHED'} or len(rendered) != len(paths):
                    raise RuntimeError('animation did not produce every requested frame')
                drain_started = time.perf_counter()
                timings = pipeline.finish()
                if pipeline_stats is not None:
                    pipeline_stats['validation_drain_seconds'] = time.perf_counter()-drain_started
                    record_pipeline(pipeline_stats,pipeline,timings)
                return [t['render_seconds']+t['validation_seconds'] for t in timings]
            finally:
                bpy.app.handlers.frame_change_pre.remove(before_frame)
                bpy.app.handlers.render_write.remove(after_write)
                scene.frame_start, scene.frame_end, scene.frame_current, scene.render.filepath = previous
    finally:
        # Context cleanup has joined the CPU readers before removing leftovers.
        # Delete only enumerated scratch files, never a recursive directory.
        for path in temporary:
            path.unlink(missing_ok=True)
        scratch.rmdir()


def record_pipeline(summary,pipeline,timings):
    summary['workers'] = max(summary.get('workers',0),pipeline.workers)
    summary['peak_in_flight'] = max(summary.get('peak_in_flight',0),pipeline.peak_in_flight)
    summary['producer_backpressure_seconds'] = summary.get('producer_backpressure_seconds',0.) + pipeline.validation_wait_seconds
    summary['validation_work_seconds'] = summary.get('validation_work_seconds',0.) + sum(t['validation_seconds'] for t in timings)
    summary['validated_frames'] = summary.get('validated_frames',0) + len(timings)


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
    counted=None
    previous_count=-1
    preserve_camera=None
    for state in states:
        if not isinstance(state,dict): raise ValueError("invalid timeline frame")
        preserve=state.get("preserve_scene_camera",False)
        if type(preserve) is not bool or preserve_camera is not None and preserve_camera!=preserve:
            raise ValueError("scene camera preservation must be consistent across the sequence")
        preserve_camera=preserve
        if preserve:
            if state.get("camera") is not None:
                raise ValueError("choose scene camera preservation or an explicit frame camera")
        else:
            validate_camera(state.get("camera"))
        angle=state.get("angle_degrees")
        if isinstance(angle,bool) or not isinstance(angle,(float,int)) or not math.isfinite(angle) or abs(angle)>1e9:
            raise ValueError("invalid timeline angle")
        fraction=state.get("buildup_fraction")
        count=state.get("visible_layers")
        if count is not None and (type(count) is not int or not 0<=count<=50000 or count<previous_count):
            raise ValueError("visible layer counts must be monotone integers within 0..50000")
        if counted is not None and counted!=(count is not None):
            raise ValueError("exact layer counts must be consistently enabled across timeline")
        counted=count is not None
        if counted:
            if fraction is not None:
                raise ValueError("choose exact layer counts or build fractions")
            previous_count=count
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


def with_analysis_warnings(payload):
    """Keep frame-specific cut limitations visible in the export job result."""
    result=dict(payload)
    warnings=list(dict.fromkeys(result.get("warnings",[])))
    for field in ("frame_build_analysis","frame_analysis"):
        for analysis in result.get(field,[]):
            if analysis is not None:
                for warning in analysis.get("warnings",[]):
                    if warning not in warnings:
                        warnings.append(warning)
    result["warnings"]=warnings
    return result


def completion_event(payload, details_path):
    result=with_analysis_warnings(payload)
    if len(json.dumps(result,allow_nan=False).encode())>900000:
        for field in ("paths","frame_seconds","frame_analysis","frame_build_analysis","build_layers"):
            result.pop(field,None)
        result["details_path"]=str(details_path)
        if len(json.dumps(result,allow_nan=False).encode())>900000:
            result["warnings_in_details"]=len(result.pop("warnings",[]))
    return result


def visible_layer_schedule(layers, stage, states, building, count):
    if stage is not None:
        return [layer_for_count(layers,stage["visible_layers"])]*count
    if states is not None and states[0].get("visible_layers") is not None:
        return [layer_for_count(layers,state["visible_layers"]) for state in states]
    if states is not None and building:
        return [layer_for_fraction(layers,state["buildup_fraction"]) for state in states]
    return buildup_schedule(layers,count) if building else [None]*count


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
    frame_rate = request.get("frame_rate",20)
    if seconds_per_turn is not None and (mode != "turntable" or request.get("frame_states") is not None or request.get("frame_states_path") is not None):
        raise ValueError("basic rotation speed requires a turntable without camera frame states")
    angles = turntable_angles(request.get("frame_count", 80) if mode != "still" else 1,
                              request.get("start_angle_degrees", 0.0), seconds_per_turn, frame_rate)
    if mode == "buildup":
        angles = [angles[0]]*len(angles)
    states=load_frame_states(request,len(angles))
    if states is not None:
        angles=[state["angle_degrees"] for state in states]
    progressive=mode=="buildup" or states is not None and (states[0]["buildup_fraction"] is not None or states[0].get("visible_layers") is not None)
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
              "frame_pipeline_hash": file_hash(Path(__file__).with_name("frame_pipeline.py")),
              "blender_version": bpy.app.version_string, "blender_build_hash": str(bpy.app.build_hash),
              "engine": engine, "size": size, "width": width, "height": height, "samples": samples, "mode": mode,
              "angles": angles, "transparent": transparent,
              "seconds_per_turn": seconds_per_turn,
              "frame_rate": frame_rate,
              "frame_states_hash":hashlib.sha256(json.dumps(states,sort_keys=True,allow_nan=False).encode()).hexdigest() if states is not None else None,
              "build_stage":stage}
    prepare_manifest(output_dir, recipe, reuse)
    render_spec=render_spec_for_mode(spec, mode)
    render_spec["build_up"]=building
    if states is not None and not states[0].get("preserve_scene_camera",False):
        render_spec["fit_turntable"]=False
        render_spec["camera"]=states[0]["camera"]
    obj, info = build_scene(render_spec, engine, size, samples, transparent, width, height)
    bpy.context.scene.render.fps = frame_rate
    if info["geometry_hash"] != recipe["geometry_hash"]:
        raise ValueError("geometry changed during scene preparation")
    prepare_seconds = time.perf_counter() - started
    visible_layers = visible_layer_schedule(info["build_layers"],stage,states,building,len(angles))
    source_mesh=obj.data.copy() if building and (spec["geometry_kind"] == "original" or spec.get("section") is not None) else None
    sources = identical_frame_sources(angles, states, visible_layers)
    batch_count = animation_prefix(mode, states, progressive, reuse, sources)
    batch_times = []
    pipeline_stats = {}
    frame_pipeline = None
    submitted_frames = []
    def report_individual(sequence,render_seconds,validation_seconds):
        index,payload = submitted_frames[sequence]
        elapsed = render_seconds+validation_seconds
        durations[index] = elapsed
        emit(request,"frame",dict(payload,elapsed_seconds=elapsed))
    def flush_pipeline():
        nonlocal frame_pipeline
        if frame_pipeline is not None:
            drain_started = time.perf_counter()
            timings = frame_pipeline.finish()
            pipeline_stats['validation_drain_seconds'] = pipeline_stats.get('validation_drain_seconds',0.)+time.perf_counter()-drain_started
            record_pipeline(pipeline_stats,frame_pipeline,timings)
            frame_pipeline = None
            submitted_frames.clear()
    frame_analysis=[]
    frame_build_analysis=[]
    prepared_layer=None
    completed = []
    durations = []
    rendered_count = 0
    copied_count = 0
    verified_count = 0
    try:
        for index, (angle, output_path) in enumerate(zip(angles, paths)):
            frame_start = time.perf_counter()
            if sources[index] < index:
                flush_pipeline()
                if not should_reuse_frame(output_path, size, reuse, lambda path: (size, size) if decode_png_size(path) == (width, height) else (0, 0)):
                    copy_identical_frame(paths[sources[index]], output_path, width, height)
                    copied_count += 1
                else:
                    verified_count += 1
                frame_analysis.append(frame_analysis[sources[index]])
                frame_build_analysis.append(frame_build_analysis[sources[index]])
                elapsed = time.perf_counter() - frame_start
                durations.append(elapsed)
                completed.append(str(output_path))
                emit(request, "frame", {"path": str(output_path), "index": index, "angle_degrees": angle,
                                        "elapsed_seconds": elapsed, "size": size, "reused": True,
                                        "reuse_kind": "identical", "source_index": sources[index],
                                        "visible_layer": visible_layers[index], "section_analysis": frame_analysis[-1],
                                        "build_analysis":frame_build_analysis[-1]})
                continue
            if visible_layers[index] is not None:
                if source_mesh is not None:
                    if prepared_layer != visible_layers[index]:
                        build_analysis={} if spec["geometry_kind"] == "original" else None
                        info["section_analysis"]=prepare_buildup_section(obj,source_mesh,spec,info["bounds_m"],visible_layers[index],build_analysis)
                        info["build_analysis"]=build_analysis
                        prepared_layer=visible_layers[index]
                else:
                    set_visible_layer(obj,visible_layers[index])
            frame_analysis.append(info.get("section_analysis"))
            frame_build_analysis.append(info.get("build_analysis"))
            if states is not None and not states[index].get("preserve_scene_camera",False):
                set_camera_spec(bpy.context.scene.camera,states[index]["camera"])
                if spec.get("layer_appearance",False) and spec.get("layer_filtering",True):
                    for material in obj.data.materials:
                        apply_layer_filter(material,bpy.context.scene.camera,width,height,min(spec["layer_height_mm"],spec.get("first_layer_height_mm",spec["layer_height_mm"])))
            if index == 0 and batch_count:
                def report_batch(frame, elapsed):
                    emit(request, "frame", {"path":str(paths[frame]), "index":frame,
                         "angle_degrees":angles[frame], "elapsed_seconds":elapsed,"size":size,"reused":False,
                         "visible_layer":visible_layers[frame],"section_analysis":info.get("section_analysis"),
                         "build_analysis":info.get("build_analysis")})
                batch_times = render_animation(obj, angles[:batch_count], paths[:batch_count], width, height, report_batch, pipeline_stats=pipeline_stats)
            if index < len(batch_times):
                rendered_count += 1
                durations.append(batch_times[index])
                completed.append(str(output_path))
                continue
            if should_reuse_frame(output_path, size, reuse, lambda path: (size, size) if decode_png_size(path) == (width, height) else (0, 0)):
                flush_pipeline()
                verified_count += 1
                elapsed = time.perf_counter() - frame_start
                durations.append(elapsed)
                completed.append(str(output_path))
                emit(request, "frame", {"path": str(output_path), "index": index, "angle_degrees": angle,
                                        "elapsed_seconds": elapsed, "size": size, "reused": True,"visible_layer":visible_layers[index],"section_analysis":info.get("section_analysis"),"build_analysis":info.get("build_analysis")})
                continue
            obj.rotation_euler.z = math.radians(angle)
            temporary = output_path.with_name(output_path.stem + ".partial.png")
            queued = False
            try:
                bpy.context.scene.render.filepath = str(temporary)
                bpy.ops.render.render(write_still=True)
                rendered_count += 1
                elapsed = time.perf_counter() - frame_start
                durations.append(0.)
                completed.append(str(output_path))
                payload=dict(path=str(output_path),index=index,angle_degrees=angle,size=size,reused=False,
                             visible_layer=visible_layers[index],section_analysis=info.get("section_analysis"),
                             build_analysis=info.get("build_analysis"))
                if frame_pipeline is None:
                    frame_pipeline = FramePipeline(width,height,report_individual,workers=min(len(angles)-index,os.cpu_count() or 1,32))
                sequence = len(submitted_frames)
                submitted_frames.append((index,payload))
                frame_pipeline.submit(sequence,temporary,output_path,elapsed)
                queued = True
            finally:
                if not queued:
                    temporary.unlink(missing_ok=True)
        flush_pipeline()
    except BaseException:
        if frame_pipeline is not None:
            try:
                frame_pipeline.finish()
            except Exception:
                pass
        raise
    analysis_path=output_dir/"frame-analysis.json"
    analysis_temporary=analysis_path.with_suffix(".partial.json")
    analysis_temporary.write_text(json.dumps(frame_analysis,allow_nan=False),encoding="utf-8")
    os.replace(analysis_temporary,analysis_path)
    payload={"paths": completed, "engine": engine, "size": size, "samples": samples,
             "frame_count":len(angles),"frame_rate":frame_rate,"build_layer_count":len(info.get("build_layers") or []),
             "unique_frame_count": len(set(sources)), "rendered_frame_count": rendered_count,
             "copied_frame_count": copied_count, "verified_frame_count": verified_count,
             "render_strategy":"animation" if batch_times else "individual_frames",
             "frame_pipeline":pipeline_stats,
             "prepare_seconds": prepare_seconds, "frame_seconds": durations,
             "total_seconds": time.perf_counter() - started,"frame_analysis":frame_analysis,"frame_build_analysis":frame_build_analysis,
             "frame_analysis_path":str(analysis_path),**info}
    payload=with_analysis_warnings(payload)
    details_path=output_dir/"render-result.json"
    details_temporary=details_path.with_suffix(".partial.json")
    details_temporary.write_text(json.dumps(payload,allow_nan=False),encoding="utf-8")
    os.replace(details_temporary,details_path)
    emit(request, "complete", completion_event(payload,details_path))


def initialize_gpu_runtime():
    """Explicit preparation only: establish Blender's actual headless context.

    This disposable 16px scene never writes a file or touches source geometry.
    Its state is removed before build_scene configures the real render.
    """
    import bpy
    previous = bpy.context.window.scene
    probe = bpy.data.scenes.new("TurninatorRuntimeProbe")
    camera = bpy.data.cameras.new("TurninatorRuntimeProbeCamera")
    obj = bpy.data.objects.new("TurninatorRuntimeProbeCamera", camera)
    probe.collection.objects.link(obj)
    probe.camera = obj
    probe.render.engine = "BLENDER_EEVEE_NEXT"
    probe.render.resolution_x = probe.render.resolution_y = 16
    probe.render.resolution_percentage = 100
    probe.eevee.taa_render_samples = 1
    probe.render.film_transparent = True
    try:
        bpy.context.window.scene = probe
        bpy.ops.render.render(write_still=False)
    finally:
        bpy.context.window.scene = previous
        bpy.data.scenes.remove(probe)
        bpy.data.objects.remove(obj, do_unlink=True)
        bpy.data.cameras.remove(camera)


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
                initialized=time.perf_counter()
                if request.get("initialize_gpu", False):
                    initialize_gpu_runtime()
                initialization_seconds=time.perf_counter()-initialized
                info=probe_devices()
                info["initialization_seconds"]=initialization_seconds
                emit(request, "probe", info)
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
