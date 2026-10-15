"""Geometry-only STL build-up regressions; run inside background Blender."""
import argparse
import json
from pathlib import Path
import struct
import sys

sys.path.insert(0,str(Path(__file__).resolve().parent))
from blender_solid_buildup import tube
from scene import build_scene, prepare_buildup_section
from worker import with_analysis_warnings


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    output=args.output.resolve()
    output.mkdir(parents=True,exist_ok=False)
    vertices, triangles=tube()
    cases={
        'open':(vertices,[face for i,face in enumerate(triangles) if i%8 not in (4,5)],10),
        'future-component':(vertices+[(x+35,y,z+5) for x,y,z in vertices],
                            triangles+[tuple(i+len(vertices) for i in face) for face in triangles],15),
    }
    proof={}
    for name,(points,faces,top) in cases.items():
        mesh=output/(name+'.meshbin')
        mesh.write_bytes(struct.pack('<8sIII',b'TNMESH01',1,len(points),len(faces))
                         +b''.join(struct.pack('<3f',*p) for p in points)
                         +b''.join(struct.pack('<3I',*p) for p in faces))
        spec=dict(schema_version=1,mesh_path=str(mesh),geometry_kind='original',
                  display_rotation_degrees=[0,0,0],color=[.1,.2,.5],roughness=.4,
                  layer_height_mm=.2,warnings=[],build_up=True,solid_layer_tops_mm=[5,top])
        obj, info=build_scene(spec,'eevee',16,1,True)
        source=obj.data.copy()
        analysis={}
        prepare_buildup_section(obj,source,spec,info['bounds_m'],0,analysis)
        if name=='open':
            assert not analysis['capped'] and not analysis['closed_input'],analysis
            assert analysis['remaining_volume_mm3'] is None,analysis
            result=with_analysis_warnings(dict(warnings=[],frame_build_analysis=[analysis,analysis]))
            assert len(result['warnings'])==1,result
        else:
            assert analysis['capped'],analysis
            assert abs(analysis['remaining_volume_mm3']-1500)<.02,analysis
            # Restore BUILD X offset to detect any coplanar future bottom faces.
            assert all(v.co.x+info['bounds_m']['center'][0]<.02 for v in obj.data.vertices),[
                tuple(v.co) for v in obj.data.vertices]
        proof[name]=analysis
    (output/'proof.json').write_text(json.dumps(proof,indent=2))
    print('SOLID_EDGE_CASE_PROOF '+json.dumps(proof))


if __name__=='__main__':
    main()
