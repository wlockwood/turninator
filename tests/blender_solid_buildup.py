"""Opt-in real geometry/render proof: blender -b --factory-startup --python
tests/blender_solid_buildup.py -- --output <fresh-directory>.

Generated closed square tube checks solid caps, real holes, source restoration,
display rotation, empty/full stages, and optional physical section composition.
"""
import argparse
import json
import math
from pathlib import Path
import struct
import sys

import bpy
import bmesh
from mathutils import Euler, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'renderer'))
from scene import build_scene, prepare_buildup_section


def tube():
    vertices = [(x, y, z) for z in (0, 10)
                for ring in ((-10, 10), (-5, 5))
                for x, y in ((ring[0], ring[0]), (ring[1], ring[0]),
                             (ring[1], ring[1]), (ring[0], ring[1]))]
    quads = []
    for i in range(4):
        j = (i+1) % 4
        quads.extend(((i, j, 8+j, 8+i), (4+j, 4+i, 12+i, 12+j),
                      (8+i, 8+j, 12+j, 12+i), (j, i, 4+i, 4+j)))
    triangles = [t for a,b,c,d in quads for t in ((a,b,c),(a,c,d))]
    return vertices, triangles


def stats(obj, rotation, envelope):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    volume = abs(bm.calc_volume(signed=True))*1e9
    closed = bool(bm.faces) and all(e.is_manifold for e in bm.edges)
    bm.free()
    offset = Vector((*envelope['center'][:2], envelope['bottom']))
    points = [rotation.inverted() @ (v.co + offset) * 1000 for v in obj.data.vertices]
    obj.data.calc_loop_triangles()
    cap_area = 0
    for tri in obj.data.loop_triangles:
        p = [points[i] for i in tri.vertices]
        if all(abs(v.z-5) < 1e-3 for v in p):
            area = (p[1]-p[0]).cross(p[2]-p[0]).length / 2
            cap_area += area
            if area < 1e-5:
                continue
            # A cap triangle crossing the real square bore would contain its center.
            cross = [(p[(i+1)%3].x-p[i].x)*(-p[i].y)
                     -(p[(i+1)%3].y-p[i].y)*(-p[i].x) for i in range(3)]
            assert not (all(c >= -1e-6 for c in cross) or all(c <= 1e-6 for c in cross)), p
    return dict(volume_mm3=volume, closed=closed, cap_area_mm2=cap_area,
                max_build_z_mm=max((p.z for p in points),default=None), triangles=len(obj.data.loop_triangles))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    vertices, triangles = tube()
    mesh = output/'tube.meshbin'
    mesh.write_bytes(struct.pack('<8sIII', b'TNMESH01', 1, len(vertices), len(triangles))
                     + b''.join(struct.pack('<3f', *p) for p in vertices)
                     + b''.join(struct.pack('<3I', *p) for p in triangles))
    spec = dict(schema_version=1, mesh_path=str(mesh), geometry_kind='original',
                display_rotation_degrees=[0,0,0], color=[0.035,0.22,0.52], roughness=0.42,
                layer_height_mm=0.2, warnings=[], build_up=True, solid_layer_tops_mm=[2.5,5,10])
    results = {}
    for pose in ([0,0,0], [53,19,37]):
        chosen = dict(spec, display_rotation_degrees=pose)
        obj, info = build_scene(chosen, 'eevee', 256, 8, True)
        source = obj.data.copy()
        rotation = Euler(tuple(math.radians(v) for v in pose),'XYZ').to_matrix()
        camera_location = tuple(bpy.context.scene.camera.location)
        assert info['build_layers'] == [0,1,2]
        for layer, name in ((-1,'empty'),(1,'partial'),(2,'full'),(1,'partial-again')):
            build_analysis = {}
            analysis = prepare_buildup_section(obj, source, chosen, info['bounds_m'], layer, build_analysis)
            assert analysis is None
            assert build_analysis['visible_layer'] == layer
            assert build_analysis['z_top_mm'] == (0 if layer == -1 else chosen['solid_layer_tops_mm'][layer])
            assert build_analysis['capped'] == (layer == 1)
            state = stats(obj, rotation, info['bounds_m'])
            assert tuple(bpy.context.scene.camera.location) == camera_location
            if layer == -1:
                assert state['triangles'] == 0, state
            elif layer == 1:
                assert state['closed'], state
                assert abs(state['volume_mm3']-1500) < .02, state
                assert abs(state['cap_area_mm2']-300) < .02, state
                assert abs(state['max_build_z_mm']-5) < .001, state
                assert all(m.name.startswith('MattePLA') for m in obj.data.materials), list(obj.data.materials)
            else:
                assert state['triangles'] == len(triangles), state
                assert abs(state['volume_mm3']-3000) < .02, state
            key = ('rotated-' if pose[0] else '')+name
            results[key] = state
            if name != 'partial-again':
                bpy.context.scene.render.filepath = str(output/(key+'.png'))
                bpy.ops.render.render(write_still=True)
        chosen['section'] = dict(axis=0,position_mm=0,keep_positive=True,cap=True)
        analysis = prepare_buildup_section(obj,source,chosen,info['bounds_m'],1)
        state = stats(obj,rotation,info['bounds_m'])
        assert state['closed'] and abs(state['volume_mm3']-750) < .02, state
        assert analysis['capped'] and abs(analysis['remaining_volume_mm3']-750)<.02, analysis
        assert any(m.name.startswith('SectionCutSurface') for m in obj.data.materials)
        results[('rotated-' if pose[0] else '')+'paired-section'] = state
    (output/'proof.json').write_text(json.dumps(results,indent=2))
    print('SOLID_BUILDUP_PROOF '+json.dumps(results))


if __name__ == '__main__':
    main()
