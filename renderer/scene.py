"""Checked physical mesh input and application-owned Blender studio.

Pure input helpers deliberately import without bpy for corruption regression tests.
"""
import hashlib
import json
import math
from pathlib import Path
import struct


def read_mesh(path, max_vertices=2_000_000, max_triangles=4_000_000):
    path = Path(path)
    with path.open("rb") as stream:
        header = stream.read(20)
        if len(header) != 20:
            raise ValueError("truncated mesh header")
        magic, version, vertices_count, triangles_count = struct.unpack("<8sIII", header)
        if magic != b"TNMESH01" or version != 1:
            raise ValueError("unsupported mesh magic/version")
        if vertices_count > max_vertices or triangles_count > max_triangles:
            raise ValueError("mesh exceeds vertex/triangle budget")
        if not vertices_count or not triangles_count:
            raise ValueError("empty mesh")
        expected = 20 + 12 * vertices_count + 12 * triangles_count
        if path.stat().st_size != expected:
            raise ValueError("mesh size differs from declared counts")
        vertex_bytes = stream.read(12 * vertices_count)
        triangle_bytes = stream.read(12 * triangles_count)
        if len(vertex_bytes) != 12 * vertices_count or len(triangle_bytes) != 12 * triangles_count:
            raise ValueError("mesh changed while reading")
        vertices = list(struct.iter_unpack("<3f", vertex_bytes))
        triangles = list(struct.iter_unpack("<3I", triangle_bytes))
    if any(not all(math.isfinite(value) for value in point) for point in vertices):
        raise ValueError("nonfinite mesh position")
    if any(max(face) >= vertices_count for face in triangles):
        raise ValueError("mesh index outside vertices")
    for face in triangles:
        a, b, c = (vertices[index] for index in face)
        u = tuple(b[i] - a[i] for i in range(3))
        v = tuple(c[i] - a[i] for i in range(3))
        cross = (u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0])
        if all(component == 0 for component in cross):
            raise ValueError("degenerate float32 mesh triangle")
    return vertices, triangles


def _finite_number(value, name, minimum=None, maximum=None):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f"{name} must be finite")
    if minimum is not None and value < minimum or maximum is not None and value > maximum:
        raise ValueError(f"{name} outside supported range")
    return value


def load_scene_spec(path):
    path = Path(path)
    if not path.is_absolute():
        raise ValueError("scene_path must be absolute")
    if path.stat().st_size > 1_048_576:
        raise ValueError("scene JSON exceeds budget")
    spec = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(spec, dict) or spec.get("schema_version") != 1:
        raise ValueError("unsupported scene schema")
    if not isinstance(spec.get("mesh_path"), str) or not Path(spec["mesh_path"]).is_absolute():
        raise ValueError("mesh_path must be absolute")
    if spec.get("geometry_kind") not in ("beads", "original"):
        raise ValueError("unsupported geometry_kind")
    if spec.get("bead_layout") not in (None, "independent_stadium_v1"):
        raise ValueError("unsupported bead_layout")
    if spec.get("bead_layout") is not None and spec["geometry_kind"] != "beads":
        raise ValueError("bead_layout requires bead geometry")
    _finite_number(spec.get("layer_height_mm"), "layer_height_mm", 0.02, 2.0)
    appearance = spec.get("layer_appearance", False)
    if type(appearance) is not bool or appearance and spec["geometry_kind"] != "original":
        raise ValueError("layer_appearance must be boolean and only enabled for original geometry")
    first = spec.get("first_layer_height_mm", spec["layer_height_mm"])
    _finite_number(first, "first_layer_height_mm", 0.02, 2.0)
    depth = spec.get("groove_depth_mm", min(0.05 * spec["layer_height_mm"], 0.25 * min(first, spec["layer_height_mm"])))
    _finite_number(depth, "groove_depth_mm", 0, 0.25 * min(first, spec["layer_height_mm"]))
    _finite_number(spec.get("roughness"), "roughness", 0, 1)
    for key in ("color", "display_rotation_degrees"):
        values = spec.get(key)
        if not isinstance(values, list) or len(values) != 3:
            raise ValueError(f"{key} must contain three values")
        for value in values:
            _finite_number(value, key, 0 if key == "color" else None, 1 if key == "color" else None)
    if not isinstance(spec.get("warnings"), list) or any(not isinstance(w, str) for w in spec["warnings"]):
        raise ValueError("warnings must be strings")
    if spec.get("camera") is not None:
        validate_camera(spec["camera"])
    if spec.get("material_preset", "matte_pla") not in ("matte_pla", "clay", "silk_pla", "glossy_plastic", "resin"):
        raise ValueError("unsupported material preset")
    if spec.get("studio_preset", "light") not in ("light", "dark", "diagram"):
        raise ValueError("unsupported studio preset")
    for name in ("resin_clarity", "resin_cloudiness"):
        _finite_number(spec.get(name, 0.8 if name == "resin_clarity" else 0.1), name, 0, 1)
    return spec


