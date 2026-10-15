//! Actual painted desktop cases, enabled only through the existing smoke harness.
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    backend: BackendId,
    #[serde(default)]
    narrow: bool,
    #[serde(default)]
    alpha: bool,
    #[serde(default)]
    resin: bool,
    #[serde(default)]
    sheet: bool,
    #[serde(default)]
    filament_viewport: bool,
    #[serde(default)]
    inspection: bool,
    #[serde(default)]
    cancel: bool,
    #[serde(default)]
    expected_error: Option<String>,
    #[serde(default)]
    export_size: Option<u32>,
}
impl Studio {
    pub(super) fn renderer_smoke(&mut self, ctx: &egui::Context, directory: &Path) {
        let result = self.renderer_smoke_step(ctx, directory);
        if let Err(error) = result {
            let _ = fs::write(directory.join("failed.txt"), error.to_string());
            let _ = fs::write(
                directory.join("progress.json"),
                serde_json::to_vec_pretty(&self.smoke_checks).unwrap_or_default(),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn renderer_smoke_step(&mut self, ctx: &egui::Context, directory: &Path) -> Result<()> {
        if self.started.elapsed() > Duration::from_secs(600) {
            return Err(format!(
                "renderer smoke timeout: {:?}; interactive {:?}",
                self.error, self.interactive_error
            )
            .into());
        }
        let cases: Vec<Case> =
            serde_json::from_slice(&fs::read(directory.join("renderer-cases.json"))?)?;
        let index = self.smoke_checks["case_index"].as_u64().unwrap_or(0) as usize;
        if index >= cases.len() {
            fs::write(
                directory.join("result.json"),
                serde_json::to_vec_pretty(&self.smoke_checks)?,
            )?;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return Ok(());
        }
        let case = &cases[index];
        if self.model.is_none() || self.importing {
            return Ok(());
        }
        if self.smoke_stage == 0 {
            self.prefs.auto_preview = false;
            self.active_tab = workspace::Tab::Export;
            self.output_format = if case.sheet {
                OutputFormat::FourView
            } else {
                OutputFormat::Png
            };
            self.renderers.select_backend(true, case.backend);
            self.renderers.viewport = if case.filament_viewport {
                crate::render::selection::ViewportRenderer::Filament
            } else {
                crate::render::selection::ViewportRenderer::Wgpu
            };
            self.show_edges = case.inspection;
            self.inspection = InspectionMode::Opaque;
            self.section = false;
            self.scene
                .select_material(if case.resin { "resin" } else { "matte_pla" });
            let reimport = self.model.as_ref().is_some_and(|model| {
                serde_json::to_string(&model.import_settings).ok()
                    != serde_json::to_string(&self.scene.import).ok()
            });
            self.transparent = case.alpha;
            self.still_size = case.export_size.unwrap_or(512);
            if let Some(samples) = self.renderers.still.quality.samples_mut() {
                *samples = 16;
            }
            self.smoke_checks["camera_before"] = self.camera.camera_spec();
            self.error = None;
            self.changed();
            self.smoke_checks["case_started"] =
                serde_json::json!(self.started.elapsed().as_secs_f64());
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if case.narrow {
                egui::vec2(940., 700.)
            } else {
                egui::vec2(1280., 850.)
            }));
            self.smoke_stage = 1;
            if reimport && let Some(path) = self.selected_source.clone() {
                self.restore_camera = true;
                self.import(path);
            }
            return Ok(());
        }
        if self.smoke_stage == 1 {
            let width = if case.narrow { 940. } else { 1280. };
            if (ctx.viewport_rect().width() - width).abs() > 2. {
                return Ok(());
            }
            self.start_export(
                self.output_format.kind(),
                directory.join(format!("{}.png", case.name)),
                case.export_size.is_none(),
            );
            self.smoke_stage = 2;
            return Ok(());
        }
        if self.smoke_stage == 2 {
            if case.cancel && self.busy.is_some() && self.progress.2 != "Starting renderer" {
                self.cancel_render();
            }
            let expected = case
                .expected_error
                .as_deref()
                .or(case.cancel.then_some("cancelled"));
            if let Some(expected) = expected {
                if self.busy.is_some() {
                    return Ok(());
                }
                if !self
                    .error
                    .as_ref()
                    .is_some_and(|error| error.to_lowercase().contains(&expected.to_lowercase()))
                {
                    if self.error.is_none() {
                        return Ok(());
                    }
                    return Err(format!(
                        "{} expected {expected:?}, got {:?}",
                        case.name, self.error
                    )
                    .into());
                }
            } else {
                if let Some(error) = &self.error {
                    return Err(format!("{}: {error}", case.name).into());
                }
                let displayed = if case.export_size.is_some() {
                    self.output_identity
                } else {
                    self.preview_identity
                };
                if self.busy.is_some()
                    || !displayed.is_some_and(|id| {
                        id.backend == case.backend && id.revision == self.revision
                    })
                {
                    return Ok(());
                }
                if case.filament_viewport && !case.inspection {
                    if let Some(error) = &self.interactive_error {
                        return Err(format!("{} viewport: {error}", case.name).into());
                    }
                    if self.interactive_requested.is_none()
                        || self.interactive_displayed != self.interactive_requested
                    {
                        return Ok(());
                    }
                }
            }
            if self.renderers.still.backend != case.backend {
                return Err("renderer selection changed silently".into());
            }
            if self.camera.camera_spec() != self.smoke_checks["camera_before"] {
                return Err("viewport renderer switch changed canonical camera".into());
            }
            if case.inspection
                && self.interactive_route().badge != Some("Inspection uses Native wgpu")
            {
                return Err("inspection badge missing".into());
            }
            let record = serde_json::json!({
                "name":case.name,"backend":case.backend,"narrow":case.narrow,"alpha":case.alpha,"resin":case.resin,"sheet":case.sheet,
                "viewport":if case.filament_viewport {"filament"} else {"wgpu"},"inspection_badge":self.interactive_route().badge,
                "selected_backend_retained":true,"camera_retained":true,"error":self.error,
                "identity":self.renderer_identities.get(&case.backend),"memory":self.prefs.render_memory,
                "seconds":self.started.elapsed().as_secs_f64()-self.smoke_checks["case_started"].as_f64().unwrap_or(0.),
                "requested_size":self.last_task.as_ref().map(|t|t.0.size),
                "notice":renderers::material_notice(&self.scene.material_preset,case.backend),
            });
            if !self.smoke_checks["cases"].is_array() {
                self.smoke_checks["cases"] = serde_json::json!([]);
            }
            self.smoke_checks["cases"]
                .as_array_mut()
                .unwrap()
                .push(record);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                format!("renderer-case-{index}"),
            )));
            self.smoke_stage = 3;
        }
        if self.smoke_stage == 3 {
            let capture = ctx.input(|input| {
                input.events.iter().find_map(|event| {
                    if let egui::Event::Screenshot {
                        image, user_data, ..
                    } = event
                        && user_data
                            .data
                            .as_ref()
                            .and_then(|data| data.downcast_ref::<String>())
                            .is_some_and(|s| *s == format!("renderer-case-{index}"))
                    {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = capture {
                let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    directory.join(format!("{}-ui.png", case.name)),
                    &rgba,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )?;
                self.smoke_checks["case_index"] = serde_json::json!(index + 1);
                fs::write(
                    directory.join("progress.json"),
                    serde_json::to_vec_pretty(&self.smoke_checks)?,
                )?;
                self.smoke_stage = 0;
            }
        }
        Ok(())
    }
}
