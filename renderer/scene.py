"""Checked physical mesh input and application-owned Blender studio.

Pure input helpers deliberately import without bpy for corruption regression tests.
"""
import hashlib
import json
import math
from pathlib import Path
import struct
from fractions import Fraction


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
    if spec.get("geometry_kind") not in ("beads", "original", "stepped"):
        raise ValueError("unsupported geometry_kind")
    if "solid_layer_tops_mm" in spec:
        if spec["geometry_kind"] != "original":
            raise ValueError("solid_layer_tops_mm requires original geometry")
        validate_solid_layer_tops(spec["solid_layer_tops_mm"])
    if spec.get("bead_layout") not in (None, "independent_stadium_v1", "joined_stadium_v1", "joined_stadium_v2", "joined_stadium_v3"):
        raise ValueError("unsupported bead_layout")
    if spec.get("bead_layout") == "joined_stadium_v2" and (type(spec.get("bead_ring_vertices")) is not int or spec["bead_ring_vertices"] not in (8,12,16,24)):
        raise ValueError("budgeted joined beads require a supported ring size")
    if spec.get("bead_layout") == "joined_stadium_v3" and (type(spec.get("bead_ring_vertices")) is not int or spec["bead_ring_vertices"] not in (8,12,16,24,32)):
        raise ValueError("pruned joined beads require a supported ring size")
    if spec.get("bead_layout") is not None and spec["geometry_kind"] != "beads":
        raise ValueError("bead_layout requires bead geometry")
    _finite_number(spec.get("layer_height_mm"), "layer_height_mm", 0.02, 2.0)
    appearance = spec.get("layer_appearance", False)
    if type(spec.get("layer_filtering",True)) is not bool:
        raise ValueError("layer_filtering must be boolean")
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
    if spec.get("section") is not None:
        validate_section(spec["section"])
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


def validate_section(section):
    if not isinstance(section, dict) or type(section.get("axis")) is not int or section["axis"] not in (0,1,2):
        raise ValueError("section axis must be 0, 1 or 2")
    _finite_number(section.get("position_mm"), "section position")
    if type(section.get("keep_positive")) is not bool or type(section.get("cap")) is not bool:
        raise ValueError("section keep_positive/cap must be boolean")


def _face_range(group, count, previous_end, allow_empty=False):
    if not isinstance(group,dict):
        raise ValueError("face group must be an object")
    start,end = group.get("start_triangle"),group.get("end_triangle")
    if (type(start) is not int or type(end) is not int
            or not previous_end <= start <= end <= count or start == end and not allow_empty):
        raise ValueError("face ranges must be ordered, disjoint and within the triangle count")
    return start,end


def validate_face_groups(groups, triangle_count):
    if not isinstance(groups,list) or len(groups)>100000:
        raise ValueError("face_groups must be a bounded list")
    previous_end=0
    materials=set()
    for group in groups:
        _,previous_end=_face_range(group,triangle_count,previous_end)
        color=group.get("color")
        if not isinstance(color,list) or len(color)!=3:
            raise ValueError("face group color must be three linear RGB values")
        for value in color:
            _finite_number(value,"face group color",0,1)
        tool=group.get("tool")
        if type(tool) is not int or not 0<=tool<=255:
            raise ValueError("face group tool must be an integer 0..255")
        if not isinstance(group.get("label"),str) or len(group["label"])>256:
            raise ValueError("face group label must be a short string")
        materials.add((tool,tuple(color)))
        if len(materials)>256:
            raise ValueError("material mapping exceeds 256 unique tool/color combinations")
    return groups


def validate_layer_ranges(ranges, triangle_count, geometry_kind="beads"):
    if not isinstance(ranges,list) or not ranges or len(ranges)>100000:
        raise ValueError("build-up requires bounded layer_face_ranges")
    previous_end=0
    previous_layer=-1
    previous_z=-math.inf
    layers=[]
    for group in ranges:
        # Empty sampled intervals are still physical steps in the STL schedule.
        # Bead ranges continue to require deposited faces in every group.
        start,end=_face_range(group,triangle_count,previous_end,allow_empty=geometry_kind=="stepped")
        layer=group.get("layer_index")
        z=_finite_number(group.get("z_top_mm"),"layer top")
        if start != previous_end or type(layer) is not int or not 0<=layer<=1000000 or layer<previous_layer or z<previous_z:
            raise ValueError("layer ranges must cover every face in increasing build order")
        if layer==previous_layer and z!=previous_z:
            raise ValueError("one layer index must have one build height")
        if layer!=previous_layer:
            layers.append(layer)
        previous_end,previous_layer,previous_z=end,layer,z
    if previous_end!=triangle_count:
        raise ValueError("layer ranges must cover every face")
    return layers


def buildup_schedule(layers,frame_count):
    turntable_angles(frame_count,0)  # Shared bounded frame-count validation.
    if not layers:
        raise ValueError("build-up requires at least one layer")
    return [layers[max(0,math.ceil((index+1)*len(layers)/frame_count)-1)] for index in range(frame_count)]


