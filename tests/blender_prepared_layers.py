"""Visible first/later layer spacing, display-space attachment and filtering."""
import json,math,sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Euler,Vector
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'renderer'))
from scene import build_scene,read_mesh,read_prepared_geometry,rotation_envelope
sys.path.insert(0,str(Path(__file__).resolve().parent))
from blender_prepared_qualification import rgba,compare_masks

root=Path(sys.argv[sys.argv.index('--')+1]).resolve()
base=root/'solid-full'
spec=json.loads((base/'scene.json').read_text())
prepared=read_prepared_geometry(base/'prepared.bin',2)
vertices,_=read_mesh(spec['mesh_path'])
results=[]
for name,pose,span in [('closeup',[0,0,0],2.4),('posed',[53,19,37],2.4),('fade',[0,0,0],60.)]:
    directory=root/('layers-'+name);directory.mkdir(exist_ok=True)
    rotation=Euler(tuple(math.radians(v) for v in pose),'XYZ').to_matrix()
    envelope=rotation_envelope([tuple(rotation@Vector(p)) for p in vertices])
    offset=Vector((*envelope['center'][:2],envelope['bottom']))
    target=rotation@Vector((5,0,1.2))-offset
    camera=dict(target_mm=list(target),direction=list(rotation@Vector((0,-1,0))),up=list(rotation@Vector((0,0,1))),distance_mm=100.,projection='orthographic',vertical_fov_degrees=35.,ortho_span_mm=span)
    chosen=dict(spec,display_rotation_degrees=pose,camera=camera,layer_appearance=True,layer_height_mm=.2,first_layer_height_mm=.3,groove_depth_mm=.04)
    images={}
    for mode in ('legacy','prepared','smooth'):
        current=dict(chosen,layer_appearance=mode!='smooth')
        obj,info=build_scene(current,'eevee',512,64,True,prepared=None if mode=='legacy' else prepared)
        bpy.context.scene.render.filepath=str(directory/(mode+'.png'));bpy.ops.render.render(write_still=True)
        images[mode]=rgba(directory/(mode+'.png'))
    result=dict(name=name,mask=compare_masks(images['legacy'],images['prepared']),
                paired_rgb_mae=float(np.abs(images['legacy'][:,:,:3]-images['prepared'][:,:,:3]).mean()),
                layer_smooth_mae=float(np.abs(images['prepared'][:,:,:3]-images['smooth'][:,:,:3]).mean()))
    # A constant front face at orthographic magnification gives an isolated 1D
    # signal: each physical layer must create one visible crest, including the
    # distinct .3mm first layer. Compare peaks against same-engine reference.
    if name!='fade':
        row=(images['prepared'][:,:,:3]-images['smooth'][:,:,:3]).mean(axis=2)[:,64:448].mean(axis=1)
        row=np.convolve(row,np.ones(5)/5,mode='same')
        peaks=[i for i in range(2,510) if row[i]>row[i-1] and row[i]>=row[i+1] and row[i]>.002]
        result['crest_pixels']=peaks
        spacings=np.diff(peaks)
        result['crest_spacing_pixels']=spacings.tolist()
        expected=.2*512/span
        assert len(peaks)>=9 and all(abs(float(s)-expected)<=1 for s in spacings[:-1]),result
        # Lowest face band is first .3mm, a different crest spacing at its join.
        assert abs((512-peaks[-1])-.3*.75*512/span)<=1,result
        result['first_crest_expected_pixel']=512-.3*.75*512/span
        assert result['layer_smooth_mae']>.005,result
    else:
        assert result['layer_smooth_mae']<=1/255,result
    results.append(result)
    (root/'layers-qualification.json').write_text(json.dumps(results,indent=2))
    print('PREPARED_LAYERS '+json.dumps(result),flush=True)
