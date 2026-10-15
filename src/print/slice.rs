//! Bounded experimental mid-layer stepped envelope, not nozzle toolpath slicing.
use super::schedule::LayerSchedule;
use crate::{
    Result,
    mesh::{Bounds, Mesh},
};
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
    /// Every scheduled interval, including empty space between source shells.
    pub layer_face_ranges: Vec<[usize; 2]>,
    pub sample_heights_mm: Vec<f64>,
    pub coordinate_tolerance_mm: f64,
    pub warnings: Vec<String>,
}

/// A validated source solid reused across build-up frames.
pub struct BuildUpSolid {
    mesh: Mesh,
    bounds: Bounds,
}
impl BuildUpSolid {
    pub fn new(mesh: &Mesh) -> Result<Self> {
        let bounds = mesh.bounds()?;
        validate_closed(mesh)?;
        Ok(Self {
            mesh: mesh.clone(),
            bounds,
        })
    }
    /// An upward-facing section of the solid at a height above its build minimum.
    /// Empty and completed builds need no added face. Exact vertex-plane contacts
    /// use the section approached from below, without displacing the cut plane.
    pub fn cap(&self, height_mm: f64) -> Result<Mesh> {
        if !height_mm.is_finite() || height_mm < 0. {
            return Err("invalid solid build-up height".into());
        }
        if height_mm == 0. || height_mm >= self.bounds.max[2] - self.bounds.min[2] {
            return Ok(Mesh::default());
        }
        let z = self.bounds.min[2] + height_mm;
        let contours = section(&self.mesh, z)?;
        let mut work = 0;
        let tolerance = (self.bounds.max[0] - self.bounds.min[0])
            .max(self.bounds.max[1] - self.bounds.min[1])
            * 2f64.powi(-45);
        validate_contours(&contours, tolerance, &mut work)
            .map_err(|error| format!("build-up plane {z}: {error}"))?;
        let mut shapes = group_contours(contours, &mut work)?;
        // Existing planar faces at this stage already close their own region.
        // Overdrawing them would change opacity in Ghost/Xray inspection.
        let coplanar: Shapes = self
            .mesh
            .triangles
            .iter()
            .filter_map(|t| {
                let points = t.map(|i| self.mesh.positions[i as usize]);
                points
                    .iter()
                    .all(|p| p[2] == z)
                    .then(|| vec![points.iter().map(|p| [p[0], p[1]]).collect()])
            })
            .collect();
        if !coplanar.is_empty() {
            let adapter = FloatPointAdapter::<Point, i64>::with_coordinate_bits(
                FloatRect {
                    min_x: self.bounds.min[0],
                    max_x: self.bounds.max[0],
                    min_y: self.bounds.min[1],
                    max_y: self.bounds.max[1],
                },
                48,
            );
            shapes = difference(&shapes, &coplanar, &adapter);
        }
        let mut builder = Builder::default();
        cap(&mut builder, &shapes, z, true)?;
        Ok(native_cap(builder.mesh))
    }
}

/// The native renderer and picker consume f32 vertices. Interpolated section
/// points can form nonzero f64 ears which become exactly collinear there. Such
/// generated ears cover no GPU area and must not invalidate the entire cap.
/// Keep the source mesh untouched and preserve every noncollapsed cap face.
fn native_cap(mut mesh: Mesh) -> Mesh {
    for point in &mut mesh.positions {
        *point = point.map(|v| v as f32 as f64);
    }
    mesh.triangles.retain_mut(|triangle| {
        let [a, b, c] = triangle.map(|i| mesh.positions[i as usize]);
        let normal = cross3(sub3(b, a), sub3(c, a));
        if normal == [0.; 3] {
            return false;
        }
        if normal[2] < 0. {
            triangle.swap(1, 2);
        }
        true
    });
    compact(&mut mesh);
    mesh
}

pub fn slice_stepped(mesh: &Mesh, boundaries_mm: &[f64]) -> Result<Mesh> {
    Ok(slice_stepped_with_report(mesh, boundaries_mm)?.mesh)
}

