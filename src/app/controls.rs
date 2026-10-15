use super::help::{self, HelpResponse};
use crate::export::{OutputFormat, VideoCodec};
use eframe::egui;

pub(super) fn build_up_checkbox(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    mut timeline: Option<&mut crate::animation::Timeline>,
) -> egui::Response {
    let mut build = *enabled || timeline.as_ref().is_some_and(|timeline| timeline.buildup);
    let response = ui
        .checkbox(&mut build, "Export build-up animation")
        .help(help::BUILD_UP);
    if response.changed() {
        *enabled = build;
        if let Some(timeline) = &mut timeline {
            timeline.buildup = build;
        }
    }
    response
}

pub(super) fn layer_settings(
    ui: &mut egui::Ui,
    scene: &mut crate::presentation::SceneSettings,
    grooves: bool,
) {
    let mut edited = false;
    if scene.import.layer_boundaries_mm.is_empty() {
        for (id, label, value, help) in [
            (
                "layer_height",
                "Layer (mm)",
                &mut scene.import.layer_height_mm,
                help::LAYER_HEIGHT,
            ),
            (
                "first_layer_height",
                "First layer (mm)",
                &mut scene.import.first_layer_height_mm,
                help::FIRST_LAYER,
            ),
        ] {
            let label = ui.label(label).help(help);
            edited |= numeric_field(
                ui,
                id,
                egui::DragValue::new(value)
                    .speed(0.01)
                    .range(0.02..=2.)
                    .suffix(" mm"),
            )
            .labelled_by(label.id)
            .help(help)
            .changed();
        }
    } else {
        ui.small("Layer heights come from the explicit adaptive boundaries.");
    }
    clamp_groove_depth(scene);
    if grooves {
        groove_settings(ui, scene);
    }
    scene.print_settings_edited |= edited;
    if ui.button("Reset print defaults").help("Restore this material's default layer heights and groove depth, and clear explicit adaptive boundaries. The source file is unchanged.").clicked() {
        scene.reset_print_settings();
    }
}

fn clamp_groove_depth(scene: &mut crate::presentation::SceneSettings) -> f64 {
    let max_groove = 0.25
        * scene
            .import
            .layer_height_mm
            .min(scene.import.first_layer_height_mm);
    scene.groove_depth_mm = scene.groove_depth_mm.min(max_groove);
    max_groove
}

pub(super) fn groove_settings(ui: &mut egui::Ui, scene: &mut crate::presentation::SceneSettings) {
    let max_groove = clamp_groove_depth(scene);
    let label = ui.label("Groove depth (mm)").help(help::GROOVE);
    let response = ui
        .push_id("groove_depth", |ui| {
            ui.add_sized(
                [ui.available_width(), ui.spacing().interact_size.y],
                egui::Slider::new(&mut scene.groove_depth_mm, 0.0..=max_groove).suffix(" mm"),
            )
        })
        .inner
        .labelled_by(label.id)
        .help(help::GROOVE);
    scene.print_settings_edited |= response.changed();
}

fn numeric_field(ui: &mut egui::Ui, id: &str, value: egui::DragValue<'_>) -> egui::Response {
    ui.push_id(id, |ui| {
        ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], value)
    })
    .inner
}

