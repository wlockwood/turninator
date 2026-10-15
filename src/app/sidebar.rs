//! Task-oriented settings; asynchronous import/export ownership remains in Studio.
use super::*;

impl Studio {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let preview_options_before = (self.preview_engine().to_owned(), self.transparent);
        let before = serde_json::to_string(&self.scene).unwrap_or_default();
        let import_before = serde_json::to_string(&self.scene.import).unwrap_or_default();
        let was_stepped = self.scene.import.stepped_envelope;
        let mut rebuild_current_down = false;
        if self.model.is_none() {
            ui.heading("Start with a model");
            ui.label("Open an STL or G-code file, or drop one into this window.");
            ui.small("Saved projects restore the model reference, print setup, camera and export settings.");
        } else {
            match self.active_tab {
                workspace::Tab::Model => rebuild_current_down = self.model_controls(ui),
                workspace::Tab::Appearance => self.appearance_controls(ui),
                workspace::Tab::View => self.view_controls(ui),
                workspace::Tab::Export => self.export_controls(ui),
            }
        }
        if rebuild_current_down && let Err(error) = self.prepare_stepped_down() {
            self.scene.import.stepped_envelope = was_stepped;
            rebuild_current_down = false;
            self.error = Some(error.to_string());
        }
        if serde_json::to_string(&self.scene).unwrap_or_default() != before {
            self.changed();
        }
        if (rebuild_current_down
            || serde_json::to_string(&self.scene.import).unwrap_or_default() != import_before)
            && let Some(path) = &self.selected_source
        {
            self.restore_camera = true;
            self.debounced_import = Some((path.clone(), Instant::now()));
            self.importing = true;
            self.gate.advance();
        }
        if (self.preview_engine().to_owned(), self.transparent) != preview_options_before {
            self.changed();
        }
    }

    fn model_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let has_model = self.model.is_some();
        let beads = self
            .model
            .as_ref()
            .is_some_and(|model| model.geometry_kind == "beads");
        let sheet = self.output_format == OutputFormat::FourView;
        let mut rebuild_current_down = false;
        ui.heading("Model setup");
        ui.label("Choose physical dimensions, print direction and layers.");
        if let Some(model) = &self.model {
            ui.collapsing("Source assumptions and features", |ui| {
                for warning in &model.warnings {
                    ui.label(warning);
                }
                let mut features = std::collections::BTreeSet::new();
                let mut tools = std::collections::BTreeSet::new();
                for group in model.groups.iter().filter(|_| beads) {
                    features.insert(&group.feature);
                    tools.insert(group.tool);
                }
                if !tools.is_empty() {
                    ui.label(format!("Tools: {tools:?}"));
                }
                for feature in features {
                    ui.label(feature);
                }
            }).help("Review import assumptions, unsupported-feature warnings, and the tools/features found in this source file.");
        }
        if has_model {
            if !sheet {
                egui::CollapsingHeader::new("Orientation and down").default_open(true).show(ui, |ui| {
                    ui.label("Display rotation (keeps print layers)");
                    rotation_editor(ui, &mut self.scene.display_rotation_degrees, "Rotate presentation while keeping the print layers attached. This does not change the original build direction.");
                    if ui
                        .add_enabled(
                            self.model.is_some() && !self.importing,
                            egui::Button::new(if self.settle_face {
                                "Cancel face selection"
                            } else {
                                "Settle on face…"
                            }),
                        ).help("Click a visible triangle in Native inspection to point its normal down. With stepped STL, selecting the face also rebuilds layers from that down. Click this button again to cancel. This aligns a face rather than simulating stability, and requires a loaded model with no import in progress.")
                        .clicked()
                    {
                        self.settle_face = !self.settle_face;
                        self.timeline_playing = false;
                        if self.timeline_angle != 0. {
                            self.timeline_angle = 0.;
                            self.changed();
                        }
                    }
                    if self.settle_face {
                        ui.label("Click a visible face in Native inspection to point it down.");
                    }
                    ui.small(
                "Uses the chosen triangle's normal; this does not simulate physical stability.",
            );
                }).help("Rotate the displayed part while keeping its physical print layers attached. Enabling stepped STL or choosing Use current down for layers makes this pose the print direction; settling a face while stepped mode is active rebuilds from that face. G-code keeps its original print direction.");
            }
            if !beads {
                egui::CollapsingHeader::new("Source scale and build rotation").default_open(sheet).show(ui, |ui| {
                {
                    workspace::number(ui, "source_scale", "Source scale (mm per unit)", "Convert STL source coordinates to millimetres. Use 1 for millimetres or 25.4 for inches. Rebuilds geometry without changing the file.", egui::DragValue::new(&mut self.scene.import.millimeters_per_unit).speed(0.1).range(0.001..=1000.));
                }
                ui.label("Build rotation (changes print layers)");
                rotation_editor(
                    ui,
                    &mut self.scene.import.build_rotation_degrees,
                    "Change the physical STL build orientation and rebuild its printed layers. The lowest point is placed on the build bed.",
                );

                }).help("Advanced STL import settings. Change unit conversion or the physical build angles directly; most parts can use Settle on face and the current-down action instead. Fixed sheets use this build rotation.");
            }
            workspace::section(ui, "Print geometry");
            if beads {
                {
                    ui.label("Filament diameter (mm)");
                    ui.add(
                    egui::DragValue::new(&mut self.scene.import.filament_diameter_mm)
                        .speed(0.05)
                        .range(0.1..=10.),
                ).help("Filament diameter in millimetres, commonly 1.75. G-code extrusion lengths use this to estimate deposited volume and bead width. It is not the nozzle diameter.");
                }
            }
            if !beads {
                if ui.add_enabled(!self.importing, egui::Checkbox::new(&mut self.scene.import.stepped_envelope,"Experimental STL stepped solid")).help("Build stepped layers from the model's current down, including a pose chosen with Settle on face. Enabling uses that pose as the print orientation; later presentation rotations keep layers attached. Sampled layer solids change the silhouette and may miss small features. For fixed four-view sheets, set the print direction with Build rotation. Finish the current import before switching. Source files are unchanged.").changed()
                && self.scene.import.stepped_envelope && !sheet { rebuild_current_down = true; }
                if self.scene.import.stepped_envelope && !sheet
                && ui.add_enabled(!self.importing,egui::Button::new("Use current down for layers")).help("Rebuild the stepped solid from the part's current pose. This combines display and build rotations, moves the lowest point to the bed, and clears the baked display rotation only after a successful import. Use after manually changing the presentation pose; settling on a face rebuilds stepped layers automatically. Finish any current import first.").clicked() { rebuild_current_down = true; }
                if self.scene.import.stepped_envelope {
                    ui.small("Sampled layer solids change the silhouette. Transparent materials may reveal internal layer interfaces.");
                }
            }
            if beads {
                ui.collapsing("G-code interpretation",|ui| {
                ui.label("Firmware");
            egui::ComboBox::from_id_salt("firmware").width(ui.available_width()).selected_text(&self.scene.import.firmware).show_ui(ui,|ui|{
                    for mode in ["marlin","klipper"] {ui.selectable_value(&mut self.scene.import.firmware,mode.into(),mode).help(if mode == "klipper" { "Use the supported Klipper interpretation rules. Custom macros and implicit offsets still require preprocessing." } else { "Use the supported Marlin-style interpretation rules. Unsupported motion-affecting commands are rejected." });}
                }).help("Select the interpretation rules for the supported G-code dialect. Unsupported motion-affecting commands and unresolved printer macros are rejected explicitly.");
                ui.checkbox(&mut self.scene.import.joined_beads,"Join contiguous bead paths").help("Connect compatible depositing moves into continuous rounded paths. Recommended for large files: cross-section detail adjusts to the geometry budget while retaining every depositing segment. Reversals, tools and features remain separate.");
                ui.checkbox(&mut self.scene.import.allow_arcs,"Allow planar G2/G3 arcs").help("Interpret supported XY-plane circular arcs and approximate them with short bead segments. Other planes and unsupported arc forms still fail explicitly; set approximation precision below.");
                if self.scene.import.allow_arcs { ui.add(egui::DragValue::new(&mut self.scene.import.chord_error_mm).range(0.001..=1.).speed(0.001).suffix(" mm arc error")).help("Maximum arc approximation error in millimetres. Smaller values follow supported arcs more closely but create more segments and use more memory."); }
                ui.checkbox(&mut self.scene.import.allow_tool_changes,"Normalized tool changes").help("Allow tool-selection commands that share one XYZ frame and a global extrusion coordinate. Resolve tool offsets and printer macros before import; this does not simulate independent tool kinematics.");
                if self.scene.import.allow_tool_changes { ui.small("Tool changes assume shared XYZ / global E coordinates. Printer macros and implicit offsets require preprocessing."); }
                let mut homing = self.scene.import.homing_position_mm.is_some();
                if ui.checkbox(&mut homing,"Explicit G28 home position").help("Provide the known XYZ position used when interpreting G28 homing commands. Enter coordinates in millimetres; the app never sends commands to a printer.").changed() {self.scene.import.homing_position_mm=homing.then_some([0.;3]);}
                if let Some(home) = &mut self.scene.import.homing_position_mm {ui.horizontal(|ui|{for (axis,value) in ["X","Y","Z"].into_iter().zip(home) {ui.add(egui::DragValue::new(value).prefix(format!("{axis} ")).speed(0.1)).help(format!("Known {axis}-axis home coordinate in millimetres, used when the G-code issues G28. Enter the machine position explicitly rather than assuming an unknown home."));}});}
            }).help("Choose the supported dialect and explicitly enable optional path features. This is a bounded geometry preview, not a printer firmware or macro emulator.");
            }
            {
                workspace::section(ui, "Layer heights");
                if beads && self.scene.import.layer_boundaries_mm.is_empty() {
                    ui.small("Slicer heights are read or estimated from the file; defaults cover other supported G-code. See import warnings for assumptions.");
                }
                controls::layer_settings(ui, &mut self.scene, false);
                if beads || self.scene.import.stepped_envelope {
                    ui.collapsing("Explicit adaptive layer boundaries",|ui| {
                ui.label("G-code / stepped STL: mm boundaries, starting at 0");
                ui.text_edit_multiline(&mut self.adaptive_text).help("Comma-separated physical layer boundaries in millimetres, starting at 0. Editing this text alone does not rebuild; choose Apply boundaries when ready.");
                if ui.button("Apply boundaries").help("Validate and rebuild using the comma-separated layer boundaries above. Begin at 0 and use strictly increasing millimetre heights. G-code deposition heights must match the boundaries.").clicked() {
                    let parsed: std::result::Result<Vec<f64>,_> = self.adaptive_text.split(',').map(|v|v.trim().parse::<f64>()).collect();
                    match parsed {
                        Ok(values) => match crate::print::schedule::LayerSchedule::explicit(values.clone()) {
                            Ok(_) => {self.scene.import.layer_boundaries_mm=values;self.scene.print_settings_edited=true;},
                            Err(error) => self.error=Some(error.to_string()),
                        },
                        Err(_) => self.error=Some("Enter numeric comma-separated layer boundaries.".into()),
                    }
                }
                if ui.button("Use uniform layers / file metadata").help("Clear the explicit adaptive schedule and return to the layer-height defaults or supported G-code HEIGHT metadata.").clicked() {self.scene.import.layer_boundaries_mm.clear();self.adaptive_text.clear();}
                ui.small(format!("{} explicit boundaries",self.scene.import.layer_boundaries_mm.len()));
            }).help("Define exact layer heights instead of uniform defaults. Enter boundary heights in millimetres, starting with 0 and increasing, for example: 0, 0.2, 0.45, 0.65.");
                }
            }
        }
        rebuild_current_down
    }

    fn appearance_controls(&mut self, ui: &mut egui::Ui) {
        let has_model = self.model.is_some();
        let beads = self
            .model
            .as_ref()
            .is_some_and(|model| model.geometry_kind == "beads");
        let sheet = self.output_format == OutputFormat::FourView;
        if sheet {
            ui.heading("Drawing appearance");
            ui.label("Four-view sheets use a fixed gray material and drawing studio.");
            ui.small(
                "Switch the output format in Export to edit presentation materials and lighting.",
            );
            if ui.button("Go to Export").help("Open output settings to choose GIF, MP4 or PNG instead of a fixed four-view drawing.").clicked() { self.active_tab = workspace::Tab::Export; }
            return;
        }
        if has_model && !sheet {
            ui.heading("Print appearance");
            let mut material = self.scene.material_preset.clone();
            ui.label("Material");
            egui::ComboBox::from_id_salt("material").width(ui.available_width())
                .selected_text(material_name(&material))
                .show_ui(ui, |ui| {
                    for value in ["matte_pla", "silk_pla", "glossy_plastic", "clay", "resin"] {
                        ui.selectable_value(&mut material, value.into(), material_name(value)).help(help::material(value));
                    }
                }).help("Choose a surface finish. Explicitly edited layer settings are preserved; choosing Resin selects Cycles for rendering.");
            if material != self.scene.material_preset {
                self.scene.select_material(&material);
                if material == "resin" {
                    self.engine = "cycles".into();
                    self.still_engine = "cycles".into();
                }
            }
            if !beads || self.scene.color_mode == "uniform" {
                ui.label("Base color");
                workspace::color(
                    ui,
                    &mut self.scene.color,
                    "Choose the base surface color. This applies when color mapping is Uniform; mapped G-code paths use the print palette.",
                );
            }
            if beads {
                ui.label("Color mapping");
                egui::ComboBox::from_id_salt("color_mapping").width(ui.available_width())
                    .selected_text(match self.scene.color_mode.as_str() { "tool" => "By tool", "feature" => "By feature", _ => "Uniform" })
                    .show_ui(ui, |ui| {
                        for mode in ["uniform", "tool", "feature"] {
                            ui.selectable_value(&mut self.scene.color_mode, mode.into(), match mode { "tool" => "By tool", "feature" => "By feature", _ => "Uniform" }).help(match mode { "tool" => "Assign colors by G-code tool number; edit the referenced swatches in Print palette.", "feature" => "Assign colors by feature tags from file comments, such as walls and infill. Features may share a palette entry.", _ => "Use one base color for the whole model." });
                        }
                    }).help("Use one uniform color, map colors by G-code tool number, or map them by feature labels found in file comments. Only referenced palette entries are shown.");
                if self.scene.color_mode != "uniform" {
                    ui.collapsing("Print palette", |ui| {
                        let mut used = std::collections::BTreeMap::<
                            usize,
                            std::collections::BTreeSet<String>,
                        >::new();
                        for group in &self.model.as_ref().expect("mapped model").groups {
                            if let Some(index) = group_color_index(group, &self.scene) {
                                used.entry(index).or_default().insert(
                                    if self.scene.color_mode == "tool" {
                                        format!("Tool {}", group.tool)
                                    } else {
                                        group.feature.clone()
                                    },
                                );
                            }
                        }
                        for (index, labels) in used {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(labels.into_iter().collect::<Vec<_>>().join(", "));
                                workspace::color(ui, &mut self.scene.tool_colors[index], "Choose the color shared by the tools or features listed beside this swatch. This changes rendered and native mapped appearance.");
                            });
                        }
                    }).help("Edit only colors referenced by the loaded paths. Labels identify their tools or features; entries that share a palette color change together.");
                }
            }
            if self.scene.material_preset != "resin" {
                workspace::slider(
                    ui,
                    "Roughness",
                    "How broad and soft surface reflections are. Lower values look polished; higher values look matte. This changes appearance rather than the source geometry.",
                    egui::Slider::new(&mut self.scene.roughness, 0.02..=1.),
                );
            }
            if !beads && !self.scene.import.stepped_envelope {
                ui.checkbox(&mut self.scene.layer_appearance, "Show layer appearance").help("Shade horizontal print-layer seams onto the original STL surface. Layers stay attached through presentation rotation. This preserves the silhouette and does not reconstruct actual slicer paths.");
            }
            if !beads && self.scene.layer_appearance && !self.scene.import.stepped_envelope {
                controls::groove_settings(ui, &mut self.scene);
                ui.small("Set physical layer heights in Model.");
            }
            if self.scene.material_preset == "resin" {
                workspace::slider(
                    ui,
                    "Clarity",
                    "Increase how much light passes through Resin and reduce its absorption. Internal surfaces and overlapping beads can affect the result, so use it as an illustration.",
                    egui::Slider::new(&mut self.scene.resin_clarity, 0.0..=1.),
                );
                workspace::slider(
                    ui,
                    "Cloudiness",
                    "Make the resin surface scatter reflections and refraction more softly. Lower values look clearer and more polished; higher values look hazier.",
                    egui::Slider::new(&mut self.scene.resin_cloudiness, 0.0..=1.),
                );
                ui.small("Resin appearance is approximate; internal voids require real geometry.");
            }
            ui.label("Studio");
            egui::ComboBox::from_id_salt("studio").width(ui.available_width())
                .selected_text(if self.scene.studio_preset == "dark" { "Dark" } else { "Light" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.scene.studio_preset, "light".into(), "Light").help("Use a lighter rendered studio floor, background and ambient illumination.");
                    ui.selectable_value(&mut self.scene.studio_preset, "dark".into(), "Dark").help("Use a darker rendered studio floor, background and ambient illumination, with the same arranged studio lights.");
                }).help("Choose the rendered background, floor and ambient illumination. Native lighting is approximate; Render preview shows the Blender result.");
        }
    }

    fn view_controls(&mut self, ui: &mut egui::Ui) {
        let has_model = self.model.is_some();
        let buildable = self
            .model
            .as_ref()
            .is_some_and(|model| !model.layer_tops_mm.is_empty());
        let sheet = self.output_format == OutputFormat::FourView;
        if has_model {
            ui.heading(if sheet { "Inspection camera" } else { "Camera" });
            ui.horizontal(|ui| {
                if ui.button("Fit").help("Center and zoom the inspection camera so the current displayed model fits. This changes the view, not the physical model size.").clicked() {
                    self.timeline_playing = false;
                    self.fit();
                    self.changed();
                }
                for (label, view) in [
                    ("Front", CanonicalView::Front),
                    ("Right", CanonicalView::Right),
                    ("Top", CanonicalView::Top),
                    ("Iso", CanonicalView::Iso),
                ] {
                    if ui.button(label).help(format!("Look from the {label} direction and fit the model into the inspection view. Sheet exports keep their fixed four cameras.")).clicked() {
                        self.timeline_playing = false;
                        self.camera.canonical(view);
                        self.fit();
                        self.changed();
                    }
                }
            });
            if ui
                .checkbox(&mut self.camera.orthographic, "Orthographic").help("Show parallel edges without perspective shrinking distant features. Turn off for a perspective view. Camera keys in one timeline must use the same projection.")
                .changed()
            {
                self.timeline_playing = false;
                self.changed();
            }
            ui.checkbox(&mut self.show_edges, "Triangle edges").help("Overlay mesh triangle edges in native inspection to reveal tessellation. This overlay is not added to rendered exports.");
            ui.label("Inspection");
            egui::ComboBox::from_id_salt("inspection").width(ui.available_width())
                .selected_text(format!("{:?}", self.inspection))
                .show_ui(ui, |ui| {
                    for mode in [
                        InspectionMode::Opaque,
                        InspectionMode::Ghost,
                        InspectionMode::Xray,
                    ] {
                        ui.selectable_value(&mut self.inspection, mode, format!("{mode:?}")).help(match mode { InspectionMode::Opaque => "Show solid native surfaces with normal depth testing.", InspectionMode::Ghost => "Blend translucent native surfaces to inspect internal structure. Adjust Inspection opacity below.", InspectionMode::Xray => "Reveal native geometry through nearer surfaces. This inspection aid does not change exported material transparency." });
                    }
                }).help("Choose how to inspect the native mesh: opaque surfaces, translucent Ghost, or Xray with visible structure through the model. This does not change export materials.");
            if self.inspection != InspectionMode::Opaque {
                workspace::slider(
                    ui,
                    "Inspection opacity",
                    "Set how much of the native model shows through in Ghost or Xray mode. This is an inspection aid; rendered material transparency uses Resin settings.",
                    egui::Slider::new(&mut self.opacity, 0.02..=0.95),
                );
            }
            if buildable {
                if ui
                    .checkbox(&mut self.buildup, "Show printed build-up").help("Reveal the part up to the selected print height. Closed STL surfaces are cut as a solid, with a filled top and real holes preserved; G-code reveals complete deposited beads. Preview, PNG, sheets and ordinary turntables retain this fixed stage. Export build-up animation grows the part over the clip.")
                    .changed()
                {
                    self.changed();
                }
                if self.buildup {
                    let top = self
                        .model
                        .as_ref()
                        .map_or(1., |m| m.bounds_mm.max[2] as f32);
                    if ui
                        .add(
                            egui::Slider::new(&mut self.buildup_height, 0.0..=top)
                                .text("Printed height (mm)"),
                        ).help("Printed height in millimetres, snapped to whole layer tops. STL uses the configured first/later layer heights with the last layer at the model top; G-code uses its depositing heights. Zero shows an empty build, and the maximum shows the complete model.")
                        .changed()
                    {
                        self.timeline_playing = false;
                        self.changed();
                    }
                    ui.checkbox(&mut self.buildup_playing, "Play build-up").help("Cycle printed height in native inspection to see how the part grows. Export build-up animation creates a GIF or MP4 and has its own timing.");
                    if let Some(warning) = self.viewport.buildup_cap_warning() {
                        ui.colored_label(egui::Color32::YELLOW, warning);
                    }
                }
            }
            ui.small("Drag to orbit • right-drag to pan • wheel to zoom");
        }
        if has_model && self.output_format.supports_section() {
            ui.collapsing("Section analysis", |ui| {
            let before = (self.section,self.section_axis,self.section_position,self.section_positive,self.section_cap,self.side_by_side);
            ui.checkbox(&mut self.section, "Show section / export cutaway").help("Cut the model on a plane in build coordinates for inspection and export. Enable caps for supported closed meshes, or compare the full model beside the section.");
            if self.section {
            ui.horizontal(|ui| { ui.label("Build axis"); for (axis, label) in [(0, "X"), (1, "Y"), (2, "Z")] { ui.selectable_value(&mut self.section_axis, axis, label).help(format!("Cut perpendicular to the {label} build axis. Plane position is measured before presentation rotation.")); } });
            ui.horizontal(|ui| { ui.label("Position (mm)"); ui.add(egui::DragValue::new(&mut self.section_position).speed(0.1)).help("Cut-plane position in millimetres along the selected build axis, before presentation rotation. Place it inside the part to expose its interior."); });
            ui.checkbox(&mut self.section_positive, "Keep positive side").help("Keep material at coordinates greater than the cut-plane position along the selected build axis. Turn off to keep the lower-coordinate side.");
            ui.checkbox(&mut self.section_cap, "Cap closed geometry").help("Fill the cut face when the input is closed and consistently wound. Native cutaways stay uncapped; Blender computes rendered caps and section statistics. Open geometry remains uncapped with an explanation.");
            ui.checkbox(&mut self.side_by_side, "Full model beside section").help("Place the uncut model beside the cutaway at the same scale. Both panels share animation and deposited stage; output width doubles while height stays the selected pixel size.");
            ui.small("STL sections show the modeled solid. G-code sections preserve deposited beads and voids.");
            ui.small("Native cutaway is uncapped. Reliable caps and area/volume metrics are computed during export.");
            if let Some(analysis) = &self.analysis {
                workspace::section(ui, "Last rendered measurements");
                for (key, label, unit) in [("cross_section_area_mm2", "Cut area", "mm²"), ("original_volume_mm3", "Original volume", "mm³"), ("remaining_volume_mm3", "Remaining volume", "mm³")] {
                    if let Some(value) = analysis[key].as_f64() { ui.label(format!("{label}: {value:.2} {unit}")); }
                }
                ui.small("Geometric estimates from the last rendered section; refresh after changes.");
                ui.collapsing("Measurement details", |ui| { ui.label(serde_json::to_string_pretty(analysis).unwrap_or_default()); }).help("Inspect the complete section measurements and geometry warnings from the last render.");
            }
            }
            if before != (self.section,self.section_axis,self.section_position,self.section_positive,self.section_cap,self.side_by_side) { self.changed(); }
        }).help("Inspect or export a physical cutaway. Closed meshes can provide caps and area/volume estimates; deposited G-code beads retain their gaps.");
        }
    }

    fn export_controls(&mut self, ui: &mut egui::Ui) {
        let has_model = self.model.is_some();
        let buildable = self
            .model
            .as_ref()
            .is_some_and(|model| !model.layer_tops_mm.is_empty());
        ui.heading("Export settings");
        if (controls::OutputEditor {
            format: &mut self.output_format,
            codec: &mut self.codec,
            frame_rate: &mut self.frame_rate,
            animation_engine: &mut self.engine,
            animation_size: &mut self.size,
            animation_samples: &mut self.samples,
            still_engine: &mut self.still_engine,
            still_size: &mut self.still_size,
            still_samples: &mut self.still_samples,
            transparent: &mut self.transparent,
        })
        .show(ui)
        {
            self.timeline_playing = false;
            self.settle_face = false;
            self.changed();
        }
        let animated = self.output_format.animated();
        if has_model && animated {
            self.timeline_controls(ui);
            if buildable {
                controls::build_up_checkbox(
                    ui,
                    &mut self.export_buildup,
                    self.timeline_enabled.then_some(&mut self.timeline),
                );
            }
        }
        if animated && !self.timeline_enabled && buildable && self.export_buildup {
            ui.small("Build-up uses the current fixed camera. Enable camera keyframes above to add movement.");
        }
        if has_model && animated && !self.timeline_enabled && !(buildable && self.export_buildup) {
            ui.group(|ui| {
                ui.label("Basic turntable");
                workspace::number(ui, "seconds_per_turn", "Seconds per turn", "Seconds for one complete rotation. Lower values rotate faster; clip length is set separately.", egui::DragValue::new(&mut self.turntable_timing.seconds_per_turn).range(0.05..=500.).speed(0.05).max_decimals(3).suffix(" s"));
                workspace::number(ui, "animation_length", "Animation length", "Total clip length, rounded to the nearest output frame. Turns equal actual length divided by seconds per turn. Matching poses render once and are reused.", egui::DragValue::new(&mut self.turntable_timing.duration_seconds).range(0.05..=500.).speed(0.05).max_decimals(2).suffix(" s"));
                match (self.turntable_timing.total_turns_at_rate(self.frame_rate), self.turntable_timing.frame_count_at_rate(self.frame_rate)) {
                    (Ok(turns), Ok(frames)) => {
                        ui.label(format!("{turns:.3} total turns"));
                        ui.small(format!("{frames} frames • {:.2} s at {} fps", f64::from(frames)/f64::from(self.frame_rate),self.frame_rate));
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        ui.colored_label(egui::Color32::YELLOW, error.to_string());
                    }
                }
                ui.small("Matching frames are rendered once and reused. Length rounds to the nearest frame.");
            });
        }
        if self.section && self.output_format.supports_section() {
            ui.small(if self.side_by_side {
                "Cutaway active: full model and section will export side by side. Edit in View."
            } else {
                "Cutaway active. Edit its plane and caps in View."
            });
        }
    }

    pub(super) fn primary_actions(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(
            self.model.is_some()
                && self.busy.is_none()
                && !self.importing
                && !self.cache_action_pending,
            |ui| {
                if ui
                    .add_sized(
                        [ui.available_width(), 32.],
                        egui::Button::new("Render preview"),
                    )
                    .help(help::PREVIEW)
                    .clicked()
                {
                    self.start_export(
                        ExportKind::Still,
                        self.data_dir.join(format!("preview-{}.png", self.revision)),
                        true,
                    );
                }
                if ui
                    .add_sized(
                        [ui.available_width(), 34.],
                        egui::Button::new(format!("Export {}…", self.output_format.label()))
                            .fill(ui.visuals().selection.bg_fill),
                    )
                    .help(help::EXPORT)
                    .clicked()
                {
                    self.pick_export(self.output_format.kind());
                }
            },
        );
        if ui.checkbox(&mut self.prefs.auto_preview, "Auto-update preview").help("Refresh the Blender preview after editing pauses. Disable this during many changes if rendering is slow. The selected output renderer is used.").changed() {
            self.save_prefs();
        }
        if self.model.is_none() {
            ui.small("Open a model to preview or export.");
        } else if self.importing {
            ui.small("Rebuilding model…");
        } else if self.busy.is_some() {
            ui.small("Rendering in progress. Cancel in the status bar.");
        } else if self.cache_action_pending {
            ui.small("Waiting for storage operation…");
        }
    }
}
