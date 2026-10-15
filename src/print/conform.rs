use crate::{
    Result,
    mesh::{MAX_TRIANGLES, MAX_VERTICES, Mesh},
};
use std::collections::BTreeMap;

const WORK_BUDGET: usize = 100_000_000;

#[derive(Default)]
struct Layer {
    x: Vec<usize>,
    y: Vec<usize>,
}

fn key(value: f64) -> u64 {
    if value == 0. { 0 } else { value.to_bits() }
}

/// Refine unmatched horizontal cap edges at existing boundary vertices.
/// Candidate searches visit only the edge's exact Z layer and expanded
/// major-axis coordinate range. The caller supplies an arithmetic roundoff
/// tolerance for segment projection; matching interior edges remain untouched.
/// The budget bounds actual candidate visits rather than the unrelated global
/// vertex/triangle product; output has the normal mesh vertex and face limits.
pub(super) fn split_boundary_edges(mesh: Mesh, tolerance: f64, work: &mut usize) -> Result<Mesh> {
    mesh.bounds()?;
    let mut canonical = BTreeMap::new();
    let vertex_ids: Vec<_> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| *canonical.entry(p.map(key)).or_insert(i as u32))
        .collect();
    let mut edge_counts = BTreeMap::<(u32, u32), u8>::new();
    for triangle in &mesh.triangles {
        charge(work, 4)?;
        for i in 0..3 {
            let a = triangle[i] as usize;
            let b = triangle[(i + 1) % 3] as usize;
            if mesh.positions[a][2] == mesh.positions[b][2] {
                let (a, b) = (vertex_ids[a], vertex_ids[b]);
                let count = edge_counts.entry((a.min(b), a.max(b))).or_default();
                *count = count.saturating_add(1);
            }
        }
    }
    let mut boundary_vertex = vec![false; mesh.positions.len()];
    for (&(a, b), &count) in &edge_counts {
        if count == 1 {
            boundary_vertex[a as usize] = true;
            boundary_vertex[b as usize] = true;
        }
    }
    let mut layers = BTreeMap::<u64, Layer>::new();
    for (index, p) in mesh.positions.iter().enumerate() {
        if boundary_vertex[vertex_ids[index] as usize] {
            layers.entry(key(p[2])).or_default().x.push(index);
        }
    }
    for layer in layers.values_mut() {
        layer.x.sort_unstable_by(|&a, &b| {
            mesh.positions[a][0]
                .total_cmp(&mesh.positions[b][0])
                .then_with(|| mesh.positions[a][1].total_cmp(&mesh.positions[b][1]))
        });
        layer
            .x
            .dedup_by(|a, b| mesh.positions[*a] == mesh.positions[*b]);
        layer.y = layer.x.clone();
        layer.y.sort_unstable_by(|&a, &b| {
            mesh.positions[a][1]
                .total_cmp(&mesh.positions[b][1])
                .then_with(|| mesh.positions[a][0].total_cmp(&mesh.positions[b][0]))
        });
    }
    let mut result = Builder::default();
    for triangle in &mesh.triangles {
        charge(work, 1)?;
        let p = triangle.map(|i| mesh.positions[i as usize]);
        let mut perimeter = Vec::with_capacity(3);
        for edge in 0..3 {
            charge(work, 1)?;
            let a = p[edge];
            let b = p[(edge + 1) % 3];
            perimeter.push(a);
            if a[2] != b[2] {
                continue;
            }
            let (a_id, b_id) = (
                vertex_ids[triangle[edge] as usize],
                vertex_ids[triangle[(edge + 1) % 3] as usize],
            );
            if edge_counts[&(a_id.min(b_id), a_id.max(b_id))] != 1 {
                continue;
            }
            let delta = [b[0] - a[0], b[1] - a[1]];
            let axis = usize::from(delta[1].abs() > delta[0].abs());
            if delta[axis] == 0. {
                return Err("stepped triangulation contains a zero-length edge".into());
            }
            let length = delta[0].hypot(delta[1]);
            let direction = delta.map(|value| value / length);
            let layer = &layers[&key(a[2])];
            let sorted = if axis == 0 { &layer.x } else { &layer.y };
            let low = a[axis].min(b[axis]) - tolerance;
            let high = a[axis].max(b[axis]) + tolerance;
            let start = sorted.partition_point(|&i| mesh.positions[i][axis] < low);
            let end = sorted.partition_point(|&i| mesh.positions[i][axis] <= high);
            charge(work, end - start)?;
            let mut splits = Vec::new();
            for &index in &sorted[start..end] {
                let candidate = mesh.positions[index];
                if candidate == a || candidate == b {
                    continue;
                }
                let offset = [candidate[0] - a[0], candidate[1] - a[1]];
                let along = offset[0] * direction[0] + offset[1] * direction[1];
                if along > 0.
                    && along < length
                    && (offset[0] - along * direction[0]).hypot(offset[1] - along * direction[1])
                        <= tolerance
                {
                    splits.push((along, candidate));
                }
            }
            splits.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            for (_, point) in splits {
                if perimeter.last() != Some(&point) {
                    perimeter.push(point);
                }
            }
        }
        if perimeter.len() == 3 {
            result.triangle(p)?;
        } else {
            let center = std::array::from_fn(|axis| {
                p[0][axis] + ((p[1][axis] - p[0][axis]) + (p[2][axis] - p[0][axis])) / 3.
            });
            for edge in 0..perimeter.len() {
                result.triangle([
                    center,
                    perimeter[edge],
                    perimeter[(edge + 1) % perimeter.len()],
                ])?;
            }
        }
    }
    Ok(result.mesh)
}