/// Mid-interval cross sections are extruded to the scheduled boundaries; the last
/// interval is clipped to the actual source top. Closed oriented inputs only.
/// Independently indexed, closed layer slabs share the renderer's f32 precision.
/// Internal layer interfaces are retained; the exterior is the sampled envelope.
pub fn slice_stepped_with_report(mesh: &Mesh, boundaries_mm: &[f64]) -> Result<SteppedEnvelope> {
    let bounds = mesh.bounds()?;
    validate_closed(mesh).map_err(|e| format!("source solid: {e}"))?;
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
    if intervals > 2000 {
        return Err(
            "experimental slice work budget exceeded (2000 intervals / 10M triangle-plane tests)"
                .into(),
        );
    }
    let arithmetic_tolerance =
        (bounds.max[0] - bounds.min[0]).max(bounds.max[1] - bounds.min[1]) * 2f64.powi(-45);
    let mut sections = Vec::new();
    let sample_heights_mm: Vec<_> = schedule
        .boundaries_mm
        .windows(2)
        .map(|p| (p[0] + p[1]) * 0.5)
        .collect();
    let active = active_triangles(mesh, &sample_heights_mm)?;
    let mut work = 0usize;
    for (&z, triangles) in sample_heights_mm.iter().zip(&active) {
        // section() shares one node for exact vertex contacts and takes the
        // limiting section from below. Moving the plane by one ULP instead
        // manufactures artificial microscopic edges.
        let mut contours = section_triangles(
            mesh,
            z,
            triangles.iter().map(|&i| &mesh.triangles[i as usize]),
        )?;
        validate_contours(&contours, arithmetic_tolerance, &mut work)
            .map_err(|error| format!("slice plane {z}: {error}"))?;
        let original_depths = contour_depths(&contours, &mut work)?;
        for contour in &mut contours {
            for point in contour.iter_mut() {
                *point = point.map(|v| v as f32 as f64);
            }
            simplify_collinear(contour);
            if contour.len() < 3 || signed_area(contour) == 0. {
                return Err(format!("slice plane {z}: a contour collapses at renderer precision; use Layer Appearance or a larger model").into());
            }
        }
        validate_contours(&contours, 0., &mut work).map_err(|e| {
            format!("slice plane {z}: renderer precision changes contour topology: {e}")
        })?;
        if contour_depths(&contours, &mut work)? != original_depths {
            return Err(format!("slice plane {z}: renderer precision changes holes or nesting; no topology repair performed").into());
        }
        sections.push(group_contours(contours, &mut work)?);
    }
    let mut output = Mesh::default();
    let mut layer_face_ranges = Vec::with_capacity(intervals);
    for (index, shapes) in sections.iter().enumerate() {
        let start = output.triangles.len();
        if shapes.is_empty() {
            layer_face_ranges.push([start, start]);
            continue;
        }
        let mut builder = Builder::default();
        let bottom = schedule.boundaries_mm[index] as f32 as f64;
        let top = schedule.boundaries_mm[index + 1] as f32 as f64;
        if top <= bottom {
            return Err(format!("layer {index} collapses at renderer height precision").into());
        }
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
        cap(&mut builder, shapes, bottom, false)?;
        cap(&mut builder, shapes, top, true)?;
        // Hole bridging in earcut may omit a collinear boundary vertex. Refine
        // those unmatched cap edges only; this does not move the sampled region.
        if validate_closed(&builder.mesh).is_err() {
            builder.mesh = super::conform::split_boundary_edges(
                builder.mesh,
                arithmetic_tolerance,
                &mut work,
            )?;
            for p in &mut builder.mesh.positions {
                *p = p.map(|v| v as f32 as f64);
            }
        }
        validate_closed(&builder.mesh).map_err(|e| format!("layer {index}: {e}"))?;
        if output.positions.len() + builder.mesh.positions.len() > crate::mesh::MAX_VERTICES
            || output.triangles.len() + builder.mesh.triangles.len() > crate::mesh::MAX_TRIANGLES
        {
            return Err("stepped mesh output budget exceeded".into());
        }
        let offset = output.positions.len() as u32;
        output.positions.extend(builder.mesh.positions);
        output.triangles.extend(
            builder
                .mesh
                .triangles
                .into_iter()
                .map(|t| t.map(|i| i + offset)),
        );
        layer_face_ranges.push([start, output.triangles.len()]);
    }
    // Remove unused vertices only; no geometry, edges or gaps are repaired.
    compact(&mut output);
    output.bounds()?;
    validate_closed(&output).map_err(|e| format!("stepped output: {e}"))?;
    Ok(SteppedEnvelope {mesh:output,layer_face_ranges,sample_heights_mm,coordinate_tolerance_mm:bounds.min.into_iter().chain(bounds.max).fold(0f64,|a,b|a.max(b.abs())) * f32::EPSILON as f64,
        warnings:vec!["Experimental mid-layer stepped envelope; this is a sliced-solid approximation, not deposited nozzle paths. Small features between sample planes may be absent by the selected sampling rule.".into(),"Closed layer slabs retain internal layer interfaces, which may be visible with transparent materials. Coordinates use renderer f32 precision; collapsed holes/gaps are rejected without source repair.".into()]})
}