def validate_solid_layer_tops(tops, model_top_mm=None):
    """Physical BUILD-Z planes; stage IDs count the original model's schedule."""
    if not isinstance(tops,list) or not tops or len(tops)>50000:
        raise ValueError("solid build-up requires bounded solid_layer_tops_mm")
    previous = 0
    for top in tops:
        _finite_number(top,"solid layer top")
        if top <= previous:
            raise ValueError("solid layer tops must be positive and strictly ascending")
        previous = top
    if model_top_mm is not None and not math.isclose(tops[-1],model_top_mm,rel_tol=1e-6,abs_tol=1e-5):
        raise ValueError("solid layer schedule must end at the true model top")
    return list(range(len(tops)))


def apply_face_groups(obj,base_material,groups):
    from array import array
    validate_face_groups(groups,len(obj.data.polygons))
    indices=array('i',[0])*len(obj.data.polygons)
    materials={}
    for group in groups:
        key=(group["tool"],tuple(group["color"]))
        if key not in materials:
            material=base_material.copy()
            material.name=f"Tool{group['tool']}_{group['label']}"
            material.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value=(*group["color"],1)
            for node in material.node_tree.nodes:
                if node.type=="VOLUME_ABSORPTION":
                    node.inputs["Color"].default_value=(*group["color"],1)
            materials[key]=len(obj.data.materials)
            obj.data.materials.append(material)
        start,end=group["start_triangle"],group["end_triangle"]
        indices[start:end]=array('i',[materials[key]])*(end-start)
    obj.data.polygons.foreach_set("material_index",indices)


def assign_layer_indices(obj,ranges,geometry_kind="beads"):
    from array import array
    layers=validate_layer_ranges(ranges,len(obj.data.polygons),geometry_kind)
    values=array('i',[0])*len(obj.data.polygons)
    for group in ranges:
        start,end=group["start_triangle"],group["end_triangle"]
        values[start:end]=array('i',[group["layer_index"]])*(end-start)
    attribute=obj.data.attributes.get("turninator_layer_index")
    if attribute is None:
        attribute=obj.data.attributes.new("turninator_layer_index","INT","FACE")
    attribute.data.foreach_set("value",values)
    return layers


def apply_buildup(obj,ranges,geometry_kind="beads"):
    import bpy
    layers=assign_layer_indices(obj,ranges,geometry_kind)
    tree=bpy.data.node_groups.new("TurninatorBuildUp","GeometryNodeTree")
    tree.interface.new_socket(name="Geometry",in_out="INPUT",socket_type="NodeSocketGeometry")
    tree.interface.new_socket(name="Geometry",in_out="OUTPUT",socket_type="NodeSocketGeometry")
    source=tree.nodes.new("NodeGroupInput")
    output=tree.nodes.new("NodeGroupOutput")
    attribute_node=tree.nodes.new("GeometryNodeInputNamedAttribute")
    attribute_node.data_type="INT"
    attribute_node.inputs["Name"].default_value="turninator_layer_index"
    visible=tree.nodes.new("ShaderNodeValue")
    visible.name="VisibleLayer"
    visible.outputs[0].default_value=layers[-1]
    compare=tree.nodes.new("ShaderNodeMath")
    compare.operation="GREATER_THAN"
    delete=tree.nodes.new("GeometryNodeDeleteGeometry")
    delete.domain="FACE"
    delete.mode="ALL"
    tree.links.new(source.outputs["Geometry"],delete.inputs["Geometry"])
    tree.links.new(attribute_node.outputs["Attribute"],compare.inputs[0])
    tree.links.new(visible.outputs[0],compare.inputs[1])
    tree.links.new(compare.outputs[0],delete.inputs["Selection"])
    tree.links.new(delete.outputs["Geometry"],output.inputs["Geometry"])
    modifier=obj.modifiers.new("BuildUp","NODES")
    modifier.node_group=tree
    return layers


def set_visible_layer(obj,layer):
    modifier=obj.modifiers.get("BuildUp")
    if modifier is None:
        raise ValueError("build-up modifier is missing")
    modifier.node_group.nodes["VisibleLayer"].outputs[0].default_value=layer
    obj.update_tag()


def layer_for_fraction(layers, fraction):
    _finite_number(fraction, "build fraction", 0, 1)
    if not layers:
        raise ValueError("build-up requires deposited layers")
    return layers[min(int(fraction * len(layers)), len(layers)-1)]


def layer_for_count(layers,count):
    if type(count) is not int or not 0<=count<=len(layers):
        raise ValueError("fixed build stage must count 0 through the actual number of deposited layers")
    return -1 if count==0 else layers[count-1]