def validate_camera(camera):
    if not isinstance(camera, dict) or camera.get("projection") not in ("perspective", "orthographic"):
        raise ValueError("invalid camera projection")
    for name in ("target_mm", "direction", "up"):
        if not isinstance(camera.get(name), list) or len(camera[name]) != 3:
            raise ValueError("camera vector must contain three numbers")
        for value in camera[name]:
            _finite_number(value, name)
    d, u = camera["direction"], camera["up"]
    cross = (d[1]*u[2]-d[2]*u[1], d[2]*u[0]-d[0]*u[2], d[0]*u[1]-d[1]*u[0])
    if sum(v*v for v in cross) < 1e-15:
        raise ValueError("camera up and direction must be independent")
    _finite_number(camera.get("distance_mm"), "camera distance", 0.000001)
    _finite_number(camera.get("ortho_span_mm"), "camera ortho span", 0.000001)
    _finite_number(camera.get("vertical_fov_degrees"), "camera field of view", 1, 150)


def turntable_angles(frame_count, start_degrees):
    if isinstance(frame_count, bool) or not isinstance(frame_count, int) or not 1 <= frame_count <= 10000:
        raise ValueError("frame_count outside 1..10000")
    _finite_number(start_degrees, "start_angle_degrees")
    return [start_degrees + 360.0 * index / frame_count for index in range(frame_count)]


def rotation_envelope(points):
    low = tuple(min(point[i] for point in points) for i in range(3))
    high = tuple(max(point[i] for point in points) for i in range(3))
    center = tuple((low[i] + high[i]) * 0.5 for i in range(3))
    radius = max(math.hypot(p[0] - center[0], p[1] - center[1]) for p in points)
    return {"center": center, "radius": radius, "height": high[2] - low[2], "bottom": low[2]}


def bead_smooth_flags(triangles, layout):
    """Only the explicitly tagged Rust producer promises side/side/start/end quads."""
    if layout is None:
        return [False] * len(triangles)
    if layout != "independent_stadium_v1" or len(triangles) % 4:
        raise ValueError("unsupported or incomplete independent_stadium_v1 layout")
    flags = []
    for offset in range(0, len(triangles), 4):
        side1, side2, start, end = triangles[offset:offset+4]
        i, j, end_j = side1
        end_i = side2[2]
        if (side2[:2] != (i, end_j) or start[1:] != (j, i)
                or end[1:] != (end_i, end_j)
                or len({i, j, end_i, end_j, start[0], end[0]}) != 6):
            raise ValueError("mesh indices do not satisfy declared independent_stadium_v1 layout")
        flags.extend((True, True, False, False))
    return flags


def studio_power(extent, nominal_power):
    # Lights are centimeters away: metre-scale default wattages overexpose them.
    return nominal_power * 0.01 * (extent / 0.1) ** 2


