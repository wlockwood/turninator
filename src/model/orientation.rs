//! Rigid orientation helpers. A face normal follows its mesh winding; no repair
//! or physical contact/stability simulation is implied by settling one face.
use crate::Result;

pub fn rotation_matrix(degrees: [f64; 3]) -> Result<[[f64; 3]; 3]> {
    if degrees.iter().any(|v| !v.is_finite()) {
        return Err("orientation angles must be finite".into());
    }
    let [x, y, z] = degrees.map(|a| a.rem_euclid(360.).to_radians());
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    Ok([
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ])
}

/// Apply `before`, then `after`, returning Euler XYZ degrees for their composition.
pub fn compose_rotations(after: [f64; 3], before: [f64; 3]) -> Result<[f64; 3]> {
    let after_matrix = rotation_matrix(after)?;
    let before_matrix = rotation_matrix(before)?;
    if after == [0.; 3] {
        return Ok(before);
    }
    Ok(matrix_to_euler(multiply(after_matrix, before_matrix)))
}

/// Preserve the current world orientation after `baked` is applied to the mesh.
/// The returned rotation maps baked coordinates into the current displayed pose.
pub fn relative_rotation(current: [f64; 3], baked: [f64; 3]) -> Result<[f64; 3]> {
    let current_matrix = rotation_matrix(current)?;
    let baked_matrix = rotation_matrix(baked)?;
    if current == baked {
        return Ok([0.; 3]);
    }
    let inverse_baked = std::array::from_fn(|i| std::array::from_fn(|j| baked_matrix[j][i]));
    Ok(matrix_to_euler(multiply(current_matrix, inverse_baked)))
}

/// Compose the shortest world-space rotation placing the selected face's
/// winding normal down (-Z). Returns Euler XYZ degrees, leaving mesh/build
/// coordinates unchanged. The same pure helper can orient a physical STL build
/// when explicitly used with that build's source-normal and prior rotation.
pub fn settled_display_rotation(old_euler: [f64; 3], build_normal: [f64; 3]) -> Result<[f64; 3]> {
    let normal =
        normalized(build_normal).ok_or("selected face normal must be finite and nonzero")?;
    let old = rotation_matrix(old_euler)?;
    let current = normalized(apply(old, normal)).ok_or("invalid rotated face normal")?;
    let down = [0., 0., -1.];
    let axis = cross(current, down);
    let sine = dot(axis, axis).sqrt();
    let cosine = dot(current, down).clamp(-1., 1.);
    let alignment = if sine > 1e-15 {
        axis_angle(axis.map(|v| v / sine), sine.atan2(cosine))
    } else if cosine < 0. {
        // Choose a deterministic, well-conditioned perpendicular for 180 degrees.
        let least = (0..3)
            .min_by(|&a, &b| current[a].abs().total_cmp(&current[b].abs()))
            .unwrap();
        let mut basis = [0.; 3];
        basis[least] = 1.;
        axis_angle(
            normalized(cross(current, basis)).unwrap(),
            std::f64::consts::PI,
        )
    } else {
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
    };
    Ok(matrix_to_euler(multiply(alignment, old)))
}

fn multiply(after: [[f64; 3]; 3], before: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| (0..3).map(|k| after[i][k] * before[k][j]).sum())
    })
}

fn matrix_to_euler(matrix: [[f64; 3]; 3]) -> [f64; 3] {
    // atan2 retains the tiny cosine near a pole; asin can turn roundoff in a
    // nominal unit sine into an artificial ~1e-8 radian tilt.
    let y = (-matrix[2][0]).atan2(matrix[0][0].hypot(matrix[1][0]));
    let (x, z) = if y.cos().abs() > 1e-12 {
        (
            matrix[2][1].atan2(matrix[2][2]),
            matrix[1][0].atan2(matrix[0][0]),
        )
    } else {
        (0., (-matrix[0][1]).atan2(matrix[1][1]))
    };
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

fn axis_angle(axis: [f64; 3], angle: f64) -> [[f64; 3]; 3] {
    let (s, c) = angle.sin_cos();
    let [x, y, z] = axis;
    let skew = [[0., -z, y], [z, 0., -x], [-y, x, 0.]];
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            axis[i] * axis[j] * (1. - c) + skew[i][j] * s + if i == j { c } else { 0. }
        })
    })
}
pub(crate) fn apply(m: [[f64; 3]; 3], p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| dot(m[i], p))
}
pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn normalized(v: [f64; 3]) -> Option<[f64; 3]> {
    if v.iter().any(|a| !a.is_finite()) {
        return None;
    }
    let scale = v.iter().fold(0f64, |m, a| m.max(a.abs()));
    if scale == 0. {
        return None;
    }
    let v = v.map(|a| a / scale);
    let length = dot(v, v).sqrt();
    Some(v.map(|a| a / length))
}
