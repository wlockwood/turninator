//! Shared scene geometry recipe v1. All geometry and normals remain in build mm.
//! Placement is explicit in resolve_display_transform; cuts never refit a model.
pub use super::ink::expand_design_edges;
use super::{contract::*, materials::resolve_materials};
use crate::{
    Result,
    export::SectionSpec,
    mesh::{MAX_TRIANGLES, MAX_VERTICES, Mesh},
    print::slice::BuildUpSolid,
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

type Point = [f64; 3];
#[derive(Clone, Copy)]
struct Vertex {
    p: Point,
    n: Point,
    id: u32,
}
#[derive(Clone)]
struct Face {
    v: [Vertex; 3],
    material: u32,
    shell: usize,
}
struct Component {
    mesh: Mesh,
    source_faces: Vec<usize>,
}

/// Derived mesh budgets include vertex splitting, normals, build coordinates and indices.
/// Empty builds are legal. Call before allocation/expansion, including in adapters.
pub fn check_derived_geometry_budget(vertices: usize, triangles: usize) -> Result<()> {
    let bytes = vertices
        .checked_mul(48)
        .and_then(|v| triangles.checked_mul(16).and_then(|t| v.checked_add(t)))
        .ok_or("derived geometry allocation overflow")?;
    if vertices > MAX_VERTICES || triangles > MAX_TRIANGLES || bytes > 2 * 1024 * 1024 * 1024 {
        return Err("derived render capacity budget exceeded".into());
    }
    Ok(())
}

pub fn realize_frame(scene: &RenderSceneSnapshot, frame: &FrameSpec) -> Result<RealizedScene> {
    scene.mesh.bounds()?;
    if !frame.angle_degrees.is_finite() {
        return Err("invalid frame angle".into());
    }
    if let Some(section) = &frame.section
        && (section.axis > 2 || !section.position_mm.is_finite())
    {
        return Err("invalid section plane".into());
    }
    let materials = resolve_materials(scene)?;
    let cap_material = materials.materials.len() as u32 - 1;
    let selected = visible_faces(scene, frame.visible_layers)?;
    check_face_budget(selected.len())?;
    let mut generated = GeneratedVertices {
        next: scene.mesh.positions.len() as u32,
        edges: BTreeMap::new(),
    };
    let stage = if scene.metadata.kind == GeometryKind::Original {
        frame
            .visible_layers
            .filter(|&n| n > 0 && (n as usize) < scene.metadata.solid_layer_tops_mm.len())
            .map(|n| SectionSpec {
                axis: 2,
                // Legacy schedules are absolute BUILD-Z planes, not heights
                // relative to the source minimum. cap_faces handles any local
                // BuildUpSolid coordinate conversion at that API boundary.
                position_mm: scene.metadata.solid_layer_tops_mm[n as usize - 1],
                keep_positive: false,
                cap: true,
            })
    } else {
        None
    };
    let flags = smooth_flags(scene)?;
    let mut smooth_normals = vec![[0.; 3]; scene.mesh.positions.len()];
    for (index, t) in scene.mesh.triangles.iter().enumerate() {
        if flags[index] {
            let [a, b, c] = t.map(|i| scene.mesh.positions[i as usize]);
            let n = cross(sub(b, a), sub(c, a));
            for &v in t {
                smooth_normals[v as usize] = add(smooth_normals[v as usize], n);
            }
        }
    }
    let components = components(scene, &selected)?;
    // Measurements are read-only and precede section clipping. In particular,
    // measuring a deposited stage must not enable the cut path's winding repair.
    let deposited_build = if scene.metadata.kind != GeometryKind::Original {
        frame
            .visible_layers
            .map(|count| deposited_build_analysis(scene, count, &components, selected.len()))
            .transpose()?
    } else {
        None
    };
    let mut staged = Vec::new();
    let mut section_caps = Vec::new();
    let mut all_closed = !components.is_empty();
    let mut source_volume = 0.;
    for (shell, mut component) in components.into_iter().enumerate() {
        let solid = if stage.is_some()
            || frame.section.is_some()
            || (scene.metadata.kind == GeometryKind::Original && frame.visible_layers.is_some())
        {
            BuildUpSolid::new(&component.mesh).ok()
        } else {
            None
        };
        all_closed &= solid.is_some();
        let mut faces = Vec::new();
        for &index in &component.source_faces {
            let t = scene.mesh.triangles[index];
            let points = t.map(|i| scene.mesh.positions[i as usize]);
            let n = unit(cross(sub(points[1], points[0]), sub(points[2], points[0])))?;
            faces.push(Face {
                v: std::array::from_fn(|i| Vertex {
                    p: points[i],
                    id: t[i],
                    n: if flags[index] {
                        unit(smooth_normals[t[i] as usize]).unwrap_or(n)
                    } else {
                        n
                    },
                }),
                material: materials.triangle_materials[index],
                shell,
            });
        }
        // Closure accepts either globally consistent orientation. Normalize only
        // the disposable closed component, including all of an original solid's
        // cavity shells together, so caps and coplanar culling have outward
        // source faces without changing the accepted mesh or cavity relations.
        if solid.is_some() && signed_volume(&faces, scene.reference_bounds_mm.min) < 0. {
            for face in &mut faces {
                face.v.swap(1, 2);
                for vertex in &mut face.v {
                    vertex.n = scale(vertex.n, -1.);
                }
            }
            for triangle in &mut component.mesh.triangles {
                triangle.swap(1, 2);
            }
        }
        source_volume += volume(&faces, scene.reference_bounds_mm.min);
        if let Some(plane) = &stage {
            faces = clip_faces(faces, plane, &mut generated)?;
            if solid.is_some() {
                faces.extend(cap_faces(&component.mesh, plane, 0, shell, &mut generated)?);
            }
        }
        checked_extend(&mut staged, faces)?;
        if let Some(plane) = &frame.section
            && solid.is_some()
            && plane.cap
            && !stage.as_ref().is_some_and(|build| {
                build.axis == plane.axis && build.position_mm == plane.position_mm
            })
        {
            let mut caps = cap_faces(&component.mesh, plane, cap_material, shell, &mut generated)?;
            if let Some(build) = &stage {
                caps = clip_faces(caps, build, &mut generated)?;
            }
            checked_extend(&mut section_caps, caps)?;
        }
    }
    let original_volume = all_closed.then(|| volume(&staged, scene.reference_bounds_mm.min));
    let build_analysis = if scene.metadata.kind == GeometryKind::Original {
        frame.visible_layers.map(|count| -> Result<BuildAnalysis> {
            let mut value = if let Some(plane) = &stage {
                let area = all_closed.then(|| staged.iter().filter(|f| f.v.iter().all(|v| v.p[2] == plane.position_mm)).map(face_area).sum::<f64>());
                json!({"axis":2,"position_mm":plane.position_mm,"keep_positive":false,
                    "capped":all_closed,"closed_input":all_closed,"cross_section_area_mm2":area,
                    "original_volume_mm3":all_closed.then_some(source_volume),"remaining_volume_mm3":original_volume,
                    "volume_basis":"Original: signed closed-mesh volume magnitude, additive for separate shells. Remaining: closed cut-mesh volume magnitude.",
                    "output_triangles":staged.len(),"warnings":if all_closed {vec![]} else {vec!["Open or inconsistently wound geometry: section remains uncapped; area/volume unavailable."]},
                    "meaning":"Geometric intersection of the modeled mesh; not strength, mass, or measured print behavior."})
            } else {
                json!({"capped":false,"closed_input":all_closed,"remaining_volume_mm3":if count==0 {Some(0.)} else {original_volume},"output_triangles":staged.len()})
            };
            value["visible_layer"] = json!(i64::from(count)-1);
            value["z_top_mm"] = json!(if count==0 {0.} else {scene.metadata.solid_layer_tops_mm[count as usize-1]});
            BuildAnalysis::from_legacy(value)
        }).transpose()?
    } else {
        deposited_build
    };
    let mut analysis = None;
    let mut output = staged;
    if let Some(section) = &frame.section {
        output = clip_faces(output, section, &mut generated)?;
        let do_cap = section.cap && all_closed;
        if do_cap {
            checked_extend(&mut output, section_caps)?;
        }
        // Include already-existing coplanar source faces once, matching legacy analysis.
        let area = do_cap.then(|| {
            output
                .iter()
                .filter(|f| {
                    f.v.iter()
                        .all(|v| v.p[section.axis as usize] == section.position_mm)
                })
                .map(face_area)
                .sum::<f64>()
        });
        if do_cap {
            for f in &mut output {
                if f.v
                    .iter()
                    .all(|v| v.p[section.axis as usize] == section.position_mm)
                {
                    f.material = cap_material;
                }
            }
        }
        let remaining = if output.is_empty() {
            Some(0.)
        } else if all_closed && (do_cap || !intersects(scene, section, stage.as_ref())) {
            Some(volume(&output, scene.reference_bounds_mm.min))
        } else {
            None
        };
        let mut warnings = Vec::new();
        if scene.metadata.kind == GeometryKind::Beads {
            warnings.push("Bead source volume is an algebraic mesh-volume estimate; overlaps between paths are not assessed.");
        }
        if section.cap
            && !all_closed
            && (!selected.is_empty() || scene.metadata.kind == GeometryKind::Original)
        {
            warnings.push("Open or inconsistently wound geometry: section remains uncapped; area/volume unavailable.");
        }
        let legacy = json!({"axis":section.axis,"position_mm":section.position_mm,"keep_positive":section.keep_positive,"capped":do_cap,"closed_input":all_closed,"cross_section_area_mm2":area,"original_volume_mm3":original_volume,"remaining_volume_mm3":remaining,"volume_basis":if scene.metadata.kind==GeometryKind::Stepped{"Additive volumes of independently closed, nonoverlapping stepped layers."}else{"Original: signed closed-mesh volume magnitude, additive for separate shells. Remaining: closed cut-mesh volume magnitude."},"output_triangles":output.len(),"warnings":warnings,"meaning":"Geometric intersection of the modeled mesh; not strength, mass, or measured print behavior."});
        let mut legacy = if frame.visible_layers == Some(0)
            && scene.metadata.kind != GeometryKind::Original
        {
            json!({"axis":section.axis,"position_mm":section.position_mm,"keep_positive":section.keep_positive,"visible_layer":-1,"visible_source_triangles":0,"output_triangles":0,"capped":false,"closed_input":false,"cross_section_area_mm2":0.0,"original_volume_mm3":0.0,"remaining_volume_mm3":0.0,"warnings":[],"meaning":"Empty build stage: no deposited geometry is visible."})
        } else {
            legacy
        };
        if scene.metadata.kind == GeometryKind::Stepped && frame.visible_layers != Some(0) {
            legacy["meaning"] = json!(
                "Geometric intersection of the modeled stepped layers; not strength, mass, or measured print behavior."
            );
        }
        if let Some(count) = frame.visible_layers {
            let mut layers: Vec<_> = scene
                .metadata
                .layer_ranges
                .iter()
                .map(|r| r.layer_index)
                .collect();
            layers.dedup();
            legacy["visible_layer"] = json!(if count == 0 {
                -1
            } else if scene.metadata.kind == GeometryKind::Original {
                i64::from(count) - 1
            } else {
                i64::from(layers[count as usize - 1])
            });
            if scene.metadata.kind != GeometryKind::Original {
                legacy["visible_source_triangles"] = json!(selected.len());
            }
        }
        analysis = Some(SectionAnalysis::from_legacy(legacy, all_closed)?);
    }
    let count = output
        .len()
        .checked_mul(3)
        .ok_or("derived geometry allocation overflow")?;
    check_face_budget(output.len())?;
    let mut mesh = Mesh::default();
    mesh.positions.try_reserve_exact(count.min(MAX_VERTICES))?;
    mesh.triangles.try_reserve_exact(output.len())?;
    let mut normals = Vec::new();
    normals.try_reserve_exact(count.min(MAX_VERTICES))?;
    let mut build_positions_mm = Vec::new();
    build_positions_mm.try_reserve_exact(count.min(MAX_VERTICES))?;
    let mut triangle_materials = Vec::new();
    triangle_materials.try_reserve_exact(output.len())?;
    let mut indices = BTreeMap::<(usize, u32, [u32; 3]), u32>::new();
    for face in &output {
        let rounded = face.v.map(|v| v.p.map(|n| n as f32 as f64));
        if cross(sub(rounded[1], rounded[0]), sub(rounded[2], rounded[0])) == [0.; 3] {
            return Err("derived geometry collapses at f32 adapter precision; rebase coordinates or change the cut plane".into());
        }
        let mut triangle = [0; 3];
        for (i, v) in face.v.iter().enumerate() {
            let p = v.p.map(|n| n as f32);
            if p.iter().any(|v| !v.is_finite()) {
                return Err("derived geometry exceeds f32 coordinate range".into());
            }
            let normal = unit(v.n)?.map(|n| n as f32);
            let key = (
                face.shell,
                v.id,
                normal.map(|v| if v == 0. { 0 } else { v.to_bits() }),
            );
            triangle[i] = if let Some(&id) = indices.get(&key) {
                id
            } else {
                check_derived_geometry_budget(mesh.positions.len() + 1, output.len())?;
                let id = mesh.positions.len() as u32;
                mesh.positions.push(v.p);
                build_positions_mm.push(p);
                normals.push(normal);
                indices.insert(key, id);
                id
            };
        }
        mesh.triangles.push(triangle);
        triangle_materials.push(face.material);
    }
    let edges = if matches!(
        frame.key.panel,
        RenderPanel::Top | RenderPanel::Iso | RenderPanel::Front | RenderPanel::Right
    ) {
        design_edges(&output)?
    } else {
        Vec::new()
    };
    Ok(RealizedScene {
        mesh: Arc::new(mesh),
        normals,
        build_positions_mm,
        triangle_materials,
        design_edges_mm: edges,
        reference_bounds_mm: scene.reference_bounds_mm,
        analysis,
        build_analysis,
    })
}

/// Whole deposited layers are selected, never clipped or capped for build-up.
/// Sum the existing component volumes only when each component is closed and
/// consistently wound. Bead overlaps are not a union and remain explicitly unknown.
fn deposited_build_analysis(
    scene: &RenderSceneSnapshot,
    count: u32,
    components: &[Component],
    triangles: usize,
) -> Result<BuildAnalysis> {
    if count == 0 {
        return BuildAnalysis::from_legacy(json!({
            "visible_layer":-1,"z_top_mm":0.,"output_triangles":0,
            "capped":false,"closed_input":false,"remaining_volume_mm3":0.,"warnings":[]
        }));
    }
    let mut layers: Vec<_> = scene
        .metadata
        .layer_ranges
        .iter()
        .map(|r| (r.layer_index, r.z_top_mm))
        .collect();
    layers.dedup();
    let &(index, top) = layers
        .get(count as usize - 1)
        .ok_or("visible layer count outside deposited schedule")?;
    let mut closed = !components.is_empty();
    let mut volume = 0.;
    for component in components {
        closed &= BuildUpSolid::new(&component.mesh).is_ok();
        let origin = component.mesh.positions[0];
        let signed: f64 = component
            .mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| sub(component.mesh.positions[i as usize], origin));
                dot(a, cross(b, c)) / 6.
            })
            .sum();
        volume += signed.abs();
    }
    let mut warnings = Vec::new();
    if scene.metadata.kind == GeometryKind::Beads {
        warnings.push("Bead source volume is an algebraic mesh-volume estimate; overlaps between paths are not assessed.");
    }
    if !closed && triangles != 0 {
        warnings.push("Open or inconsistently wound deposited geometry: volume unavailable.");
    }
    BuildAnalysis::from_legacy(json!({
        "visible_layer":index,"z_top_mm":top,"output_triangles":triangles,
        "capped":false,"closed_input":closed,
        "remaining_volume_mm3":if triangles==0 {Some(0.)} else {closed.then_some(volume)},"warnings":warnings,
        "volume_basis":if scene.metadata.kind==GeometryKind::Stepped {
            "Additive volumes of independently closed, nonoverlapping stepped layers."
        } else {"Algebraic mesh-volume magnitudes of closed bead components; overlaps between paths are not assessed."},
        "meaning":"Whole deposited layer selection before sectioning; not strength, mass, or measured print behavior."
    }))
}