def prepare_buildup_section(obj, source_mesh, spec, envelope, layer, build_analysis=None):
    """Restore immutable geometry, reveal in BUILD space, then optionally section."""
    import bpy
    import bmesh
    from mathutils import Euler
    old = obj.data
    obj.data = source_mesh.copy()
    if not old.users and old != source_mesh:
        bpy.data.meshes.remove(old)
    obj.rotation_euler.z = 0
    rotation = Euler(tuple(math.radians(v) for v in spec["display_rotation_degrees"]),"XYZ").to_matrix()
    extent = max(envelope["radius"]*2,envelope["height"],0.001)
    if spec["geometry_kind"] == "original":
        layers = validate_solid_layer_tops(spec.get("solid_layer_tops_mm"))
        if type(layer) is not int or not -1 <= layer < len(layers):
            raise ValueError("solid visible layer outside schedule")
        if layer == -1:
            bm = bmesh.new()
            bm.to_mesh(obj.data)
            bm.free()
            reveal_analysis = dict(capped=False,closed_input=False,remaining_volume_mm3=0.0,output_triangles=0)
        elif layer < layers[-1]:
            # apply_section transforms the BUILD plane with the same display
            # rotation/centering as the immutable source. Spin is applied later.
            reveal = dict(axis=2,position_mm=spec["solid_layer_tops_mm"][layer],keep_positive=False,cap=True)
            reveal_analysis = apply_section(obj,reveal,rotation,envelope["center"][:2],envelope["bottom"],extent,
                                            "original",cap_material_index=0)
        else:
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=0.0)
            closed = bool(bm.faces) and all(e.is_manifold and e.is_contiguous for e in bm.edges)
            volume = abs(bm.calc_volume(signed=True))*1e9 if closed else None
            bm.free()
            obj.data.calc_loop_triangles()
            reveal_analysis = dict(capped=False,closed_input=closed,remaining_volume_mm3=volume,
                                   output_triangles=len(obj.data.loop_triangles))
        if build_analysis is not None:
            build_analysis.update(reveal_analysis,visible_layer=layer,
                                  z_top_mm=0.0 if layer == -1 else spec["solid_layer_tops_mm"][layer])
        analysis = None
        if spec.get("section") is not None:
            analysis = apply_section(obj,spec["section"],rotation,envelope["center"][:2],
                                     envelope["bottom"],extent,"original")
            analysis["visible_layer"] = layer
        # Diagram edges must follow the actual stage, including the empty bed.
        edge_object = bpy.data.objects.get("DesignEdges")
        if edge_object is not None:
            bpy.data.objects.remove(edge_object,do_unlink=True)
        if spec.get("studio_preset") == "diagram" and obj.data.polygons:
            obj.data.calc_loop_triangles()
            points = [tuple(v.co) for v in obj.data.vertices]
            triangles = [tuple(t.vertices) for t in obj.data.loop_triangles]
            pixel_mm = spec.get("camera",{}).get("ortho_span_mm",extent*1000)/bpy.context.scene.render.resolution_y
            add_design_edges(points,triangles,points,pixel_mm,bpy.context.scene)
            bpy.data.objects["DesignEdges"].parent = obj
        for material in list(bpy.data.materials):
            if not material.users and material.name.startswith(("SectionCutSurface","DesignEdgeInk")):
                bpy.data.materials.remove(material)
        return analysis
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bm.faces.ensure_lookup_table()
    remove = []
    for group in spec["layer_face_ranges"]:
        if group["layer_index"] > layer:
            remove.extend(bm.faces[i] for i in range(group["start_triangle"],group["end_triangle"]))
    bmesh.ops.delete(bm,geom=remove,context="FACES")
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()
    if not obj.data.polygons:
        return {"axis":spec["section"]["axis"],"position_mm":spec["section"]["position_mm"],
                "keep_positive":spec["section"]["keep_positive"],"capped":False,"closed_input":False,
                "cross_section_area_mm2":0.0,"original_volume_mm3":0.0,"remaining_volume_mm3":0.0,
                "visible_layer":layer,"visible_source_triangles":0,"output_triangles":0,
                "warnings":[],"meaning":"Empty build stage: no deposited geometry is visible."}
    analysis = apply_section(obj,spec["section"],rotation,envelope["center"][:2],envelope["bottom"],
                             max(envelope["radius"]*2,envelope["height"],0.001),spec["geometry_kind"])
    analysis["visible_layer"] = layer
    analysis["visible_source_triangles"] = sum(g["end_triangle"]-g["start_triangle"] for g in spec["layer_face_ranges"] if g["layer_index"]<=layer)
    # Each frame's disposable cap material can be freed after its mesh is replaced.
    for material in list(bpy.data.materials):
        if not material.users and material.name.startswith("SectionCutSurface"):
            bpy.data.materials.remove(material)
    return analysis


def set_camera_spec(camera, chosen):
    from mathutils import Vector, Matrix
    validate_camera(chosen)
    target=Vector(chosen["target_mm"])*0.001
    direction=Vector(chosen["direction"]).normalized()
    right=Vector(chosen["up"]).cross(direction).normalized()
    up=direction.cross(right).normalized()
    distance=chosen["distance_mm"]*0.001
    camera.location=target+direction*distance
    camera.rotation_euler=Matrix((right,up,direction)).transposed().to_euler()
    camera.data.sensor_fit="VERTICAL"
    camera.data.type="ORTHO" if chosen["projection"]=="orthographic" else "PERSP"
    camera.data.ortho_scale=chosen["ortho_span_mm"]*0.001
    camera.data.lens=camera.data.sensor_height/(2*math.tan(math.radians(chosen["vertical_fov_degrees"])/2))
    camera.data.clip_start=max(0.000001,distance*0.0001)
    camera.data.clip_end=max(distance*100,100.)


