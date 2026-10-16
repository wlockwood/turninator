#![allow(dead_code)]
use serde_json::json;
use std::sync::Arc;
use turninator::{mesh::Mesh, render::contract::*};
pub fn cube(min: [f64; 3], size: f64) -> Mesh {
    let positions = [
        [0., 0., 0.],
        [1., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [1., 0., 1.],
        [1., 1., 1.],
        [0., 1., 1.],
    ]
    .map(|p| std::array::from_fn(|i| min[i] + size * p[i]))
    .to_vec();
    let triangles = vec![
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
    Mesh {
        positions,
        triangles,
    }
}
pub fn append(a: &mut Mesh, b: Mesh) {
    let offset = a.positions.len() as u32;
    a.positions.extend(b.positions);
    a.triangles
        .extend(b.triangles.into_iter().map(|t| t.map(|i| i + offset)));
}
pub fn scene(mesh: Mesh) -> RenderSceneSnapshot {
    snapshot_scene(Arc::new(mesh),json!({"schema_version":1,"mesh_path":"D:/fixture/mesh.bin","color":[0.2,0.4,0.6],"roughness":0.45,"geometry_kind":"original","layer_height_mm":0.2,"display_rotation_degrees":[0.,0.,0.],"warnings":[]})).unwrap()
}
pub fn frame(s: &RenderSceneSnapshot) -> FrameSpec {
    FrameSpec {
        key: FrameKey {
            index: 0,
            panel: RenderPanel::Single,
        },
        camera: s.camera.clone(),
        angle_degrees: 0.,
        visible_layers: None,
        section: None,
        width: 512,
        height: 512,
        time_seconds: 0.,
    }
}
pub fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-4_f64.max(b.abs() * 1e-5), "{a} != {b}")
}
