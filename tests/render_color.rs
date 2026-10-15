use turninator::render::{color::normalize_rgba, contract::*};
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