def indexed_face_components(faces, vertex_count, max_components=2000):
    """Partition by shared indices, never by coordinates of touching shells."""
    from array import array
    parent=array('I',range(len(faces)))
    owner=array('i',[-1])*vertex_count
    def root(index):
        while parent[index]!=index:
            parent[index]=parent[parent[index]]
            index=parent[index]
        return index
    for index,face in enumerate(faces):
        for vertex in face:
            if owner[vertex]<0:
                owner[vertex]=index
            else:
                parent[root(index)]=root(owner[vertex])
    groups={}
    for index in range(len(faces)):
        groups.setdefault(root(index),[]).append(index)
    if len(groups)>max_components:
        raise ValueError("stepped section component work budget exceeded")
    return list(groups.values())


def section_cap_material(obj):
    import bpy
    material=bpy.data.materials.new("SectionCutSurface")
    material.use_nodes=True
    bsdf=material.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value=(0.5,0.18,0.065,1)
    bsdf.inputs["Roughness"].default_value=0.65
    obj.data.materials.append(material)
    return len(obj.data.materials)-1


def total_section_measurement(analyses,name):
    total=0.0
    for analysis in analyses:
        value=analysis[name]
        # A closed layer removed by the section contributes no remaining solid.
        # Unknown measurements from open inputs or nonempty outputs stay unknown.
        if (value is None and name=="remaining_volume_mm3" and analysis["closed_input"]
                and analysis["output_triangles"]==0):
            value=0.0
        if value is None:
            return None
        total+=value
    return total


def apply_stepped_section(obj,section,rotation,center_xy,bottom,extent,cap_material_index):
    """Section closed slabs separately so coincident interfaces stay independent."""
    import bpy
    faces=[tuple(p.vertices) for p in obj.data.polygons]
    attribute=obj.data.attributes.get("turninator_layer_index")
    if attribute is not None:
        groups={}
        for index,item in enumerate(attribute.data):
            groups.setdefault(item.value,[]).append(index)
        groups=list(groups.values())
        if len(groups)>2000:
            raise ValueError("stepped section layer work budget exceeded")
    else:
        groups=indexed_face_components(faces,len(obj.data.vertices))
    cap_index=section_cap_material(obj) if cap_material_index is None else cap_material_index
    materials=list(obj.data.materials)
    vertices=[]
    result_faces=[]
    face_materials=[]
    smooth=[]
    analyses=[]
    for group in groups:
        source_indices=sorted({i for face_index in group for i in faces[face_index]})
        remap={original:local for local,original in enumerate(source_indices)}
        mesh=bpy.data.meshes.new("DisposableSteppedSectionGeometry")
        part=None
        try:
            mesh.from_pydata([tuple(obj.data.vertices[i].co) for i in source_indices],[],
                             [tuple(remap[i] for i in faces[index]) for index in group])
            for material in materials:
                mesh.materials.append(material)
            for polygon,index in zip(mesh.polygons,group):
                polygon.material_index=obj.data.polygons[index].material_index
                polygon.use_smooth=obj.data.polygons[index].use_smooth
            mesh.update()
            part=bpy.data.objects.new("DisposableSteppedSection",mesh)
            bpy.context.scene.collection.objects.link(part)
            analyses.append(apply_section(part,section,rotation,center_xy,bottom,extent,
                                          "stepped_layer",cap_index))
            # Boolean evaluation may replace the part's mesh datablock.
            cut=part.data
            used=sorted({i for polygon in cut.polygons for i in polygon.vertices})
            result_remap={original:len(vertices)+local for local,original in enumerate(used)}
            vertices.extend(tuple(cut.vertices[i].co) for i in used)
            for polygon in cut.polygons:
                result_faces.append(tuple(result_remap[i] for i in polygon.vertices))
                face_materials.append(polygon.material_index)
                smooth.append(polygon.use_smooth)
        finally:
            cut=part.data if part is not None else mesh
            original_mesh=cut==mesh
            if part is not None:
                bpy.data.objects.remove(part,do_unlink=True)
            if not cut.users:
                bpy.data.meshes.remove(cut)
            if not original_mesh and not mesh.users:
                bpy.data.meshes.remove(mesh)
    output=bpy.data.meshes.new("TurninatorSectionedLayers")
    output.from_pydata(vertices,[],result_faces)
    for material in materials:
        output.materials.append(material)
    for polygon,material_index,is_smooth in zip(output.polygons,face_materials,smooth):
        polygon.material_index=material_index
        polygon.use_smooth=is_smooth
    output.update()
    previous=obj.data
    obj.data=output
    if not previous.users:
        bpy.data.meshes.remove(previous)
    obj.data.calc_loop_triangles()
    return dict(axis=section["axis"],position_mm=section["position_mm"],keep_positive=section["keep_positive"],
                capped=bool(analyses) and all(a["capped"] for a in analyses),
                closed_input=bool(analyses) and all(a["closed_input"] for a in analyses),
                cross_section_area_mm2=total_section_measurement(analyses,"cross_section_area_mm2"),
                original_volume_mm3=total_section_measurement(analyses,"original_volume_mm3"),
                remaining_volume_mm3=total_section_measurement(analyses,"remaining_volume_mm3"),
                output_triangles=len(obj.data.loop_triangles),
                warnings=list(dict.fromkeys(w for a in analyses for w in a["warnings"])),
                volume_basis="Additive volumes of independently closed, nonoverlapping stepped layers.",
                meaning="Geometric intersection of the modeled stepped layers; not strength, mass, or measured print behavior.")


