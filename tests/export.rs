use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
use turninator::export::{encode::validate_frames, jobs::ExportRequest};

#[test]
fn four_view_layout_and_cameras_keep_one_physical_scale() {
    let layout = turninator::export::jobs::diagram_layout(4096).unwrap();
    assert_eq!(
        (
            layout.margin,
            layout.panel_width,
            layout.panel_height,
            layout.label_height
        ),
        (64, 1952, 1856, 96)
    );
    let cameras = turninator::export::jobs::diagram_cameras(
        [20.0, 12.0, 80.0],
        layout.panel_width,
        layout.panel_height,
    );
    assert_eq!(cameras[0]["direction"], serde_json::json!([0., -1., 0.]));
    assert_eq!(cameras[2]["up"], serde_json::json!([0., 1., 0.]));
    assert!(
        cameras
            .iter()
            .all(|c| c["ortho_span_mm"] == cameras[0]["ortho_span_mm"])
    );
}

#[test]
fn actual_native_encoder_handles_percent_path_and_preserves_failure_destination() {
    use std::sync::atomic::AtomicBool;
    let tools = turninator::render::ToolPaths::default();
    if !tools.ffmpeg.is_file() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "turninator-native-%04d-é-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    for (i, color) in [[255, 0, 0], [0, 0, 255]].into_iter().enumerate() {
        image::RgbImage::from_pixel(24, 24, image::Rgb(color))
            .save(dir.join(format!("frame_{i:04}.png")))
            .unwrap();
    }
    let destination = dir.join("animation.gif");
    let cancel = AtomicBool::new(false);
    let result =
        turninator::export::encode::encode_gif(&dir, &destination, &tools, 2, 24, &cancel).unwrap();
    assert_eq!((result.frame_count, result.duration_ms), (2, 100));
    let previous = fs::read(&destination).unwrap();
    fs::write(dir.join("frame_0001.png"), b"corrupt").unwrap();
    assert!(
        turninator::export::encode::encode_gif(&dir, &destination, &tools, 2, 24, &cancel).is_err()
    );
    assert_eq!(fs::read(&destination).unwrap(), previous);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn frames_are_decoded_and_dimensions_checked() {
    let dir = std::env::temp_dir().join(format!(
        "turninator-export-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    image::RgbImage::new(24, 24)
        .save(dir.join("frame_0000.png"))
        .unwrap();
    assert!(validate_frames(&dir, 1, 24).is_ok());
    assert!(validate_frames(&dir, 2, 24).is_err());
    assert!(validate_frames(&dir, 1, 32).is_err());
    fs::write(dir.join("frame_0000.png"), b"broken png").unwrap();
    assert!(validate_frames(&dir, 1, 24).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn output_defaults_match_primary_loop_contract() {
    let encoding = turninator::export::encode::gif_encoding_recipe();
    assert_eq!(encoding["palette"], "full,opaque,sierra2_4a");
    assert_eq!(encoding["fps"], 20);
    assert_eq!(encoding["final_delay_cs"], 5);
    let spec = ExportRequest::default();
    assert_eq!(spec.size, 960);
    assert_eq!(spec.frame_count, 80);
    assert!(!spec.transparent);
    assert_eq!(spec.video_codec, turninator::export::jobs::VideoCodec::H264);
    assert_eq!(spec.layout, turninator::export::jobs::ExportLayout::Single);
    assert!(spec.section.is_none());
    assert_eq!(spec.animation, turninator::export::AnimationMode::Turntable);
}

#[test]
fn actual_av1_encoder_reports_av1_codec_and_preserves_timing() {
    use std::sync::atomic::AtomicBool;
    let tools = turninator::render::ToolPaths::default();
    if !tools.ffmpeg.is_file() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "turninator-av1-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    for (i, color) in [[255, 0, 0], [0, 0, 255]].into_iter().enumerate() {
        image::RgbImage::from_pixel(64, 64, image::Rgb(color))
            .save(dir.join(format!("frame_{i:04}.png")))
            .unwrap();
    }
    let result = turninator::export::encode::encode_mp4_with_codec(
        &dir,
        &dir.join("av1.mp4"),
        &tools,
        2,
        64,
        64,
        turninator::export::jobs::VideoCodec::Av1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!((result.frame_count, result.duration_ms), (2, 100));
    fs::remove_dir_all(dir).unwrap();
}
