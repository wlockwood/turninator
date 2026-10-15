//! Hover help for enabled controls and the same controls while unavailable.
use eframe::egui;

pub(super) trait HelpResponse: Sized {
    fn help(self, text: impl Into<egui::WidgetText>) -> Self;
}
impl HelpResponse for egui::Response {
    fn help(self, text: impl Into<egui::WidgetText>) -> Self {
        let text = text.into();
        self.on_hover_text(text.clone())
            .on_disabled_hover_text(text)
    }
}
impl<T> HelpResponse for egui::InnerResponse<T> {
    fn help(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.response = self.response.help(text);
        self
    }
}
impl<T> HelpResponse for egui::CollapsingResponse<T> {
    fn help(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.header_response = self.header_response.help(text);
        self
    }
}

pub(super) const ENCODING: &str = "Choose the file you want to create. Only settings for that output are shown; switching formats keeps your other preferences.";
pub(super) const RENDERER: &str = "EEVEE gives a fast studio preview. Cycles traces light for more realistic reflections and resin, and requires a supported NVIDIA OptiX GPU in this app. Both use your configured Blender.";
pub(super) const SAMPLES: &str = "More samples reduce render noise and help resolve fine detail, but take longer. Start with 32 for EEVEE or 64 for Cycles, then increase if the result is noisy.";
pub(super) const BUILD_UP: &str = "Grow the part through whole print layers during GIF or MP4 export, ending with the complete model. Closed STL surfaces show a solid top cross-section while preserving modeled holes; G-code retains deposited paths and voids. Works with camera motion, materials and sections. This represents layer order, not printer time or nozzle travel.";
pub(super) const PREVIEW: &str = "Render the current view with the selected Blender renderer; sheets preview all four views. Uses 768 pixels and 16 samples for a quick check. Open a model and finish any current render/import first.";
pub(super) const LAYER_HEIGHT: &str = "Spacing between later printed layers, in millimetres; 0.20 mm is a common FDM setting. Smaller values create finer detail. For G-code this is a fallback where HEIGHT metadata is absent.";
pub(super) const FIRST_LAYER: &str = "Thickness of the first printed layer, in millimetres. It can differ from later layers. For G-code this is a fallback where HEIGHT metadata is absent.";
pub(super) const GROOVE: &str = "Depth of the visual layer seam, in millimetres. Increase for stronger lines or reduce for a smoother finish. This surface shading keeps the original STL silhouette; depth is limited to a quarter of the smaller layer height.";
pub(super) const NATIVE_VIEW: &str = "Drag with the left button to orbit, right-drag to pan, and scroll to zoom. Native inspection responds immediately; use Render preview to check Blender materials and lighting.";
pub(super) const EXPORT: &str = "Choose a destination and export the selected format using the settings shown above. The suggested filename follows the loaded model. A model must be loaded, with no render, import or storage operation in progress.";

pub(super) fn encoding(format: crate::export::OutputFormat) -> &'static str {
    use crate::export::OutputFormat;
    match format {
        OutputFormat::Gif => {
            "A looping animation that is easy to share. Uses 20 frames per second and a shared color palette; detailed gradients can be less smooth than MP4."
        }
        OutputFormat::Mp4 => {
            "A 20 fps video with smoother gradients than GIF. Choose H.264 or AV1 below, then open the saved video in your player."
        }
        OutputFormat::Png => {
            "One high-resolution image of the current view, including any selected section or deposited stage. Can keep a transparent background."
        }
        OutputFormat::FourView => {
            "A fixed gray drawing sheet: TOP / ISO above FRONT / RIGHT, at one scale. Each view labels its largest projected dimension in millimetres. Presentation pose, studio and palette do not affect this layout."
        }
    }
}

pub(super) fn material(value: &str) -> &'static str {
    match value {
        "matte_pla" => {
            "A softly reflective plastic finish for everyday FDM parts. Adjust roughness and layer appearance to match your print."
        }
        "silk_pla" => {
            "A directional, slightly metallic sheen that emphasizes curved surfaces. It approximates silk filament rather than simulating its chemistry."
        }
        "glossy_plastic" => {
            "A polished plastic finish with a clear reflective coat. Lower roughness produces sharper highlights."
        }
        "clay" => {
            "A neutral matte finish for judging shape. Its default turns layer appearance off; you can enable layers again for an STL."
        }
        "resin" => {
            "A translucent material with adjustable clarity and cloudiness. Selects Cycles for rendering. Internal or overlapping bead surfaces remain an optical approximation."
        }
        _ => {
            "Choose a surface finish. Material defaults preserve layer settings you have edited explicitly."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_help_is_readable_on_enabled_and_disabled_controls() {
        for widget in ["button", "number", "menu", "section"] {
            for enabled in [true, false] {
                let ctx = egui::Context::default();
                ctx.all_styles_mut(|style| {
                    style.interaction.tooltip_delay = 0.;
                    style.interaction.tooltip_grace_time = 0.;
                });
                let tip = if enabled {
                    "Render the selected output."
                } else {
                    "Open a model before rendering."
                };
                let mut rect = egui::Rect::NOTHING;
                let mut value = 0.2;
                let mut found = false;
                for frame in 0..4 {
                    let input = egui::RawInput {
                        time: Some(f64::from(frame)),
                        events: if frame == 0 {
                            vec![]
                        } else {
                            vec![egui::Event::PointerMoved(rect.center())]
                        },
                        ..Default::default()
                    };
                    let mut output = ctx.run_ui(input, |ui| {
                        let response = ui
                            .add_enabled_ui(enabled, |ui| match widget {
                                "number" => ui.add(egui::DragValue::new(&mut value)).help(tip),
                                "menu" => {
                                    egui::ComboBox::from_label("Encoding")
                                        .selected_text("GIF")
                                        .show_ui(ui, |ui| {
                                            ui.label("GIF");
                                        })
                                        .help(tip)
                                        .response
                                }
                                "section" => {
                                    ui.collapsing("Settings", |ui| {
                                        ui.label("Details");
                                    })
                                    .help(tip)
                                    .header_response
                                }
                                _ => ui.button("Export").help(tip),
                            })
                            .inner;
                        rect = response.rect;
                        assert_eq!(response.enabled(), enabled);
                    });
                    fn contains(shape: &egui::epaint::Shape, tip: &str) -> bool {
                        match shape {
                            egui::epaint::Shape::Text(text) => text.galley.job.text == tip,
                            egui::epaint::Shape::Vec(shapes) => {
                                shapes.iter().any(|shape| contains(shape, tip))
                            }
                            _ => false,
                        }
                    }
                    found |= output
                        .shapes
                        .iter()
                        .any(|shape| contains(&shape.shape, tip));
                    output.textures_delta.clear();
                }
                assert!(found, "missing hover help for {widget}, enabled={enabled}");
                assert_eq!(value, 0.2, "hover must not edit the value");
            }
        }
    }
}
