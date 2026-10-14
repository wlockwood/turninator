//! Bounded experimental mid-layer stepped envelope, not nozzle toolpath slicing.
use super::schedule::LayerSchedule;
use crate::{Result, mesh::Mesh};
use i_overlay::{
    core::{fill_rule::FillRule, overlay::ShapeType, overlay_rule::OverlayRule},
    float::overlay::{FloatOverlay, OverlayOptions},
    i_float::{adapter::FloatPointAdapter, float::rect::FloatRect},
};
use std::collections::{BTreeMap, BTreeSet};

type Point = [f64; 2];
type Contour = Vec<Point>;
type Shapes = Vec<Vec<Contour>>;
const WORK_BUDGET: usize = 100_000_000;

pub struct SteppedEnvelope {
    pub mesh: Mesh,
    pub sample_heights_mm: Vec<f64>,
    pub coordinate_tolerance_mm: f64,
    pub warnings: Vec<String>,
}
pub fn slice_stepped(mesh: &Mesh, boundaries_mm: &[f64]) -> Result<Mesh> {
    Ok(slice_stepped_with_report(mesh, boundaries_mm)?.mesh)
}

/// Mid-interval cross sections are extruded to the scheduled boundaries; the last
/// interval is clipped to the actual source top. Closed oriented inputs only.
/// Integer overlay uses one shared 48-bit grid; unresolved features fail explicitly.
pub fn slice_stepped_with_report(mesh: &Mesh, boundaries_mm: &[f64]) -> Result<SteppedEnvelope> {
    let bounds = mesh.bounds()?;
    validate_closed(mesh)?;
    if bounds.min[2] != 0.
        || bounds.max[2] <= 0.
        || bounds
            .min
            .iter()
            .chain(bounds.max.iter())
            .any(|v| v.abs() > 1e7)
    {
        return Err(
            "stepped envelope requires build minimum Z=0 and bounded millimeter coordinates".into(),
        );
    }
    let schedule = LayerSchedule::explicit(boundaries_mm.to_vec())?.clipped(bounds.max[2])?;
    let intervals = schedule.boundaries_mm.len() - 1;
    if intervals > 2000 || intervals.saturating_mul(mesh.triangles.len()) > 10_000_000 {
        return Err(
            "experimental slice work budget exceeded (2000 intervals / 10M triangle-plane tests)"
                .into(),
        );
    }
    let rect = FloatRect {
        min_x: bounds.min[0],
        max_x: bounds.max[0],
        min_y: bounds.min[1],
        max_y: bounds.max[1],
    };
    let adapter = FloatPointAdapter::<Point, i64>::with_coordinate_bits(rect, 48);
    let tolerance =
        (bounds.max[0] - bounds.min[0]).max(bounds.max[1] - bounds.min[1]) * 2f64.powi(-45);
    let mut sections = Vec::new();
    let mut sample_heights_mm = Vec::new();
    let mut work = 0usize;
    for pair in schedule.boundaries_mm.windows(2) {
        let mut z = (pair[0] + pair[1]) * 0.5;
        // Deterministic one-ULP side selection avoids ambiguous coplanar triangles.
        while mesh.positions.iter().any(|p| p[2] == z) {
            z = z.next_up();
            if z >= pair[1] {
                return Err("slice interval cannot resolve a noncoplanar sample".into());
            }
        }
        sample_heights_mm.push(z);
        let mut contours = section(mesh, z)?;
        let count: usize = contours.iter().map(Vec::len).sum();
        work = work.saturating_add(count.saturating_mul(count));
        if work > WORK_BUDGET {
            return Err("contour intersection work budget exceeded".into());
        }
        validate_contours(&contours, tolerance)?;
        for contour in &mut contours {
            let mut seen = BTreeSet::new();
            for point in contour {
                let integer = adapter.float_to_int(point);
                if !seen.insert((integer.x, integer.y)) {
                    return Err(
                        "slice feature collapses on the bounded overlay grid; no repair performed"
                            .into(),
                    );
                }
                *point = adapter.int_to_float(&integer);
            }
        }
        sections.push(group_contours(contours)?);
    }
    let mut builder = Builder::default();
    for (index, shapes) in sections.iter().enumerate() {
        let bottom = schedule.boundaries_mm[index];
        let top = schedule.boundaries_mm[index + 1];
        for contour in shapes.iter().flatten() {
            for i in 0..contour.len() {
                let a = contour[i];
                let b = contour[(i + 1) % contour.len()];
                builder.triangle([
                    [a[0], a[1], bottom],
                    [b[0], b[1], bottom],
                    [b[0], b[1], top],
                ])?;
                builder.triangle([[a[0], a[1], bottom], [b[0], b[1], top], [a[0], a[1], top]])?;
            }
        }
        if index == 0 {
            cap(&mut builder, shapes, bottom, false)?;
        }
        if index + 1 == sections.len() {
            cap(&mut builder, shapes, top, true)?;
        } else {
            let above = &sections[index + 1];
            cap(
                &mut builder,
                &difference(shapes, above, &adapter),
                top,
                true,
            )?;
            cap(
                &mut builder,
                &difference(above, shapes, &adapter),
                top,
                false,
            )?;
        }
    }
    let mut output = split_boundary_edges(builder.mesh, tolerance)
        .map_err(|e| format!("edge conformance: {e}"))?;
    // Remove unused vertices only; no geometry, edges or gaps are repaired.
    compact(&mut output);
    output.bounds()?;
    validate_closed(&output)?;
    Ok(SteppedEnvelope {mesh:output,sample_heights_mm,coordinate_tolerance_mm:tolerance,
        warnings:vec!["Experimental mid-layer stepped envelope; this is a sliced-solid approximation, not deposited nozzle paths. Small features between sample planes may be absent by the selected sampling rule.".into(),format!("Boolean overlay uses a shared 48-bit coordinate grid; boundary stitching tolerance {tolerance:e} mm. Open, branching, touching/self-intersecting or unresolved contours are rejected without gap closure or topology repair.")]})
}

