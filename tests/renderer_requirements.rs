use std::path::PathBuf;
use turninator::{
    export::ExportKind,
    render::{
        BackendId, RenderSelection, ToolPaths,
        capabilities::BackendAvailability,
        registry::BackendRegistry,
        requirements::{ToolRequirements, requirements_for, validate_requirements},
    },
};

#[test]
fn operations_require_only_the_tools_they_use() {
    for backend in BackendId::ALL {
        let selection = RenderSelection::for_backend(backend, false);
        let blender = matches!(backend, BackendId::BlenderEevee | BackendId::BlenderCycles);
        for kind in [
            ExportKind::Still,
            ExportKind::Diagram,
            ExportKind::SectionStill,
            ExportKind::Gif,
            ExportKind::SectionTurntable,
            ExportKind::Mp4,
        ] {
            for encode_only in [false, true] {
                let required = requirements_for(&selection, kind, encode_only);
                assert_eq!(
                    required,
                    ToolRequirements {
                        blender: blender && !encode_only,
                        ffmpeg: matches!(
                            kind,
                            ExportKind::Gif | ExportKind::SectionTurntable | ExportKind::Mp4
                        ),
                        ffprobe: kind == ExportKind::Mp4,
                    }
                );
                let valid = std::env::current_exe().unwrap();
                let invalid = PathBuf::from("missing-tool.exe");
                for index in 0..3 {
                    let mut tools = ToolPaths {
                        blender: valid.clone(),
                        ffmpeg: valid.clone(),
                        ffprobe: valid.clone(),
                    };
                    match index {
                        0 => tools.blender = invalid.clone(),
                        1 => tools.ffmpeg = invalid.clone(),
                        _ => tools.ffprobe = invalid.clone(),
                    }
                    assert_eq!(
                        validate_requirements(&tools, &required).is_err(),
                        [required.blender, required.ffmpeg, required.ffprobe][index],
                        "{backend:?} {kind:?} encode_only={encode_only} tool={index}"
                    );
                }
            }
        }
    }
}

#[test]
fn registry_status_is_lazy_and_embedded_choices_remain_unavailable() {
    let registry = BackendRegistry::default();
    for backend in BackendId::ALL {
        let status = registry.status(backend);
        assert_eq!(status.backend, backend);
        match backend {
            BackendId::BlenderEevee | BackendId::BlenderCycles => {
                assert_eq!(status.availability, BackendAvailability::NotProbed)
            }
            _ => assert!(matches!(
                status.availability,
                BackendAvailability::Unavailable { .. }
            )),
        }
    }
}