fn visible_faces(scene: &RenderSceneSnapshot, count: Option<u32>) -> Result<Vec<usize>> {
    let Some(count) = count else {
        return Ok((0..scene.mesh.triangles.len()).collect());
    };
    if count == 0 {
        return Ok(vec![]);
    }
    if scene.metadata.kind == GeometryKind::Original {
        if scene.metadata.solid_layer_tops_mm.is_empty() {
            return Err("original build stage requires solid layer tops".into());
        }
        if count as usize > scene.metadata.solid_layer_tops_mm.len() {
            return Err("visible layer count outside schedule".into());
        }
        return Ok(if count == 0 {
            vec![]
        } else {
            (0..scene.mesh.triangles.len()).collect()
        });
    }
    let mut layers: Vec<_> = scene
        .metadata
        .layer_ranges
        .iter()
        .map(|r| r.layer_index)
        .collect();
    layers.dedup();
    if layers.is_empty() || count as usize > layers.len() {
        return Err("visible layer count outside schedule".into());
    }
    let mut result = Vec::new();
    if count > 0 {
        let top = layers[count as usize - 1];
        for r in &scene.metadata.layer_ranges {
            if r.layer_index <= top {
                result.extend(r.start_triangle as usize..r.end_triangle as usize);
            }
        }
    }
    Ok(result)
}
fn components(scene: &RenderSceneSnapshot, selected: &[usize]) -> Result<Vec<Component>> {
    if selected.is_empty() {
        return Ok(vec![]);
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    match scene.metadata.kind {
        GeometryKind::Original => {
            groups.insert(0, selected.to_vec());
        }
        GeometryKind::Stepped => {
            let mut range = 0;
            for &i in selected {
                while range < scene.metadata.layer_ranges.len()
                    && scene.metadata.layer_ranges[range].end_triangle as usize <= i
                {
                    range += 1;
                }
                let r = scene
                    .metadata
                    .layer_ranges
                    .get(range)
                    .ok_or("missing stepped layer range")?;
                groups.entry(r.layer_index as usize).or_default().push(i);
            }
        }
        GeometryKind::Beads => {
            let mut parent: Vec<usize> = (0..selected.len()).collect();
            let mut owner = vec![usize::MAX; scene.mesh.positions.len()];
            fn root(p: &mut [usize], mut i: usize) -> usize {
                while p[i] != i {
                    p[i] = p[p[i]];
                    i = p[i];
                }
                i
            }
            for (local, &i) in selected.iter().enumerate() {
                for &vertex in &scene.mesh.triangles[i] {
                    let slot = &mut owner[vertex as usize];
                    if *slot == usize::MAX {
                        *slot = local;
                    } else {
                        let a = root(&mut parent, local);
                        let b = root(&mut parent, *slot);
                        parent[a] = b;
                    }
                }
            }
            for (local, &i) in selected.iter().enumerate() {
                let group = root(&mut parent, local);
                groups.entry(group).or_default().push(i);
            }
        }
    }
    groups
        .into_values()
        .map(|source_faces| {
            let mut mesh = Mesh::default();
            let mut indices = BTreeMap::new();
            let mut points = BTreeMap::new();
            for &i in &source_faces {
                let mut t = [0; 3];
                for (j, &v) in scene.mesh.triangles[i].iter().enumerate() {
                    let p = scene.mesh.positions[v as usize];
                    let id = if scene.metadata.kind == GeometryKind::Original {
                        *points.entry(point_key(p)).or_insert_with(|| {
                            let id = mesh.positions.len() as u32;
                            mesh.positions.push(p);
                            id
                        })
                    } else {
                        *indices.entry(v).or_insert_with(|| {
                            let id = mesh.positions.len() as u32;
                            mesh.positions.push(p);
                            id
                        })
                    };
                    t[j] = id;
                }
                mesh.triangles.push(t);
            }
            Ok(Component { mesh, source_faces })
        })
        .collect()
}
struct GeneratedVertices {
    next: u32,
    edges: BTreeMap<(u32, u32, u8, u64), u32>,
}
impl GeneratedVertices {
    fn allocate(&mut self) -> Result<u32> {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or("derived vertex identity overflow")?;
        Ok(id)
    }
    fn crossing(&mut self, a: u32, b: u32, plane: &SectionSpec) -> Result<u32> {
        let key = (a.min(b), a.max(b), plane.axis, plane.position_mm.to_bits());
        if let Some(&id) = self.edges.get(&key) {
            return Ok(id);
        }
        let id = self.allocate()?;
        self.edges.insert(key, id);
        Ok(id)
    }
}
fn check_face_budget(count: usize) -> Result<()> {
    let bytes = count
        .checked_mul(std::mem::size_of::<Face>())
        .ok_or("derived render capacity allocation overflow")?;
    if count > MAX_TRIANGLES || bytes > 2 * 1024 * 1024 * 1024 {
        return Err("derived render capacity budget exceeded".into());
    }
    Ok(())
}
fn clip_faces(
    faces: Vec<Face>,
    plane: &SectionSpec,
    generated: &mut GeneratedVertices,
) -> Result<Vec<Face>> {
    let axis = plane.axis as usize;
    let mut result = Vec::new();
    for face in faces {
        let mut polygon = Vec::with_capacity(4);
        for i in 0..3 {
            let a = face.v[i];
            let b = face.v[(i + 1) % 3];
            let inside = |v: Vertex| {
                if plane.keep_positive {
                    v.p[axis] >= plane.position_mm
                } else {
                    v.p[axis] <= plane.position_mm
                }
            };
            if inside(a) {
                polygon.push(a);
            }
            if inside(a) != inside(b) {
                // Sort endpoints so adjacent triangles calculate identical crossings.
                let (a, b) = if point_key(a.p) < point_key(b.p) {
                    (a, b)
                } else {
                    (b, a)
                };
                let t = (plane.position_mm - a.p[axis]) / (b.p[axis] - a.p[axis]);
                let mut p = add(a.p, scale(sub(b.p, a.p), t));
                p[axis] = plane.position_mm;
                let n = unit(add(scale(a.n, 1. - t), scale(b.n, t)))?;
                let id = if a.p[axis] == plane.position_mm {
                    a.id
                } else if b.p[axis] == plane.position_mm {
                    b.id
                } else {
                    generated.crossing(a.id, b.id, plane)?
                };
                polygon.push(Vertex { p, n, id });
            }
        }
        for i in 1..polygon.len().saturating_sub(1) {
            let v = [polygon[0], polygon[i], polygon[i + 1]];
            if cross(sub(v[1].p, v[0].p), sub(v[2].p, v[0].p)) == [0.; 3] {
                continue;
            }
            // A retained halfspace with no depth is empty, not a floating boundary face.
            if v.iter().all(|v| v.p[axis] == plane.position_mm)
                && face.v.iter().all(|v| v.p[axis] == plane.position_mm)
            {
                let normal = cross(sub(v[1].p, v[0].p), sub(v[2].p, v[0].p));
                if (plane.keep_positive && normal[axis] > 0.)
                    || (!plane.keep_positive && normal[axis] < 0.)
                {
                    continue;
                }
            }
            check_face_budget(result.len() + 1)?;
            result.push(Face {
                v,
                material: face.material,
                shell: face.shell,
            });
        }
    }
    Ok(result)
}
fn checked_extend(target: &mut Vec<Face>, faces: Vec<Face>) -> Result<()> {
    let count = target
        .len()
        .checked_add(faces.len())
        .ok_or("derived face count overflow")?;
    check_face_budget(count)?;
    target.try_reserve(faces.len())?;
    target.extend(faces);
    Ok(())
}

fn cap_faces(
    mesh: &Mesh,
    plane: &SectionSpec,
    material: u32,
    shell: usize,
    generated: &mut GeneratedVertices,
) -> Result<Vec<Face>> {
    // Cyclic permutations preserve orientation. Flipping X and Z selects the
    // opposite keep direction while retaining a right-handed cap coordinate frame.
    let axis = plane.axis as usize;
    let sign = if plane.keep_positive { -1. } else { 1. };
    let transform = |p: Point| [sign * p[(axis + 1) % 3], p[(axis + 2) % 3], sign * p[axis]];
    let restore = |p: Point| {
        let mut out = [0.; 3];
        out[(axis + 1) % 3] = sign * p[0];
        out[(axis + 2) % 3] = p[1];
        out[axis] = sign * p[2];
        out
    };
    let rotated = Mesh {
        positions: mesh.positions.iter().map(|&p| transform(p)).collect(),
        triangles: mesh.triangles.clone(),
    };
    let min = rotated
        .positions
        .iter()
        .map(|p| p[2])
        .fold(f64::INFINITY, f64::min);
    let height = sign * plane.position_mm - min;
    if height <= 0. {
        return Ok(vec![]);
    }
    let cap = BuildUpSolid::new(&rotated)?.cap_precise(height)?;
    let mut n = [0.; 3];
    n[axis] = sign;
    let ids: Vec<_> = (0..cap.positions.len())
        .map(|_| generated.allocate())
        .collect::<Result<_>>()?;
    Ok(cap
        .triangles
        .iter()
        .map(|t| Face {
            v: t.map(|i| Vertex {
                p: restore(cap.positions[i as usize]),
                n,
                id: ids[i as usize],
            }),
            material,
            shell,
        })
        .collect())
}
fn volume(faces: &[Face], origin: Point) -> f64 {
    // Deposited components/layers are independently closed and additive. The
    // original solid remains one component so cavity shells still subtract.
    let mut components = BTreeMap::<usize, f64>::new();
    for face in faces {
        *components.entry(face.shell).or_default() += face_volume(face, origin);
    }
    components.values().map(|v| v.abs()).sum()
}
fn signed_volume(faces: &[Face], origin: Point) -> f64 {
    faces
        .iter()
        .map(|face| face_volume(face, origin))
        .sum::<f64>()
}
fn face_volume(face: &Face, origin: Point) -> f64 {
    let [a, b, c] = face.v.map(|v| sub(v.p, origin));
    dot(a, cross(b, c)) / 6.
}
fn face_area(f: &Face) -> f64 {
    let [a, b, c] = f.v.map(|v| v.p);
    dot(cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(c, a))).sqrt() * 0.5
}
fn intersects(
    scene: &RenderSceneSnapshot,
    section: &SectionSpec,
    stage: Option<&SectionSpec>,
) -> bool {
    let a = section.axis as usize;
    let min = scene.reference_bounds_mm.min[a];
    let max = if a == 2 {
        stage.map_or(scene.reference_bounds_mm.max[a], |s| s.position_mm)
    } else {
        scene.reference_bounds_mm.max[a]
    };
    min < section.position_mm && max > section.position_mm
}

