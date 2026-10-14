use super::{
    beads::{MAX_BEAD_SEGMENTS, validate_segment},
    schedule::{LayerSchedule, MAX_LAYERS, validate_height},
};
use crate::{Result, print::beads::BeadSegment};
use std::{collections::BTreeMap, f64::consts::PI};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GcodeDialect {
    #[default]
    Marlin,
    Klipper,
}

#[derive(Clone, Debug)]
pub struct ParseOptions {
    pub layer_height_mm: f64,
    pub first_layer_height_mm: f64,
    pub filament_diameter_mm: f64,
    pub dialect: GcodeDialect,
    pub allow_arcs: bool,
    /// Explicit normalized toolchange contract: shared XYZ frame/global E counter,
    /// no implicit tool offset, parking, purge, script or firmware motion.
    pub allow_tool_changes: bool,
    pub chord_error_mm: f64,
    pub homing_position_mm: Option<[f64; 3]>,
    pub layer_schedule: Option<LayerSchedule>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            layer_height_mm: 0.2,
            first_layer_height_mm: 0.2,
            filament_diameter_mm: 1.75,
            dialect: GcodeDialect::Marlin,
            allow_arcs: false,
            allow_tool_changes: false,
            chord_error_mm: 0.02,
            homing_position_mm: None,
            layer_schedule: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolpathData {
    pub segments: Vec<BeadSegment>,
    pub warnings: Vec<String>,
}

pub fn parse_gcode(text: &str, options: &ParseOptions) -> Result<ToolpathData> {
    validate_height(options.layer_height_mm)?;
    validate_height(options.first_layer_height_mm)?;
    if !options.chord_error_mm.is_finite() || !(0.001..=1.0).contains(&options.chord_error_mm) {
        return Err("arc chord error must be in 0.001..=1 mm".into());
    }
    if options
        .homing_position_mm
        .is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1e7))
    {
        return Err("homing position must be finite and within coordinate budget".into());
    }
    if let Some(schedule) = &options.layer_schedule {
        LayerSchedule::explicit(schedule.boundaries_mm.clone())?;
    }
    if !options.filament_diameter_mm.is_finite()
        || !(0.1..=10.).contains(&options.filament_diameter_mm)
    {
        return Err("filament diameter must be finite and in 0.1..=10 mm".into());
    }
    if text.len() > 128 * 1024 * 1024 {
        return Err("G-code exceeds 128 MiB input budget".into());
    }
    let mut state = State::default();
    let mut result = ToolpathData {
        segments: Vec::new(),
        warnings: vec![format!(
            "Estimated stadium beads, not physical deposition prediction. Assumed filament diameter {} mm; default first/later heights {}/{} mm when HEIGHT metadata is absent. Initial position/counters are zero and positioning is absolute; only the explicitly selected firmware subset is interpreted.",
            options.filament_diameter_mm, options.first_layer_height_mm, options.layer_height_mm
        )],
    };
    result.warnings.push(format!(
        "Selected {:?} dialect; arc support {} with <= {} mm chord error.",
        options.dialect, options.allow_arcs, options.chord_error_mm
    ));
    if options.allow_tool_changes {
        result.warnings.push("Tool changes assume normalized shared XYZ coordinates and a global E counter; implicit tool offsets, parking, purge and macros are not simulated. Retraction debt is tracked per tool.".into());
    }
    if let Some(position) = options.homing_position_mm {
        result.warnings.push(format!("G28 homing uses explicitly supplied physical position {position:?} mm and resets selected-axis coordinate offsets."));
    }
    if options.layer_schedule.is_some() {
        result.warnings.push("Explicit adaptive layer boundaries supplied; every depositing Z must match a boundary. Heights are not automatically inferred.".into());
    }
    for (index, line) in text.lines().enumerate() {
        let source_line = index + 1;
        if source_line > 2_000_000 || line.len() > 16_384 {
            return Err(
                format!("line {source_line}: G-code line count/length budget exceeded").into(),
            );
        }
        let (code, comment) = line.split_once(';').unwrap_or((line, ""));
        state
            .metadata(comment)
            .map_err(|e| format!("line {source_line}: {e}"))?;
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let words = tokenize(code).map_err(|e| format!("line {source_line}: {e}"))?;
        let command = words[0];
        if !['G', 'M', 'T'].contains(&command.0)
            || command.1.fract() != 0.
            || command.1 < 0.
            || command.1 > 9999.
        {
            return Err(format!("line {source_line}: unsupported command {code}").into());
        }
        let mut parameters = BTreeMap::new();
        for &(key, value) in &words[1..] {
            if parameters.insert(key, value).is_some() {
                return Err(format!("line {source_line}: duplicate parameter {key}").into());
            }
        }
        state
            .command(
                command.0,
                command.1 as u32,
                &parameters,
                source_line,
                options,
                &mut result,
            )
            .map_err(|e| format!("line {source_line}: {e}"))?;
    }
    if result.segments.is_empty() {
        return Err("G-code contains no supported depositing moves".into());
    }
    Ok(result)
}