def apply_layer_appearance(material, mesh, source_positions_mm, spec):
    """Physical bump appearance; keeps source silhouette and does not add beads."""
    tree = material.node_tree
    nodes, links = tree.nodes, tree.links
    build_z = mesh.attributes.new("turninator_build_z_m", "FLOAT", "POINT")
    minimum = min(point[2] for point in source_positions_mm)
    build_z.data.foreach_set("value", [(point[2] - minimum) * 0.001 for point in source_positions_mm])
    attribute = nodes.new("ShaderNodeAttribute")
    attribute.attribute_name = "turninator_build_z_m"

    def math_node(operation, left, right=None):
        node = nodes.new("ShaderNodeMath")
        node.operation = operation
        for index, value in enumerate((left, right)):
            if value is None:
                continue
            if isinstance(value, (float, int)):
                node.inputs[index].default_value = value
            else:
                links.new(value, node.inputs[index])
        return node.outputs[0]

    height = spec["layer_height_mm"]
    first = spec.get("first_layer_height_mm", height)
    depth = spec.get("groove_depth_mm", min(0.05 * height, 0.25 * min(first, height)))
    z = math_node("MULTIPLY", attribute.outputs["Fac"], 1000.0)
    below = math_node("LESS_THAN", z, first)
    first_phase = math_node("DIVIDE", z, first)
    other_phase = math_node("DIVIDE", math_node("SUBTRACT", z, first), height)
    phase = math_node("ADD", math_node("MULTIPLY", below, first_phase),
                      math_node("MULTIPLY", math_node("SUBTRACT", 1.0, below), other_phase))
    wave = math_node("COSINE", math_node("MULTIPLY", phase, 2 * math.pi))
    physical_height = math_node("MULTIPLY", math_node("ADD", 1.0, wave), -0.5 * depth * 0.001)
    geometry = nodes.new("ShaderNodeNewGeometry")
    to_object = nodes.new("ShaderNodeVectorTransform")
    to_object.vector_type = "NORMAL"
    to_object.convert_from = "WORLD"
    to_object.convert_to = "OBJECT"
    links.new(geometry.outputs["True Normal"], to_object.inputs["Vector"])
    undo_display = nodes.new("ShaderNodeVectorRotate")
    undo_display.rotation_type = "EULER_XYZ"
    undo_display.invert = True
    undo_display.inputs["Rotation"].default_value = tuple(math.radians(v) for v in spec["display_rotation_degrees"])
    links.new(to_object.outputs["Vector"], undo_display.inputs["Vector"])
    components = nodes.new("ShaderNodeSeparateXYZ")
    links.new(undo_display.outputs["Vector"], components.inputs[0])
    mask = math_node("SUBTRACT", 1.0, math_node("MULTIPLY", components.outputs["Z"], components.outputs["Z"]))
    bump = nodes.new("ShaderNodeBump")
    bump.inputs["Distance"].default_value = 1.0
    bump.inputs["Strength"].default_value = 1.0
    links.new(math_node("MULTIPLY", physical_height, mask), bump.inputs["Height"])
    links.new(bump.outputs["Normal"], nodes.get("Principled BSDF").inputs["Normal"])


def probe_devices():
    import bpy
    prefs = bpy.context.preferences.addons["cycles"].preferences
    devices = []
    optix = False
    try:
        prefs.compute_device_type = "OPTIX"
        prefs.get_devices()
        for device in prefs.devices:
            device.use = device.type == "OPTIX"
            devices.append({"name": device.name, "type": device.type, "id": device.id, "enabled": bool(device.use)})
        optix = any(d["type"] == "OPTIX" for d in devices)
    except Exception as error:
        devices.append({"error": str(error)})
    build_hash = bpy.app.build_hash
    return {"blender_version": bpy.app.version_string,
            "blender_build_hash": build_hash.decode() if isinstance(build_hash, bytes) else str(build_hash),
            "engines": ["eevee"] + (["cycles"] if optix else []),
            "cycles_backend": "OPTIX" if optix else None, "devices": devices}