/// Discard only repeated points and exactly straight intermediate vertices.
/// Both the caps and walls consume the same simplified contour.
fn simplify_collinear(contour: &mut Contour) {
    fn between(a: Point, b: Point, c: Point) -> bool {
        cross(a, b, c) == 0.
            && (0..2).all(|axis| b[axis] >= a[axis].min(c[axis]) && b[axis] <= a[axis].max(c[axis]))
    }
    contour.dedup();
    if contour.first() == contour.last() {
        contour.pop();
    }
    let mut result = Vec::with_capacity(contour.len());
    for p in contour.drain(..) {
        while result.len() >= 2 && between(result[result.len() - 2], result[result.len() - 1], p) {
            result.pop();
        }
        result.push(p);
    }
    while result.len() >= 3
        && between(
            result[result.len() - 2],
            result[result.len() - 1],
            result[0],
        )
    {
        result.pop();
    }
    while result.len() >= 3 && between(result[result.len() - 1], result[0], result[1]) {
        result.remove(0);
    }
    *contour = result;
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
    if let Some((&(a, b), &(count, balance))) = edges
        .iter()
        .find(|(_, (count, balance))| *count != 2 || *balance != 0)
    {
        let invalid = edges
            .values()
            .filter(|&&(count, balance)| count != 2 || balance != 0)
            .count();
        return Err(format!("solid sections require a closed consistently oriented manifold; open/nonmanifold edges are not repaired ({invalid} invalid edges; first {:?}->{:?}, incidence {count}, balance {balance})",mesh.positions[a as usize],mesh.positions[b as usize]).into());
    }
    Ok(())
}

fn plane_roundoff(height: f64, z: f64) -> f64 {
    8. * f64::EPSILON * z.abs().max(height.abs()).max(f64::MIN_POSITIVE)
}

/// Index only triangles whose Z range reaches a sample. Bound the actual
/// triangle-plane tests and index storage, rather than rejecting dense meshes
/// for the Cartesian product of all faces and all layers.
fn active_triangles(mesh: &Mesh, samples: &[f64]) -> Result<Vec<Vec<u32>>> {
    let mut active = vec![Vec::new(); samples.len()];
    let mut work = 0usize;
    for (index, triangle) in mesh.triangles.iter().enumerate() {
        let heights = triangle.map(|i| mesh.positions[i as usize][2]);
        let min = heights.into_iter().fold(f64::INFINITY, f64::min);
        let max = heights.into_iter().fold(f64::NEG_INFINITY, f64::max);
        let start = samples.partition_point(|&z| z < min - plane_roundoff(min, z));
        let end = samples.partition_point(|&z| z <= max + plane_roundoff(max, z));
        work = work.saturating_add(end - start);
        if work > 10_000_000 {
            return Err(
                "experimental slice work budget exceeded (10M active triangle-plane tests)".into(),
            );
        }
        for layer in &mut active[start..end] {
            layer.push(index as u32);
        }
    }
    Ok(active)
}

