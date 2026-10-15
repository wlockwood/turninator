//! Indexed picking of the same f32 triangle surface sent to the native GPU.
//! Construct on an import thread; clicks reuse this immutable median-split BVH.
use super::{OrbitCamera, ViewportSettings, camera::rotation_rows};
use crate::{
    Result,
    mesh::Mesh,
    model::orientation::{apply, cross, dot, normalized, sub},
};
use std::sync::Mutex;

/// Screen Y points down; picking NDC Y points up. Outside clicks are rejected.
pub fn pointer_coordinates(
    rect: eframe::egui::Rect,
    point: eframe::egui::Pos2,
) -> Option<([f64; 2], f64)> {
    if !rect.is_finite()
        || !point.is_finite()
        || rect.width() <= 0.
        || rect.height() <= 0.
        || !rect.contains(point)
    {
        return None;
    }
    Some((
        [
            (2. * (point.x - rect.left()) / rect.width() - 1.) as f64,
            (1. - 2. * (point.y - rect.top()) / rect.height()) as f64,
        ],
        rect.width() as f64 / rect.height() as f64,
    ))
}

#[derive(Clone, Copy, Debug)]
pub struct FaceHit {
    pub triangle_index: usize,
    /// Original mesh/build millimeters, before display centering or rotation.
    pub point_mm: [f64; 3],
    /// Unit winding normal in mesh/build coordinates; never camera-flipped.
    pub normal_mm: [f64; 3],
}
#[derive(Clone, Copy)]
struct Node {
    min: [f32; 3],
    max: [f32; 3],
    start: u32,
    count: u32,
    left: u32,
    right: u32,
}
pub struct PickingMesh {
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    order: Vec<u32>,
    nodes: Vec<Node>,
    layer_tops: Vec<f32>,
    center: Mutex<Option<([f32; 3], [f32; 3])>>,
}
struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
    near: f64,
    far: f64,
}