pub(super) struct OutputEditor<'a> {
    pub format: &'a mut OutputFormat,
    pub codec: &'a mut VideoCodec,
    pub frame_rate: &'a mut u32,
    pub animation_engine: &'a mut String,
    pub animation_size: &'a mut u32,
    pub animation_samples: &'a mut u32,
    pub still_engine: &'a mut String,
    pub still_size: &'a mut u32,
    pub still_samples: &'a mut u32,
    pub transparent: &'a mut bool,
}
impl OutputEditor<'_> {
    pub fn show(self, ui: &mut egui::Ui) -> bool {
        let previous = *self.format;
        let encoding_label = ui.label("Encoding").help(help::ENCODING);
        egui::ComboBox::from_id_salt("output_encoding")
            .width(ui.available_width())
            .selected_text(self.format.label())
            .show_ui(ui, |ui| {
                for format in [
                    OutputFormat::Gif,
                    OutputFormat::Mp4,
                    OutputFormat::Png,
                    OutputFormat::FourView,
                ] {
                    ui.selectable_value(self.format, format, format.label())
                        .help(help::encoding(format));
                }
            })
            .response
            .labelled_by(encoding_label.id)
            .help(help::ENCODING);
        if *self.format == OutputFormat::Mp4 {
            let codec_help = "Choose the codec inside the MP4 file. H.264 favors broad compatibility; AV1 can reduce file size but needs a compatible encoder and player. Unavailable codecs produce an error rather than silently switching.";
            let codec_label = ui.label("MP4 codec").help(codec_help);
            egui::ComboBox::from_id_salt("output_mp4_codec")
                .width(ui.available_width())
                .selected_text(if *self.codec == VideoCodec::Av1 {
                    "AV1"
                } else {
                    "H.264"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(self.codec, VideoCodec::H264, "H.264").help("A widely supported video format and a good default for sharing. Uses the configured FFmpeg H.264 encoder.");
                    ui.selectable_value(self.codec, VideoCodec::Av1, "AV1").help("An alternative video codec that can give smaller files. Encoding can take longer and some players may not support it. Requires AV1 support in your FFmpeg.");
                }).response.labelled_by(codec_label.id).help(codec_help);
        }
        if self.format.animated() {
            frame_rate_control(ui, self.frame_rate);
        }
        let (engine, size, samples) = if self.format.animated() {
            (
                self.animation_engine,
                self.animation_size,
                self.animation_samples,
            )
        } else {
            (self.still_engine, self.still_size, self.still_samples)
        };
        let renderer_label = ui.label("Renderer").help(help::RENDERER);
        egui::ComboBox::from_id_salt(("output_renderer", self.format.animated()))
            .width(ui.available_width())
            .selected_text(if engine == "cycles" { "Cycles / OptiX" } else { "EEVEE (fast)" })
            .show_ui(ui, |ui| {
                ui.selectable_value(engine, "eevee".into(), "EEVEE (fast)").help("Fast Blender rendering for lighting, color and layer-detail checks. Transmission and fine shadows are approximate.");
                ui.selectable_value(engine, "cycles".into(), "Cycles / OptiX").help("Trace light for more realistic reflections, shadows and resin. Requires a supported NVIDIA OptiX GPU; there is no silent CPU fallback.");
            }).response.labelled_by(renderer_label.id).help(help::RENDERER);
        let size_help = if *self.format == OutputFormat::FourView {
            "Width and height of the complete sheet, in pixels, from 256 to 8000. More pixels make small labels and edges clearer, but use more render time and storage."
        } else {
            "Output height and per-view width, in pixels, from 64 to 8000. Paired sections are twice as wide. Doubling this value creates roughly four times as many pixels, increasing render time and storage."
        };
        let size_label = ui
            .label(if *self.format == OutputFormat::FourView {
                "Sheet pixels"
            } else {
                "Output pixels"
            })
            .help(size_help);
        let minimum = if *self.format == OutputFormat::FourView {
            256
        } else {
            64
        };
        *size = (*size).clamp(minimum, 8000);
        let quality_id = if self.format.animated() {
            "animation_size"
        } else {
            "still_size"
        };
        numeric_field(
            ui,
            quality_id,
            egui::DragValue::new(size)
                .range(minimum..=8000)
                .speed(64)
                .suffix(" px"),
        )
        .labelled_by(size_label.id)
        .help(size_help);
        let samples_label = ui.label("Samples").help(help::SAMPLES);
        let quality_id = if self.format.animated() {
            "animation_samples"
        } else {
            "still_samples"
        };
        numeric_field(
            ui,
            quality_id,
            egui::DragValue::new(samples).range(1..=4096),
        )
        .labelled_by(samples_label.id)
        .help(help::SAMPLES);
        if self.format.supports_transparency() {
            ui.checkbox(self.transparent, "Transparent background").help("Save empty background pixels with an alpha channel instead of a studio floor. Useful for placing the PNG over another background. Material transparency is controlled separately by Resin clarity.");
        }
        if *self.format == OutputFormat::FourView {
            ui.small("TOP / ISO above FRONT / RIGHT, with fixed views and studio.");
        }
        previous != *self.format
    }
}

