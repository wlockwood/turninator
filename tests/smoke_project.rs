use turninator::viewport::OrbitCamera;
#[test]
fn desktop_camera_json_roundtrip_preserves_exact_physical_framing() {
    let original = OrbitCamera {
        distance_mm: 305.84026680890406,
        target_mm: [
            -1.4441013718169415e-8,
            -1.8896532196777116e-6,
            53.000003901870286,
        ],
        ortho_span_mm: 183.93588390274343,
        ..Default::default()
    };
    let camera: OrbitCamera =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    assert_eq!(camera.camera_spec(), original.camera_spec());
}
