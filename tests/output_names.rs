use std::path::Path;
use turninator::export::ExportKind;

#[test]
fn export_suggestions_follow_the_loaded_model_and_output_format() {
    let first = Path::new("fixtures/spiral egg - A.stl");
    let second = Path::new("fixtures/Deck Post Jig.stl");
    for (kind, extension) in [
        (ExportKind::Gif, "gif"),
        (ExportKind::Still, "png"),
        (ExportKind::Diagram, "png"),
        (ExportKind::Mp4, "mp4"),
        (ExportKind::SectionStill, "png"),
        (ExportKind::SectionTurntable, "gif"),
    ] {
        assert_eq!(
            kind.suggested_file_name(first),
            format!("spiral egg - A.{extension}")
        );
        assert_eq!(
            kind.suggested_file_name(second),
            format!("Deck Post Jig.{extension}")
        );
        assert_eq!(kind.extension(), extension);
    }
}

#[test]
fn output_names_preserve_unicode_and_dots_without_leaking_directories() {
    assert_eq!(
        ExportKind::Still.suggested_file_name(Path::new("fixtures/歯車.v2.STL")),
        "歯車.v2.png"
    );
    assert_eq!(
        ExportKind::Gif.suggested_file_name(Path::new("fixtures/tool.paths.gcode")),
        "tool.paths.gif"
    );
    assert_eq!(
        ExportKind::Still.suggested_file_name(Path::new("fixtures/model")),
        "model.png"
    );
    assert_eq!(
        ExportKind::Still.suggested_file_name(Path::new("")),
        "turninator.png"
    );
}
