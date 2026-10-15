"""Run with: python -m unittest discover -s tests -p test_renderer.py -v"""
import importlib.util
import io
import json
import math
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

SPEC = importlib.util.spec_from_file_location("turninator_scene", Path(__file__).resolve().parents[1] / "renderer" / "scene.py")
scene = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(scene)
WORKER_SPEC = importlib.util.spec_from_file_location("turninator_worker", Path(__file__).resolve().parents[1] / "renderer" / "worker.py")
worker = importlib.util.module_from_spec(WORKER_SPEC)
WORKER_SPEC.loader.exec_module(worker)


def binary(vertices=None, triangles=None):
    vertices = vertices or [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    triangles = triangles or [(0, 1, 2)]
    return (struct.pack("<8sIII", b"TNMESH01", 1, len(vertices), len(triangles))
            + b"".join(struct.pack("<3f", *p) for p in vertices)
            + b"".join(struct.pack("<3I", *p) for p in triangles))


class MeshTests(unittest.TestCase):
    def test_event_line_is_written_in_one_call(self):
        output = mock.Mock()
        with mock.patch.object(worker.sys, 'stdout', output):
            worker.emit({'request_id':'one'}, 'frame', {'index':0})
        output.write.assert_called_once()
        raw = output.write.call_args.args[0]
        self.assertTrue(raw.startswith(worker.PREFIX))
        self.assertTrue(raw.endswith('\n'))
        self.assertEqual(json.loads(raw[len(worker.PREFIX):])['payload']['index'],0)
        output.flush.assert_called_once()

    def test_animation_prefix_requires_a_fresh_static_turntable(self):
        self.assertEqual(worker.animation_prefix('turntable', None, False, False, [0,1,2,0,1,2]),3)
        self.assertEqual(worker.animation_prefix('turntable', None, False, False, [0,1,2]),3)
        for mode,states,progressive,reuse,sources in (
            ('still',None,False,False,[0]), ('buildup',None,True,False,[0,1]),
            ('turntable',[{}],False,False,[0,1]), ('turntable',None,False,True,[0,1]),
            ('turntable',None,False,False,[0,0,2]),
        ):
            self.assertEqual(worker.animation_prefix(mode,states,progressive,reuse,sources),0)

    def test_animation_batch_publishes_frames_and_removes_handlers_on_failure(self):
        for fail in (None,'decode','rotation'):
            with self.subTest(fail=fail), tempfile.TemporaryDirectory() as directory:
                scene_mock=SimpleNamespace(frame_start=1,frame_end=250,frame_current=0,
                                           render=SimpleNamespace(filepath='old'))
                handlers=SimpleNamespace(frame_change_pre=[],render_write=[])
                class Rotation:
                    angle=0
                    @property
                    def z(self): return self.angle
                    @z.setter
                    def z(self,value):
                        if fail=='rotation' and value>1: raise ValueError('rotation update')
                        self.angle=value
                obj=SimpleNamespace(rotation_euler=Rotation())
                written=[]
                def render(**kwargs):
                    self.assertEqual(kwargs,dict(animation=True))
                    for i in range(scene_mock.frame_start,scene_mock.frame_end+1):
                        scene_mock.frame_current=i
                        for handler in list(handlers.frame_change_pre):
                            try: handler(scene_mock)
                            except Exception: pass  # Blender reports and swallows handler errors.
                        if fail is None: self.assertAlmostEqual(obj.rotation_euler.z,math.radians(i*90))
                        Path(scene_mock.render.filepath+f'{i:04d}.png').write_bytes(b'png')
                        for handler in list(handlers.render_write): handler(scene_mock)
                    return {'FINISHED'}
                fake=SimpleNamespace(context=SimpleNamespace(scene=scene_mock),app=SimpleNamespace(handlers=handlers),
                                     ops=SimpleNamespace(render=SimpleNamespace(render=render)))
                def checked(path):
                    if fail=='decode': raise ValueError('corrupt PNG')
                    self.assertEqual(Path(path).read_bytes(),b'png')
                    return (16,16)
                paths=[Path(directory)/f'frame_{i:04d}.png' for i in range(3)]
                with mock.patch.dict(sys.modules,{'bpy':fake}):
                    if fail:
                        with self.assertRaisesRegex(ValueError,'rotation update' if fail=='rotation' else 'corrupt PNG'):
                            worker.render_animation(obj,[0,90,180],paths,16,16,lambda i,elapsed:written.append(i),checked)
                    else:
                        elapsed=worker.render_animation(obj,[0,90,180],paths,16,16,lambda i,elapsed:written.append(i),checked)
                        self.assertEqual(len(elapsed),3)
                        self.assertEqual(written,[0,1,2])
                        self.assertTrue(all(path.read_bytes()==b'png' for path in paths))
                self.assertFalse(handlers.frame_change_pre)
                self.assertFalse(handlers.render_write)
                self.assertEqual(scene_mock.render.filepath,'old')
                self.assertEqual((scene_mock.frame_start,scene_mock.frame_end),(1,250))
                self.assertFalse((Path(directory)/'.render-animation').exists())

    def read(self, data, **kwargs):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.meshbin"
            path.write_bytes(data)
            return scene.read_mesh(path, **kwargs)

    def test_roundtrip_and_physical_bounds(self):
        vertices, triangles = self.read(binary())
        self.assertEqual(vertices[1], (1.0, 0.0, 0.0))
        self.assertEqual(triangles, [(0, 1, 2)])

    def test_truncation_trailing_bad_magic_and_version(self):
        for data in (binary()[:-1], binary() + b"x", b"NOPE!!!!" + binary()[8:],
                     binary()[:8] + struct.pack("<I", 2) + binary()[12:]):
            with self.subTest(data=data[:20]):
                with self.assertRaises(ValueError):
                    self.read(data)

    def test_nan_and_invalid_indices(self):
        for data in (binary(vertices=[(math.nan, 0, 0), (1, 0, 0), (0, 1, 0)]),
                     binary(triangles=[(0, 1, 3)])):
            with self.assertRaises(ValueError):
                self.read(data)

    def test_degenerate_float32_triangles_rejected(self):
        with self.assertRaisesRegex(ValueError, "degenerate"):
            self.read(binary(vertices=[(0, 0, 0), (1, 0, 0), (2, 0, 0)]))

    def test_budget_checked_before_payload(self):
        with self.assertRaisesRegex(ValueError, "budget"):
            self.read(struct.pack("<8sIII", b"TNMESH01", 1, 0xffffffff, 1))
        with self.assertRaisesRegex(ValueError, "budget"):
            self.read(binary(), max_vertices=2)


class SceneTests(unittest.TestCase):
    def test_studio_floor_retains_backdrop_coverage_without_oversized_faces(self):
        for extent in (0.001, 0.256, 10):
            with self.subTest(extent=extent):
                vertices, faces = scene.studio_floor_mesh(extent)
                self.assertEqual(min(v[0] for v in vertices), -100 * extent)
                self.assertEqual(max(v[0] for v in vertices), 100 * extent)
                self.assertEqual(min(v[1] for v in vertices), -100 * extent)
                self.assertEqual(max(v[1] for v in vertices), 100 * extent)
                self.assertTrue(all(v[2] == -0.00001 for v in vertices))
                area = 0
                edges = {}
                for face in faces:
                    a, b, c, d = [vertices[i] for i in face]
                    signed_area = (b[0]-a[0]) * (d[1]-a[1]) - (b[1]-a[1]) * (d[0]-a[0])
                    self.assertGreater(signed_area, 0)
                    area += signed_area
                    for start, end in zip(face, face[1:] + face[:1]):
                        self.assertLessEqual(math.dist(vertices[start], vertices[end]), extent * 16)
                        key = tuple(sorted((start, end)))
                        edges[key] = edges.get(key, 0) + 1
                self.assertAlmostEqual(area, (200 * extent)**2, delta=(200 * extent)**2 * 1e-12)
                self.assertTrue(all(count in (1, 2) for count in edges.values()))
                self.assertEqual(sum(count == 1 for count in edges.values()), 64)
                self.assertLessEqual(len(faces), 512)

    def test_section_contract_requires_physical_plane(self):
        valid={"axis":0,"position_mm":2.5,"keep_positive":True,"cap":True}
        scene.validate_section(valid)
        for key,value in (("axis",3),("position_mm",float("nan")),("cap","yes"),("keep_positive",0)):
            with self.assertRaises(ValueError):scene.validate_section(dict(valid,**{key:value}))

    def test_diagram_edges_exclude_coplanar_triangle_diagonal(self):
        vertices=[(0.,0.,0.),(1.,0.,0.),(1.,1.,0.),(0.,1.,0.)]
        edges=scene.crease_edges(vertices,[(0,1,2),(0,2,3)])
        self.assertEqual(len(edges),4)
        self.assertNotIn((0,2),edges)
        self.assertNotIn((2,0),edges)

    def test_camera_rejects_parallel_up_and_invalid_projection(self):
        valid = dict(target_mm=[0, 0, 5], direction=[0, -1, 1], up=[0, 0, 1],
                     projection="perspective", distance_mm=50, vertical_fov_degrees=35, ortho_span_mm=20)
        scene.validate_camera(valid)
        for key, value in (("up", [0, -1, 1]), ("distance_mm", 0), ("projection", "fisheye")):
            with self.assertRaises(ValueError):
                scene.validate_camera(dict(valid, **{key: value}))

    def test_explicit_bead_layout_splits_cap_normals(self):
        triangles = [(0, 1, 5), (0, 5, 4), (8, 1, 0), (9, 4, 5)]
        self.assertEqual(scene.bead_smooth_flags(triangles, "independent_stadium_v1"), [True, True, False, False])
        self.assertEqual(scene.bead_smooth_flags(triangles, None), [False] * 4)
        with self.assertRaises(ValueError):
            scene.bead_smooth_flags([(0, 1, 5)] * 4, "independent_stadium_v1")
        with self.assertRaises(ValueError):
            scene.bead_smooth_flags(triangles, "guessed-layout")

    def test_studio_power_scales_with_model_area(self):
        self.assertAlmostEqual(scene.studio_power(0.1, 550), 5.5)
        self.assertAlmostEqual(scene.studio_power(0.2, 550), 22)

    def test_scene_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mesh = root / "mesh.bin"
            mesh.write_bytes(binary())
            spec = dict(schema_version=1, mesh_path=str(mesh), geometry_kind="beads",
                        layer_height_mm=0.2, color=[0.05, 0.25, 0.6], roughness=0.4,
                        display_rotation_degrees=[0, 0, 0], warnings=[])
            path = root / "scene.json"
            path.write_text(json.dumps(spec))
            self.assertEqual(scene.load_scene_spec(path)["geometry_kind"], "beads")
            for key, value in (("roughness", math.nan), ("color", [1, 2]),
                               ("mesh_path", "relative.bin"), ("geometry_kind", "unknown")):
                bad = dict(spec, **{key: value})
                path.write_text(json.dumps(bad))
                with self.assertRaises(ValueError):
                    scene.load_scene_spec(path)
            spec.update(geometry_kind="original", layer_appearance=True, first_layer_height_mm=0.3,
                        groove_depth_mm=0.01)
            path.write_text(json.dumps(spec))
            self.assertTrue(scene.load_scene_spec(path)["layer_appearance"])
            for key, value in (("first_layer_height_mm", 0), ("groove_depth_mm", 0.06),
                               ("layer_appearance", "yes"), ("geometry_kind", "beads")):
                path.write_text(json.dumps(dict(spec, **{key: value})))
                with self.assertRaises(ValueError):
                    scene.load_scene_spec(path)

    def test_turntable_angles(self):
        angles = scene.turntable_angles(80, 0)
        self.assertEqual(len(angles), 80)
        self.assertEqual(angles[-1], 355.5)
        with self.assertRaises(ValueError):
            scene.turntable_angles(0, 0)

    def test_turntable_speed_repeats_exact_phases_and_preserves_partial_turns(self):
        angles = scene.turntable_angles(240, 17, seconds_per_turn=4)
        self.assertEqual(angles[:80], angles[80:160])
        self.assertEqual(angles[:80], angles[160:])
        partial = scene.turntable_angles(125, -360, seconds_per_turn=2.5)
        self.assertEqual(partial[:50], partial[50:100])
        self.assertEqual(partial[:25], partial[100:])
        self.assertEqual(partial[1], 7.2)
        # Non-integral frames per turn: the exact phase repeats after five turns.
        fractional = scene.turntable_angles(506, 0, seconds_per_turn=2.53)
        self.assertEqual(fractional[:253], fractional[253:])
        self.assertNotEqual(fractional[0], fractional[50])
        for speed in (0, True, float('nan'), .049, 501):
            with self.assertRaises(ValueError):
                scene.turntable_angles(20, 0, seconds_per_turn=speed)

    def test_envelope_includes_every_rotated_corner(self):
        points = [(-5, -1, 0), (5, 1, 20)]
        envelope = scene.rotation_envelope(points)
        self.assertAlmostEqual(envelope["radius"], math.sqrt(26))
        self.assertEqual(envelope["height"], 20)
        self.assertEqual(envelope["center"], (0, 0, 10))


class ProtocolTests(unittest.TestCase):
    def test_identical_frames_require_exact_camera_angle_and_visible_layers(self):
        angles = [0, 90, 360, 450, 0.000000000001]
        self.assertEqual(worker.identical_frame_sources(angles, None, [None]*5), [0, 1, 0, 1, 4])
        self.assertEqual(worker.identical_frame_sources(angles, None, [0, 0, 1, 1, 1]), list(range(5)))
        camera = dict(target_mm=[0, 0, 0])
        states = [dict(camera=camera), dict(camera=camera),
                  dict(camera=dict(target_mm=[0, 0, 0.001]))]
        self.assertEqual(worker.identical_frame_sources([0, 360, 720], states, [None]*3), [0, 0, 2])
        # Repeated fractions may retain the same complete deposited layer.
        self.assertEqual(worker.identical_frame_sources([0, 360, 720], None, [2, 2, 3]), [0, 0, 2])

    def test_identical_frame_copy_is_atomic_and_preserves_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = Path(directory)/'source.png', Path(directory)/'target.png'
            source.write_bytes(b'verified-image')
            target.write_bytes(b'previous-image')
            worker.copy_identical_frame(source, target, 64, 32, lambda p: (64, 32))
            self.assertEqual(target.read_bytes(), source.read_bytes())
            source.write_bytes(b'corrupted')
            with self.assertRaises(ValueError):
                worker.copy_identical_frame(source, target, 64, 32, lambda p: (16, 16))
            self.assertEqual(target.read_bytes(), b'verified-image')
            self.assertFalse(target.with_name('target.partial.png').exists())

    def test_layer_filter_fades_unresolved_pitch_and_grazing_views(self):
        self.assertEqual(scene.layer_filter_gain(0.2,0.0001,1.0),0.0)
        self.assertEqual(scene.layer_filter_gain(0.2,0.00002,1.0),1.0)
        self.assertEqual(scene.layer_filter_gain(0.2,0.00002,0.05),0.0)
        midpoint=scene.layer_filter_gain(0.2,0.0002/(3*math.sqrt(2)),1.0)
        self.assertAlmostEqual(midpoint,0.5)
        self.assertEqual(scene.layer_filter_gain(0.2,0.00002,-1.0),1.0)

    def test_material_ranges_reject_overlap_and_out_of_bounds(self):
        groups=[dict(start_triangle=0,end_triangle=2,color=[1,0,0],tool=0,label="Red"),
                dict(start_triangle=2,end_triangle=4,color=[0,0,1],tool=1,label="Blue")]
        self.assertEqual(scene.validate_face_groups(groups,5),groups)
        for change in (dict(start_triangle=1),dict(end_triangle=8),dict(color=[2,0,0]),dict(tool=-1)):
            with self.assertRaises(ValueError):
                scene.validate_face_groups([groups[0],dict(groups[1],**change)],5)

    def test_buildup_reveals_ordered_whole_layers_and_finishes_complete(self):
        ranges=[dict(layer_index=2,start_triangle=0,end_triangle=4,z_top_mm=0.2),
                dict(layer_index=5,start_triangle=4,end_triangle=8,z_top_mm=0.4)]
        self.assertEqual(scene.validate_layer_ranges(ranges,8),[2,5])
        self.assertEqual(scene.buildup_schedule([2,5],4),[2,2,5,5])
        self.assertEqual(scene.buildup_schedule([2,5],1),[5])
        for invalid in (ranges[:1],[dict(ranges[0],start_triangle=1),ranges[1]],
                        [ranges[0],dict(ranges[1],layer_index=1)],
                        [ranges[0],dict(ranges[1],z_top_mm=0.1)]):
            with self.assertRaises(ValueError):
                scene.validate_layer_ranges(invalid,8)

    def test_stepped_layers_retain_empty_intervals_in_build_schedule(self):
        ranges = [dict(layer_index=0,start_triangle=0,end_triangle=0,z_top_mm=0.2),
                  dict(layer_index=1,start_triangle=0,end_triangle=4,z_top_mm=0.4),
                  dict(layer_index=2,start_triangle=4,end_triangle=4,z_top_mm=0.6),
                  dict(layer_index=3,start_triangle=4,end_triangle=8,z_top_mm=0.8)]
        layers = scene.validate_layer_ranges(ranges,8,geometry_kind='stepped')
        self.assertEqual(layers,[0,1,2,3])
        self.assertEqual(scene.buildup_schedule(layers,4),[0,1,2,3])
        self.assertEqual([scene.layer_for_count(layers,n) for n in range(5)],[-1,0,1,2,3])
        for kind in ('beads','original'):
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                scene.validate_layer_ranges(ranges,8,geometry_kind=kind)
        for invalid in ([dict(ranges[0],end_triangle=-1),*ranges[1:]],
                        [ranges[0],dict(ranges[1],start_triangle=1),*ranges[2:]],
                        [*ranges[:-1],dict(ranges[-1],end_triangle=7)]):
            with self.assertRaises(ValueError):
                scene.validate_layer_ranges(invalid,8,geometry_kind='stepped')

    def test_stepped_scene_accepts_physical_layers_and_rejects_surface_layer_effect(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'scene.json'
            spec = dict(schema_version=1,mesh_path=str(Path(directory)/'mesh.bin'),
                        geometry_kind='stepped',layer_height_mm=0.2,color=[0.1]*3,
                        roughness=0.4,display_rotation_degrees=[0]*3,warnings=[],
                        layer_appearance=False,bead_layout=None,
                        layer_face_ranges=[dict(layer_index=0,start_triangle=0,end_triangle=0,z_top_mm=0.2),
                                           dict(layer_index=1,start_triangle=0,end_triangle=4,z_top_mm=0.4)])
            path.write_text(json.dumps(spec))
            self.assertEqual(scene.load_scene_spec(path)['geometry_kind'],'stepped')
            for invalid in (dict(layer_appearance=True),dict(bead_layout='independent_stadium_v1'),
                            dict(solid_layer_tops_mm=[0.2,0.4])):
                path.write_text(json.dumps(dict(spec,**invalid)))
                with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                    scene.load_scene_spec(path)

    def test_stepped_section_components_follow_indices_and_keep_shell_faces_together(self):
        # Each tetrahedron is a closed indexed shell. The renderer may place
        # their coordinates against each other without sharing vertex IDs.
        faces = [(0,2,1),(4,6,5),(0,1,3),(4,5,7),
                 (1,2,3),(5,6,7),(2,0,3),(6,4,7)]
        self.assertEqual(scene.indexed_face_components(faces,8),[[0,2,4,6],[1,3,5,7]])
        self.assertEqual(scene.indexed_face_components([],0),[])
        with self.assertRaisesRegex(ValueError,'budget'):
            scene.indexed_face_components(faces,8,max_components=1)

    def test_stepped_section_volume_counts_fully_removed_closed_layers_as_zero(self):
        kept = dict(closed_input=True,output_triangles=12,remaining_volume_mm3=750.0,
                    original_volume_mm3=1500.0,cross_section_area_mm2=150.0)
        removed = dict(closed_input=True,output_triangles=0,remaining_volume_mm3=None,
                       original_volume_mm3=1500.0,cross_section_area_mm2=0.0)
        self.assertEqual(scene.total_section_measurement([kept,removed],'remaining_volume_mm3'),750.0)
        self.assertEqual(scene.total_section_measurement([removed],'remaining_volume_mm3'),0.0)
        self.assertEqual(scene.total_section_measurement([kept,removed],'original_volume_mm3'),3000.0)
        self.assertEqual(scene.total_section_measurement([kept,removed],'cross_section_area_mm2'),150.0)
        for invalid in (dict(removed,closed_input=False),dict(removed,output_triangles=1)):
            with self.subTest(invalid=invalid):
                self.assertIsNone(scene.total_section_measurement([kept,invalid],'remaining_volume_mm3'))
        for name in ('remaining_volume_mm3','original_volume_mm3','cross_section_area_mm2'):
            self.assertEqual(scene.total_section_measurement([],name),0)

    def test_solid_build_schedule_requires_ordered_physical_tops_and_complete_model(self):
        self.assertEqual(scene.validate_solid_layer_tops([0.2, 0.4, 0.45], 0.45), [0, 1, 2])
        for tops in (None, [], [0], [-0.1], [True], [float('nan')], [float('inf')],
                     [0.4, 0.2], [0.2, 0.2], [0.2] * 50001):
            with self.subTest(tops=str(tops)[:60]), self.assertRaises(ValueError):
                scene.validate_solid_layer_tops(tops)
        for tops in ([0.2, 0.4], [0.2, 0.5]):
            with self.assertRaisesRegex(ValueError, 'model top'):
                scene.validate_solid_layer_tops(tops, 0.45)

    def test_scene_rejects_invalid_solid_schedule_before_rendering(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'scene.json'
            spec = dict(schema_version=1, mesh_path=str(Path(directory)/'mesh.bin'),
                        geometry_kind='original', layer_height_mm=0.2, color=[0.1]*3,
                        roughness=0.4, display_rotation_degrees=[0]*3, warnings=[],
                        solid_layer_tops_mm=[0.2, 0.2])
            path.write_text(json.dumps(spec))
            with self.assertRaisesRegex(ValueError, 'solid'):
                scene.load_scene_spec(path)
            spec['solid_layer_tops_mm'] = [0.2, 0.4]
            path.write_text(json.dumps(spec))
            self.assertEqual(scene.load_scene_spec(path)['solid_layer_tops_mm'], [0.2, 0.4])
            spec['geometry_kind'] = 'beads'
            path.write_text(json.dumps(spec))
            with self.assertRaisesRegex(ValueError, 'original'):
                scene.load_scene_spec(path)

    def test_default_camera_accounts_for_the_narrower_panel_dimension(self):
        square = scene.default_camera_half_angle(128,128)
        wide = scene.default_camera_half_angle(128,96)
        self.assertLess(wide,square)
        self.assertAlmostEqual(math.tan(wide)/math.tan(square),96/128)
        self.assertEqual(scene.default_camera_half_angle(96,128),square)

    def test_joined_ranges_smooth_sides_only_and_reject_bad_contract(self):
        triangles = [(i, (i+1)%32, 32+(i+1)%32) for i in range(32)]
        faces = []
        for i, face in enumerate(triangles):
            faces.extend((face, (i, 32+(i+1)%32,32+i)))
        faces.extend([(64,1,0),(65,32,33)])
        flags = scene.bead_smooth_flags(faces,"joined_stadium_v1",[[0,64]])
        self.assertEqual(flags,[True]*64+[False]*2)
        for ranges in (None, [[0,65]], [[0,64],[0,64]], [[0,128]], [[True,64]]):
            with self.assertRaises(ValueError):
                scene.bead_smooth_flags(faces,"joined_stadium_v1",ranges)
        broken = faces.copy()
        broken[1] = (0,33,99)
        with self.assertRaises(ValueError):
            scene.bead_smooth_flags(broken,"joined_stadium_v1",[[0,64]])

    def test_render_mode_flags_do_not_mutate_scene_snapshot(self):
        source = {"schema_version": 1, "geometry_kind": "beads"}
        self.assertEqual(worker.render_spec_for_mode(source, "still"), source)
        turntable = worker.render_spec_for_mode(source, "turntable")
        self.assertTrue(turntable["fit_turntable"])
        self.assertNotIn("build_up", turntable)
        buildup = worker.render_spec_for_mode(source, "buildup")
        self.assertTrue(buildup["fit_turntable"])
        self.assertTrue(buildup["build_up"])
        self.assertEqual(source, {"schema_version": 1, "geometry_kind": "beads"})

    def test_budgeted_joined_rings_keep_checked_side_and_cap_normals(self):
        for ring in (8,12,16,24):
            faces = []
            for i in range(ring):
                j = (i+1)%ring
                faces.extend(((i,j,ring+j),(i,ring+j,ring+i)))
            faces.extend(((ring*2,1,0),(ring*2+1,ring,ring+1)))
            flags = scene.bead_smooth_flags(faces,"joined_stadium_v2",[[0,ring*2]],ring)
            self.assertEqual(flags,[True]*(ring*2)+[False]*2)
            with self.assertRaises(ValueError):
                scene.bead_smooth_flags(faces,"joined_stadium_v2",[[0,ring*2]],32)

    def test_pruned_joined_sides_use_explicit_ranges(self):
        faces = [(0,1,2),(1,3,2),(4,1,0)]
        self.assertEqual(scene.bead_smooth_flags(faces,"joined_stadium_v3",[[0,2]],32), [True,True,False])
        for ranges in (None,[[0,0]],[[0,4]],[[0,2],[1,3]],[[True,2]]):
            with self.assertRaises(ValueError):
                scene.bead_smooth_flags(faces,"joined_stadium_v3",ranges,32)

    def test_timeline_frames_require_exact_count_valid_camera_and_monotone_build(self):
        camera = dict(target_mm=[0,0,1],direction=[1,-1,1],up=[0,0,1],projection="perspective",distance_mm=20,vertical_fov_degrees=35,ortho_span_mm=10)
        frames = [dict(camera=camera,angle_degrees=0,buildup_fraction=0),dict(camera=camera,angle_degrees=180,buildup_fraction=1)]
        self.assertEqual(worker.validate_frame_states(frames,2),frames)
        for invalid in (frames[:1],[frames[1],frames[0]],[dict(frames[0],angle_degrees=float('nan')),frames[1]],
                        [dict(frames[0],camera=dict(camera,distance_mm=0)),frames[1]],
                        [dict(frames[0],buildup_fraction=None),frames[1]]):
            with self.assertRaises(ValueError): worker.validate_frame_states(invalid,2)

    def test_complete_layer_selection_is_bounded_and_never_partial(self):
        self.assertEqual([scene.layer_for_fraction([2,5,9],value) for value in (0,.2,.5,1)],[2,2,5,9])
        for value in (-.01,1.01,float('nan'),True):
            with self.assertRaises(ValueError): scene.layer_for_fraction([2,5],value)

    def test_static_build_stage_counts_actual_layers_including_empty_bed(self):
        self.assertEqual([scene.layer_for_count([2,5,9],n) for n in range(4)],[-1,2,5,9])
        for count in (-1,4,1.5,True):
            with self.assertRaises(ValueError): scene.layer_for_count([2,5,9],count)

    def test_long_timeline_completion_stays_within_protocol_line_budget(self):
        payload=dict(paths=["x"*200]*10000,frame_seconds=[.1]*10000,frame_analysis=[dict(area=1)]*10000,build_layers=list(range(50000)),frame_count=10000,total_seconds=1)
        bounded=worker.completion_event(payload,"C:/job/render-result.json")
        self.assertLess(len(json.dumps(bounded)),900000)
        self.assertEqual(bounded["frame_count"],10000)
        self.assertEqual(bounded["details_path"],"C:/job/render-result.json")
        self.assertEqual(len(payload["paths"]),10000)

    def test_completion_surfaces_open_solid_warning_once_across_all_build_frames(self):
        warning = 'Open or inconsistently wound geometry: section remains uncapped; area/volume unavailable.'
        payload = dict(warnings=['Source import warning'],
                       frame_build_analysis=[dict(warnings=[warning]), dict(warnings=[warning]), None],
                       frame_analysis=[None, dict(warnings=[warning])])
        result = worker.with_analysis_warnings(payload)
        self.assertEqual(result['warnings'], ['Source import warning', warning])
        self.assertEqual(payload['warnings'], ['Source import warning'])
        self.assertEqual(worker.completion_event(payload, 'details.json')['warnings'],result['warnings'])

    def test_reuse_requires_matching_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            recipe = {"geometry_hash": "abc", "size": 32}
            worker.prepare_manifest(root, recipe, False)
            worker.prepare_manifest(root, recipe, True)
            with self.assertRaises(ValueError):
                worker.prepare_manifest(root, {**recipe, "size": 64}, True)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                worker.prepare_manifest(Path(directory), recipe, True)

    def test_reuse_only_accepts_decodable_requested_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "frame.png"
            self.assertFalse(worker.should_reuse_frame(path, 32, True, lambda p: (32, 32)))
            path.write_bytes(b"fixture")
            self.assertTrue(worker.should_reuse_frame(path, 32, True, lambda p: (32, 32)))
            self.assertFalse(worker.should_reuse_frame(path, 32, True, lambda p: (64, 32)))
            def broken(p):
                raise RuntimeError("bad PNG")
            self.assertFalse(worker.should_reuse_frame(path, 32, True, broken))
            with self.assertRaises(ValueError):
                worker.should_reuse_frame(path, 32, False, lambda p: (32, 32))

    def test_rejected_request_then_probe_and_shutdown_keep_identity(self):
        requests = [[], {"protocol_version": 9, "request_id": "bad", "scene_revision": 2, "kind": "probe"},
                    {"protocol_version": 1, "request_id": "probe", "scene_revision": 3, "kind": "probe"},
                    {"protocol_version": 1, "request_id": "done", "scene_revision": 3, "kind": "shutdown"}]
        output = io.StringIO()
        with mock.patch.object(worker.sys, "stdin", io.StringIO("\n".join(map(json.dumps, requests)) + "\n")), \
                mock.patch.object(worker.sys, "stdout", output), \
                mock.patch.object(worker.sys, "stderr", io.StringIO()), \
                mock.patch.object(worker, "probe_devices", return_value={"test": True}):
            worker.main()
        events = [json.loads(line[len(worker.PREFIX):]) for line in output.getvalue().splitlines()]
        self.assertEqual([event["kind"] for event in events], ["ready", "error", "error", "probe", "shutdown"])
        self.assertEqual(events[3]["request_id"], "probe")
        self.assertEqual(events[3]["scene_revision"], 3)
        self.assertEqual(events[2]["payload"]["error_type"], "ValueError")

    def test_exception_during_render_does_not_end_worker(self):
        request = dict(protocol_version=1, request_id="render", scene_revision=7, kind="render")
        following = dict(protocol_version=1, request_id="done", scene_revision=7, kind="shutdown")
        output = io.StringIO()
        with mock.patch.object(worker.sys, "stdin", io.StringIO(json.dumps(request) + "\n" + json.dumps(following) + "\n")), \
                mock.patch.object(worker.sys, "stdout", output), \
                mock.patch.object(worker.sys, "stderr", io.StringIO()), \
                mock.patch.object(worker, "render", side_effect=RuntimeError("render failed")):
            worker.main()
        events = [json.loads(line[len(worker.PREFIX):]) for line in output.getvalue().splitlines()]
        self.assertEqual([event["kind"] for event in events], ["ready", "error", "shutdown"])
        self.assertEqual(events[1]["request_id"], "render")


if __name__ == "__main__":
    unittest.main()