fn smooth_flags(scene: &RenderSceneSnapshot) -> Result<Vec<bool>> {
    let mut flags = vec![false; scene.mesh.triangles.len()];
    if scene.metadata.kind != GeometryKind::Beads {
        return Ok(flags);
    }
    match scene.metadata.bead_layout.as_deref() {
        None => {}
        Some("independent_stadium_v1") => {
            if !flags.len().is_multiple_of(4) {
                return Err("incomplete independent bead layout".into());
            }
            for (index, group) in scene.mesh.triangles.chunks_exact(4).enumerate() {
                let [i, j, end_j] = group[0];
                let end_i = group[1][2];
                let mut ids = [i, j, end_i, end_j, group[2][0], group[3][0]];
                ids.sort_unstable();
                if group[1][..2] != [i, end_j]
                    || group[2][1..] != [j, i]
                    || group[3][1..] != [end_i, end_j]
                    || ids.windows(2).any(|p| p[0] == p[1])
                {
                    return Err(
                        "mesh indices do not satisfy declared independent bead layout".into(),
                    );
                }
                flags[index * 4..index * 4 + 2].fill(true);
            }
        }
        Some(layout @ ("joined_stadium_v1" | "joined_stadium_v2" | "joined_stadium_v3")) => {
            let ring = scene.metadata.bead_ring_vertices.unwrap_or(32);
            if ![8, 12, 16, 24, 32].contains(&ring)
                || layout == "joined_stadium_v1" && ring != 32
                || scene.metadata.bead_side_face_ranges.is_empty()
            {
                return Err("invalid joined bead ring layout".into());
            }
            let mut previous = 0;
            for &[start, end] in &scene.metadata.bead_side_face_ranges {
                if start < previous
                    || end <= start
                    || end as usize > flags.len()
                    || layout != "joined_stadium_v3" && !(end - start).is_multiple_of(ring * 2)
                {
                    return Err("invalid joined bead side range".into());
                }
                if layout != "joined_stadium_v3" {
                    for pair in scene.mesh.triangles[start as usize..end as usize].chunks_exact(2) {
                        let [a, b, d] = pair[0];
                        let [aa, dd, c] = pair[1];
                        let mut ids = [a, b, c, d];
                        ids.sort_unstable();
                        let delta = b as i64 - a as i64;
                        if aa != a
                            || dd != d
                            || ids.windows(2).any(|p| p[0] == p[1])
                            || delta != d as i64 - c as i64
                            || ![1, 1 - ring as i64].contains(&delta)
                        {
                            return Err(
                                "joined side faces do not match declared ring contract".into()
                            );
                        }
                    }
                }
                flags[start as usize..end as usize].fill(true);
                previous = end;
            }
        }
        _ => return Err("unsupported bead smoothing layout".into()),
    }
    Ok(flags)
}
fn design_edges(faces: &[Face]) -> Result<Vec<[[f32; 3]; 2]>> {
    type EdgeKey = ([u64; 3], [u64; 3]);
    let mut edges = BTreeMap::<EdgeKey, (Point, Point, Point, usize, bool)>::new();
    for f in faces {
        let [a, b, c] = f.v.map(|v| v.p);
        let n = unit(cross(sub(b, a), sub(c, a)))?;
        for (a, b) in [(a, b), (b, c), (c, a)] {
            let ka = point_key(a);
            let kb = point_key(b);
            let key = if ka < kb { (ka, kb) } else { (kb, ka) };
            let e = edges.entry(key).or_insert((a, b, n, 0, false));
            e.3 += 1;
            e.4 |= dot(e.2, n) < 35f64.to_radians().cos();
        }
    }
    let mut result = Vec::new();
    for (_, (a, b, _, count, sharp)) in edges {
        if count != 2 || sharp {
            if result.len() >= 200_000 {
                return Err("diagram design-edge budget exceeded (200000)".into());
            }
            result.push([a.map(|v| v as f32), b.map(|v| v as f32)]);
        }
    }
    Ok(result)
}
fn point_key(p: Point) -> [u64; 3] {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}
fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: Point) -> Result<Point> {
    let length = dot(a, a).sqrt();
    if !length.is_finite() || length == 0. {
        return Err("invalid or degenerate geometry normal".into());
    }
    Ok(scale(a, 1. / length))
}

