use turninator::render::color::AgxRecipe;
use turninator::render::{color::normalize_rgba, contract::*};

// Catches omitted/wrong Rec.709 conversions, transposed matrices, wrong polynomial,
// wrong gamma/transfer ordering, clipping before tone mapping, and exposure drift.
#[test]
fn hdr_mapping_matches_independent_filament_sdk_chart() {
    let chart: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/renderers/color-chart.json")).unwrap();
    let tolerance = chart["absolute_tolerance"].as_f64().unwrap() as f32;
    for patch in chart["patches"].as_array().unwrap() {
        let input = std::array::from_fn(|i| patch["scene_rgba"][i].as_f64().unwrap() as f32);
        let recipe = AgxRecipe {
            exposure_ev: patch["exposure_ev"].as_f64().unwrap() as f32,
        };
        for (field, actual) in [
            (
                "display_linear_rgba",
                recipe.map_linear_rgba(input).unwrap(),
            ),
            ("display_srgb_rgba", recipe.map_srgb_rgba(input).unwrap()),
        ] {
            for (i, value) in actual.into_iter().enumerate() {
                let expected = patch[field][i].as_f64().unwrap() as f32;
                assert!(
                    (value - expected).abs() <= tolerance,
                    "{} {field}[{i}]: got {value}, expected {expected}",
                    patch["name"]
                );
            }
        }
    }
}

#[test]
fn hdr_alpha_is_independent_including_transparent_rgb() {
    let recipe = AgxRecipe::default();
    let opaque = recipe.map_srgb_rgba([4., 0.18, 0.01, 1.]).unwrap();
    for alpha in [0., 0.25, 0.5, 1.] {
        let output = recipe.map_srgb_rgba([4., 0.18, 0.01, alpha]).unwrap();
        assert_eq!(&output[..3], &opaque[..3]);
        assert_eq!(output[3], alpha);
    }
}

#[test]
fn hdr_rejects_nonfinite_invalid_alpha_exposure_and_scaled_overflow() {
    for input in [
        [f32::NAN, 1., 1., 1.],
        [1., f32::INFINITY, 1., 1.],
        [1., 1., f32::NEG_INFINITY, 1.],
        [1., 1., 1., f32::NAN],
        [1., 1., 1., -0.01],
        [1., 1., 1., 1.01],
    ] {
        assert!(AgxRecipe::default().map_linear_rgba(input).is_err());
        assert!(AgxRecipe::default().map_srgb_rgba(input).is_err());
    }
    for exposure_ev in [f32::NAN, f32::INFINITY, -32.01, 32.01] {
        assert!(AgxRecipe { exposure_ev }.map_linear_rgba([1.; 4]).is_err());
    }
    assert!(
        AgxRecipe { exposure_ev: 1. }
            .map_linear_rgba([f32::MAX, 0., 0., 1.])
            .is_err()
    );
}

#[test]
fn hdr_transfer_quantizes_to_srgb_cpu_pixels_once() {
    let mapped = AgxRecipe::default()
        .map_srgb_rgba([0.18, 0.18, 0.18, 0.5])
        .unwrap();
    let bytes: Vec<_> = mapped.map(|v| (v * 255.).round() as u8).into();
    assert_eq!(normalize_rgba(pixels(bytes.clone())).unwrap().bytes, bytes);
    assert_eq!(bytes, [128, 128, 128, 128]);
}

#[test]
fn hdr_exposure_changes_identity_and_matches_scene_scaling() {
    let default = AgxRecipe::default();
    assert_eq!(default.exposure_ev, 0.);
    let up = AgxRecipe { exposure_ev: 2. };
    assert_ne!(default.digest(), up.digest());
    assert_eq!(default.digest(), AgxRecipe { exposure_ev: -0. }.digest());
    let a = up.map_linear_rgba([0.18, 0.05, 1., 0.5]).unwrap();
    let b = default.map_linear_rgba([0.72, 0.2, 4., 0.5]).unwrap();
    assert_eq!(a, b);
}

