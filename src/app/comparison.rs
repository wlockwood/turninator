//! Shared display geometry for native inspection and the rendered image.
use eframe::egui;

pub(super) struct Panes {
    pub native: egui::Rect,
    pub preview: egui::Rect,
    pub playback: Option<egui::Rect>,
}

#[cfg(test)]
pub(super) fn layout(
    ui: &mut egui::Ui,
    preview_title: &str,
    status: Option<&str>,
    image_size: egui::Vec2,
    animated: bool,
) -> Panes {
    layout_renderer(
        ui,
        "Native inspection",
        preview_title,
        status,
        image_size,
        animated,
    )
}

pub(super) fn layout_renderer(
    ui: &mut egui::Ui,
    native_title: &str,
    preview_title: &str,
    status: Option<&str>,
    image_size: egui::Vec2,
    animated: bool,
) -> Panes {
    ui.columns(2, |columns| {
        columns[0].heading(native_title);
        columns[1].horizontal_wrapped(|ui| {
            ui.heading(preview_title);
            if let Some(status) = status {
                ui.label(status);
            }
        });
        let top = columns[0].cursor().top().max(columns[1].cursor().top());
        let bottom = columns[0]
            .max_rect()
            .bottom()
            .min(columns[1].max_rect().bottom());
        let gap = columns[0].spacing().item_spacing.y;
        let footer_height = if animated {
            columns[0].spacing().interact_size.y + gap
        } else {
            0.
        };
        let available = egui::vec2(
            columns[0]
                .available_width()
                .min(columns[1].available_width()),
            (bottom - top - footer_height).max(1.),
        );
        let size = image_size * (available.x / image_size.x).min(available.y / image_size.y);
        let rects = std::array::from_fn::<_, 2, _>(|i| {
            egui::Rect::from_min_size(
                egui::pos2(columns[i].max_rect().center().x - size.x / 2., top),
                size,
            )
        });
        for (column, rect) in columns.iter_mut().zip(rects) {
            column.advance_cursor_after_rect(rect);
        }
        let playback = animated.then(|| {
            egui::Rect::from_min_size(
                egui::pos2(rects[1].left(), rects[1].bottom() + gap),
                egui::vec2(size.x, columns[0].spacing().interact_size.y),
            )
        });
        if let Some(rect) = playback {
            columns[1].advance_cursor_after_rect(rect);
        }
        Panes {
            native: rects[0],
            preview: rects[1],
            playback,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_stay_aligned_when_windows_images_and_playback_controls_change() {
        let ctx = egui::Context::default();
        for window in [
            egui::vec2(1420., 1020.),
            egui::vec2(900., 650.),
            egui::vec2(2400., 900.),
        ] {
            for image in [egui::vec2(768., 768.), egui::vec2(1920., 960.)] {
                for animated in [false, true] {
                    let mut result = None;
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, window)),
                            ..Default::default()
                        },
                        |ui| {
                            egui::Panel::left("controls")
                                .exact_size(285.)
                                .show(ui, |_| {});
                            egui::CentralPanel::default().show(ui, |ui| {
                                result = Some(layout(
                                    ui,
                                    "Rendered preview",
                                    Some("Updating…"),
                                    image,
                                    animated,
                                ));
                            });
                        },
                    );
                    output.textures_delta.clear();
                    let panes = result.unwrap();
                    assert_eq!(panes.native.size(), panes.preview.size());
                    assert_eq!(panes.native.top(), panes.preview.top());
                    assert_eq!(panes.native.bottom(), panes.preview.bottom());
                    assert!((panes.native.aspect_ratio() - image.x / image.y).abs() < 1e-5);
                    assert!(panes.native.right() < panes.preview.left());
                    assert!(panes.preview.bottom() <= window.y);
                    if let Some(footer) = panes.playback {
                        assert!(footer.top() >= panes.preview.bottom());
                        assert!(footer.bottom() <= window.y);
                    }
                }
            }
        }
    }

    #[test]
    fn resizing_changes_both_camera_projections_by_the_same_scale() {
        let ctx = egui::Context::default();
        let camera = crate::viewport::OrbitCamera::default();
        let before = camera.camera_spec();
        for window in [
            egui::vec2(1420., 1020.),
            egui::vec2(900., 650.),
            egui::vec2(2400., 900.),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, window)),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let panes =
                            layout(ui, "Rendered preview", None, egui::vec2(768., 768.), false);
                        let project = |rect: egui::Rect| {
                            let p = camera
                                .project([10., 5., 0.], rect.aspect_ratio() as f64)
                                .unwrap();
                            [
                                (p[0] + 1.) * rect.width() as f64 / 2.,
                                (1. - p[1]) * rect.height() as f64 / 2.,
                            ]
                        };
                        assert_eq!(project(panes.native), project(panes.preview));
                    });
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(
            camera.camera_spec(),
            before,
            "resizing must not alter the saved render camera"
        );
    }
}
