//! CPU-only DLL contract tests. Build explicitly with tools/build-renderers.ps1 -Fixture.
#![cfg(windows)]
use std::{path::PathBuf, sync::OnceLock};
use turninator::render::{
    BackendId, RenderSelection,
    contract::{
        BackendIdentity, PhysicalMaterial, RenderAppearance, RenderCamera, RenderLimits,
        RenderProjection,
    },
    embedded::{
        EmbeddedColorConfig, EmbeddedConfig, EmbeddedFrame, EmbeddedLayerFilter, EmbeddedLibrary,
        EmbeddedSceneMetadata, EmbeddedState, SceneUpload,
    },
};

fn fixture(name: &str) -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let source = PathBuf::from(
            std::env::var_os("TN_RENDERER_FIXTURE_DIR")
                .expect("Run tools/build-renderers.ps1 -Fixture and set TN_RENDERER_FIXTURE_DIR"),
        );
        let dest = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("renderers/fixtures");
        std::fs::create_dir_all(&dest).unwrap();
        for file in std::fs::read_dir(source).unwrap() {
            let file = file.unwrap();
            if file.path().extension().is_some_and(|s| s == "dll") {
                std::fs::copy(file.path(), dest.join(file.file_name())).unwrap();
            }
        }
        // The dependent fixture is isolated from its intentionally omitted dependency.
        std::fs::create_dir_all(dest.join("missing-dependency")).unwrap();
        std::fs::copy(
            dest.join("tn_fixture_dependent.dll"),
            dest.join("missing-dependency/tn_fixture_dependent.dll"),
        )
        .unwrap();
        dest
    })
    .join(name)
}
fn config() -> EmbeddedConfig {
    EmbeddedConfig {
        selection: RenderSelection::for_backend(BackendId::Filament, true),
        limits: RenderLimits::default(),
        cache_directory: None,
        identity: BackendIdentity {
            backend: BackendId::Filament,
            adapter_recipe_version: 1,
            runtime_digest: "test-only".into(),
            bridge_digest: None,
            shader_digest: None,
            material_digest: None,
            color_digest: "fixture".into(),
            denoiser_digest: None,
            device: "cpu-fixture".into(),
            api: "fixture".into(),
            driver: "none".into(),
        },
    }
}
#[test]
fn runtime_diagnostics_tracks_actual_completed_worker_frames() {
    let library =
        EmbeddedLibrary::load_fixture(&fixture("tn_fixture_diagnostic_snapshot.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    let initial = context.runtime_info().unwrap();
    assert_eq!(initial.diagnostics.unwrap().counters["frames"], 0);
    upload(&mut context, 1);
    context.begin_frame(&frame(1)).unwrap();
    wait_for_state(&mut context, EmbeddedState::Ready);
    let info = context.runtime_info().unwrap();
    assert_eq!(info.device_id, initial.device_id);
    let diagnostics = info.diagnostics.unwrap();
    assert_eq!(diagnostics.schema_version, 1);
    assert_eq!(diagnostics.counters["frames"], 1);
    assert_eq!(diagnostics.timings_ms["render"], 1.);
    assert_eq!(diagnostics.settings["effective_aa"], "cpu-complete");
    let mut pixels = [0; 16];
    context.read_pixels(&mut pixels).unwrap();
    context.begin_frame(&frame(1)).unwrap();
    wait_for_state(&mut context, EmbeddedState::Ready);
    assert_eq!(
        context
            .runtime_info()
            .unwrap()
            .diagnostics
            .unwrap()
            .counters["frames"],
        2
    );
}
fn frame(revision: u64) -> EmbeddedFrame {
    EmbeddedFrame {
        geometry_revision: revision,
        width: 2,
        height: 2,
        camera: RenderCamera {
            position_mm: [0., -10., 5.],
            target_mm: [0.; 3],
            up: [0., 0., 1.],
            projection: RenderProjection::Perspective {
                vertical_fov_degrees: 40.,
            },
        },
        model_to_world: [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        quality: config().selection.quality,
        transparent: true,
        sample_seed: 255,
    }
}
fn upload(context: &mut turninator::render::embedded::EmbeddedContext, revision: u64) {
    context
        .upload_scene(&SceneUpload {
            revision,
            positions_mm: &[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            normals: &[[0., 0., 1.]; 3],
            build_positions_mm: &[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            triangles: &[[0, 1, 2]],
            triangle_materials: &[0],
            metadata: &metadata(),
        })
        .unwrap();
}

fn wait_for_state(
    context: &mut turninator::render::embedded::EmbeddedContext,
    expected: EmbeddedState,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if context.poll().unwrap().state == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waiting for {expected:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn wait_for_failure(context: &mut turninator::render::embedded::EmbeddedContext) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Err(error) = context.poll() {
            return error.to_string();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waiting for worker failure"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn rejects_untrusted_missing_bad_abi_and_fixture_as_production() {
    let ambient = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("tn_fixture_dependency.dll");
    std::fs::copy(fixture("tn_fixture_dependency.dll"), &ambient).unwrap();
    for name in [
        "absent.dll",
        "tn_fixture_bad_version.dll",
        "tn_fixture_bad_size.dll",
        "tn_fixture_bad_backend.dll",
        "missing-dependency/tn_fixture_dependent.dll",
        "tn_fixture_missing_export.dll",
    ] {
        assert!(
            EmbeddedLibrary::load_fixture(&fixture(name)).is_err(),
            "accepted {name}"
        );
    }
    assert!(EmbeddedLibrary::load(&fixture("tn_fixture.dll"), BackendId::Filament).is_err());
    let outside = std::env::temp_dir().join("turninator-untrusted-fixture.dll");
    std::fs::copy(fixture("tn_fixture.dll"), &outside).unwrap();
    assert!(EmbeddedLibrary::load_fixture(&outside).is_err());
    std::fs::remove_file(outside).unwrap();
    let production = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("renderers/filament");
    std::fs::create_dir_all(&production).unwrap();
    let path = production.join("tn_renderer_filament.dll");
    std::fs::copy(fixture("tn_fixture.dll"), &path).unwrap();
    assert!(EmbeddedLibrary::load(&path, BackendId::Filament).is_err());
    let delayed = EmbeddedLibrary::load_fixture(&fixture("tn_fixture_delayed.dll")).unwrap();
    let mut context = delayed.create(&config()).unwrap();
    upload(&mut context, 1);
    context.begin_frame(&frame(1)).unwrap();
    let message = wait_for_failure(&mut context);
    assert!(
        message.contains("delayed dependency safely unavailable"),
        "{message}"
    );
    std::fs::remove_file(ambient).unwrap();
    let dependent = EmbeddedLibrary::load_fixture(&fixture("tn_fixture_dependent.dll")).unwrap();
    assert!(dependent.create(&config()).is_ok());
}

#[test]
fn upload_frame_read_retry_and_geometry_revision_reuse() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    assert_eq!(context.poll().unwrap().state, EmbeddedState::Idle);
    assert!(context.begin_frame(&frame(7)).is_err());
    upload(&mut context, 7);
    assert!(context.read_pixels(&mut [0; 16]).is_err());
    context.begin_frame(&frame(7)).unwrap();
    assert!(context.begin_frame(&frame(7)).is_err());
    assert!(
        context
            .upload_scene(&SceneUpload {
                revision: 8,
                positions_mm: &[],
                normals: &[],
                build_positions_mm: &[],
                triangles: &[],
                triangle_materials: &[],
                metadata: &metadata()
            })
            .is_err()
    );
    wait_for_state(&mut context, EmbeddedState::Ready);
    let mut small = [99; 15];
    assert!(context.read_pixels(&mut small).is_err());
    assert_eq!(small, [99; 15]);
    assert_eq!(context.poll().unwrap().state, EmbeddedState::Ready);
    let mut pixels = [99; 20];
    assert_eq!(context.read_pixels(&mut pixels).unwrap(), (2, 2));
    assert_eq!(
        &pixels[..16],
        &[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 0, 0, 0, 0]
    );
    assert_eq!(&pixels[16..], &[99; 4]);
    assert_eq!(context.poll().unwrap().state, EmbeddedState::Idle);
    context.begin_frame(&frame(7)).unwrap();
    assert!(matches!(
        context.poll().unwrap().state,
        EmbeddedState::Rendering | EmbeddedState::Ready
    ));
}

#[test]
fn cancel_acknowledges_only_after_pending_readback_drain() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    for cancel_ready in [false, true] {
        let mut context = library.create(&config()).unwrap();
        upload(&mut context, 3);
        context.begin_frame(&frame(3)).unwrap();
        if cancel_ready {
            wait_for_state(&mut context, EmbeddedState::Ready);
        }
        context.cancel().unwrap();
        context.cancel().unwrap();
        assert!(context.begin_frame(&frame(3)).is_err());
        assert!(context.read_pixels(&mut [0; 16]).is_err());
        wait_for_state(&mut context, EmbeddedState::Cancelled);
        assert_eq!(context.poll().unwrap().state, EmbeddedState::Idle);
        context.begin_frame(&frame(3)).unwrap();
    }
}

#[test]
fn validates_geometry_dimensions_limits_and_translates_exceptions() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    let metadata = metadata();
    let bad = SceneUpload {
        revision: 1,
        positions_mm: &[[0.; 3]],
        normals: &[],
        build_positions_mm: &[],
        triangles: &[[0, 0, 8]],
        triangle_materials: &[0],
        metadata: &metadata,
    };
    assert!(context.upload_scene(&bad).is_err());
    upload(&mut context, 1);
    for (width, height) in [(0, 2), (2, 0), (8001, 2), (u32::MAX, u32::MAX)] {
        let mut frame = frame(1);
        frame.width = width;
        frame.height = height;
        assert!(context.begin_frame(&frame).is_err());
    }
    let mut cfg = config();
    cfg.limits.gpu_bytes = 15;
    let mut limited = library.create(&cfg).unwrap();
    limited
        .upload_scene(&SceneUpload {
            revision: 1,
            positions_mm: &[],
            normals: &[],
            build_positions_mm: &[],
            triangles: &[],
            triangle_materials: &[],
            metadata: &metadata,
        })
        .unwrap();
    assert!(limited.begin_frame(&frame(1)).is_err());
    let throwing = EmbeddedLibrary::load_fixture(&fixture("tn_fixture_throw.dll")).unwrap();
    let mut context = throwing.create(&config()).unwrap();
    upload(&mut context, 1);
    context.begin_frame(&frame(1)).unwrap();
    let error = wait_for_failure(&mut context);
    assert!(error.contains("fixture exception"), "{error}");
    assert!(context.poll().is_err());
    assert!(context.cancel().is_err());
    assert!(context.begin_frame(&frame(1)).is_err());
}

fn metadata() -> EmbeddedSceneMetadata {
    EmbeddedSceneMetadata {
        appearance: RenderAppearance {
            color: [1., 0., 0.],
            roughness: 0.4,
            material_preset: "matte".into(),
            studio_preset: "softbox".into(),
            resin_clarity: 0.,
            resin_cloudiness: 0.,
            layer_appearance: false,
            first_layer_height_mm: 0.2,
            layer_height_mm: 0.2,
            groove_depth_mm: 0.02,
            display_rotation_degrees: [0.; 3],
        },
        materials: vec![PhysicalMaterial {
            unlit: false,
            name: "red".into(),
            base_color_linear: [1., 0., 0.],
            absorption_color_linear: [1.; 3],
            roughness: 0.4,
            metallic: 0.,
            anisotropy: 0.,
            coat: 0.,
            coat_roughness: 0.1,
            transmission: 0.,
            ior: 1.46,
            absorption_density_per_mm: 0.,
            scattering_per_mm: 0.,
            scattering_anisotropy: 0.,
        }],
        reference_bounds_mm: turninator::mesh::Bounds {
            min: [0.; 3],
            max: [1.; 3],
        },
        geometry_kind: turninator::render::contract::GeometryKind::Original,
        layer_filter: EmbeddedLayerFilter::standard(false),
        color: EmbeddedColorConfig::for_backend(BackendId::Filament).unwrap(),
    }
}

#[test]
fn describes_actual_runtime_and_rejects_incompatible_selected_device() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    let info = context.runtime_info().unwrap();
    assert_eq!(info.schema_version, 1);
    assert_eq!(info.backend, "fixture");
    assert_eq!(info.device_id, "cpu-fixture");
    assert_eq!(info.api, "fixture");
    let mut cfg = config();
    cfg.selection.device = Some("a-different-device".into());
    assert!(library.create(&cfg).is_err());
    for name in [
        "tn_fixture_bad_info.dll",
        "tn_fixture_bad_info_utf8.dll",
        "tn_fixture_large_info.dll",
    ] {
        let library = EmbeddedLibrary::load_fixture(&fixture(name)).unwrap();
        assert!(library.create(&config()).is_err(), "accepted {name}");
    }
}

#[test]
fn frame_transparency_seed_and_asymmetric_row_major_matrix_cross_the_abi() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    upload(&mut context, 1);
    let mut f = frame(1);
    f.transparent = false;
    f.sample_seed = 23;
    // Exact 90-degree rotation plus asymmetric translation. The fixture embeds a
    // fingerprint of all 16 received doubles in otherwise-zero pixel channels.
    f.model_to_world = [
        0., -1., 0., 7., 1., 0., 0., 19., 0., 0., 1., 37., 0., 0., 0., 1.,
    ];
    context.begin_frame(&f).unwrap();
    wait_for_state(&mut context, EmbeddedState::Ready);
    let mut bytes = [0; 16];
    context.read_pixels(&mut bytes).unwrap();
    assert_eq!(
        bytes,
        [
            23, 129, 172, 255, 163, 255, 182, 255, 0, 0, 255, 255, 0, 0, 0, 255
        ]
    );
}

#[test]
fn realized_geometry_expansion_does_not_reapply_configurable_source_limits() {
    // A valid source triangle can become four triangles with split/aligned
    // vertices downstream. Source snapshot validation belongs upstream; this
    // test passes only already-realized geometry to the ABI host.
    let source = turninator::mesh::Mesh {
        positions: vec![[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]],
        triangles: vec![[0, 1, 2]],
    };
    let positions = [
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [1., 0., 0.],
        [2., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
        [1., 1., 0.],
        [0., 2., 0.],
        [1., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
    ];
    let triangles = [[0, 1, 2], [3, 4, 5], [6, 7, 8], [9, 10, 11]];
    let mut meta = metadata();
    meta.reference_bounds_mm = source.bounds().unwrap();
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    for small_vertex_limit in [true, false] {
        let mut cfg = config();
        if small_vertex_limit {
            cfg.limits.max_source_vertices = source.positions.len();
        } else {
            cfg.limits.max_source_triangles = source.triangles.len();
        }
        let mut context = library.create(&cfg).unwrap();
        context
            .upload_scene(&SceneUpload {
                revision: 41,
                positions_mm: &positions,
                normals: &[[0., 0., 1.]; 12],
                build_positions_mm: &positions,
                triangles: &triangles,
                triangle_materials: &[0; 4],
                metadata: &meta,
            })
            .unwrap();
        context.begin_frame(&frame(41)).unwrap();
        wait_for_state(&mut context, EmbeddedState::Ready);
        let mut pixels = [0; 16];
        context.read_pixels(&mut pixels).unwrap();
        assert_eq!(pixels[0..4], [255, 0, 0, 255]);
    }
}

#[test]
fn validates_physical_metadata_material_indices_and_upload_memory_budget() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    let mut meta = metadata();
    meta.materials[0].roughness = f32::NAN;
    let mut scene = SceneUpload {
        revision: 1,
        positions_mm: &[[0.; 3]],
        normals: &[[0., 0., 1.]],
        build_positions_mm: &[[0.; 3]],
        triangles: &[[0, 0, 0]],
        triangle_materials: &[0],
        metadata: &meta,
    };
    assert!(context.upload_scene(&scene).is_err());
    let good = metadata();
    scene.metadata = &good;
    scene.triangle_materials = &[1];
    assert!(context.upload_scene(&scene).is_err());
    scene.triangle_materials = &[0];
    context.upload_scene(&scene).unwrap();
    let mut cfg = config();
    cfg.limits.host_bytes = 64;
    let mut limited = library.create(&cfg).unwrap();
    assert!(limited.upload_scene(&scene).is_err());
}

#[test]
fn optional_unlit_ink_and_259_material_table_cross_native_schema_guard() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    for count in [259, 260] {
        let mut meta = metadata();
        meta.materials = vec![meta.materials[0].clone(); count];
        meta.materials[count - 1].unlit = true;
        let upload = SceneUpload {
            revision: count as u64,
            positions_mm: &[[0.; 3]],
            normals: &[[0., 0., 1.]],
            build_positions_mm: &[[0.; 3]],
            triangles: &[[0, 0, 0]],
            triangle_materials: &[(count - 1) as u32],
            metadata: &meta,
        };
        assert_eq!(context.upload_scene(&upload).is_ok(), count == 259);
    }
}

#[test]
fn context_owns_library_through_pending_work_teardown() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    let path = fixture("tn_fixture_lifetime.dll");
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    for _ in 0..20 {
        let library = EmbeddedLibrary::load_fixture(&path).unwrap();
        let mut context = library.create(&config()).unwrap();
        upload(&mut context, 1);
        context.begin_frame(&frame(1)).unwrap();
        drop(library);
        assert!(!unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null());
        context.poll().unwrap();
        drop(context); // Native destructor drains pending work before FreeLibrary.
        assert!(unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null());
    }
}

#[test]
fn rejects_invalid_cache_directories_before_native_create_without_creating_them() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture_cache.dll")).unwrap();
    let missing = fixture("cache-directory-must-not-be-created");
    assert!(!missing.exists());
    let file = fixture("tn_fixture_cache.dll");
    for path in [
        PathBuf::new(),
        PathBuf::from("relative/cache"),
        PathBuf::from(r"C:cache"),
        PathBuf::from(r"\cache"),
        PathBuf::from(r"\\.\pipe\cache"),
        PathBuf::from(r"\\?\GLOBALROOT\cache"),
        PathBuf::from("C:\\bad\0path"),
        PathBuf::from(format!("C:\\{}", "x".repeat(4094))),
        PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800])),
        missing.clone(),
        file,
    ] {
        let mut cfg = config();
        cfg.cache_directory = Some(path.clone());
        let error = match library.create(&cfg) {
            Ok(_) => panic!("accepted invalid cache directory: {path:?}"),
            Err(error) => error.to_string(),
        };
        assert!(error.starts_with("cache directory"), "{path:?}: {error}");
    }
    assert!(!missing.exists());
}