def apply_section(obj, section, rotation, center_xy, bottom, extent, geometry_kind, cap_material_index=None):
    """Cut a disposable render mesh; a plane never fills an infill envelope."""
    import bpy
    import bmesh
    from mathutils import Vector
    validate_section(section)
    if geometry_kind=="stepped":
        return apply_stepped_section(obj,section,rotation,center_xy,bottom,extent,cap_material_index)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    if geometry_kind == "original":
        # STL can duplicate identical triangle corners. Exact equality only;
        # no tolerance merging of gaps or proximity-based mesh repair.
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=0.0)
    closed = bool(bm.faces) and all(edge.is_manifold and edge.is_contiguous for edge in bm.edges)
    original_volume = abs(bm.calc_volume(signed=True))*1e9 if closed else None
    bm.to_mesh(obj.data)
    bm.free()
    axis = Vector((1 if i == section["axis"] else 0 for i in range(3)))
    normal = rotation @ axis
    plane = rotation @ (axis * (section["position_mm"] * 0.001)) - Vector((center_xy[0],center_xy[1],bottom))
    direction = normal if section["keep_positive"] else -normal
    center = sum((Vector(corner) for corner in obj.bound_box), Vector()) / 8
    anchor = center + normal * ((plane-center).dot(normal))
    warnings = []
    if geometry_kind == "beads":
        warnings.append("Bead source volume is an algebraic mesh-volume estimate; overlaps between paths are not assessed.")
    do_cap = section["cap"] and closed
    if section["cap"] and not closed:
        warnings.append("Open or inconsistently wound geometry: section remains uncapped; area/volume unavailable.")
    distances = [(vertex.co-plane).dot(direction) for vertex in obj.data.vertices]
    tolerance = max(extent*1e-7, 1e-9)
    intersects = bool(distances) and min(distances) < -tolerance and max(distances) > tolerance
    if do_cap and intersects:
        bpy.ops.mesh.primitive_cube_add(size=2, location=anchor+direction*extent*4)
        cutter = bpy.context.object
        cutter.name = "DisposableSectionHalfspace"
        cutter.rotation_euler = direction.to_track_quat("Z", "Y").to_euler()
        cutter.scale = (extent*4,extent*4,extent*4)
        modifier = obj.modifiers.new("PhysicalSection", "BOOLEAN")
        modifier.operation = "INTERSECT"
        modifier.solver = "EXACT"
        modifier.use_self = True
        modifier.use_hole_tolerant = True
        modifier.object = cutter
        bpy.context.view_layer.objects.active = obj
        try:
            bpy.ops.object.modifier_apply(modifier=modifier.name)
        finally:
            bpy.data.objects.remove(cutter, do_unlink=True)
    elif not do_cap or distances and max(distances) <= tolerance:
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts)+list(bm.edges)+list(bm.faces),
                              plane_co=plane, plane_no=direction, dist=tolerance,
                              clear_inner=True, clear_outer=False)
        bm.to_mesh(obj.data)
        bm.free()
    obj.data.update()
    area = 0.0
    if cap_material_index is None:
        cap_index = section_cap_material(obj)
    else:
        cap_index = cap_material_index
    for polygon in obj.data.polygons:
        if abs((polygon.center-plane).dot(normal)) <= tolerance*5 and abs(polygon.normal.dot(normal)) > 0.999:
            area += polygon.area*1e6
            polygon.use_smooth = False
            if do_cap:
                polygon.material_index = cap_index
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    closed_after = bool(bm.faces) and all(edge.is_manifold for edge in bm.edges)
    volume = abs(bm.calc_volume(signed=True))*1e9 if closed_after else 0.0 if not bm.faces else None
    bm.free()
    obj.data.calc_loop_triangles()
    return {"axis":section["axis"],"position_mm":section["position_mm"],"keep_positive":section["keep_positive"],
            "capped":do_cap,"closed_input":closed,"cross_section_area_mm2":area if do_cap else None,
            "original_volume_mm3":original_volume,"remaining_volume_mm3":volume,
            "volume_basis":"Original: signed closed-mesh volume magnitude, additive for separate shells. Remaining: closed cut-mesh volume magnitude.",
            "output_triangles":len(obj.data.loop_triangles),"warnings":warnings,
            "meaning":"Geometric intersection of the modeled mesh; not strength, mass, or measured print behavior."}


