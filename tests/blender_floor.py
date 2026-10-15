"""Opt-in EEVEE image regression: blender -b --factory-startup --python
tests/blender_floor.py -- --output <fresh-directory> [--scene <scene.json>].

The default is a generated tall cylinder; --scene permits a retained user-scene
snapshot without changing its mesh. No Pillow or other Python packages required.
"""
import argparse
import json
import math
from pathlib import Path
import struct
import sys

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "renderer"))
from scene import build_scene, load_scene_spec


def stripe_score(path):
    image = bpy.data.images.load(str(path), check_existing=False)
    try:
        width, height = image.size
        pixels = list(image.pixels)
        # Empty upper-left background; average away stochastic sample noise.
        rows = []
        for y in range(16, 200):
            row = [(height - 1 - y) * width * 4 + x * 4 for x in range(16, 128)]
            rows.append(sum(sum(pixels[i:i+3]) for i in row) / (len(row) * 3))
        return sum(abs(rows[i-1] - 2*rows[i] + rows[i+1])
                   for i in range(1, len(rows)-1)) / (len(rows)-2)
    finally:
        bpy.data.images.remove(image)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scene", type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    if args.scene:
        spec = load_scene_spec(args.scene.resolve())
    else:
        vertices = [(85.7338*math.cos(i*math.tau/64), 85.7338*math.sin(i*math.tau/64), z)
                    for z in (0, 256) for i in range(64)]
        triangles = []
        for i in range(64):
            j = (i+1) % 64
            triangles.extend([(i, j, 64+j), (i, 64+j, 64+i)])
        mesh = output / "cylinder.meshbin"
        mesh.write_bytes(struct.pack("<8sIII", b"TNMESH01", 1, len(vertices), len(triangles))
                         + b"".join(struct.pack("<3f", *v) for v in vertices)
                         + b"".join(struct.pack("<3I", *t) for t in triangles))
        spec = dict(schema_version=1, mesh_path=str(mesh), geometry_kind="original",
                    display_rotation_degrees=[0, 0, 0], color=[0.035, 0.22, 0.52],
                    roughness=0.42, layer_appearance=False, layer_height_mm=0.2,
                    studio_preset="dark", warnings=[], fit_turntable=True,
                    camera=dict(direction=[0.86104128, -0.09277882, 0.5],
                                up=[-0.49712242, 0.05356588, 0.8660254],
                                target_mm=[0, 0, 128], distance_mm=674.20436,
                                ortho_span_mm=405.47432, projection="perspective",
                                vertical_fov_degrees=35))
    scores = {}
    for name in ("single-quad", "fixed"):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        obj, _ = build_scene(spec, "eevee", 768, 32, False)
        floor = bpy.data.objects["StudioFloor"]
        if name == "single-quad":
            xs = [v.co.x for v in floor.data.vertices]
            ys = [v.co.y for v in floor.data.vertices]
            mesh = bpy.data.meshes.new("LegacyFloor")
            mesh.from_pydata([(min(xs), min(ys), -0.00001), (max(xs), min(ys), -0.00001),
                              (max(xs), max(ys), -0.00001), (min(xs), max(ys), -0.00001)],
                             [], [(0, 1, 2, 3)])
            mesh.materials.append(floor.data.materials[0])
            floor.data = mesh
        else:
            assert len(floor.data.polygons) == 256
        path = output / (name + ".png")
        bpy.context.scene.render.filepath = str(path)
        bpy.ops.render.render(write_still=True)
        scores[name] = stripe_score(path)
        if name == "fixed":
            for angle in (90, 180):
                obj.rotation_euler.z = math.radians(angle)
                path = output / ("fixed-" + str(angle) + ".png")
                bpy.context.scene.render.filepath = str(path)
                bpy.ops.render.render(write_still=True)
                scores["fixed-" + str(angle)] = stripe_score(path)
    result = dict(blender=bpy.app.version_string, scores=scores,
                  reduction=scores["single-quad"] / max(scores["fixed"], 1e-12))
    (output / "result.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    assert result["reduction"] > 10, result
    assert all(score < 0.002 for name, score in scores.items() if name.startswith("fixed")), result
    for transparent, studio in ((True, "dark"), (False, "diagram")):
        build_scene(dict(spec, studio_preset=studio), "eevee", 768, 32, transparent)
        assert "StudioFloor" not in bpy.data.objects
    print("EEVEE floor regression passed:", result)


if __name__ == "__main__":
    main()