def build_scene(spec, engine, size, samples, transparent, width=None, height=None):
    import bpy
    from mathutils import Euler, Vector, Matrix
    vertices, triangles = read_mesh(spec["mesh_path"])
    smooth_flags = bead_smooth_flags(triangles, spec.get("bead_layout")) if spec["geometry_kind"] == "beads" else None
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in list(bpy.data.meshes):
        if not block.users:
            bpy.data.meshes.remove(block)
    for block in list(bpy.data.materials):
        if not block.users:
            bpy.data.materials.remove(block)
    for collection in (bpy.data.cameras, bpy.data.lights):
        for block in list(collection):
            if not block.users:
                collection.remove(block)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE_NEXT" if engine == "eevee" else "CYCLES"
    device_info = probe_devices()
    if engine == "cycles":
        if device_info["cycles_backend"] != "OPTIX":
            raise RuntimeError("Cycles requires an available OPTIX device; no silent CPU fallback")
        scene.cycles.device = "GPU"
        scene.cycles.samples = samples
        scene.cycles.use_denoising = True
        scene.cycles.seed = 0
        scene.cycles.use_animated_seed = False
        scene.render.use_persistent_data = True
    else:
        scene.eevee.taa_render_samples = samples
    scene.render.resolution_x = width or size
    scene.render.resolution_y = height or size
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA" if transparent else "RGB"
    scene.render.image_settings.color_depth = "8"
    scene.render.film_transparent = transparent
    scene.render.use_file_extension = True
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.exposure = 0
    scene.view_settings.gamma = 1
    scene.render.fps = 20
    # Bake display rotation, then center the actual bead envelope on the spin axis.
    rotation = Euler(tuple(math.radians(v) for v in spec["display_rotation_degrees"]), "XYZ").to_matrix()
    points = [tuple(rotation @ Vector(tuple(v * 0.001 for v in point))) for point in vertices]
    envelope = rotation_envelope(points)
    cx, cy, _ = envelope["center"]
    bottom = envelope["bottom"]
    points = [(x-cx, y-cy, z-bottom) for x, y, z in points]
    mesh = bpy.data.meshes.new("TurninatorGeometry")
    mesh.from_pydata(points, [], triangles)
    mesh.update()
    obj = bpy.data.objects.new("PrintedPart", mesh)
    scene.collection.objects.link(obj)
    if smooth_flags is not None:
        for polygon, smooth in zip(mesh.polygons, smooth_flags):
            polygon.use_smooth = smooth
    material = bpy.data.materials.new("MattePLA")
    material.use_nodes = True
    bsdf = material.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*spec["color"], 1)
    bsdf.inputs["Roughness"].default_value = spec["roughness"]
    preset = spec.get("material_preset", "matte_pla")
    if preset == "silk_pla":
        bsdf.inputs["Metallic"].default_value = 0.18
        bsdf.inputs["Anisotropic IOR Level"].default_value = 0.65
    elif preset == "glossy_plastic":
        bsdf.inputs["Coat Weight"].default_value = 0.35
        bsdf.inputs["Coat Roughness"].default_value = 0.12
    elif preset == "resin":
        clarity = spec.get("resin_clarity", 0.8)
        cloudiness = spec.get("resin_cloudiness", 0.1)
        bsdf.inputs["Transmission Weight"].default_value = clarity
        bsdf.inputs["IOR"].default_value = 1.46
        bsdf.inputs["Roughness"].default_value = max(0.025, cloudiness * 0.5)
        volume = material.node_tree.nodes.new("ShaderNodeVolumeAbsorption")
        volume.inputs["Color"].default_value = (*spec["color"], 1)
        volume.inputs["Density"].default_value = 20.0 * (1 - clarity) + 2.0
        material.node_tree.links.new(volume.outputs["Volume"], material.node_tree.nodes.get("Material Output").inputs["Volume"])
    obj.data.materials.append(material)
    if spec.get("layer_appearance", False):
        apply_layer_appearance(material, mesh, vertices, spec)
    radius = envelope["radius"]
    height = envelope["height"]
    sphere_radius = max(math.hypot(radius, height * 0.5), 0.0001)
    target = Vector((0, 0, height * 0.5))
    camera_data = bpy.data.cameras.new("Camera")
    camera_data.lens = 70
    camera_data.sensor_fit = "HORIZONTAL"
    camera = bpy.data.objects.new("Camera", camera_data)
    scene.collection.objects.link(camera)
    scene.camera = camera
    half_angle = math.atan(camera_data.sensor_width / (2 * camera_data.lens))
    distance = sphere_radius / math.sin(half_angle) * 1.10
    direction = Vector((1.25, -1.7, 1.0)).normalized()
    camera.location = target + direction * distance
    camera.rotation_euler = (target - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera_data.clip_start = max(0.00001, distance * 0.001)
    camera_data.clip_end = distance * 100
    if spec.get("camera") is not None:
        chosen = spec["camera"]
        validate_camera(chosen)
        target = Vector(chosen["target_mm"]) * 0.001
        direction = Vector(chosen["direction"]).normalized()
        right = Vector(chosen["up"]).cross(direction).normalized()
        actual_up = direction.cross(right).normalized()
        distance = chosen["distance_mm"] * 0.001
        camera.location = target + direction * distance
        camera.rotation_euler = Matrix((right, actual_up, direction)).transposed().to_euler()
        camera_data.sensor_fit = "VERTICAL"
        camera_data.type = "ORTHO" if chosen["projection"] == "orthographic" else "PERSP"
        camera_data.ortho_scale = chosen["ortho_span_mm"] * 0.001
        camera_data.lens = camera_data.sensor_height / (2 * math.tan(math.radians(chosen["vertical_fov_degrees"]) / 2))
        camera_data.clip_start = max(0.000001, distance * 0.0001)
        camera_data.clip_end = max(distance * 100, sphere_radius * 100)
    extent = max(radius * 2, height, 0.001)
    studio = spec.get("studio_preset", "light")
    if not transparent and studio != "diagram":
        bpy.ops.mesh.primitive_plane_add(size=extent * 200, location=(0, 0, -0.00001))
        floor = bpy.context.object
        floor.name = "StudioFloor"
        mat = bpy.data.materials.new("StudioFloorMaterial")
        mat.use_nodes = True
        mat.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value = (0.025, 0.03, 0.04, 1) if studio == "dark" else (0.18, 0.20, 0.24, 1)
        mat.node_tree.nodes.get("Principled BSDF").inputs["Roughness"].default_value = 0.7
        floor.data.materials.append(mat)
    if scene.world is None:
        scene.world = bpy.data.worlds.new("StudioWorld")
    scene.world.use_nodes = True
    scene.world.node_tree.nodes.get("Background").inputs["Color"].default_value = (1, 1, 1, 1) if studio == "diagram" else (0.025, 0.03, 0.04, 1) if studio == "dark" else (0.22, 0.25, 0.3, 1)
    scene.world.node_tree.nodes.get("Background").inputs["Strength"].default_value = 0.12 if studio == "dark" else 0.35
    if studio == "diagram":
        scene.render.film_transparent = True
    for name, location, energy, area in (
            ("Key", (-2.5, -3.0, 4.0), 550, 2.2),
            ("Fill", (3.0, -1.0, 2.0), 180, 2.5),
            ("Rim", (0.0, 3.0, 3.0), 650, 1.6)):
        data = bpy.data.lights.new(name, "AREA")
        data.energy = studio_power(extent, energy)
        data.shape = "DISK"
        data.size = extent * area
        light = bpy.data.objects.new(name, data)
        scene.collection.objects.link(light)
        light_vector = Vector(location)
        if studio == "diagram":
            light_vector = camera.rotation_euler.to_matrix() @ light_vector
        light.location = target + light_vector * extent
        light.rotation_euler = (target - light.location).to_track_quat("-Z", "Y").to_euler()
    digest = hashlib.sha256()
    with Path(spec["mesh_path"]).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return obj, {"geometry_hash": digest.hexdigest(), "vertices": len(vertices),
                 "triangles": len(triangles), "bounds_m": envelope,
                 "device": device_info, "warnings": spec["warnings"]}