/// Full-source physical placement, row-major affine rows. Rz(turn)*Rz*Ry*Rx.
pub fn resolve_display_transform(
    scene: &RenderSceneSnapshot,
    angle_degrees: f64,
) -> Result<[[f64; 4]; 4]> {
    if !angle_degrees.is_finite()
        || scene
            .appearance
            .display_rotation_degrees
            .iter()
            .any(|v| !v.is_finite())
    {
        return Err("invalid display angle".into());
    }
    let [x, y, z] = scene
        .appearance
        .display_rotation_degrees
        .map(f64::to_radians);
    let rotate = |mut p: Point| {
        let (s, c) = x.sin_cos();
        p = [p[0], c * p[1] - s * p[2], s * p[1] + c * p[2]];
        let (s, c) = y.sin_cos();
        p = [c * p[0] + s * p[2], p[1], -s * p[0] + c * p[2]];
        let (s, c) = z.sin_cos();
        [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]]
    };
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for &p in &scene.mesh.positions {
        let p = rotate(p);
        for a in 0..3 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
    }
    let center = [
        min[0] * 0.5 + max[0] * 0.5,
        min[1] * 0.5 + max[1] * 0.5,
        min[2],
    ];
    let (s, c) = angle_degrees.to_radians().sin_cos();
    let spin = |p: Point| [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]];
    let mut out = [[0.; 4]; 4];
    for j in 0..3 {
        let mut axis = [0.; 3];
        axis[j] = 1.;
        let column = spin(rotate(axis));
        for i in 0..3 {
            out[i][j] = column[i];
        }
    }
    let translation = spin(scale(center, -1.));
    for i in 0..3 {
        out[i][3] = translation[i];
    }
    out[3][3] = 1.;
    if out.iter().flatten().any(|v| !v.is_finite()) {
        return Err("nonfinite display placement".into());
    }
    Ok(out)
}

