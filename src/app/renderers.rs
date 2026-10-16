//! Descriptive UI state; querying it never initializes a renderer.
use crate::{
    export::{ExportRequest, OutputFormat},
    render::{
        BackendId, RenderSelection, RendererPreferences, capabilities::*,
        contract::RenderSceneSnapshot, selection::RenderUsage,
    },
};

pub fn label(backend: BackendId) -> &'static str {
    match backend {
        BackendId::Wgpu => "Native wgpu",
        BackendId::Filament => "Filament",
        BackendId::LuxCore => "LuxCore",
        BackendId::BlenderEevee => "Blender Eevee",
        BackendId::BlenderCycles => "Blender Cycles / OptiX",
    }
}
pub struct RendererOption {
    pub backend: BackendId,
    pub label: &'static str,
    pub available: bool,
    pub reason: Option<String>,
}
pub fn renderer_options(statuses: &[BackendStatus], usage: RenderUsage) -> Vec<RendererOption> {
    BackendId::ALL
        .into_iter()
        .filter(|b| {
            usage != RenderUsage::Interactive || matches!(b, BackendId::Wgpu | BackendId::Filament)
        })
        .map(|backend| {
            let status = statuses.iter().find(|s| s.backend == backend);
            let (available, mut reason) = match status.map(|s| &s.availability) {
                Some(BackendAvailability::Available) => (true, None),
                Some(BackendAvailability::Unavailable { reason }) => (false, Some(reason.clone())),
                _ => (false, Some("Checked when rendering starts".into())),
            };
            let supported = status
                .and_then(|s| s.capabilities.as_ref())
                .is_none_or(|c| c.usages.contains(&usage));
            if !supported {
                reason = Some("This renderer does not support this output".into());
            }
            RendererOption {
                backend,
                label: label(backend),
                available: available && supported,
                reason,
            }
        })
        .collect()
}
pub fn preview_selection(prefs: &RendererPreferences, format: OutputFormat) -> &RenderSelection {
    if format.animated() {
        &prefs.animation
    } else {
        &prefs.still
    }
}
pub fn material_notice(material: &str, backend: BackendId) -> Option<&'static str> {
    (material == "resin" && matches!(backend, BackendId::Wgpu | BackendId::Filament | BackendId::BlenderEevee))
        .then_some("Resin is approximate with this renderer. Choose LuxCore or Blender Cycles / OptiX for traced transmission and absorption.")
}
#[derive(Default)]
pub struct SelectionAssessment {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}
pub fn assess_selection(
    selection: &RenderSelection,
    scene: &RenderSceneSnapshot,
    request: &ExportRequest,
    statuses: &[BackendStatus],
) -> SelectionAssessment {
    let mut result = SelectionAssessment::default();
    if let Err(e) = selection.validate() {
        result.errors.push(e.to_string());
    }
    if let Some(status) = statuses.iter().find(|s| s.backend == selection.backend) {
        if let BackendAvailability::Unavailable { reason } = &status.availability {
            result.errors.push(reason.clone());
        }
        if let Some(c) = &status.capabilities {
            let usage = match request.kind {
                crate::export::ExportKind::Still | crate::export::ExportKind::SectionStill => {
                    RenderUsage::Still
                }
                crate::export::ExportKind::Diagram => RenderUsage::Sheet,
                _ => RenderUsage::Animation,
            };
            if !c.usages.contains(&usage) {
                result
                    .errors
                    .push("This renderer does not support this output".into());
            }
            if request.size > c.maximum_output_side {
                result.errors.push(format!(
                    "Requested size exceeds this renderer's {} pixel limit",
                    c.maximum_output_side
                ));
            }
            for support in [
                request.transparent.then_some(&c.alpha),
                request.section.as_ref().map(|_| &c.sections),
                scene
                    .appearance
                    .layer_appearance
                    .then_some(&c.layer_appearance),
            ]
            .into_iter()
            .flatten()
            {
                match support {
                    FeatureSupport::Unsupported { reason } => result.errors.push(reason.clone()),
                    FeatureSupport::Approximate { reason } => result.warnings.push(reason.clone()),
                    _ => {}
                }
            }
        }
    }
    if let Some(notice) = material_notice(&scene.appearance.material_preset, selection.backend) {
        result.warnings.push(notice.into());
    }
    if selection.backend == BackendId::LuxCore {
        result
            .warnings
            .extend(crate::render::backends::luxcore::warnings(
                scene,
                request.kind,
                request.transparent,
                &selection.quality,
            ));
    }
    result
}
