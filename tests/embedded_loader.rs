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
    let message = context.begin_frame(&frame(1)).unwrap_err().to_string();
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
    assert_eq!(context.poll().unwrap().state, EmbeddedState::Rendering);
    assert_eq!(context.poll().unwrap().state, EmbeddedState::Ready);
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
    assert_eq!(context.poll().unwrap().state, EmbeddedState::Rendering);
}

#[test]
fn cancel_acknowledges_only_after_pending_readback_drain() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    for cancel_ready in [false, true] {
        let mut context = library.create(&config()).unwrap();
        upload(&mut context, 3);
        context.begin_frame(&frame(3)).unwrap();
        if cancel_ready {
            context.poll().unwrap();
            context.poll().unwrap();
        }
        context.cancel().unwrap();
        context.cancel().unwrap();
        assert_eq!(context.poll().unwrap().state, EmbeddedState::Cancelling);
        assert!(context.begin_frame(&frame(3)).is_err());
        assert!(context.read_pixels(&mut [0; 16]).is_err());
        assert_eq!(context.poll().unwrap().state, EmbeddedState::Cancelled);
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
    let error = context.begin_frame(&frame(1)).unwrap_err().to_string();
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
fn frame_transparency_seed_and_row_major_translation_cross_the_abi() {
    let library = EmbeddedLibrary::load_fixture(&fixture("tn_fixture.dll")).unwrap();
    let mut context = library.create(&config()).unwrap();
    upload(&mut context, 1);
    let mut f = frame(1);
    f.transparent = false;
    f.sample_seed = 23;
    f.model_to_world[3] = 10.;
    context.begin_frame(&f).unwrap();
    context.poll().unwrap();
    context.poll().unwrap();
    let mut bytes = [0; 16];
    context.read_pixels(&mut bytes).unwrap();
    assert_eq!(
        bytes,
        [23, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 255]
    );
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