fn charge(work: &mut usize, count: usize) -> Result<()> {
    *work = work.saturating_add(count);
    if *work > WORK_BUDGET {
        return Err("experimental mesh edge-conformance budget exceeded".into());
    }
    Ok(())
}

#[derive(Default)]
struct Builder {
    mesh: Mesh,
    indices: BTreeMap<[u64; 3], u32>,
}

impl Builder {
    fn triangle(&mut self, p: [[f64; 3]; 3]) -> Result<()> {
        if self.mesh.triangles.len() >= MAX_TRIANGLES {
            return Err("stepped mesh output budget exceeded".into());
        }
        let ab = std::array::from_fn::<_, 3, _>(|i| p[1][i] - p[0][i]);
        let ac = std::array::from_fn::<_, 3, _>(|i| p[2][i] - p[0][i]);
        let normal = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        if p.iter().flatten().any(|v| !v.is_finite())
            || normal.iter().any(|v| !v.is_finite())
            || normal == [0.; 3]
        {
            return Err(format!("stepped triangulation generated a degenerate face: {p:?}").into());
        }
        let mut indices = [0; 3];
        for (index, point) in p.into_iter().enumerate() {
            let position_key = point.map(key);
            indices[index] = if let Some(&existing) = self.indices.get(&position_key) {
                existing
            } else {
                if self.mesh.positions.len() >= MAX_VERTICES {
                    return Err("stepped mesh output budget exceeded".into());
                }
                let next = self.mesh.positions.len() as u32;
                self.mesh.positions.push(point);
                self.indices.insert(position_key, next);
                next
            };
        }
        self.mesh.triangles.push(indices);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn junction_tetrahedron(z: f64) -> Mesh {
        Mesh {
            positions: vec![
                [0., 0., z],
                [2., 0., z],
                [0., 2., z],
                [0., 0., z + 2.],
                [1., 0., z],
            ],
            triangles: vec![[0, 2, 1], [0, 4, 3], [4, 1, 3], [0, 3, 2], [1, 2, 3]],
        }
    }

    fn assert_closed_oriented(mesh: &Mesh) {
        let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
        for t in &mesh.triangles {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                let entry = edges.entry((a.min(b), a.max(b))).or_default();
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
        }
        assert!(
            edges.values().all(|&value| value == (2, 0)),
            "every edge must have two oppositely wound incident faces"
        );
    }

    #[test]
    fn splits_horizontal_t_junction_into_a_closed_oriented_surface() {
        let mesh = split_boundary_edges(junction_tetrahedron(0.), 1e-8, &mut 0).unwrap();
        assert_eq!(mesh.triangles.len(), 8);
        assert_closed_oriented(&mesh);
        let volume: f64 = mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
                (a[0] * (b[1] * c[2] - b[2] * c[1])
                    + a[1] * (b[2] * c[0] - b[0] * c[2])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.
            })
            .sum();
        assert!((volume - 4. / 3.).abs() < 1e-12);
    }

    #[test]
    fn handles_layer_stack_whose_global_vertex_triangle_product_exceeds_old_budget() {
        let mut stack = Mesh::default();
        for layer in 0..2048 {
            let part = junction_tetrahedron(layer as f64 * 3.);
            let offset = stack.positions.len() as u32;
            stack.positions.extend(part.positions);
            stack
                .triangles
                .extend(part.triangles.into_iter().map(|t| t.map(|i| i + offset)));
        }
        assert!(stack.positions.len() * stack.triangles.len() > WORK_BUDGET);
        let result = split_boundary_edges(stack, 1e-8, &mut 0).unwrap();
        assert_eq!(result.triangles.len(), 2048 * 8);
        assert_closed_oriented(&result);
    }

    #[test]
    fn stitches_boundary_vertices_within_arithmetic_tolerance() {
        for swap_axes in [false, true] {
            let mut source = junction_tetrahedron(0.);
            source.positions[4][1] = 1e-10;
            if swap_axes {
                for point in &mut source.positions {
                    point.swap(0, 1);
                }
                for triangle in &mut source.triangles {
                    triangle.swap(0, 1);
                }
            }
            let result = split_boundary_edges(source, 1e-8, &mut 0).unwrap();
            assert_eq!(result.triangles.len(), 8);
            assert_closed_oriented(&result);
        }
    }

    #[test]
    fn does_not_refine_already_matched_edges_of_a_thin_cap() {
        let source = Mesh {
            positions: vec![[0., 0., 0.], [2., 0., 0.], [1., 1e-10, 0.], [0., 0., 2.]],
            triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
        };
        let result = split_boundary_edges(source, 1e-8, &mut 0).unwrap();
        assert_eq!(result.triangles.len(), 4);
        assert_closed_oriented(&result);
    }

    #[test]
    fn preserves_gaps_larger_than_arithmetic_tolerance() {
        let mesh = Mesh {
            positions: vec![
                [0., 0., 0.],
                [2., 0., 0.],
                [0., 2., 0.],
                [1., -1e-6, 0.],
                [1., -1., 0.],
                [2., -1., 0.],
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
        };
        let result = split_boundary_edges(mesh, 1e-8, &mut 0).unwrap();
        assert_eq!(
            result.triangles.len(),
            2,
            "nearby off-edge vertices must not bend either boundary"
        );
        assert_eq!(result.positions.len(), 6);
    }
}
