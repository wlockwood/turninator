"""Bounded prepared-geometry wire contract; no Blender/GPU required."""
import importlib.util
from pathlib import Path
import struct
import sys
import json
import hashlib
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('prepared_scene', ROOT / 'renderer/scene.py')
scene = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scene)
sys.path.insert(0,str(ROOT/'renderer'))
import worker

def wire():
    vertices = [(0,0,0),(10,0,0),(0,10,0)]
    return (struct.pack('<8sIIII', b'TNPRP001',1,3,1,1)
            + b''.join(struct.pack('<9f',*p,0,0,1,*p) for p in vertices)
            + struct.pack('<4I',0,1,2,1) + struct.pack('<6f',0,0,0,10,0,0))

class PreparedGeometryTests(unittest.TestCase):
    def test_manifest_bounds_revisions_and_static_batch(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); geometry=root/'geometry.bin'; geometry.write_bytes(wire())
            manifest=dict(version=1,material_count=2,revisions=[dict(path=str(geometry),sha256=hashlib.sha256(wire()).hexdigest(),section=None,build=None)],frames=[0,0,0])
            path=root/'prepared.json'; path.write_text(json.dumps(manifest))
            self.assertEqual(worker.load_prepared_manifest(str(path),3),manifest)
            for change in [dict(version=2),dict(version=True),dict(frames=[0,1,0]),dict(frames=[0,0]),dict(material_count=259)]:
                path.write_text(json.dumps(dict(manifest,**change)))
                with self.assertRaises(ValueError): worker.load_prepared_manifest(str(path),3)
            self.assertEqual(worker.animation_prefix('turntable',None,False,False,[0,1,2]),3)

    def test_denoising_boolean_validation_and_legacy_default(self):
        self.assertTrue(worker.denoise_setting({}))
        self.assertFalse(worker.denoise_setting(dict(denoise=False)))
        for value in [0,1,None,'false']:
            with self.assertRaises(ValueError): worker.denoise_setting(dict(denoise=value))

    def read(self, data, materials=2):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'prepared.bin'
            path.write_bytes(data)
            return scene.read_prepared_geometry(path, materials)

    def test_preserves_all_aligned_attributes_and_edges(self):
        value = self.read(wire())
        self.assertEqual(value['vertices'], [(0.,0.,0.),(10.,0.,0.),(0.,10.,0.)])
        self.assertEqual(value['build_positions'], value['vertices'])
        self.assertEqual(value['normals'], [(0.,0.,1.)]*3)
        self.assertEqual(value['triangles'], [(0,1,2)])
        self.assertEqual(value['materials'], [1])
        self.assertEqual(value['edges'], [((0.,0.,0.),(10.,0.,0.))])

    def test_empty_stage_is_valid(self):
        value = self.read(struct.pack('<8sIIII',b'TNPRP001',1,0,0,0))
        self.assertEqual(value['vertices'], [])
        self.assertEqual(value['triangles'], [])

    def test_rejects_corruption_before_upload(self):
        corruptions = [wire()[:-1],wire()+b'x',wire().replace(b'TNPRP001',b'TNPRP002'),
                       struct.pack('<8sIIII',b'TNPRP001',1,2_000_001,0,0)]
        for offset, data in [(24,struct.pack('<f',float('nan'))),
                             (36,struct.pack('<3f',0,0,0)),
                             (132,struct.pack('<I',3)),
                             (144,struct.pack('<I',2))]:
            payload=bytearray(wire()); payload[offset:offset+len(data)]=data
            corruptions.append(payload)
        for payload in corruptions:
            with self.subTest(payload=payload[:24]), self.assertRaises(ValueError):
                self.read(payload)

if __name__ == '__main__':
    unittest.main()
