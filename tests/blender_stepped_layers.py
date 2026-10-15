"""Opt-in Blender proof of closed stepped layers, holes, stages and sections.
Run: blender -b --factory-startup --python tests/blender_stepped_layers.py -- --output <new-dir>
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

sys.path.insert(0, str(Path(__file__).resolve().parent))
from blender_solid_buildup import tube
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'renderer'))
from scene import build_scene, set_visible_layer, prepare_buildup_section


def stats(obj, pose, envelope, top):
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = evaluated.to_mesh()
    try:
        bm = bmesh.new()
        bm.from_mesh(mesh)
        closed = bool(bm.faces) and all(e.is_manifold and e.is_contiguous for e in bm.edges)
        volume = abs(bm.calc_volume(signed=True))*1e9
        bm.free()
        rotation = Euler(tuple(math.radians(v) for v in pose), 'XYZ').to_matrix()
        offset = Vector((*envelope['center'][:2], envelope['bottom']))
        points = [rotation.inverted() @ (v.co + offset)*1000 for v in mesh.vertices]
        mesh.calc_loop_triangles()
        cap_area = 0.0
        for tri in mesh.loop_triangles:
            a,b,c = [points[i] for i in tri.vertices]
            if all(abs(p.z-top)<0.001 for p in (a,b,c)):
                area = (b-a).cross(c-a).z*0.5
                if area > 0:
                    cap_area += area
                    crosses = [(q.x-p.x)*(-p.y)-(q.y-p.y)*(-p.x) for p,q in ((a,b),(b,c),(c,a))]
                    assert not all(v >= 0 for v in crosses), 'filled the modeled bore'
        return dict(closed=closed, volume_mm3=volume, cap_area_mm2=cap_area,
                    triangles=len(mesh.loop_triangles), max_build_z_mm=max((p.z for p in points),default=0))
    finally:
        evaluated.to_mesh_clear()


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    output=args.output.resolve()
    output.mkdir(parents=True,exist_ok=False)
    points,faces=tube()
    vertices=[]
    triangles=[]
    ranges=[]
    for layer,(bottom,top) in enumerate(((0,2.5),(2.5,5),(5,10))):
        offset=len(vertices)
        vertices.extend((x,y,bottom if z==0 else top) for x,y,z in points)
        start=len(triangles)
        triangles.extend(tuple(i+offset for i in t) for t in faces)
        ranges.append(dict(start_triangle=start,end_triangle=len(triangles),layer_index=layer,z_top_mm=top))
    mesh=output/'layers.meshbin'
    mesh.write_bytes(struct.pack('<8sIII',b'TNMESH01',1,len(vertices),len(triangles))
                     +b''.join(struct.pack('<3f',*p) for p in vertices)
                     +b''.join(struct.pack('<3I',*t) for t in triangles))
    results={}
    for pose in ([0,0,0],[53,19,37]):
        spec=dict(schema_version=1,mesh_path=str(mesh),geometry_kind='stepped',
                  display_rotation_degrees=pose,color=[0.035,0.22,0.52],roughness=0.42,
                  layer_height_mm=0.2,warnings=[],build_up=True,layer_face_ranges=ranges)
        obj,info=build_scene(spec,'eevee',256,8,True)
        for layer,top in ((-1,0),(0,2.5),(1,5),(2,10),(1,5)):
            set_visible_layer(obj,layer)
            bpy.context.view_layer.update()
            state=stats(obj,pose,info['bounds_m'],top)
            if layer==-1:
                assert state['triangles']==0,state
            else:
                assert state['closed'],state
                assert abs(state['volume_mm3']-300*top)<0.03,state
                assert abs(state['cap_area_mm2']-300)<0.03,state
                assert abs(state['max_build_z_mm']-top)<0.001,state
            results[f'{pose}-{layer}']=state
        bpy.context.scene.render.filepath=str(output/('rotated-partial.png' if pose[0] else 'partial.png'))
        bpy.ops.render.render(write_still=True)
        spec['section']=dict(axis=0,position_mm=0,keep_positive=True,cap=True)
        obj,info=build_scene(spec,'eevee',256,8,True)
        source=obj.data.copy()
        analysis=prepare_buildup_section(obj,source,spec,info['bounds_m'],1)
        state=stats(obj,pose,info['bounds_m'],5)
        assert state['closed'] and abs(state['volume_mm3']-750)<0.03,state
        assert analysis['capped'] and abs(analysis['remaining_volume_mm3']-750)<0.03,analysis
        results[f'{pose}-section']=dict(state=state,analysis=analysis)
    (output/'proof.json').write_text(json.dumps(results,indent=2),encoding='utf-8')
    print('STEPPED_LAYER_PROOF '+json.dumps(results))


if __name__=='__main__':
    main()
