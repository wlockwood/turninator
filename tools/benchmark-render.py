"""Run inside Blender to compare matched still calls with native animation.

blender -b --factory-startup --python tools/benchmark-render.py --
  --scene <scene.json> --output <new-directory> --frames 24
The scene/mesh inputs are read only. Every case retains and validates its PNGs.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys
import time

import bpy

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'renderer'))
from scene import build_scene, load_scene_spec
from worker import decode_png_size, render_animation


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--scene', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--frames', type=int, default=24)
    parser.add_argument('--size', type=int, default=960)
    parser.add_argument('--samples', type=int, default=32)
    parser.add_argument('--engine', choices=('eevee', 'cycles'), default='eevee')
    parser.add_argument('--cases', nargs='+', choices=('still-loop','native-animation','worker-animation'),
                        default=['still-loop','native-animation','worker-animation'])
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    if not 2 <= args.frames <= 1000:
        raise ValueError('frame count outside 2..1000')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    spec = load_scene_spec(args.scene.resolve())
    spec = dict(spec, fit_turntable=True)
    results = dict(blender=bpy.app.version_string, engine=args.engine, size=args.size,
                   samples=args.samples, frames=args.frames, cases=[])
    results['source_sha256'] = {str(path):hashlib.sha256(path.read_bytes()).hexdigest()
        for path in [args.scene.resolve(),Path(spec['mesh_path']),Path(__file__).resolve(),
                     ROOT/'renderer/worker.py',ROOT/'renderer/scene.py',ROOT/'renderer/frame_pipeline.py']}
    for case_index, name in enumerate(args.cases):
        directory = output / f'{case_index}-{name}'
        directory.mkdir()
        started = time.perf_counter()
        obj, info = build_scene(copy.deepcopy(spec), args.engine, args.size, args.samples, False)
        prepare = time.perf_counter() - started
        started = time.perf_counter()
        bpy.ops.render.render(write_still=False)
        warmup = time.perf_counter() - started
        durations = []
        decode = 0.0
        pipeline_stats = {}
        paths = [directory / f'frame_{i:04d}.png' for i in range(args.frames)]
        start = time.perf_counter()
        cpu = time.process_time()
        if name == 'still-loop':
            for index, path in enumerate(paths):
                frame_start = time.perf_counter()
                obj.rotation_euler.z = 2 * math.pi * index / args.frames
                bpy.context.scene.render.filepath = str(path)
                bpy.ops.render.render(write_still=True)
                check = time.perf_counter()
                assert decode_png_size(path) == (args.size, args.size)
                decode += time.perf_counter() - check
                durations.append(time.perf_counter() - frame_start)
        elif name == 'worker-animation':
            durations=render_animation(obj,[360*i/args.frames for i in range(args.frames)],paths,
                                       args.size,args.size,lambda *_:None,pipeline_stats=pipeline_stats)
            decode = pipeline_stats['validation_work_seconds']
        else:
            scene = bpy.context.scene
            scene.frame_start = 0
            scene.frame_end = args.frames - 1
            scene.render.filepath = str(directory / 'frame_')
            last = [time.perf_counter()]
            def before_frame(scene, *_):
                obj.rotation_euler.z = 2 * math.pi * scene.frame_current / args.frames
            def after_write(scene, *_):
                now = time.perf_counter()
                durations.append(now - last[0])
                last[0] = now
            bpy.app.handlers.frame_change_pre.append(before_frame)
            bpy.app.handlers.render_write.append(after_write)
            try:
                bpy.ops.render.render(animation=True)
            finally:
                bpy.app.handlers.frame_change_pre.remove(before_frame)
                bpy.app.handlers.render_write.remove(after_write)
            check = time.perf_counter()
            for path in paths:
                assert decode_png_size(path) == (args.size, args.size)
            decode = time.perf_counter() - check
        elapsed = time.perf_counter() - start
        result = dict(name=name, prepare_seconds=prepare, warmup_seconds=warmup,
                      total_seconds=elapsed, cpu_seconds=time.process_time()-cpu,
                      png_decode_seconds=decode, median_frame_seconds=statistics.median(durations),
                      frame_seconds=durations, triangles=info['triangles'])
        if pipeline_stats:
            result['frame_pipeline'] = pipeline_stats
        results['cases'].append(result)
        (output / 'results.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
        print('RENDER_BENCHMARK ' + json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