def turntable_angles(frame_count, start_degrees, seconds_per_turn=None):
    if isinstance(frame_count, bool) or not isinstance(frame_count, int) or not 1 <= frame_count <= 10000:
        raise ValueError("frame_count outside 1..10000")
    _finite_number(start_degrees, "start_angle_degrees")
    if seconds_per_turn is None:
        period = Fraction(frame_count)
    else:
        _finite_number(seconds_per_turn, "seconds_per_turn", 0.05, 500)
        period = Fraction(str(seconds_per_turn)) * 20
    # Decimal timing has an exact rational phase. Repeated turns therefore use
    # identical angles even when a turn spans a non-integral number of frames.
    return [(start_degrees % 360 + float((Fraction(index) / period) % 1) * 360) % 360
            for index in range(frame_count)]


def rotation_envelope(points):
    low = tuple(min(point[i] for point in points) for i in range(3))
    high = tuple(max(point[i] for point in points) for i in range(3))
    center = tuple((low[i] + high[i]) * 0.5 for i in range(3))
    radius = max(math.hypot(p[0] - center[0], p[1] - center[1]) for p in points)
    return {"center": center, "radius": radius, "height": high[2] - low[2], "bottom": low[2]}


def bead_smooth_flags(triangles, layout, side_ranges=None, ring_vertices=32):
    """Only the explicitly tagged Rust producer promises side/side/start/end quads."""
    if layout is None:
        return [False] * len(triangles)
    if layout == "joined_stadium_v3":
        if type(ring_vertices) is not int or ring_vertices not in (8,12,16,24,32) or not isinstance(side_ranges,list) or not side_ranges:
            raise ValueError("pruned joined beads require explicit side ranges and ring size")
        flags = [False]*len(triangles)
        previous_end = 0
        for item in side_ranges:
            if (not isinstance(item,list) or len(item) != 2 or any(type(v) is not int for v in item)
                    or item[0] < previous_end or item[1] <= item[0] or item[1] > len(triangles)):
                raise ValueError("pruned bead side ranges overlap or exceed the mesh")
            flags[item[0]:item[1]] = [True]*(item[1]-item[0])
            previous_end = item[1]
        return flags
    if layout in ("joined_stadium_v1", "joined_stadium_v2"):
        if type(ring_vertices) is not int or (layout == "joined_stadium_v1" and ring_vertices != 32) or ring_vertices not in (8,12,16,24,32):
            raise ValueError("invalid joined bead ring size")
        if not isinstance(side_ranges,list) or not side_ranges:
            raise ValueError("joined_stadium_v1 requires explicit bead_side_face_ranges")
        flags = [False]*len(triangles)
        previous_end = 0
        for face_range in side_ranges:
            if (not isinstance(face_range,list) or len(face_range) != 2
                    or any(type(value) is not int for value in face_range)):
                raise ValueError("joined bead side range must contain two integers")
            start,end = face_range
            if start < previous_end or start < 0 or end > len(triangles) or end <= start or (end-start)%(ring_vertices*2):
                raise ValueError("joined bead side ranges overlap or exceed the declared mesh")
            for index in range(start,end,2):
                a,b,d = triangles[index]
                aa,dd,c = triangles[index+1]
                if aa != a or dd != d or len({a,b,c,d}) != 4 or b-a != d-c or b-a not in (1,1-ring_vertices):
                    raise ValueError("joined side faces do not match the declared ring contract")
                flags[index:index+2] = [True,True]
            previous_end = end
        return flags
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


def default_camera_half_angle(width, height):
    """70 mm lens / 36 mm horizontal sensor; fit the smaller image angle."""
    return math.atan(36.0 / (2*70.0) * min(1.0, height/width))


def crease_edges(vertices, triangles, crease_degrees=35):
    """Exact-coordinate topology, solely for visible design-edge overlay."""
    canonical = {}
    ids = []
    for index, point in enumerate(vertices):
        ids.append(canonical.setdefault(tuple(point), index))
    edges = {}
    threshold = math.cos(math.radians(crease_degrees))
    for face in triangles:
        a, b, c = (vertices[i] for i in face)
        u, v = ([b[i]-a[i] for i in range(3)], [c[i]-a[i] for i in range(3)])
        normal = (u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0])
        length = math.sqrt(sum(value*value for value in normal))
        if length == 0:
            continue
        normal = tuple(value/length for value in normal)
        for i, j in ((0,1),(1,2),(2,0)):
            key = tuple(sorted((ids[face[i]], ids[face[j]])))
            if key not in edges:
                edges[key] = [normal, 1, False]
            else:
                prior = edges[key]
                prior[1] += 1
                prior[2] = prior[2] or sum(x*y for x,y in zip(prior[0],normal)) < threshold
    return [edge for edge, (_, count, sharp) in edges.items() if count != 2 or sharp]