#[test]
fn canonical_unicode_cache_directory_crosses_native_parser_without_changing_pixels() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture_cache.dll")).unwrap();
    // A missing optional field still reaches the factory; this fixture requires
    // it so dropping the field during serialization cannot silently pass.
    let absent = match library.create(&config()) {
        Ok(_) => panic!("cache fixture accepted an absent cache directory"),
        Err(error) => error.to_string(),
    };
    assert!(
        absent.contains("cache fixture requires explicit cache directory"),
        "{absent}"
    );
    let root = fixture("cache-directory-歯車");
    let child = root.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let mut cfg = config();
    cfg.cache_directory = Some(child.join(".."));
    let mut context = library.create(&cfg).unwrap();
    upload(&mut context, 1);
    context.begin_frame(&frame(1)).unwrap();
    wait_for_state(&mut context, EmbeddedState::Ready);
    let mut pixels = [0; 16];
    context.read_pixels(&mut pixels).unwrap();
    assert_eq!(
        pixels,
        [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 0, 0, 0, 0]
    );
    drop(context);
    std::fs::remove_dir(&child).unwrap();
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn canonical_existing_windows_del_and_c1_cache_directories_cross_native_parser() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    for codepoint in ['\u{7f}', '\u{80}', '\u{85}', '\u{9f}'] {
        let root = fixture(&format!("cache-directory-{codepoint}"));
        std::fs::create_dir(&root).unwrap();
        let canonical = root.canonicalize().unwrap();
        assert!(canonical.to_str().unwrap().ends_with(codepoint));
        let mut cfg = config();
        cfg.cache_directory = Some(canonical);
        let context = library.create(&cfg);
        std::fs::remove_dir(&root).unwrap();
        assert!(
            context.is_ok(),
            "Windows permits U+{:04X}: {:?}",
            codepoint as u32,
            context.err()
        );
    }
}