/// Analytic physical bump normal, evaluated at build Z above the source minimum.
/// Adapters evaluate this per shading sample, before the display transform. gain
/// is the shared projected-pitch filter amplitude; it never changes layer pitch.
pub fn layer_surface_normal(
    a: &RenderAppearance,
    build_z_mm: f64,
    normal: Point,
    gain: f64,
) -> Result<Point> {
    let n = unit(normal)?;
    if !build_z_mm.is_finite() || !gain.is_finite() || !(0.0..=1.).contains(&gain) {
        return Err("invalid layer shading coordinate or gain".into());
    }
    if !a.layer_appearance {
        return Ok(n);
    }
    let first = a.first_layer_height_mm;
    let height = a.layer_height_mm;
    if !first.is_finite()
        || first <= 0.
        || !height.is_finite()
        || height <= 0.
        || !a.groove_depth_mm.is_finite()
    {
        return Err("invalid layer appearance".into());
    }
    let (phase, pitch) = if build_z_mm < first {
        (build_z_mm / first, first)
    } else {
        ((build_z_mm - first) / height, height)
    };
    let derivative = std::f64::consts::PI * a.groove_depth_mm / pitch
        * (2. * std::f64::consts::PI * phase).sin()
        * gain
        * (1. - n[2] * n[2]);
    let tangent = sub([0., 0., 1.], scale(n, n[2]));
    unit(sub(n, scale(tangent, derivative)))
}
/// Shared projected-pitch filter, with footprint measured in build millimeters.
/// This is the legacy Blender smoothstep amplitude, with no pitch rescaling.
pub fn layer_filter_gain(pitch_mm: f64, pixel_footprint_mm: f64, view_cosine: f64) -> Result<f64> {
    if !pitch_mm.is_finite()
        || pitch_mm <= 0.
        || !pixel_footprint_mm.is_finite()
        || pixel_footprint_mm <= 0.
        || !view_cosine.is_finite()
        || view_cosine.abs() > 1.
    {
        return Err("invalid physical layer filter inputs".into());
    }
    let pixels = pitch_mm * view_cosine.abs() / (2f64.sqrt() * pixel_footprint_mm.max(1e-12));
    let value = ((pixels - 2.) / 2.).clamp(0., 1.);
    Ok(value * value * (3. - 2. * value))
}
