//! Shared navigation and visual rhythm for the desktop workspace.
use super::help::HelpResponse;
use eframe::egui;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Model,
    Appearance,
    View,
    Export,
}
impl Tab {
    pub const ALL: [Self; 4] = [Self::Model, Self::Appearance, Self::View, Self::Export];
    pub fn label(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Appearance => "Appearance",
            Self::View => "View",
            Self::Export => "Export",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Model => {
                "Set physical size, print direction and layer geometry. Review source assumptions here. Changing tabs keeps all settings."
            }
            Self::Appearance => {
                "Choose material, color, layer shading and studio lighting. Changing tabs keeps all settings."
            }
            Self::View => {
                "Compose the camera, inspect the mesh, reveal printed layers or set a cutaway. Changing tabs keeps all settings."
            }
            Self::Export => {
                "Choose output format, render quality and animation. Preview and export actions stay available on every tab."
            }
        }
    }
}

pub(super) fn configure(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8., 7.);
        style.spacing.button_padding = egui::vec2(9., 5.);
        style.spacing.interact_size.y = 28.;
        style.spacing.slider_width = 140.;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.));
    });
}

pub(super) fn navigation(ui: &mut egui::Ui, selected: &mut Tab) -> Vec<egui::Rect> {
    ui.horizontal(|ui| {
        let widths = Tab::ALL.map(|tab| {
            ui.painter()
                .layout_no_wrap(
                    tab.label().into(),
                    egui::FontId::proportional(13.),
                    ui.visuals().text_color(),
                )
                .size()
                .x
                + 2. * ui.spacing().button_padding.x
        });
        let extra = ((ui.available_width()
            - ui.spacing().item_spacing.x * 3.
            - widths.iter().sum::<f32>())
            / 4.)
            .max(0.);
        Tab::ALL
            .into_iter()
            .zip(widths)
            .map(|(tab, width)| {
                let response = ui
                    .add_sized(
                        [width + extra, 32.],
                        egui::Button::new(egui::RichText::new(tab.label()).size(13.))
                            .selected(*selected == tab),
                    )
                    .help(tab.help());
                if response.clicked() {
                    *selected = tab;
                }
                response.rect
            })
            .collect()
    })
    .inner
}

pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(5.);
    ui.strong(title);
}

pub(super) fn number(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    help: &str,
    value: egui::DragValue<'_>,
) -> egui::Response {
    let label = ui.label(label).help(help);
    ui.push_id(id, |ui| {
        ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], value)
    })
    .inner
    .labelled_by(label.id)
    .help(help)
}

pub(super) fn slider(
    ui: &mut egui::Ui,
    label: &str,
    help: &str,
    value: egui::Slider<'_>,
) -> egui::Response {
    let label = ui.label(label).help(help);
    ui.add(value).labelled_by(label.id).help(help)
}

pub(super) fn color(ui: &mut egui::Ui, value: &mut [f32; 3], help: &str) -> egui::Response {
    // egui roundtrips RGB through HSV even when the picker is only painted.
    let mut edited = *value;
    let response = ui.color_edit_button_rgb(&mut edited).help(help);
    if response.changed() {
        *value = edited;
    }
    response
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum FileCommand {
    OpenModel,
    OpenProject,
    SaveProject,
}
pub(super) fn file_shortcut(ctx: &egui::Context) -> Option<FileCommand> {
    ctx.input_mut(|input| {
        input.events.retain(|event| !matches!(event, egui::Event::Key { key: egui::Key::O | egui::Key::S, repeat: true, modifiers, .. } if modifiers.command));
        if input.consume_key(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::O,
        ) {
            Some(FileCommand::OpenProject)
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::O) {
            Some(FileCommand::OpenModel)
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
            Some(FileCommand::SaveProject)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn displaying_a_color_control_preserves_exact_render_identity() {
        let ctx = egui::Context::default();
        let original = [0.035, 0.22, 0.52];
        let mut value = original;
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(!color(ui, &mut value, "Surface color").changed());
            });
            output.textures_delta.clear();
            assert_eq!(
                value, original,
                "Opening Appearance must not invalidate rendered frames"
            );
        }
    }

    #[test]
    fn holding_a_file_shortcut_does_not_open_repeated_dialogs() {
        let ctx = egui::Context::default();
        for frame in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(f64::from(frame)),
                    events: vec![egui::Event::Key {
                        key: egui::Key::O,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            command: true,
                            ctrl: true,
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                },
                |_| {
                    assert_eq!(
                        file_shortcut(&ctx),
                        if frame == 0 {
                            Some(FileCommand::OpenModel)
                        } else {
                            None
                        }
                    );
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn project_shortcut_is_distinct_from_model_shortcut_and_consumed_once() {
        for (key, shift, expected) in [
            (egui::Key::O, true, FileCommand::OpenProject),
            (egui::Key::O, false, FileCommand::OpenModel),
            (egui::Key::S, false, FileCommand::SaveProject),
        ] {
            let ctx = egui::Context::default();
            let mut result = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            command: true,
                            ctrl: true,
                            shift,
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                },
                |_ui| {
                    result = file_shortcut(&ctx);
                    assert!(file_shortcut(&ctx).is_none());
                },
            );
            output.textures_delta.clear();
            assert_eq!(result, Some(expected));
        }
    }

    #[test]
    fn task_navigation_fits_narrow_panels_and_selects_the_clicked_task() {
        for width in [280., 320., 420.] {
            let ctx = egui::Context::default();
            configure(&ctx);
            let mut selected = Tab::Model;
            let mut rects = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                rects = navigation(ui, &mut selected);
            });
            output.textures_delta.clear();
            assert_eq!(rects.len(), 4);
            assert!(rects.iter().all(|r| r.height() >= 28.));
            assert!(
                rects
                    .windows(2)
                    .all(|pair| pair[0].right() <= pair[1].left())
            );
            assert!(rects.last().unwrap().right() - rects[0].left() <= width + 0.1);
            let pos = rects[3].center();
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
                    ui.set_width(width);
                    navigation(ui, &mut selected);
                },
            );
            output.textures_delta.clear();
            assert_eq!(selected, Tab::Export);
        }
    }
}
