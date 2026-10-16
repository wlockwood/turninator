"""Actual same-engine legacy/prepared geometry qualification, frozen invariants.

Run Blender --background --factory-startup --python this.py -- <fixture-directory>.
Inputs come from render_prepared::generate_blender_prepared_qualification.
"""
import json
import math
from pathlib import Path
import sys
import time
import traceback
import struct
import bpy
import numpy as np

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'renderer'))
from scene import build_scene,read_prepared_geometry,prepare_buildup_section

def camera_spec(camera):
    delta=[p-t for p,t in zip(camera['position_mm'],camera['target_mm'])]
    distance=math.sqrt(sum(v*v for v in delta))
    projection=camera['projection']
    return dict(target_mm=camera['target_mm'],direction=[v/distance for v in delta],up=camera['up'],distance_mm=distance,
                projection='orthographic' if projection['kind']=='orthographic' else 'perspective',
                vertical_fov_degrees=projection.get('vertical_fov_degrees',35),ortho_span_mm=projection.get('vertical_span_mm',1))

def rgba(path):
    image=bpy.data.images.load(str(path),check_existing=False)
    try:
        data=np.empty(len(image.pixels),dtype=np.float32);image.pixels.foreach_get(data)
        return data.reshape(image.size[1],image.size[0],4)[::-1].copy()
    finally:
        bpy.data.images.remove(image)

def within(a,b):
    return abs(a-b)<=max(1e-4,abs(b)*1e-5)

def bounds(obj):
    if not obj.data.vertices:return None
    points=np.array([tuple(v.co) for v in obj.data.vertices])*1000
    return [points.min(axis=0).tolist(),points.max(axis=0).tolist()]

def compare_analysis(expected,actual):
    if expected is None or actual is None:
        assert expected==actual,(expected,actual)
        return
    for key in ('axis','capped','closed_input','visible_layer'):
        if key in expected:assert expected[key]==actual.get(key),(key,expected,actual)
    for key in ('position_mm','cross_section_area_mm2','original_volume_mm3','remaining_volume_mm3','z_top_mm'):
        if key not in expected:continue
        a,b=expected[key],actual.get(key)
        assert (a is None and b is None) or (a is not None and b is not None and within(a,b)),(key,a,b)

def compare_masks(a,b):
    a,b=a[:,:,3]>=0.5,b[:,:,3]>=0.5
    union=np.logical_or(a,b).sum(); intersection=np.logical_and(a,b).sum()
    iou=float(intersection/union) if union else 1.
    # Every differing pixel must be within one Chebyshev pixel of the other mask.
    def dilate(mask):
        padded=np.pad(mask,1)
        return np.logical_or.reduce([padded[y:y+mask.shape[0],x:x+mask.shape[1]] for y in range(3) for x in range(3)])
    outside=int(np.logical_and(a,~dilate(b)).sum()+np.logical_and(b,~dilate(a)).sum())
    assert iou>=0.995 and outside==0,dict(iou=iou,outside=outside)
    return dict(iou=iou,outside_one_pixel=outside,legacy_pixels=int(a.sum()),prepared_pixels=int(b.sum()))

def main():
    root=Path(sys.argv[sys.argv.index('--')+1]).resolve()
    cases=json.loads((root/'cases.json').read_text())
    results=[];started=time.perf_counter()
    for case in cases:
        result=dict(name=case['name']);results.append(result)
        try:
            directory=Path(case['directory']);frame=case['frame']
            original=json.loads((directory/'scene.json').read_text())
            legacy=dict(original); legacy['build_up']=frame['visible_layers'] is not None
            obj,info=build_scene(legacy,'eevee',512,32,True,frame['width'],frame['height'])
            if frame['visible_layers'] is not None:
                source=obj.data.copy();build={}
                info['section_analysis']=prepare_buildup_section(obj,source,legacy,info['bounds_m'],frame['visible_layers']-1,build)
                info['build_analysis']=build
                bpy.data.meshes.remove(source)
            result['legacy_bounds_mm']=bounds(obj)
            if case['name']=='sheet':
                edge=bpy.data.objects['DesignEdges'];result['legacy_edges']=dict(count=len(edge.data.splines),radius_m=edge.data.bevel_depth)
                assert len(edge.data.splines)==12
                assert within(edge.data.bevel_depth*1000,16/512*.4)
            result['legacy_section']=info.get('section_analysis');result['legacy_build']=info.get('build_analysis')
            bpy.context.scene.render.filepath=str(directory/'legacy.png');bpy.ops.render.render(write_still=True)
            prepared=read_prepared_geometry(directory/'prepared.bin',case['material_count'])
            new=dict(original,camera=camera_spec(frame['camera']))
            obj,info=build_scene(new,'eevee',512,32,True,frame['width'],frame['height'],prepared=prepared)
            assert len(obj.data.materials)==case['material_count']
            if case['name']=='review-rounded-materials':
                result['material_slots']=[]
                for index,expected in enumerate(case['canonical_materials'][1:],1):
                    material=obj.data.materials[index]
                    assert material.name==expected['name'],(index,material.name,expected['name'])
                    actual=tuple(material.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value[:3])
                    color=struct.unpack('<3f',struct.pack('<3f',*expected['base_color_linear']))
                    assert actual==color,(index,actual,color)
                    result['material_slots'].append(dict(index=index,name=material.name,color=actual))
                assert set(p.material_index for p in obj.data.polygons)=={0,1,2,3}
            result['prepared_bounds_mm']=bounds(obj)
            if case['name']=='sheet':
                edge=bpy.data.objects['DesignEdges'];result['prepared_edges']=dict(count=len(edge.data.splines),radius_m=edge.data.bevel_depth)
                assert result['legacy_edges']==result['prepared_edges']
            result['prepared_section']=case['section'];result['prepared_build']=case['build']
            bpy.context.scene.render.filepath=str(directory/'prepared.png');bpy.ops.render.render(write_still=True)
            a,b=result['legacy_bounds_mm'],result['prepared_bounds_mm']
            assert (a is None and b is None) or all(within(x,y) for pa,pb in zip(a,b) for x,y in zip(pa,pb)),(a,b)
            compare_analysis(case['section'],result['legacy_section']);compare_analysis(case['build'],result['legacy_build'])
            a,b=rgba(directory/'legacy.png'),rgba(directory/'prepared.png')
            result['mask']=compare_masks(a,b)
            if case['name']=='seam-closeup':
                assert np.all(b[:,:,3]==1), 'transparent crack across nonbinary f32 T-junction'
                result['seam_opaque_pixels']=int(np.count_nonzero(b[:,:,3]==1))
            result['rgb_mean_absolute_difference']=float(np.abs(a[:,:,:3]-b[:,:,:3]).mean())
            if case['name'] in ('cube-mask','hollow-mask'):
                expected=rgba(ROOT/'tests/fixtures/renderers'/f"{case['name']}-512.png")
                # Frozen files are opaque grayscale masks, independent of alpha.
                expected[:,:,3]=expected[:,:,0]
                result['frozen_mask']=compare_masks(expected,b)
            result['passed']=True
        except Exception as error:
            result['passed']=False;result['error']=str(error);result['traceback']=traceback.format_exc()
        (root/'qualification.json').write_text(json.dumps(dict(seconds=time.perf_counter()-started,results=results),indent=2))
        print('PREPARED_QUALIFICATION '+json.dumps(result),flush=True)
    if not all(r['passed'] for r in results):raise RuntimeError('prepared qualification failed; see qualification.json')

if __name__=='__main__':main()
