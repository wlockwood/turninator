"""Probe/state protocol tests need no Blender installation or GPU."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
def load(name):
    spec = importlib.util.spec_from_file_location('adapter_' + name, ROOT / 'renderer' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
scene = load('scene')
worker = load('worker')

class BlenderAdapterProtocolTests(unittest.TestCase):
    def test_uninitialized_headless_probe_reports_limitation_without_rendering(self):
        def unavailable():
            raise SystemError('GPU context unavailable')
        gpu = SimpleNamespace(platform=SimpleNamespace(backend_type_get=unavailable))
        current = lambda: None
        ctypes = SimpleNamespace(WinDLL=lambda _:SimpleNamespace(wglGetCurrentContext=current), c_void_p=object)
        with mock.patch.dict(sys.modules, {'gpu':gpu,'ctypes':ctypes}):
            self.assertIn('error', scene.probe_gpu_runtime())

    def test_probe_reports_actual_gpu_runtime_without_rendering(self):
        prefs = SimpleNamespace(devices=[], get_devices=lambda: None)
        bpy = SimpleNamespace(context=SimpleNamespace(preferences=SimpleNamespace(addons={'cycles':SimpleNamespace(preferences=prefs)})),
                              app=SimpleNamespace(version_string='4.5 test',build_hash=b'abc'))
        gpu = SimpleNamespace(platform=SimpleNamespace(backend_type_get=lambda:'OPENGL',vendor_get=lambda:'NVIDIA',
                    renderer_get=lambda:'test GPU',version_get=lambda:'driver 1'))
        with mock.patch.dict(sys.modules, {'bpy':bpy,'gpu':gpu}):
            info = scene.probe_devices()
        self.assertEqual(info['gpu'], dict(backend='OPENGL',vendor='NVIDIA',renderer='test GPU',version='driver 1'))

    def test_frame_states_accept_exact_whole_layer_counts_including_empty(self):
        camera = dict(target_mm=[0,0,0],direction=[1,-1,1],up=[0,0,1],distance_mm=100,
                      projection='perspective',vertical_fov_degrees=35,ortho_span_mm=50)
        states = [dict(camera=camera,angle_degrees=0,buildup_fraction=None,visible_layers=n) for n in (0,1,4)]
        self.assertEqual(worker.validate_frame_states(states,3), states)
        for counts in ((1,0,4),(0,None,4),(0,True,4),(0,-1,4),(0,50001,50002)):
            invalid = [dict(state,visible_layers=n) for state,n in zip(states,counts)]
            with self.assertRaises(ValueError):
                worker.validate_frame_states(invalid,3)

    def test_angle_and_layer_states_can_preserve_legacy_auto_fit(self):
        state = dict(camera=None,angle_degrees=15,buildup_fraction=None,preserve_scene_camera=True)
        self.assertEqual(worker.validate_frame_states([state],1), [state])
        with self.assertRaises(ValueError):
            worker.validate_frame_states([dict(state,preserve_scene_camera=False)],1)

    def test_exact_layer_schedule_renders_zero_as_empty(self):
        states = [dict(visible_layers=n) for n in (0,1,3)]
        self.assertEqual(worker.visible_layer_schedule([2,5,8],None,states,True,3), [-1,2,8])

if __name__ == '__main__':
    unittest.main()
