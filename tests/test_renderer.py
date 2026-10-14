"""Run with: python -m unittest discover -s tests -p test_renderer.py -v"""
import importlib.util
import io
import json
import math
from pathlib import Path
import struct
import tempfile
import unittest
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

    def test_envelope_includes_every_rotated_corner(self):
        points = [(-5, -1, 0), (5, 1, 20)]
        envelope = scene.rotation_envelope(points)
        self.assertAlmostEqual(envelope["radius"], math.sqrt(26))
        self.assertEqual(envelope["height"], 20)
        self.assertEqual(envelope["center"], (0, 0, 10))


class ProtocolTests(unittest.TestCase):
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