fn frame_rate_control(ui: &mut egui::Ui, frame_rate: &mut u32) -> egui::Response {
    let label = ui.label("Framerate (fps)").help(help::FRAME_RATE);
    numeric_field(
        ui,
        "output_frame_rate",
        egui::DragValue::new(frame_rate)
            .range(1..=crate::animation::MAX_FRAME_RATE)
            .speed(1)
            .suffix(" fps"),
    )
    .labelled_by(label.id)
    .help(help::FRAME_RATE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::OutputPreferences;

    #[test]
    fn frame_rate_control_displays_its_timing_and_cost_tooltip() {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| {
            style.interaction.tooltip_delay = 0.;
            style.interaction.tooltip_grace_time = 0.;
        });
        let mut rate = 30;
        let mut rect = egui::Rect::NOTHING;
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
            let output = ctx.run_ui(input, |ui| {
                rect = frame_rate_control(ui, &mut rate).rect;
            });
            found |= text_output(output).iter().any(|(text, _)| {
                text.contains("frames per second")
                    && text.contains("render and store")
                    && text.contains("GIF timing")
            });
        }
        assert!(
            found,
            "framerate tooltip should explain smoothness, cost and GIF timing"
        );
        assert_eq!(rate, 30);
    }

    #[test]
    fn one_build_checkbox_can_disable_a_saved_timeline_flag() {
        let ctx = egui::Context::default();
        let mut enabled = false;
        let mut timeline = crate::animation::Timeline {
            buildup: true,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let mut first = ctx.run_ui(egui::RawInput::default(), |ui| {
            rect = build_up_checkbox(ui, &mut enabled, Some(&mut timeline)).rect;
        });
        first.textures_delta.clear();
        assert!(timeline.buildup);
        for expected in [false, true] {
            let pos = rect.center();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    build_up_checkbox(ui, &mut enabled, Some(&mut timeline));
                },
            );
            output.textures_delta.clear();
            assert_eq!(enabled, expected);
            assert_eq!(timeline.buildup, expected);
        }
    }

    fn rendered_output(
        format: OutputFormat,
        options: &mut OutputPreferences,
        width: f32,
    ) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut format = format;
        let mut engine = "eevee".into();
        let mut size = 960;
        let mut samples = 7;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 1000.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.spacing_mut().interact_size.y = 32.;
                    OutputEditor {
                        format: &mut format,
                        codec: &mut options.video_codec,
                        frame_rate: &mut options.frame_rate,
                        animation_engine: &mut engine,
                        animation_size: &mut size,
                        animation_samples: &mut samples,
                        still_engine: &mut options.still_engine,
                        still_size: &mut options.still_size,
                        still_samples: &mut options.still_samples,
                        transparent: &mut options.transparent,
                    }
                    .show(ui);
                });
            },
        );
        output.textures_delta.clear();
        output
    }

    fn rendered_text(format: OutputFormat, options: &mut OutputPreferences) -> Vec<(String, f32)> {
        text_output(rendered_output(format, options, 600.))
    }

    fn painted_texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        fn collect(shape: &egui::epaint::Shape, texts: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::epaint::Shape::Text(text) => texts.push((
                    text.galley.job.text.clone(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                )),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, texts);
                    }
                }
                _ => (),
            }
        }
        let mut texts = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn output_labels_are_above_their_controls_at_sidebar_widths() {
        for width in [240., 380.] {
            let output =
                rendered_output(OutputFormat::Mp4, &mut OutputPreferences::default(), width);
            let texts = painted_texts(&output);
            for (label, value) in [
                ("Encoding", "MP4 video"),
                ("MP4 codec", "H.264"),
                ("Framerate (fps)", "20"),
                ("Renderer", "EEVEE (fast)"),
                ("Output pixels", "960"),
                ("Samples", "7"),
            ] {
                let label_rect = texts.iter().find(|(text, _)| text == label).unwrap().1;
                let value_rect = texts
                    .iter()
                    .find(|(text, _)| text == value)
                    .unwrap_or_else(|| panic!("missing visible value {value:?}: {texts:?}"))
                    .1;
                assert!(
                    label_rect.bottom() < value_rect.top(),
                    "{label} must sit above {value}"
                );
                assert!(value_rect.right() <= width, "{value} must fit the sidebar");
            }
        }
    }

    #[test]
    fn output_combos_fill_available_width_and_inherit_control_height() {
        for width in [240., 380.] {
            let output =
                rendered_output(OutputFormat::Mp4, &mut OutputPreferences::default(), width);
            let texts = painted_texts(&output);
            let mut boxes = Vec::new();
            fn collect(shape: &egui::epaint::Shape, boxes: &mut Vec<egui::Rect>) {
                match shape {
                    egui::epaint::Shape::Rect(rect) => boxes.push(rect.rect),
                    egui::epaint::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect(shape, boxes);
                        }
                    }
                    _ => (),
                }
            }
            for shape in &output.shapes {
                collect(&shape.shape, &mut boxes);
            }
            for value in ["MP4 video", "H.264", "EEVEE (fast)"] {
                let text_rect = texts.iter().find(|(text, _)| text == value).unwrap().1;
                let control = boxes
                    .iter()
                    .filter(|rect| rect.contains(text_rect.center()))
                    .min_by(|a, b| a.area().total_cmp(&b.area()))
                    .unwrap();
                assert!(
                    control.width() >= width - 20.,
                    "{value} must fill the content width: {control:?}"
                );
                assert!(
                    control.height() >= 32.,
                    "{value} must inherit the parent control height"
                );
            }
        }
    }

    #[test]
    fn print_height_labels_are_above_values_with_units() {
        let ctx = egui::Context::default();
        let mut scene = crate::presentation::SceneSettings::default();
        scene.import.first_layer_height_mm = 0.3;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            layer_settings(ui, &mut scene, true);
        });
        output.textures_delta.clear();
        let texts = painted_texts(&output);
        for (label, value) in [("Layer (mm)", "0.20"), ("First layer (mm)", "0.30")] {
            let label_rect = texts.iter().find(|(text, _)| text == label).unwrap().1;
            let value_rect = texts
                .iter()
                .find(|(text, _)| text == value)
                .unwrap_or_else(|| panic!("missing value {value:?}: {texts:?}"))
                .1;
            assert!(
                label_rect.bottom() < value_rect.top(),
                "{label} must sit above its value"
            );
        }
        assert!(
            !scene.print_settings_edited,
            "painting must preserve the saved print preferences"
        );
    }

    #[test]
    fn adaptive_print_settings_keep_grooves_below_the_adaptive_explanation() {
        let mut scene = crate::presentation::SceneSettings::default();
        scene.import.layer_boundaries_mm = vec![0., 0.2, 0.5];
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            layer_settings(ui, &mut scene, true);
        });
        let texts = text_output(output);
        let explanation_y = texts
            .iter()
            .find(|(text, _)| text.contains("explicit adaptive boundaries"))
            .unwrap()
            .1;
        let groove_y = texts
            .iter()
            .find(|(text, _)| text == "Groove depth (mm)")
            .unwrap()
            .1;
        assert!(groove_y > explanation_y);
        assert!(!texts.iter().any(|(text, _)| text == "Layer (mm)"));
        assert!(!scene.print_settings_edited);
    }

    #[test]
    fn standalone_grooves_enforce_layer_bounds_and_record_edits() {
        let ctx = egui::Context::default();
        let mut scene = crate::presentation::SceneSettings::default();
        scene.import.layer_height_mm = 0.12;
        scene.import.first_layer_height_mm = 0.08;
        scene.groove_depth_mm = 0.4;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            groove_settings(ui, &mut scene);
        });
        output.textures_delta.clear();
        assert_eq!(scene.groove_depth_mm, 0.02);
        assert!(!scene.print_settings_edited);
        let texts = painted_texts(&output);
        assert!(texts.iter().any(|(text, _)| text == "Groove depth (mm)"));
        assert!(!texts.iter().any(|(text, _)| text == "Layer (mm)"
            || text == "First layer (mm)"
            || text == "Reset print defaults"));
        fn slider_rail(shape: &egui::epaint::Shape) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Rect(rect)
                    if rect.rect.height() <= 8. && rect.rect.width() > 40. =>
                {
                    Some(rect.rect)
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(slider_rail),
                _ => None,
            }
        }
        let rail = output
            .shapes
            .iter()
            .find_map(|shape| slider_rail(&shape.shape))
            .unwrap();
        let pos = rail.center();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                groove_settings(ui, &mut scene);
            },
        );
        output.textures_delta.clear();
        assert!(scene.groove_depth_mm > 0. && scene.groove_depth_mm < 0.02);
        assert!(
            scene.print_settings_edited,
            "editing grooves must preserve the custom print settings on save"
        );
    }

    fn text_output(mut output: egui::FullOutput) -> Vec<(String, f32)> {
        fn collect(shape: &egui::epaint::Shape, texts: &mut Vec<(String, f32)>) {
            match shape {
                egui::epaint::Shape::Text(text) => {
                    texts.push((text.galley.job.text.clone(), text.pos.y))
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, texts);
                    }
                }
                _ => (),
            }
        }
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for shape in output.shapes {
            collect(&shape.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn adaptive_boundaries_hide_overridden_height_defaults() {
        let mut scene = crate::presentation::SceneSettings::default();
        scene.import.layer_boundaries_mm = vec![0., 0.2, 0.5];
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            layer_settings(ui, &mut scene, false);
        });
        let texts = text_output(output);
        assert!(!texts.iter().any(|(text, _)| text == "Layer (mm)"
            || text == "First layer (mm)"
            || text == "Groove depth (mm)"));
        assert!(texts.iter().any(|(text, _)| text == "Reset print defaults"));
        assert_eq!(scene.import.layer_boundaries_mm, [0., 0.2, 0.5]);
    }

    #[test]
    fn codec_is_below_encoding_and_only_drawn_for_mp4() {
        let mut options = OutputPreferences {
            video_codec: VideoCodec::Av1,
            frame_rate: 24,
            transparent: true,
            ..Default::default()
        };
        for format in [
            OutputFormat::Gif,
            OutputFormat::Mp4,
            OutputFormat::Png,
            OutputFormat::FourView,
        ] {
            let texts = rendered_text(format, &mut options);
            let encoding_y = texts.iter().find(|(text, _)| text == "Encoding").unwrap().1;
            let codec = texts.iter().find(|(text, _)| text == "MP4 codec");
            let rate = texts.iter().find(|(text, _)| text == "Framerate (fps)");
            assert_eq!(rate.is_some(), format.animated());
            if let Some((_, y)) = rate {
                assert!(*y > codec.map_or(encoding_y, |(_, y)| *y));
            }
            assert_eq!(options.frame_rate, 24);
            if format == OutputFormat::Mp4 {
                assert!(codec.unwrap().1 > encoding_y);
            } else {
                assert!(codec.is_none());
            }
            assert_eq!(
                texts
                    .iter()
                    .any(|(text, _)| text == "Transparent background"),
                format == OutputFormat::Png
            );
            assert_eq!(options.video_codec, VideoCodec::Av1);
            assert!(
                options.transparent,
                "hiding a control must preserve its preference"
            );
        }
    }

    #[test]
    fn visible_quality_belongs_to_the_selected_output() {
        let mut options = OutputPreferences {
            still_samples: 128,
            ..Default::default()
        };
        for format in [
            OutputFormat::Gif,
            OutputFormat::Mp4,
            OutputFormat::Png,
            OutputFormat::FourView,
        ] {
            let texts = rendered_text(format, &mut options);
            assert_eq!(
                texts.iter().any(|(text, _)| text == "Cycles / OptiX"),
                !format.animated()
            );
            assert_eq!(
                texts.iter().any(|(text, _)| text == "128"),
                !format.animated()
            );
            assert_eq!(texts.iter().any(|(text, _)| text == "7"), format.animated());
            assert_eq!(options.still_samples, 128);
        }
        options.still_size = 64;
        rendered_text(OutputFormat::FourView, &mut options);
        assert_eq!(
            options.still_size, 256,
            "sheets enforce the compositor's minimum size"
        );
    }
}