impl PickingMesh {
    pub fn new(mesh: &Mesh) -> Result<Self> {
        mesh.bounds()?;
        let positions: Vec<_> = mesh.positions.iter().map(|p| p.map(|v| v as f32)).collect();
        if positions.iter().flatten().any(|v| !v.is_finite()) {
            return Err("picking mesh exceeds f32 precision range".into());
        }
        for (index, triangle) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = triangle.map(|i| positions[i as usize].map(f64::from));
            if normalized(cross(sub(b, a), sub(c, a))).is_none() {
                return Err(format!(
                    "picking triangle {index} collapses at GPU precision: {a:?}, {b:?}, {c:?}"
                )
                .into());
            }
        }
        let mut result = Self {
            positions,
            triangles: mesh.triangles.clone(),
            order: (0..mesh.triangles.len() as u32).collect(),
            nodes: Vec::new(),
            layer_tops: Vec::new(),
            center: Mutex::new(None),
        };
        result.build(0, result.order.len());
        Ok(result)
    }
    /// Use original triangle order; empty clears whole-layer filtering.
    pub fn set_triangle_layer_tops(&mut self, tops: &[f32]) -> Result<()> {
        if !tops.is_empty()
            && (tops.len() != self.triangles.len()
                || tops
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1e7).contains(v)))
        {
            return Err(
                "picking layer tops must match triangles and be finite build heights".into(),
            );
        }
        self.layer_tops = tops.to_vec();
        Ok(())
    }
    pub fn pick(
        &self,
        camera: &OrbitCamera,
        settings: &ViewportSettings,
        ndc: [f64; 2],
        aspect: f64,
    ) -> Option<FaceHit> {
        if settings.validate().is_err() {
            return None;
        }
        let shift = {
            let mut cache = self.center.lock().ok()?;
            if cache
                .as_ref()
                .is_none_or(|(pose, _)| *pose != settings.display_rotation_degrees)
            {
                let rows =
                    rotation_rows(settings.display_rotation_degrees).map(|r| r.map(f64::from));
                let mut min = [f64::INFINITY; 3];
                let mut max = [f64::NEG_INFINITY; 3];
                for p in &self.positions {
                    let rotated = apply(rows, p.map(f64::from));
                    for i in 0..3 {
                        min[i] = min[i].min(rotated[i]);
                        max[i] = max[i].max(rotated[i]);
                    }
                }
                *cache = Some((
                    settings.display_rotation_degrees,
                    [
                        ((min[0] + max[0]) * 0.5) as f32,
                        ((min[1] + max[1]) * 0.5) as f32,
                        min[2] as f32,
                    ],
                ));
            }
            cache.as_ref()?.1
        };
        self.pick_with_shift(camera, settings, ndc, aspect, shift)
    }
    pub(crate) fn pick_with_shift(
        &self,
        camera: &OrbitCamera,
        settings: &ViewportSettings,
        ndc: [f64; 2],
        aspect: f64,
        shift: [f32; 3],
    ) -> Option<FaceHit> {
        if settings.validate().is_err() {
            return None;
        }
        let mut ray = camera_ray(camera, ndc, aspect)?;
        let rotation = rotation_rows(settings.display_rotation_degrees).map(|r| r.map(f64::from));
        let (s, c) = settings.turntable_angle_degrees.to_radians().sin_cos();
        let (s, c) = (s as f64, c as f64);
        let spin_inverse = [
            [c / (c * c + s * s), s / (c * c + s * s), 0.],
            [-s / (c * c + s * s), c / (c * c + s * s), 0.],
            [0., 0., 1.],
        ];
        let unspun = apply(spin_inverse, ray.origin);
        let inverse = inverse(rotation)?;
        ray.origin = apply(
            inverse,
            std::array::from_fn(|i| unspun[i] + shift[i] as f64),
        );
        ray.direction = apply(inverse, apply(spin_inverse, ray.direction));
        self.intersect(&ray, settings, &mut 0)
    }
    fn build(&mut self, start: usize, end: usize) -> u32 {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for &i in &self.order[start..end] {
            for &v in &self.triangles[i as usize] {
                for axis in 0..3 {
                    let p = self.positions[v as usize][axis];
                    min[axis] = min[axis].min(p);
                    max[axis] = max[axis].max(p);
                }
            }
        }
        let index = self.nodes.len() as u32;
        self.nodes.push(Node {
            min,
            max,
            start: start as u32,
            count: (end - start) as u32,
            left: 0,
            right: 0,
        });
        if end - start > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (max[a] as f64 - min[a] as f64).total_cmp(&(max[b] as f64 - min[b] as f64))
                })
                .unwrap();
            let middle = (end - start) / 2;
            let positions = &self.positions;
            let triangles = &self.triangles;
            let centroid = |i: u32| {
                triangles[i as usize]
                    .iter()
                    .map(|&v| positions[v as usize][axis] as f64)
                    .sum::<f64>()
            };
            self.order[start..end].select_nth_unstable_by(middle, |&a, &b| {
                centroid(a).total_cmp(&centroid(b)).then(a.cmp(&b))
            });
            let left = self.build(start, start + middle);
            let right = self.build(start + middle, end);
            self.nodes[index as usize].count = 0;
            self.nodes[index as usize].left = left;
            self.nodes[index as usize].right = right;
        }
        index
    }
    fn intersect(
        &self,
        ray: &Ray,
        settings: &ViewportSettings,
        tested: &mut usize,
    ) -> Option<FaceHit> {
        let mut pending = vec![(0u32, ray.near)];
        let mut closest = ray.far;
        let mut found: Option<FaceHit> = None;
        while let Some((index, entry)) = pending.pop() {
            if entry > closest {
                continue;
            }
            let node = &self.nodes[index as usize];
            if node.count == 0 {
                let a = box_entry(&self.nodes[node.left as usize], ray, closest)
                    .map(|t| (node.left, t));
                let b = box_entry(&self.nodes[node.right as usize], ray, closest)
                    .map(|t| (node.right, t));
                match (a, b) {
                    (Some(a), Some(b)) => {
                        if a.1 < b.1 {
                            pending.push(b);
                            pending.push(a);
                        } else {
                            pending.push(a);
                            pending.push(b);
                        }
                    }
                    (Some(a), None) | (None, Some(a)) => pending.push(a),
                    _ => {}
                }
            } else {
                for &index in &self.order[node.start as usize..(node.start + node.count) as usize] {
                    *tested += 1;
                    let index = index as usize;
                    let [a, b, c] =
                        self.triangles[index].map(|i| self.positions[i as usize].map(f64::from));
                    let Some((distance, normal)) = triangle_hit(ray, a, b, c) else {
                        continue;
                    };
                    if distance < ray.near || distance > closest {
                        continue;
                    }
                    let point =
                        std::array::from_fn(|i| ray.origin[i] + distance * ray.direction[i]);
                    if settings.section.is_some_and(|p| {
                        (point[p.axis as usize] - p.position_mm as f64)
                            * if p.keep_positive { 1. } else { -1. }
                            < 0.
                    }) {
                        continue;
                    }
                    if let Some(height) = settings.effective_buildup_height()
                        && self.layer_tops.get(index).map_or(point[2], |v| *v as f64)
                            > height as f64
                    {
                        continue;
                    }
                    if distance == closest && found.is_some_and(|old| old.triangle_index < index) {
                        continue;
                    }
                    closest = distance;
                    found = Some(FaceHit {
                        triangle_index: index,
                        point_mm: point,
                        normal_mm: normal,
                    });
                }
            }
        }
        found
    }
}
fn camera_ray(camera: &OrbitCamera, ndc: [f64; 2], aspect: f64) -> Option<Ray> {
    if camera.validate().is_err()
        || !aspect.is_finite()
        || aspect <= 0.
        || ndc.iter().any(|v| !v.is_finite() || v.abs() > 1.)
    {
        return None;
    }
    let (right, up, back) = camera.basis();
    let mut origin = std::array::from_fn(|i| camera.target_mm[i] + back[i] * camera.distance_mm);
    let direction = if camera.orthographic {
        for i in 0..3 {
            origin[i] += (right[i] * ndc[0] * aspect + up[i] * ndc[1]) * camera.ortho_span_mm * 0.5;
        }
        back.map(|v| -v)
    } else {
        let tangent = (camera.vertical_fov_degrees * 0.5).to_radians().tan();
        normalized(std::array::from_fn(|i| {
            -back[i] + (right[i] * ndc[0] * aspect + up[i] * ndc[1]) * tangent
        }))?
    };
    let depth_rate = -dot(direction, back);
    Some(Ray {
        origin,
        direction,
        near: (camera.distance_mm * 0.0001).max(1e-5) / depth_rate,
        far: (camera.distance_mm * 100. + 100.) / depth_rate,
    })
}
fn inverse(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let columns = [cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1])];
    let determinant = dot(m[0], columns[0]);
    if !determinant.is_finite() || determinant == 0. {
        return None;
    }
    Some(std::array::from_fn(|i| {
        std::array::from_fn(|j| columns[j][i] / determinant)
    }))
}
fn box_entry(node: &Node, ray: &Ray, closest: f64) -> Option<f64> {
    let mut near = ray.near;
    let mut far = closest;
    for axis in 0..3 {
        let direction = ray.direction[axis];
        let origin = ray.origin[axis];
        if direction == 0. {
            if origin < node.min[axis] as f64 || origin > node.max[axis] as f64 {
                return None;
            }
        } else {
            let a = (node.min[axis] as f64 - origin) / direction;
            let b = (node.max[axis] as f64 - origin) / direction;
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    Some(near)
}
fn triangle_hit(ray: &Ray, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<(f64, [f64; 3])> {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let normal = cross(ab, ac);
    let p = cross(ray.direction, ac);
    let determinant = dot(ab, p);
    if determinant == 0. {
        return None;
    }
    let relative = sub(ray.origin, a);
    let u = dot(relative, p) / determinant;
    let q = cross(relative, ab);
    let v = dot(ray.direction, q) / determinant;
    if !(-1e-10..=1. + 1e-10).contains(&u) || v < -1e-10 || u + v > 1. + 1e-10 {
        return None;
    }
    let distance = dot(ac, q) / determinant;
    if !distance.is_finite() {
        return None;
    }
    Some((distance, normalized(normal)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bvh_limits_triangle_work_and_preserves_the_nearest_hit() {
        let mut mesh = Mesh::default();
        for x in 0..100 {
            for z in 0..100 {
                let base = mesh.positions.len() as u32;
                let (x, z) = (x as f64 * 2., z as f64 * 2.);
                mesh.positions.extend([
                    [x, 0., z],
                    [x + 1., 0., z],
                    [x + 1., 0., z + 1.],
                    [x, 0., z + 1.],
                ]);
                mesh.triangles
                    .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
            }
        }
        let picker = PickingMesh::new(&mesh).unwrap();
        let ray = Ray {
            origin: [80.25, -10., 120.75],
            direction: [0., 1., 0.],
            near: 0.001,
            far: 100.,
        };
        let mut tested = 0;
        let hit = picker
            .intersect(&ray, &ViewportSettings::default(), &mut tested)
            .unwrap();
        assert_eq!(hit.triangle_index, (40 * 100 + 60) * 2 + 1);
        assert!(tested < 32, "BVH tested {tested} triangles out of20000");
        assert_eq!(hit.normal_mm, [0., -1., 0.]);
    }
}
