use turninator::model::orientation::{
    compose_rotations, relative_rotation, rotation_matrix, settled_display_rotation,
};

fn apply(matrix: [[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|j| matrix[i][j] * vector[j]).sum())
}

fn close(actual: [f64; 3], expected: [f64; 3]) {
    for i in 0..3 {
        assert!(
            (actual[i] - expected[i]).abs() < 1e-11,
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn composition_applies_before_then_after_without_adding_euler_angles() {
    let before = [90., 0., 0.];
    let after = [0., 0., 90.];
    let composed = rotation_matrix(compose_rotations(after, before).unwrap()).unwrap();
    close(apply(composed, [1., 0., 0.]), [0., 1., 0.]);
    close(apply(composed, [0., 1., 0.]), [0., 0., 1.]);
    close(apply(composed, [0., 0., 1.]), [1., 0., 0.]);
    let reversed = rotation_matrix(compose_rotations(before, after).unwrap()).unwrap();
    close(apply(reversed, [1., 0., 0.]), [0., 0., 1.]);
}

#[test]
fn composition_preserves_directions_at_gimbal_lock_and_half_turns() {
    for (after, before) in [
        ([0., 90., 30.], [0.; 3]),
        ([0., -90., 30.], [0.; 3]),
        ([180., 0., 0.], [23., -71., 142.]),
        ([27., 15., -42.], [0., 90., 30.]),
        ([27., 15., -42.], [0., -90., 30.]),
    ] {
        let composed = rotation_matrix(compose_rotations(after, before).unwrap()).unwrap();
        for direction in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
            close(
                apply(composed, direction),
                apply(
                    rotation_matrix(after).unwrap(),
                    apply(rotation_matrix(before).unwrap(), direction),
                ),
            );
        }
    }
}

#[test]
fn settling_a_face_on_an_already_rotated_build_composes_with_the_build_pose() {
    let source_normal = [1., 0., 0.];
    let build = [0., 0., 90.];
    let build_normal = apply(rotation_matrix(build).unwrap(), source_normal);
    let settled = settled_display_rotation([90., 0., 0.], build_normal).unwrap();
    let next_build = compose_rotations(settled, build).unwrap();
    close(
        apply(rotation_matrix(next_build).unwrap(), source_normal),
        [0., 0., -1.],
    );
}

#[test]
fn residual_preserves_world_directions_after_baking_with_intervening_pose_edits() {
    for (baked, current) in [
        ([27., 71., -53.], [-32., 18., 109.]),
        ([0., 90., 30.], [75., -90., -42.]),
        ([180., 0., 0.], [0.; 3]),
    ] {
        let residual = rotation_matrix(relative_rotation(current, baked).unwrap()).unwrap();
        for direction in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0.3, -0.2, 0.7]] {
            close(
                apply(residual, apply(rotation_matrix(baked).unwrap(), direction)),
                apply(rotation_matrix(current).unwrap(), direction),
            );
        }
    }
}

#[test]
fn baking_the_exact_current_pose_leaves_no_residual_tilt() {
    for pose in [[23., -71., 142.], [0., 90., 30.], [1e100, -1e100, 1e100]] {
        assert_eq!(relative_rotation(pose, pose).unwrap(), [0.; 3]);
    }
}

#[test]
fn composing_with_no_new_rotation_preserves_the_original_angles_exactly() {
    for pose in [[23., -71., 142.], [0., 90., 30.], [1e100, -1e100, 1e100]] {
        assert_eq!(compose_rotations([0.; 3], pose).unwrap(), pose);
    }
}

#[test]
fn composition_and_residual_reject_nonfinite_angles_before_identity_shortcuts() {
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for axis in 0..3 {
            let mut pose = [0.; 3];
            pose[axis] = invalid;
            assert!(compose_rotations([0.; 3], pose).is_err());
            assert!(compose_rotations(pose, [0.; 3]).is_err());
            assert!(relative_rotation([0.; 3], pose).is_err());
            assert!(relative_rotation(pose, [0.; 3]).is_err());
            assert!(relative_rotation(pose, pose).is_err());
        }
    }
}
