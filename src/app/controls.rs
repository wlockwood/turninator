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
        for (label, value) in [
            ("Layer (mm)", &mut scene.import.layer_height_mm),
            ("First layer (mm)", &mut scene.import.first_layer_height_mm),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                edited |= ui
                    .add(egui::DragValue::new(value).speed(0.01).range(0.02..=2.))
                    .help(if label.starts_with("First") {
                        help::FIRST_LAYER
                    } else {
                        help::LAYER_HEIGHT
                    })
                    .changed();
            });
        }
    } else {
        ui.small("Layer heights come from the explicit adaptive boundaries.");
    }
    let max_groove = 0.25
        * scene
            .import
            .layer_height_mm
            .min(scene.import.first_layer_height_mm);
    scene.groove_depth_mm = scene.groove_depth_mm.min(max_groove);
    if grooves {
        edited |= ui
            .add(
                egui::Slider::new(&mut scene.groove_depth_mm, 0.0..=max_groove)
                    .text("Groove depth (mm)"),
            )
            .help(help::GROOVE)
            .changed();
    }
    scene.print_settings_edited |= edited;
    if ui.button("Reset print defaults").help("Restore this material's default layer heights and groove depth, and clear explicit adaptive boundaries. The source file is unchanged.").clicked() {
        scene.reset_print_settings();
    }
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
        egui::ComboBox::from_label("Encoding")
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
            .help(help::ENCODING);
        if *self.format == OutputFormat::Mp4 {
            egui::ComboBox::from_label("MP4 codec")
                .selected_text(if *self.codec == VideoCodec::Av1 {
                    "AV1"
                } else {
                    "H.264"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(self.codec, VideoCodec::H264, "H.264").help("A widely supported video format and a good default for sharing. Uses the configured FFmpeg H.264 encoder.");
                    ui.selectable_value(self.codec, VideoCodec::Av1, "AV1").help("An alternative video codec that can give smaller files. Encoding can take longer and some players may not support it. Requires AV1 support in your FFmpeg.");
                }).help("Choose the codec inside the MP4 file. H.264 favors broad compatibility; AV1 can reduce file size but needs a compatible encoder and player. Unavailable codecs produce an error rather than silently switching.");
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
        egui::ComboBox::from_label("Renderer")
            .selected_text(engine.as_str())
            .show_ui(ui, |ui| {
                ui.selectable_value(engine, "eevee".into(), "EEVEE (fast)").help("Fast Blender rendering for lighting, color and layer-detail checks. Transmission and fine shadows are approximate.");
                ui.selectable_value(engine, "cycles".into(), "Cycles / OptiX").help("Trace light for more realistic reflections, shadows and resin. Requires a supported NVIDIA OptiX GPU; there is no silent CPU fallback.");
            }).help(help::RENDERER);
        ui.horizontal(|ui| {
            ui.label(if *self.format == OutputFormat::FourView {
                "Sheet pixels"
            } else {
                "Output pixels"
            });
            let minimum = if *self.format == OutputFormat::FourView {
                256
            } else {
                64
            };
            *size = (*size).clamp(minimum, 8000);
            ui.add(egui::DragValue::new(size).range(minimum..=8000).speed(64)).help(if *self.format == OutputFormat::FourView {
                "Width and height of the complete sheet, in pixels, from 256 to 8000. More pixels make small labels and edges clearer, but use more render time and storage."
            } else { "Output height and per-view width, in pixels, from 64 to 8000. Paired sections are twice as wide. Doubling this value creates roughly four times as many pixels, increasing render time and storage." });
        });
        ui.horizontal(|ui| {
            ui.label("Samples");
            ui.add(egui::DragValue::new(samples).range(1..=4096))
                .help(help::SAMPLES);
        });
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
    ui.horizontal(|ui| {
        ui.label("Framerate (fps)").help(help::FRAME_RATE);
        ui.add(
            egui::DragValue::new(frame_rate)
                .range(1..=crate::animation::MAX_FRAME_RATE)
                .speed(1),
        )
        .help(help::FRAME_RATE)
    })
    .inner
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

    fn rendered_text(format: OutputFormat, options: &mut OutputPreferences) -> Vec<(String, f32)> {
        let ctx = egui::Context::default();
        let mut format = format;
        let mut engine = "eevee".into();
        let mut size = 960;
        let mut samples = 7;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600., 800.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
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
        text_output(output)
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
                texts.iter().any(|(text, _)| text == "cycles"),
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