def add_design_edges(vertices, triangles, displayed_positions, pixel_mm, scene):
    import bpy
    edges = crease_edges(vertices, triangles)
    if len(edges) > 200_000:
        raise ValueError("diagram design-edge budget exceeded (200000); simplify input before diagram export")
    curves = bpy.data.curves.new("DesignEdges", "CURVE")
    curves.dimensions = "3D"
    curves.resolution_u = 1
    curves.bevel_depth = max(pixel_mm * 0.0004, 0.00000001)
    curves.bevel_resolution = 0
    curves.resolution_u = 1
    for a, b in edges:
        spline = curves.splines.new("POLY")
        spline.points.add(1)
        spline.points[0].co = (*displayed_positions[a], 1)
        spline.points[1].co = (*displayed_positions[b], 1)
    edge_object = bpy.data.objects.new("DesignEdges", curves)
    scene.collection.objects.link(edge_object)
    material = bpy.data.materials.new("DesignEdgeInk")
    material.use_nodes = True
    material.node_tree.nodes.clear()
    emission = material.node_tree.nodes.new("ShaderNodeEmission")
    emission.inputs["Color"].default_value = (0.018, 0.022, 0.03, 1)
    output = material.node_tree.nodes.new("ShaderNodeOutputMaterial")
    material.node_tree.links.new(emission.outputs[0], output.inputs["Surface"])
    curves.materials.append(material)


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
    bump.name = "PhysicalLayerBump"
    bump.inputs["Distance"].default_value = 1.0
    bump.inputs["Strength"].default_value = 1.0
    links.new(math_node("MULTIPLY", physical_height, mask), bump.inputs["Height"])
    links.new(bump.outputs["Normal"], nodes.get("Principled BSDF").inputs["Normal"])


def layer_filter_gain(pitch_mm,pixel_footprint_m,view_cosine):
    """Lower bound on projected layer spacing; fade only bump amplitude."""
    pixels=pitch_mm*0.001*abs(view_cosine)/(math.sqrt(2)*max(pixel_footprint_m,1e-15))
    value=max(0,min(1,(pixels-2)/2))
    return value*value*(3-2*value)


def apply_layer_filter(material,camera,resolution_x,resolution_y,pitch_mm):
    from mathutils import Vector
    nodes,links=material.node_tree.nodes,material.node_tree.links
    bump=nodes.get("PhysicalLayerBump")
    if bump is None:
        return
    for existing in list(nodes):
        if existing.name.startswith("TurninatorFootprint"):
            nodes.remove(existing)
    existing_nodes=set(nodes)
    def scalar(operation,a,b=None):
        node=nodes.new("ShaderNodeMath");node.operation=operation
        for index,value in enumerate((a,b)):
            if value is None:continue
            if isinstance(value,(int,float)):node.inputs[index].default_value=value
            else:links.new(value,node.inputs[index])
        return node.outputs[0]
    geometry=nodes.new("ShaderNodeNewGeometry")
    dot=nodes.new("ShaderNodeVectorMath");dot.operation="DOT_PRODUCT"
    links.new(geometry.outputs["True Normal"],dot.inputs[0])
    if camera.data.type=="ORTHO":
        dot.inputs[1].default_value=camera.rotation_euler.to_matrix() @ Vector((0,0,1))
        footprint=camera.data.ortho_scale/(resolution_y if camera.data.sensor_fit=="VERTICAL" else resolution_x)
    else:
        toward=nodes.new("ShaderNodeVectorMath");toward.operation="SUBTRACT"
        toward.inputs[0].default_value=camera.location
        links.new(geometry.outputs["Position"],toward.inputs[1])
        normal=nodes.new("ShaderNodeVectorMath");normal.operation="NORMALIZE"
        links.new(toward.outputs["Vector"],normal.inputs[0]);links.new(normal.outputs["Vector"],dot.inputs[1])
        data=nodes.new("ShaderNodeCameraData")
        sensor,pixels=(camera.data.sensor_height,resolution_y) if camera.data.sensor_fit=="VERTICAL" else (camera.data.sensor_width,resolution_x)
        footprint=scalar("MULTIPLY",data.outputs["View Distance"],sensor/camera.data.lens/pixels)
    # Dividing by |N.V| bounds the surface footprint near grazing incidence.
    # sqrt(2) bounds diagonal pixel support. The physical height field and cap
    # mask stay intact; Strength attenuation avoids differentiating the gain.
    pitch_pixels=scalar("DIVIDE",scalar("MULTIPLY",scalar("ABSOLUTE",dot.outputs["Value"]),pitch_mm*0.001),
                        scalar("MAXIMUM",scalar("MULTIPLY",footprint,math.sqrt(2)),1e-15))
    fraction=scalar("MINIMUM",scalar("MAXIMUM",scalar("DIVIDE",scalar("SUBTRACT",pitch_pixels,2),2),0),1)
    gain=scalar("MULTIPLY",scalar("MULTIPLY",fraction,fraction),scalar("SUBTRACT",3,scalar("MULTIPLY",fraction,2)))
    links.new(gain,bump.inputs["Strength"])
    for node in nodes:
        if node not in existing_nodes:
            node.name="TurninatorFootprint"


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


def studio_floor_mesh(extent):
    """Retain the broad backdrop without giant EEVEE shadow-receiver faces."""
    # A single 200-extent quad produces horizontal shadow bands in EEVEE.
    # Keep its coverage for camera pans, but split it into 12.5-extent cells.
    cells = 16
    vertices = [((x / cells * 200 - 100) * extent,
                 (y / cells * 200 - 100) * extent, -0.00001)
                for y in range(cells + 1) for x in range(cells + 1)]
    faces = []
    for y in range(cells):
        for x in range(cells):
            a = y * (cells + 1) + x
            faces.append((a, a + 1, a + cells + 2, a + cells + 1))
    return vertices, faces