struct State {
    position: [f64; 3],
    offsets: [f64; 3],
    e_position: f64,
    unit: f64,
    xyz_absolute: bool,
    e_override: Option<bool>,
    retract_debt: f64,
    height: Option<f64>,
    width: Option<f64>,
    feature: String,
    deposition_z: Option<f64>,
    layer_index: u32,
    prime_warning: bool,
    tool: u32,
    tool_debts: BTreeMap<u32, f64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            offsets: [0.; 3],
            e_position: 0.,
            unit: 1.,
            xyz_absolute: true,
            e_override: None,
            retract_debt: 0.,
            height: None,
            width: None,
            feature: "Unknown".into(),
            deposition_z: None,
            layer_index: 0,
            prime_warning: false,
            tool: 0,
            tool_debts: BTreeMap::new(),
        }
    }
}

impl State {
    fn metadata(&mut self, comment: &str) -> Result<()> {
        let Some((key, value)) = comment.trim().split_once(':') else {
            return Ok(());
        };
        match key.trim() {
            "HEIGHT" | "LAYER_HEIGHT" | "layer_height" => {
                let h = value.trim().parse::<f64>()?;
                validate_height(h)?;
                self.height = Some(h);
            }
            "WIDTH" | "LINE_WIDTH" | "line_width" => {
                let w = value.trim().parse::<f64>()?;
                if !w.is_finite() || w <= 0. || w > 10. {
                    return Err("invalid WIDTH metadata in millimeters".into());
                }
                self.width = Some(w);
            }
            "TYPE" | "FEATURE" | "feature" => self.feature = value.trim().to_string(),
            _ => {}
        }
        Ok(())
    }

    fn command(
        &mut self,
        family: char,
        number: u32,
        p: &BTreeMap<char, f64>,
        line: usize,
        options: &ParseOptions,
        result: &mut ToolpathData,
    ) -> Result<()> {
        match (family,number) {
            ('G',0|1)=> { allowed(p,"XYZEF")?; self.motion(p,line,options,result,None)?; }
            ('G',2|3) if options.allow_arcs=>{allowed(p,"XYZEFIJR")?;self.arc_motion(p,number==2,line,options,result)?;}
            ('G',17)=>allowed(p,"")?,
            ('G',20|21)=> { allowed(p,"")?;if number==20 && options.dialect==GcodeDialect::Klipper {return Err("Klipper does not support G20 inch mode".into());}self.unit=if number==20 {25.4} else {1.}; }
            ('G',90|91)=> { allowed(p,"")?;self.xyz_absolute=number==90;if options.dialect==GcodeDialect::Marlin {self.e_override=None;} }
            ('M',82|83)=> { allowed(p,"")?;self.e_override=Some(number==82); }
            ('G',92)=> {
                allowed(p,"XYZE")?;
                if p.is_empty() { return Err("G92 without explicit coordinates is unsupported".into()); }
                for (i,axis) in ['X','Y','Z'].into_iter().enumerate() {
                    if let Some(value)=p.get(&axis) { self.offsets[i]=self.position[i]-value*self.unit; }
                }
                if let Some(value)=p.get(&'E') { self.e_position=value*self.unit; }
            }
            ('T',0) if !options.allow_tool_changes=> { allowed(p,"")?; }
            ('T',_) if options.allow_tool_changes && number<16=> { allowed(p,"")?;self.tool_debts.insert(self.tool,self.retract_debt);self.tool=number;self.retract_debt = *self.tool_debts.get(&number).unwrap_or(&0.); }
            ('G',28) if options.homing_position_mm.is_some()=>{allowed(p,"XYZ")?;if p.values().any(|&v|v!=0.) {return Err("homing axis flags must have no value or zero".into());}let home=options.homing_position_mm.unwrap();for (i,axis) in ['X','Y','Z'].into_iter().enumerate() {if p.is_empty() || p.contains_key(&axis) {self.position[i]=home[i];self.offsets[i]=0.;}}}
            ('M',104|109)=> { allowed(p,"SRT")?; if p.get(&'T').is_some_and(|&t|t.fract()!=0. || !(0. ..16.).contains(&t) || (t!=0. && !options.allow_tool_changes)) { return Err("temperature command selects an unsupported tool".into()); } }
            ('M',140|190)=>allowed(p,"SR")?,
            ('M',106)=>allowed(p,"SP")?,
            ('M',107)=>allowed(p,"P")?,
            ('M',105|114|115|400)=>allowed(p,"")?,
            ('G',4)=>allowed(p,"PS")?,
            _=>return Err(format!("unsupported command {family}{number}; modes, homing, arcs, volumetric extrusion, tools and macros require an explicit supported contract").into()),
        }
        if self
            .position
            .iter()
            .chain(self.offsets.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e7)
            || !self.e_position.is_finite()
            || self.e_position.abs() > 1e12
        {
            return Err("G-code coordinates exceed finite processing budget".into());
        }
        Ok(())
    }