fn section(mesh: &Mesh, z: f64) -> Result<Vec<Contour>> {
    section_triangles(mesh, z, mesh.triangles.iter())
}
fn section_triangles<'a>(
    mesh: &Mesh,
    z: f64,
    triangles: impl Iterator<Item = &'a [u32; 3]>,
) -> Result<Vec<Contour>> {
    let mut points = BTreeMap::<(u32, u32), Point>::new();
    let mut neighbors = BTreeMap::<(u32, u32), Vec<(u32, u32)>>::new();
    let mut segments = BTreeMap::new();
    for triangle in triangles {
        let mut hits = Vec::new();
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let key = (a.min(b), a.max(b));
            let p = mesh.positions[key.0 as usize];
            let q = mesh.positions[key.1 as usize];
            // Decimal schedule arithmetic can put a nominal vertex contact a
            // few ULPs to either side. Classify only arithmetic roundoff as an
            // exact contact; never offset the plane or close a geometric gap.
            let on_plane = |height: f64| (height - z).abs() <= plane_roundoff(height, z);
            let p_on = on_plane(p[2]);
            let q_on = on_plane(q[2]);
            if (p[2] < z && !p_on) == (q[2] < z && !q_on) {
                continue;
            }
            let t = (z - p[2]) / (q[2] - p[2]);
            // All incident edges meeting the plane at a vertex share one node.
            let key = if p_on {
                (key.0, key.0)
            } else if q_on {
                (key.1, key.1)
            } else {
                key
            };
            points.entry(key).or_insert(if p_on {
                [p[0], p[1]]
            } else if q_on {
                [q[0], q[1]]
            } else {
                [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]
            });
            hits.push(key);
        }
        hits.sort_unstable();
        hits.dedup();
        if hits.len() < 2 {
            continue;
        }
        if hits.len() != 2 {
            return Err("ambiguous triangle-plane intersection".into());
        }
        *segments.entry((hits[0], hits[1])).or_insert(0u32) += 1;
    }
    for ((a, b), count) in segments {
        if count > 2 {
            return Err("ambiguous overlapping triangle-plane segments".into());
        }
        // Two retained triangles sharing an edge on the plane only touch it;
        // that internal edge is not the boundary of the limiting section.
        if count == 2 {
            continue;
        }
        neighbors.entry(a).or_default().push(b);
        neighbors.entry(b).or_default().push(a);
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
fn validate_contours(contours: &[Contour], tolerance: f64, work: &mut usize) -> Result<()> {
    let mut edges: Vec<_> = contours
        .iter()
        .enumerate()
        .flat_map(|(c, contour)| {
            (0..contour.len()).map(move |i| (c, i, contour[i], contour[(i + 1) % contour.len()]))
        })
        .collect();
    edges.sort_unstable_by(|a, b| a.2[0].min(a.3[0]).total_cmp(&b.2[0].min(b.3[0])));
    for (i, &(ca, ia, a, b)) in edges.iter().enumerate() {
        for &(cb, ib, c, d) in &edges[i + 1..] {
            if c[0].min(d[0]) > a[0].max(b[0]) + tolerance {
                break;
            }
            *work = work.saturating_add(1);
            if *work > WORK_BUDGET {
                return Err("contour intersection work budget exceeded".into());
            }
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
                return Err(format!("slice contours intersect, touch or contain features below coordinate resolution; no repair performed (edges {ca}/{ia} {a:?}->{b:?} and {cb}/{ib} {c:?}->{d:?}, distance {distance:e}, tolerance {tolerance:e}, crossing {intersects})").into());
            }
        }
    }
    Ok(())
}
fn contour_work(work: &mut usize, count: usize) -> Result<()> {
    *work = work.saturating_add(count);
    if *work > WORK_BUDGET {
        return Err("contour work budget exceeded".into());
    }
    Ok(())
}
fn contour_depths(contours: &[Contour], work: &mut usize) -> Result<Vec<usize>> {
    let mut rects = Vec::with_capacity(contours.len());
    for contour in contours {
        contour_work(work, contour.len())?;
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in contour {
            for axis in 0..2 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        rects.push((min, max));
    }
    let mut depths = vec![0; contours.len()];
    for (i, contour) in contours.iter().enumerate() {
        for (j, other) in contours.iter().enumerate() {
            contour_work(work, 1)?;
            if i == j
                || (0..2).any(|axis| {
                    contour[0][axis] < rects[j].0[axis] || contour[0][axis] > rects[j].1[axis]
                })
            {
                continue;
            }
            contour_work(work, other.len())?;
            if inside(contour[0], other) {
                depths[i] += 1;
            }
        }
    }
    Ok(depths)
}
fn group_contours(mut contours: Vec<Contour>, work: &mut usize) -> Result<Shapes> {
    let depths = contour_depths(&contours, work)?;
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
            contour_work(work, 1)?;
            if depths[j] == depths[i] + 1 {
                contour_work(work, contours[i].len())?;
                if !inside(contours[j][0], &contours[i]) {
                    continue;
                }
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
            let area = cross([p[0][0], p[0][1]], [p[1][0], p[1][1]], [p[2][0], p[2][1]]);
            // Earcut can include a zero-area ear among nearly collinear grid
            // intersections. It covers no region; boundary conformance restores
            // any intervening boundary vertices on the remaining faces.
            if area == 0. {
                continue;
            }
            if (area > 0.) != up {
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

#[cfg(test)]
mod buildup_tests {
    use super::*;

    #[test]
    fn native_cap_discards_only_faces_with_no_area_at_renderer_precision() {
        let cap = Mesh {
            positions: vec![
                [0., 0., 1.1],
                [2., 0., 1.1],
                [1., 1e-50, 1.1],
                [2., 2., 1.1],
                [0., 2., 1.1],
            ],
            triangles: vec![[0, 1, 2], [0, 1, 3], [0, 3, 4]],
        };
        let cap = native_cap(cap);
        assert_eq!(cap.triangles.len(), 2);
        assert_eq!(cap.positions.len(), 4);
        assert_eq!(area(&cap), 4.);
        assert!(
            cap.positions
                .iter()
                .flatten()
                .all(|&v| v == v as f32 as f64)
        );
        crate::viewport::PickingMesh::new(&cap).unwrap();
    }

    #[test]
    fn native_cap_omits_the_reported_collinear_gpu_face() {
        let cap = Mesh {
            positions: vec![
                [44.69956588745117, 5.065960884094238, 28.399999618530273],
                [44.888004302978516, 4.91094445182251, 28.399999618530273],
                [45.07644271850586, 4.755928039550781, 28.399999618530273],
            ],
            triangles: vec![[0, 1, 2]],
        };
        assert!(area(&cap) > 0., "the original f64 ear is nonzero");
        let cap = native_cap(cap);
        assert!(cap.triangles.is_empty());
        assert!(cap.positions.is_empty());
    }

    #[test]
    fn native_cap_keeps_a_small_nonzero_face_without_an_area_cutoff() {
        let cap = native_cap(Mesh {
            positions: vec![[0., 0., 0.], [1e-12, 0., 0.], [0., 1e-12, 0.]],
            triangles: vec![[0, 1, 2]],
        });
        assert_eq!(cap.triangles.len(), 1);
        assert!(area(&cap) > 0.);
        crate::viewport::PickingMesh::new(&cap).unwrap();
    }

    #[test]
    fn native_cap_preserves_upward_winding_after_coordinate_rounding() {
        let cap = Mesh {
            positions: vec![
                [7.873971570789526, 3.2956212316547955, 0.],
                [9.873971570789525, 4.2956212316547955, 0.],
                [8.873971570789525, 3.7956212416547954, 0.],
            ],
            triangles: vec![[0, 1, 2]],
        };
        assert!(area(&cap) > 0.);
        let cap = native_cap(cap);
        assert_eq!(cap.triangles.len(), 1);
        assert!(area(&cap) > 0.);
        crate::viewport::PickingMesh::new(&cap).unwrap();
    }

    #[test]
    fn cap_omits_zero_area_ears_from_nearly_collinear_overlay() {
        let shape = vec![vec![
            [11.63769531187063, 6.116708395313026],
            [11.588859558105469, 5.9186660051345825],
            [11.635055541396838, 6.106003401249325],
            [11.6376953125, 6.116708397865295],
        ]];
        let mut builder = Builder::default();
        cap(&mut builder, &vec![shape], 6.4, true).unwrap();
        assert!(!builder.mesh.triangles.is_empty());
        assert!(area(&builder.mesh) > 0.);
    }

    fn square_solid(inner: Option<f64>) -> Mesh {
        let outer = vec![[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]];
        let mut shape = vec![outer];
        if let Some(r) = inner {
            shape.push(vec![[-r, -r], [-r, r], [r, r], [r, -r]]);
        }
        let mut builder = Builder::default();
        cap(&mut builder, &vec![shape.clone()], 0., false).unwrap();
        cap(&mut builder, &vec![shape.clone()], 2., true).unwrap();
        for contour in shape {
            for i in 0..contour.len() {
                let a = contour[i];
                let b = contour[(i + 1) % contour.len()];
                builder
                    .triangle([[a[0], a[1], 0.], [b[0], b[1], 0.], [b[0], b[1], 2.]])
                    .unwrap();
                builder
                    .triangle([[a[0], a[1], 0.], [b[0], b[1], 2.], [a[0], a[1], 2.]])
                    .unwrap();
            }
        }
        builder.mesh
    }
    fn area(mesh: &Mesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
                let n = cross3(sub3(b, a), sub3(c, a));
                assert!(n[2] > 0., "cap normal must face up");
                n[2] * 0.5
            })
            .sum()
    }
    #[test]
    fn buildup_cap_cube_has_full_upward_area_at_build_height() {
        let mut mesh = square_solid(None);
        for p in &mut mesh.positions {
            p[2] += 7.;
        }
        let solid = BuildUpSolid::new(&mesh).unwrap();
        let cap = solid.cap(0.75).unwrap();
        assert_eq!(area(&cap), 4.);
        assert!(cap.positions.iter().all(|p| p[2] == 7.75));
        assert!(solid.cap(0.).unwrap().triangles.is_empty());
        assert!(solid.cap(2.).unwrap().triangles.is_empty());
    }
    #[test]
    fn buildup_cap_preserves_holes_and_disconnected_nested_islands() {
        let mut mesh = square_solid(Some(0.5));
        let mut add = |mut other: Mesh, shift: [f64; 3], scale: f64| {
            let offset = mesh.positions.len() as u32;
            for p in &mut other.positions {
                for axis in 0..3 {
                    p[axis] = p[axis] * scale + shift[axis];
                }
            }
            mesh.positions.extend(other.positions);
            mesh.triangles
                .extend(other.triangles.iter().map(|t| t.map(|i| i + offset)));
        };
        add(square_solid(None), [0., 0., 0.], 0.25);
        add(square_solid(None), [4., 0., 0.], 1.);
        let cap = BuildUpSolid::new(&mesh).unwrap().cap(0.25).unwrap();
        assert!((area(&cap) - 7.25).abs() < 1e-12);
        // A point in the tube bore outside the island must remain uncovered.
        assert!(cap.triangles.iter().all(|t| {
            let [a, b, c] = t.map(|i| mesh_point(&cap, i));
            let p = [0.4, 0.];
            !(cross(a, b, p) >= 0. && cross(b, c, p) >= 0. && cross(c, a, p) >= 0.)
        }));
    }
    fn mesh_point(mesh: &Mesh, i: u32) -> Point {
        let p = mesh.positions[i as usize];
        [p[0], p[1]]
    }
    #[test]
    fn buildup_cap_handles_plane_through_existing_vertices_and_edges() {
        let mut mesh = square_solid(None);
        // Sloped top crosses z=1 at two vertices; one top triangle edge lies on the cut.
        for p in &mut mesh.positions {
            if p[2] == 2. {
                p[2] = if p[0] < 0. { 1. } else { 2. };
            }
        }
        let cap = BuildUpSolid::new(&mesh).unwrap().cap(1.).unwrap();
        assert!((area(&cap) - 4.).abs() < 1e-12);
    }
    #[test]
    fn buildup_cap_rejects_open_sources_without_fabricating_a_solid() {
        let mut mesh = square_solid(None);
        mesh.triangles.pop();
        assert!(BuildUpSolid::new(&mesh).is_err());
    }
    #[test]
    fn buildup_cap_does_not_duplicate_existing_horizontal_surfaces() {
        let mut mesh = square_solid(None);
        let mut short = square_solid(None);
        let offset = mesh.positions.len() as u32;
        for p in &mut short.positions {
            p[0] += 4.;
            p[2] *= 0.5;
        }
        mesh.positions.extend(short.positions);
        mesh.triangles
            .extend(short.triangles.iter().map(|t| t.map(|i| i + offset)));
        let cap = BuildUpSolid::new(&mesh).unwrap().cap(1.).unwrap();
        assert_eq!(
            area(&cap),
            4.,
            "short solid already supplies its own top face"
        );
    }
}