def build_scene(spec, engine, size, samples, transparent, width=None, height=None):
    import bpy
    from mathutils import Euler, Vector, Matrix
    vertices, triangles = read_mesh(spec["mesh_path"])
    if "face_groups" in spec:
        validate_face_groups(spec["face_groups"],len(triangles))
    if "layer_face_ranges" in spec:
        validate_layer_ranges(spec["layer_face_ranges"],len(triangles),spec["geometry_kind"])
    if spec["geometry_kind"] == "original" and (spec.get("build_up",False) or "solid_layer_tops_mm" in spec):
        validate_solid_layer_tops(spec.get("solid_layer_tops_mm"),max(v[2] for v in vertices))
    smooth_flags = bead_smooth_flags(triangles, spec.get("bead_layout"), spec.get("bead_side_face_ranges"), spec.get("bead_ring_vertices",32)) if spec["geometry_kind"] == "beads" else None
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in list(bpy.data.meshes):
        if not block.users:
            bpy.data.meshes.remove(block)
    for block in list(bpy.data.materials):
        if not block.users:
            bpy.data.materials.remove(block)
    for collection in (bpy.data.cameras, bpy.data.lights, bpy.data.curves):
        for block in list(collection):
            if not block.users:
                collection.remove(block)
    for group in list(bpy.data.node_groups):
        if not group.users and group.name.startswith("Turninator"):
            bpy.data.node_groups.remove(group)
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
    if spec["geometry_kind"]=="stepped":
        assign_layer_indices(obj,spec.get("layer_face_ranges"),"stepped")
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
        bsdf.inputs["Anisotropic"].default_value = 0.65
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
    if spec.get("face_groups"):
        apply_face_groups(obj,material,spec["face_groups"])
    build_layers = None
    if spec.get("build_up",False):
        if spec["geometry_kind"] == "original":
            build_layers = validate_solid_layer_tops(spec.get("solid_layer_tops_mm"))
        else:
            build_layers = (validate_layer_ranges(spec.get("layer_face_ranges"),len(triangles),spec["geometry_kind"])
                            if spec.get("section") is not None
                            else apply_buildup(obj,spec.get("layer_face_ranges"),spec["geometry_kind"]))
    section_analysis = None
    if spec.get("section") is not None and not spec.get("build_up",False):
        section_analysis = apply_section(obj,spec["section"],rotation,(cx,cy),bottom,
                                         max(envelope["radius"]*2,envelope["height"],0.001),spec["geometry_kind"])
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
    half_angle = default_camera_half_angle(scene.render.resolution_x, scene.render.resolution_y)
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
        if spec.get("fit_turntable", False):
            target = Vector((0, 0, envelope["height"] * 0.5))
            half = math.radians(chosen["vertical_fov_degrees"]) * 0.5
            aspect = scene.render.resolution_x / scene.render.resolution_y
            half = math.atan(math.tan(half) * min(aspect, 1))
            distance = sphere_radius / math.sin(half) * 1.1
            camera.location = target + direction * distance
            camera_data.ortho_scale = 2 * sphere_radius * 1.1 / min(aspect, 1)
            camera_data.clip_start = max(0.000001, distance * 0.0001)
            camera_data.clip_end = distance * 100
    extent = max(radius * 2, height, 0.001)
    if spec.get("layer_appearance",False) and spec.get("layer_filtering",True):
        for layer_material in obj.data.materials:
            apply_layer_filter(layer_material,camera,scene.render.resolution_x,scene.render.resolution_y,
                               min(spec["layer_height_mm"],spec.get("first_layer_height_mm",spec["layer_height_mm"])))
    studio = spec.get("studio_preset", "light")
    if studio == "diagram" and spec["geometry_kind"] == "original" and not spec.get("build_up",False):
        pixel_mm = spec.get("camera", {}).get("ortho_span_mm", extent * 1000) / scene.render.resolution_y
        add_design_edges(vertices, triangles, points, pixel_mm, scene)
    if not transparent and studio != "diagram":
        floor_mesh = bpy.data.meshes.new("StudioFloorGeometry")
        floor_vertices, floor_faces = studio_floor_mesh(extent)
        floor_mesh.from_pydata(floor_vertices, [], floor_faces)
        floor_mesh.update()
        floor = bpy.data.objects.new("StudioFloor", floor_mesh)
        scene.collection.objects.link(floor)
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
    warnings=list(spec["warnings"])
    if preset=="resin" and spec["geometry_kind"]=="beads":
        warnings.append("Resin on bead meshes retains internal/overlapping interfaces; transmission is an optical illustration, not a fused-resin simulation.")
    if preset=="resin" and spec["geometry_kind"]=="stepped":
        warnings.append("Stepped solids retain closed layer interfaces; resin transmission is an optical illustration and may show internal layer surfaces.")
    return obj, {"geometry_hash": digest.hexdigest(), "vertices": len(vertices),
                 "triangles": len(triangles), "bounds_m": envelope,
                 "device": device_info, "warnings": warnings,"section_analysis":section_analysis,"build_layers":build_layers}