    fn e_absolute(&self, options: &ParseOptions) -> bool {
        match options.dialect {
            GcodeDialect::Marlin => self.e_override.unwrap_or(self.xyz_absolute),
            GcodeDialect::Klipper => self.xyz_absolute && self.e_override.unwrap_or(true),
        }
    }

    fn arc_motion(
        &mut self,
        p: &BTreeMap<char, f64>,
        clockwise: bool,
        line: usize,
        options: &ParseOptions,
        result: &mut ToolpathData,
    ) -> Result<()> {
        let start = self.position;
        let end: [f64; 3] = std::array::from_fn(|i| {
            p.get(&['X', 'Y', 'Z'][i]).map_or(start[i], |value| {
                if self.xyz_absolute {
                    value * self.unit + self.offsets[i]
                } else {
                    start[i] + value * self.unit
                }
            })
        });
        if (end[2] - start[2]).abs() > 1e-8 {
            return Err("only planar XY arcs are supported; helical/vase arcs are rejected".into());
        }
        if end.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
            return Err("arc endpoint exceeds coordinate budget".into());
        }
        let has_center = p.contains_key(&'I') || p.contains_key(&'J');
        if has_center == p.contains_key(&'R') {
            return Err("arc requires either I/J center offsets or R, never both".into());
        }
        let center = if let Some(r) = p.get(&'R') {
            let radius = r * self.unit;
            let dx = end[0] - start[0];
            let dy = end[1] - start[1];
            let chord = dx.hypot(dy);
            if chord <= 1e-10 || radius == 0. || chord > 2. * radius.abs() {
                return Err("invalid R arc radius/endpoints; full circles require I/J".into());
            }
            let offset = (radius * radius - chord * chord * 0.25).max(0.).sqrt();
            let sign = if clockwise ^ (radius < 0.) { -1. } else { 1. };
            [
                (start[0] + end[0]) * 0.5 - sign * dy / chord * offset,
                (start[1] + end[1]) * 0.5 + sign * dx / chord * offset,
            ]
        } else {
            [
                start[0] + p.get(&'I').copied().unwrap_or(0.) * self.unit,
                start[1] + p.get(&'J').copied().unwrap_or(0.) * self.unit,
            ]
        };
        if center.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
            return Err("arc center exceeds coordinate budget".into());
        }
        let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
        let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
        if radius <= 1e-10
            || !radius.is_finite()
            || (radius - end_radius).abs() > options.chord_error_mm * 0.25
        {
            return Err(
                "arc endpoints disagree with the center radius beyond the error budget".into(),
            );
        }
        let first = (start[1] - center[1]).atan2(start[0] - center[0]);
        let last = (end[1] - center[1]).atan2(end[0] - center[0]);
        let mut sweep = if clockwise {
            -(first - last).rem_euclid(2. * PI)
        } else {
            (last - first).rem_euclid(2. * PI)
        };
        if start[0] == end[0] && start[1] == end[1] {
            sweep = if clockwise { -2. * PI } else { 2. * PI };
        }
        if sweep.abs() < 1e-12 {
            return Err("arc sweep is too small to resolve".into());
        }
        // Half the error budget is reserved for endpoint radius rounding.
        let maximum_angle =
            (2. * (1. - (options.chord_error_mm * 0.5 / radius).min(1.)).acos()).min(PI / 8.);
        if maximum_angle <= 0. {
            return Err("arc exceeds numeric subdivision precision".into());
        }
        let steps = (sweep.abs() / maximum_angle).ceil() as usize;
        if steps == 0
            || steps > MAX_BEAD_SEGMENTS
            || result.segments.len() + steps > MAX_BEAD_SEGMENTS
        {
            return Err("arc subdivision exceeds bead budget".into());
        }
        let initial_e = self.e_position;
        let total_e = p.get(&'E').map_or(0., |value| {
            if self.e_absolute(options) {
                value * self.unit - initial_e
            } else {
                value * self.unit
            }
        });
        if !total_e.is_finite() || total_e.abs() > 1e9 {
            return Err("arc extrusion exceeds budget".into());
        }
        let length = radius * sweep.abs() / steps as f64;
        for index in 1..=steps {
            let progress = index as f64 / steps as f64;
            let angle = first + sweep * progress;
            let point = if index == steps {
                end
            } else {
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                    start[2],
                ]
            };
            let mut command = BTreeMap::new();
            for (axis, key) in ['X', 'Y', 'Z'].into_iter().enumerate() {
                command.insert(
                    key,
                    if self.xyz_absolute {
                        (point[axis] - self.offsets[axis]) / self.unit
                    } else {
                        (point[axis] - self.position[axis]) / self.unit
                    },
                );
            }
            if p.contains_key(&'E') {
                command.insert(
                    'E',
                    if self.e_absolute(options) {
                        (initial_e + total_e * progress) / self.unit
                    } else {
                        total_e / steps as f64 / self.unit
                    },
                );
            }
            if let Some(feed) = p.get(&'F') {
                command.insert('F', *feed);
            }
            self.motion(&command, line, options, result, Some(length))?;
        }
        Ok(())
    }

    fn motion(
        &mut self,
        p: &BTreeMap<char, f64>,
        line: usize,
        options: &ParseOptions,
        result: &mut ToolpathData,
        length_override: Option<f64>,
    ) -> Result<()> {
        if p.get(&'F').is_some_and(|&f| f <= 0.) {
            return Err("feed rate must be positive".into());
        }
        let old = self.position;
        for (i, axis) in ['X', 'Y', 'Z'].into_iter().enumerate() {
            if let Some(value) = p.get(&axis) {
                self.position[i] = if self.xyz_absolute {
                    value * self.unit + self.offsets[i]
                } else {
                    old[i] + value * self.unit
                };
            }
        }
        let delta_e = if let Some(value) = p.get(&'E') {
            let commanded = value * self.unit;
            if self.e_absolute(options) {
                let delta = commanded - self.e_position;
                self.e_position = commanded;
                delta
            } else {
                self.e_position += commanded;
                commanded
            }
        } else {
            0.
        };
        if !delta_e.is_finite() || delta_e.abs() > 1e9 {
            return Err("extrusion delta exceeds finite processing budget".into());
        }
        if delta_e <= 0. {
            self.retract_debt -= delta_e;
            return Ok(());
        }
        let repaid = self.retract_debt.min(delta_e);
        self.retract_debt -= repaid;
        let deposited = delta_e - repaid;
        if deposited <= 1e-12 {
            return Ok(());
        }
        if (self.position[2] - old[2]).abs() > 1e-8 {
            return Err("nonplanar depositing move is unsupported".into());
        }
        let length = length_override
            .unwrap_or_else(|| (self.position[0] - old[0]).hypot(self.position[1] - old[1]));
        if length <= 1e-10 {
            if !self.prime_warning {
                result.warnings.push(
                    "Extrusion-only priming/unretraction has no spatial bead and is omitted."
                        .into(),
                );
                self.prime_warning = true;
            }
            return Ok(());
        }
        let z = self.position[2];
        if let Some(previous) = self.deposition_z {
            if z < previous - 1e-8 {
                return Err("deposition returns to a lower layer; sequential objects/nonplanar schedules are unsupported".into());
            }
            if z > previous + 1e-8 {
                self.layer_index += 1;
            }
        }
        if self.layer_index as usize >= MAX_LAYERS {
            return Err("deposition layer budget exceeded".into());
        }
        let schedule_height = if let Some(schedule) = &options.layer_schedule {
            let boundary = schedule
                .boundaries_mm
                .iter()
                .position(|&boundary| (boundary - z).abs() <= 1e-7)
                .ok_or("depositing Z does not match an explicit layer boundary")?;
            if boundary == 0 {
                return Err("deposition at the build bed is unsupported".into());
            }
            self.layer_index = (boundary - 1) as u32;
            let height = schedule.boundaries_mm[boundary] - schedule.boundaries_mm[boundary - 1];
            if self.height.is_some_and(|h| (h - height).abs() > 1e-7) {
                return Err("HEIGHT metadata conflicts with explicit layer schedule".into());
            }
            Some(height)
        } else {
            None
        };
        let h = schedule_height
            .or(self.height)
            .unwrap_or(if self.layer_index == 0 {
                options.first_layer_height_mm
            } else {
                options.layer_height_mm
            });
        if z < h - 1e-8 {
            return Err("estimated bead extends below the build bed; provide correct first-layer height/HEIGHT metadata".into());
        }
        let fraction = repaid / delta_e;
        let start = std::array::from_fn(|i| old[i] + (self.position[i] - old[i]) * fraction);
        let volume = deposited * PI * options.filament_diameter_mm.powi(2) / 4.;
        let deposited_length = length * (1. - fraction);
        let inferred_width = volume / deposited_length / h + h * (1. - PI / 4.);
        let segment = BeadSegment {
            start,
            end: self.position,
            width_mm: self.width.unwrap_or(inferred_width),
            height_mm: h,
            volume_mm3: volume,
            source_line: line,
            feature: self.feature.clone(),
            tool: self.tool,
            layer_index: self.layer_index,
            width_source: if self.width.is_some() {
                "WIDTH metadata"
            } else {
                "extrusion volume and assumed filament diameter"
            }
            .into(),
            height_source: if schedule_height.is_some() {
                "configured explicit layer schedule"
            } else if self.height.is_some() {
                "HEIGHT metadata"
            } else if self.layer_index == 0 {
                "configured first layer"
            } else {
                "configured uniform layer"
            }
            .into(),
        };
        validate_segment(&segment)?;
        if result.segments.len() >= MAX_BEAD_SEGMENTS {
            return Err(
                format!("bead budget exceeded: maximum {MAX_BEAD_SEGMENTS} segments").into(),
            );
        }
        result.segments.push(segment);
        self.deposition_z = Some(z);
        Ok(())
    }
}