#[test]
fn hdr_over_range_colors_attenuate_chroma_instead_of_clipping_input() {
    let recipe = AgxRecipe::default();
    let red = recipe.map_linear_rgba([1., 0., 0., 1.]).unwrap();
    let hdr = recipe.map_linear_rgba([16., 0., 0., 1.]).unwrap();
    assert!(hdr[1] > red[1] + 0.5 && hdr[2] > red[2] + 0.5);
    assert!(hdr[0] - hdr[2] < red[0] - red[2]);
}

#[test]
fn portable_wgsl_parses_and_validates_without_gpu() {
    use eframe::wgpu::naga;
    let module = naga::front::wgsl::parse_str(turninator::render::color::AGX_WGSL)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(turninator::render::color::AGX_WGSL)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn portable_layer_filter_constants_preserve_qualified_cpu_transition() {
    let recipe: serde_json::Value =
        serde_json::from_str(turninator::render::color::AGX_RECIPE_JSON).unwrap();
    let layer = &recipe["layer_filter"];
    let start = layer["start_pixels"].as_f64().unwrap();
    let end = layer["end_pixels"].as_f64().unwrap();
    let diagonal = layer["footprint_diagonal_factor"].as_f64().unwrap();
    for (pixels, expected) in [(start, 0.), ((start + end) / 2., 0.5), (end, 1.)] {
        let actual =
            turninator::render::geometry::layer_filter_gain(pixels * diagonal, 1., 1.).unwrap();
        assert!((actual - expected).abs() < 1e-12);
    }
}
fn pixels(bytes: Vec<u8>) -> NativePixels {
    NativePixels {
        width: 1,
        height: 1,
        row_stride: 4,
        bytes,
        origin: RowOrigin::Top,
        channels: PixelChannels::Rgba,
        alpha: PixelAlpha::Straight,
        transfer: PixelTransfer::Srgb,
    }
}
#[test]
fn srgb_patch_is_not_encoded_twice() {
    assert_eq!(
        normalize_rgba(pixels(vec![128, 64, 32, 255]))
            .unwrap()
            .bytes,
        [128, 64, 32, 255]
    );
}
#[test]
fn bgra_bottom_up_padding_is_removed() {
    let mut p = pixels(vec![3, 2, 1, 255, 99, 99, 6, 5, 4, 255, 88, 88]);
    p.height = 2;
    p.row_stride = 6;
    p.channels = PixelChannels::Bgra;
    p.origin = RowOrigin::Bottom;
    assert_eq!(
        normalize_rgba(p).unwrap().bytes,
        [4, 5, 6, 255, 1, 2, 3, 255]
    );
}
#[test]
fn unpremultiplies_in_declared_producer_domain() {
    for (transfer, expected) in [(PixelTransfer::Srgb, 128), (PixelTransfer::Linear, 188)] {
        let mut p = pixels(vec![64, 64, 64, 128]);
        p.alpha = PixelAlpha::Premultiplied;
        p.transfer = transfer;
        let b = normalize_rgba(p).unwrap().bytes;
        assert!((b[0] as i32 - expected).abs() <= 1);
        assert_eq!(b[3], 128);
    }
}
#[test]
fn transparent_rgb_is_zero_for_both_alpha_modes() {
    for alpha in [PixelAlpha::Straight, PixelAlpha::Premultiplied] {
        let mut p = pixels(vec![255, 12, 80, 0]);
        p.alpha = alpha;
        assert_eq!(normalize_rgba(p).unwrap().bytes, [0, 0, 0, 0]);
    }
}
#[test]
fn linear_gray_encodes_once() {
    let mut p = pixels(vec![128, 128, 128, 255]);
    p.transfer = PixelTransfer::Linear;
    assert_eq!(normalize_rgba(p).unwrap().bytes, [188, 188, 188, 255]);
}
#[test]
fn invalid_dimensions_stride_and_checked_lengths_are_rejected() {
    for (w, h, stride, bytes) in [
        (0, 1, 4, vec![0; 4]),
        (8001, 1, 32004, vec![]),
        (1, 1, 3, vec![0; 4]),
        (1, 2, usize::MAX, vec![]),
        (1, 1, 4, vec![0; 3]),
        (1, 1, 4, vec![0; 5]),
    ] {
        let mut p = pixels(bytes);
        p.width = w;
        p.height = h;
        p.row_stride = stride;
        assert!(normalize_rgba(p).is_err());
    }
}