fn validate_closed(mesh: &Mesh) -> Result<()> {
    let mut edges = BTreeMap::<(u32, u32), (u32, i32)>::new();
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|i| mesh.positions[i as usize]);
        if cross3(sub3(b, a), sub3(c, a)) == [0.; 3] {
            return Err("slice input/output contains a degenerate triangle".into());
        }
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let value = edges.entry((a.min(b), a.max(b))).or_default();
            value.0 += 1;
            value.1 += if a < b { 1 } else { -1 };
        }
    }
    if edges
        .values()
        .any(|&(count, balance)| count != 2 || balance != 0)
    {
        return Err("stepped envelope requires a closed consistently oriented manifold; open/nonmanifold edges are not repaired".into());
    }
    Ok(())
}

fn section(mesh: &Mesh, z: f64) -> Result<Vec<Contour>> {
    let mut points = BTreeMap::<(u32, u32), Point>::new();
    let mut neighbors = BTreeMap::<(u32, u32), Vec<(u32, u32)>>::new();
    for triangle in &mesh.triangles {
        let mut hits = Vec::new();
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let key = (a.min(b), a.max(b));
            let p = mesh.positions[key.0 as usize];
            let q = mesh.positions[key.1 as usize];
            if (p[2] < z) == (q[2] < z) {
                continue;
            }
            let t = (z - p[2]) / (q[2] - p[2]);
            points
                .entry(key)
                .or_insert([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            hits.push(key);
        }
        if hits.is_empty() {
            continue;
        }
        if hits.len() != 2 {
            return Err("ambiguous triangle-plane intersection".into());
        }
        neighbors.entry(hits[0]).or_default().push(hits[1]);
        neighbors.entry(hits[1]).or_default().push(hits[0]);
    }
    if neighbors.values().any(|n| n.len() != 2) {
        return Err("slice contour is open or branching; no gap closure performed".into());
    }
    let mut visited = BTreeSet::new();
    let mut contours = Vec::new();
    for &start in neighbors.keys() {
        if visited.contains(&start) {
            continue;
        }
        let mut contour = Vec::new();
        let mut current = start;
        let mut previous = None;
        loop {
            if !visited.insert(current) {
                if current == start {
                    break;
                }
                return Err("slice contours intersect topologically".into());
            }
            contour.push(points[&current]);
            let adjacent = &neighbors[&current];
            let next = if Some(adjacent[0]) == previous {
                adjacent[1]
            } else {
                adjacent[0]
            };
            previous = Some(current);
            current = next;
        }
        if contour.len() < 3 || signed_area(&contour) == 0. {
            return Err("slice contour has zero area".into());
        }
        contours.push(contour);
    }
    Ok(contours)
}
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn signed_area(points: &[Point]) -> f64 {
    (0..points.len())
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        * 0.5
}
fn inside(point: Point, contour: &[Point]) -> bool {
    let mut inside = false;
    for i in 0..contour.len() {
        let a = contour[i];
        let b = contour[(i + 1) % contour.len()];
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}
fn point_distance(p: Point, a: Point, b: Point) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length = d[0] * d[0] + d[1] * d[1];
    if length == 0. {
        return (p[0] - a[0]).hypot(p[1] - a[1]);
    }
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length).clamp(0., 1.);
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}
fn validate_contours(contours: &[Contour], tolerance: f64) -> Result<()> {
    let edges: Vec<_> = contours
        .iter()
        .enumerate()
        .flat_map(|(c, contour)| {
            (0..contour.len()).map(move |i| (c, i, contour[i], contour[(i + 1) % contour.len()]))
        })
        .collect();
    for (i, &(ca, ia, a, b)) in edges.iter().enumerate() {
        for &(cb, ib, c, d) in &edges[i + 1..] {
            if ca == cb
                && ((ia + 1) % contours[ca].len() == ib || (ib + 1) % contours[ca].len() == ia)
            {
                continue;
            }
            if (0..2).any(|axis| {
                a[axis].max(b[axis]) + tolerance < c[axis].min(d[axis])
                    || c[axis].max(d[axis]) + tolerance < a[axis].min(b[axis])
            }) {
                continue;
            }
            let intersects =
                cross(a, b, c) * cross(a, b, d) < 0. && cross(c, d, a) * cross(c, d, b) < 0.;
            let distance = point_distance(a, c, d)
                .min(point_distance(b, c, d))
                .min(point_distance(c, a, b))
                .min(point_distance(d, a, b));
            if intersects || distance <= tolerance {
                return Err("slice contours intersect, touch or contain features below coordinate resolution; no repair performed".into());
            }
        }
    }
    Ok(())
}
fn group_contours(mut contours: Vec<Contour>) -> Result<Shapes> {
    let depths: Vec<_> = contours
        .iter()
        .enumerate()
        .map(|(i, c)| {
            contours
                .iter()
                .enumerate()
                .filter(|(j, other)| *j != i && inside(c[0], other))
                .count()
        })
        .collect();
    for (i, contour) in contours.iter_mut().enumerate() {
        if (signed_area(contour) > 0.) != (depths[i] % 2 == 0) {
            contour.reverse();
        }
    }
    let mut result = Vec::new();
    for i in 0..contours.len() {
        if depths[i] % 2 != 0 {
            continue;
        }
        let mut shape = vec![contours[i].clone()];
        for j in 0..contours.len() {
            if depths[j] == depths[i] + 1 && inside(contours[j][0], &contours[i]) {
                shape.push(contours[j].clone());
            }
        }
        result.push(shape);
    }
    Ok(result)
}
fn difference(subject: &Shapes, clip: &Shapes, adapter: &FloatPointAdapter<Point, i64>) -> Shapes {
    let mut options = OverlayOptions::default();
    options.preserve_input_collinear = true;
    options.preserve_output_collinear = true;
    options.clean_result = false;
    FloatOverlay::<Point, i64>::new_custom(adapter.clone(), options, Default::default(), 0)
        .unsafe_add_source(subject, ShapeType::Subject)
        .unsafe_add_source(clip, ShapeType::Clip)
        .overlay(OverlayRule::Difference, FillRule::EvenOdd)
}
#[derive(Default)]
struct Builder {
    mesh: Mesh,
    indices: BTreeMap<[u64; 3], u32>,
}
impl Builder {
    fn vertex(&mut self, p: [f64; 3]) -> u32 {
        let key = p.map(|v| if v == 0. { 0 } else { v.to_bits() });
        *self.indices.entry(key).or_insert_with(|| {
            let index = self.mesh.positions.len() as u32;
            self.mesh.positions.push(p);
            index
        })
    }
    fn triangle(&mut self, p: [[f64; 3]; 3]) -> Result<()> {
        if self.mesh.triangles.len() >= crate::mesh::MAX_TRIANGLES
            || self.mesh.positions.len() + 3 > crate::mesh::MAX_VERTICES
        {
            return Err("stepped mesh output budget exceeded".into());
        }
        if cross3(sub3(p[1], p[0]), sub3(p[2], p[0])) == [0.; 3] {
            return Err(format!("stepped triangulation generated a degenerate face: {p:?}").into());
        }
        let t = p.map(|p| self.vertex(p));
        self.mesh.triangles.push(t);
        Ok(())
    }
}
fn cap(builder: &mut Builder, shapes: &Shapes, z: f64, up: bool) -> Result<()> {
    for shape in shapes {
        let mut vertices = Vec::new();
        let mut holes = Vec::new();
        for (index, contour) in shape.iter().enumerate() {
            if index > 0 {
                holes.push(vertices.len() / 2);
            }
            for point in contour {
                vertices.extend(point);
            }
        }
        let triangles = earcutr::earcut(&vertices, &holes, 2)
            .map_err(|_| "stepped cap triangulation failed")?;
        for indices in triangles.chunks_exact(3) {
            let mut p = std::array::from_fn::<_, 3, _>(|i| {
                [vertices[indices[i] * 2], vertices[indices[i] * 2 + 1], z]
            });
            if (cross([p[0][0], p[0][1]], [p[1][0], p[1][1]], [p[2][0], p[2][1]]) > 0.) != up {
                p.swap(1, 2);
            }
            builder.triangle(p)?;
        }
    }
    Ok(())
}
fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Split exactly shared layer-boundary edges at existing boolean vertices. This
/// adds topology only; it never closes a gap or removes a feature.
fn split_boundary_edges(mesh: Mesh, tolerance: f64) -> Result<Mesh> {
    if mesh.positions.len().saturating_mul(mesh.triangles.len()) > WORK_BUDGET {
        return Err("experimental mesh edge-conformance budget exceeded".into());
    }
    let mut result = Builder::default();
    for triangle in &mesh.triangles {
        let p = triangle.map(|i| mesh.positions[i as usize]);
        let mut perimeter = Vec::new();
        for i in 0..3 {
            let a = p[i];
            let b = p[(i + 1) % 3];
            perimeter.push(a);
            if a[2] != b[2] {
                continue;
            }
            let delta = [b[0] - a[0], b[1] - a[1]];
            let length2 = delta[0] * delta[0] + delta[1] * delta[1];
            let mut splits = Vec::new();
            for candidate in &mesh.positions {
                if candidate[2] != a[2] || *candidate == a || *candidate == b {
                    continue;
                }
                let t =
                    ((candidate[0] - a[0]) * delta[0] + (candidate[1] - a[1]) * delta[1]) / length2;
                if t > 0.
                    && t < 1.
                    && point_distance([candidate[0], candidate[1]], [a[0], a[1]], [b[0], b[1]])
                        <= tolerance
                {
                    splits.push((t, *candidate));
                }
            }
            splits.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, point) in splits {
                if perimeter.last() != Some(&point) {
                    perimeter.push(point);
                }
            }
        }
        if perimeter.len() == 3 {
            result.triangle(p)?;
        } else {
            let center = std::array::from_fn(|i| (p[0][i] + p[1][i] + p[2][i]) / 3.);
            for i in 0..perimeter.len() {
                result.triangle([center, perimeter[i], perimeter[(i + 1) % perimeter.len()]])?;
            }
        }
    }
    Ok(result.mesh)
}
fn compact(mesh: &mut Mesh) {
    let used: BTreeSet<_> = mesh.triangles.iter().flatten().copied().collect();
    let mut remap = BTreeMap::new();
    let mut positions = Vec::new();
    for index in used {
        remap.insert(index, positions.len() as u32);
        positions.push(mesh.positions[index as usize]);
    }
    for triangle in &mut mesh.triangles {
        for index in triangle {
            *index = remap[index];
        }
    }
    mesh.positions = positions;
}