fn allowed(parameters: &BTreeMap<char, f64>, accepted: &str) -> Result<()> {
    if let Some(key) = parameters.keys().find(|key| !accepted.contains(**key)) {
        return Err(format!("unsupported parameter {key} for this command").into());
    }
    Ok(())
}

// Deliberately accepts decimal G-code words, not exponent notation, checksums or macros.
// Uppercase E is an extrusion word: G1X10E.4 must not be parsed as scientific notation.
fn tokenize(code: &str) -> Result<Vec<(char, f64)>> {
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut result = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let key = bytes[i];
        if !key.is_ascii_uppercase() {
            return Err(
                "expected uppercase G-code word; exponents/macros/checksums are unsupported".into(),
            );
        }
        i += 1;
        let start = i;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        let mut digits = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            if result.first() == Some(&('G', 28.))
                && matches!(key, b'X' | b'Y' | b'Z')
                && i == start
            {
                result.push((key as char, 0.));
                continue;
            }
            return Err(format!("word {} requires a decimal number", key as char).into());
        }
        let value = code[start..i].parse::<f64>()?;
        if !value.is_finite() {
            return Err("non-finite G-code number".into());
        }
        result.push((key as char, value));
        if result.len() > 32 {
            return Err("too many words on one G-code line".into());
        }
    }
    if result.is_empty() {
        return Err("missing G-code command".into());
    }
    Ok(result)
}
