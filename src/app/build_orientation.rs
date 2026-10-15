//! Transfer a displayed STL pose into slicing without rotating accepted geometry twice.
use crate::{Result, presentation::SceneSettings};

pub(super) struct PendingBuildOrientation {
    display_rotation: [f64; 3],
}
impl PendingBuildOrientation {
    pub fn prepare(scene: &mut SceneSettings) -> Result<Self> {
        let rotation = crate::model::orientation::compose_rotations(
            scene.display_rotation_degrees,
            scene.import.build_rotation_degrees,
        )?;
        let pending = Self {
            display_rotation: scene.display_rotation_degrees,
        };
        scene.import.build_rotation_degrees = rotation;
        scene.import.stepped_envelope = true;
        Ok(pending)
    }
    pub fn accepted_display_rotation(&self, current: [f64; 3]) -> Result<[f64; 3]> {
        crate::model::orientation::relative_rotation(current, self.display_rotation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_pose_is_baked_for_slicing_but_kept_until_import_succeeds() {
        let mut scene = SceneSettings {
            display_rotation_degrees: [0., 90., 0.],
            ..Default::default()
        };
        let pending = PendingBuildOrientation::prepare(&mut scene).unwrap();
        assert!(scene.import.stepped_envelope);
        assert_eq!(scene.import.build_rotation_degrees, [0., 90., 0.]);
        assert_eq!(
            scene.display_rotation_degrees,
            [0., 90., 0.],
            "accepted old geometry stays in place during reimport"
        );
        assert_eq!(
            pending
                .accepted_display_rotation(scene.display_rotation_degrees)
                .unwrap(),
            [0.; 3]
        );
    }
    #[test]
    fn invalid_pose_leaves_the_import_and_display_settings_unchanged() {
        let mut scene = SceneSettings {
            display_rotation_degrees: [f64::INFINITY, 0., 0.],
            ..Default::default()
        };
        let before = scene.import.clone();
        assert!(PendingBuildOrientation::prepare(&mut scene).is_err());
        assert_eq!(scene.import, before);
        assert_eq!(scene.display_rotation_degrees, [f64::INFINITY, 0., 0.]);
    }
    #[test]
    fn changing_presentation_during_import_preserves_only_the_residual_pose() {
        use crate::model::orientation::rotation_matrix;
        let mut scene = SceneSettings {
            display_rotation_degrees: [24., 35., -12.],
            ..Default::default()
        };
        scene.import.build_rotation_degrees = [-16., 22., 77.];
        let prior = rotation_matrix(scene.import.build_rotation_degrees).unwrap();
        let pending = PendingBuildOrientation::prepare(&mut scene).unwrap();
        let current = [17., -48., 11.];
        let residual =
            rotation_matrix(pending.accepted_display_rotation(current).unwrap()).unwrap();
        let baked = rotation_matrix(scene.import.build_rotation_degrees).unwrap();
        let desired = rotation_matrix(current).unwrap();
        for i in 0..3 {
            for j in 0..3 {
                let actual = (0..3).map(|k| residual[i][k] * baked[k][j]).sum::<f64>();
                let expected = (0..3).map(|k| desired[i][k] * prior[k][j]).sum::<f64>();
                assert!((actual - expected).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn stepped_layers_start_from_the_current_down_and_survive_project_roundtrip() {
        use crate::presentation::{load_model, model_scene};
        let scratch = crate::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
        let path = scratch.0.join("asymmetric.stl");
        let vertices = [
            [0., 0., 0.],
            [20., 0., 0.],
            [20., 12., 0.],
            [0., 12., 0.],
            [0., 0., 10.],
            [20., 0., 10.],
            [20., 12., 10.],
            [0., 12., 10.],
        ];
        let triangles = [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        let mut stl = String::from("solid box\n");
        for t in triangles {
            stl.push_str("facet normal 0 0 0\nouter loop\n");
            for i in t {
                let [x, y, z] = vertices[i];
                stl.push_str(&format!("vertex {x} {y} {z}\n"));
            }
            stl.push_str("endloop\nendfacet\n");
        }
        stl.push_str("endsolid box\n");
        std::fs::write(&path, &stl).unwrap();
        let mut scene = SceneSettings {
            display_rotation_degrees: [0., 90., 0.],
            ..Default::default()
        };
        let original = load_model(&path, &scene.import).unwrap();
        assert_eq!(original.bounds_mm.max[2], 10.);
        let pending = PendingBuildOrientation::prepare(&mut scene).unwrap();
        let model = load_model(&path, &scene.import).unwrap();
        scene.display_rotation_degrees = pending
            .accepted_display_rotation(scene.display_rotation_degrees)
            .unwrap();
        assert_eq!(model.geometry_kind, "stepped");
        assert!((model.bounds_mm.max[2] - 20.).abs() < 1e-5);
        assert_eq!(model.bounds_mm.min[2], 0.);
        assert_eq!(model.layer_tops_mm.len(), 100);
        assert_ne!(model.snapshot_hash, original.snapshot_hash);
        let snapshot = model_scene(&model, &scene, &scratch.0.join("mesh.bin"), None).unwrap();
        assert_eq!(
            snapshot["display_rotation_degrees"],
            serde_json::json!([0., 0., 0.])
        );
        let restored: SceneSettings =
            serde_json::from_slice(&serde_json::to_vec(&scene).unwrap()).unwrap();
        assert_eq!(
            load_model(&path, &restored.import).unwrap().snapshot_hash,
            model.snapshot_hash
        );
        // Reapplying the already-accepted down does not rotate or reslice twice.
        let accepted = scene.import.clone();
        let _ = PendingBuildOrientation::prepare(&mut scene).unwrap();
        assert_eq!(scene.import, accepted);
        assert_eq!(std::fs::read_to_string(path).unwrap(), stl);
    }
}
