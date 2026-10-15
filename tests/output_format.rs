use turninator::export::{ExportKind, OutputFormat};

#[test]
fn output_selection_maps_to_the_actual_export_and_applicable_settings() {
    for (format, kind, animated, transparency, section) in [
        (OutputFormat::Gif, ExportKind::Gif, true, false, true),
        (OutputFormat::Mp4, ExportKind::Mp4, true, false, true),
        (OutputFormat::Png, ExportKind::Still, false, true, true),
        (
            OutputFormat::FourView,
            ExportKind::Diagram,
            false,
            false,
            false,
        ),
    ] {
        assert_eq!(format.kind(), kind);
        assert_eq!(format.animated(), animated);
        assert_eq!(format.supports_transparency(), transparency);
        assert_eq!(format.supports_section(), section);
    }
    assert_eq!(OutputFormat::default(), OutputFormat::Gif);
}
