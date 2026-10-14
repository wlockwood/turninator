use super::{
    beads::{MAX_BEAD_SEGMENTS, validate_segment},
    schedule::{MAX_LAYERS, validate_height},
};
use crate::{Result, print::beads::BeadSegment};
use std::{collections::BTreeMap, f64::consts::PI};

#[derive(Clone, Debug)]
pub struct ParseOptions {
    pub layer_height_mm: f64,
    pub first_layer_height_mm: f64,
    pub filament_diameter_mm: f64,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            layer_height_mm: 0.2,
            first_layer_height_mm: 0.2,
            filament_diameter_mm: 1.75,
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
            "Estimated stadium beads, not physical deposition prediction. Assumed filament diameter {} mm; default first/later heights {}/{} mm when HEIGHT metadata is absent. Initial position/counters are zero and positioning is absolute; supported single-tool Marlin subset only. Independent capped segments overlap at corners.",
            options.filament_diameter_mm, options.first_layer_height_mm, options.layer_height_mm
        )],
    };
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
        }
    }
}

impl State {
    fn metadata(&mut self, comment: &str) -> Result<()> {
        let Some((key, value)) = comment.trim().split_once(':') else {
            return Ok(());
        };
        match key.trim() {
            "HEIGHT" => {
                let h = value.trim().parse::<f64>()?;
                validate_height(h)?;
                self.height = Some(h);
            }
            "WIDTH" => {
                let w = value.trim().parse::<f64>()?;
                if !w.is_finite() || w <= 0. || w > 10. {
                    return Err("invalid WIDTH metadata in millimeters".into());
                }
                self.width = Some(w);
            }
            "TYPE" => self.feature = value.trim().to_string(),
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
            ('G',0|1)=> { allowed(p,"XYZEF")?; self.motion(p,line,options,result)?; }
            ('G',20|21)=> { allowed(p,"")?; self.unit=if number==20 {25.4} else {1.}; }
            ('G',90|91)=> { allowed(p,"")?;self.xyz_absolute=number==90;self.e_override=None; }
            ('M',82|83)=> { allowed(p,"")?;self.e_override=Some(number==82); }
            ('G',92)=> {
                allowed(p,"XYZE")?;
                if p.is_empty() { return Err("G92 without explicit coordinates is unsupported".into()); }
                for (i,axis) in ['X','Y','Z'].into_iter().enumerate() {
                    if let Some(value)=p.get(&axis) { self.offsets[i]=self.position[i]-value*self.unit; }
                }
                if let Some(value)=p.get(&'E') { self.e_position=value*self.unit; }
            }
            ('T',0)=> { allowed(p,"")?; }
            ('M',104|109)=> { allowed(p,"SRT")?; if p.get(&'T').is_some_and(|&t|t!=0.) { return Err("only tool 0 is supported".into()); } }
            ('M',140|190)=>allowed(p,"SR")?,
            ('M',106)=>allowed(p,"SP")?,
            ('M',107)=>allowed(p,"P")?,
            ('M',105|114|115|400)=>allowed(p,"")?,
            ('G',4)=>allowed(p,"PS")?,
            _=>return Err(format!("unsupported command {family}{number}; geometry-affecting modes, homing, arcs, volumetric extrusion, tools and macros are not guessed").into()),
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

    fn motion(
        &mut self,
        p: &BTreeMap<char, f64>,
        line: usize,
        options: &ParseOptions,
        result: &mut ToolpathData,
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
            if self.e_override.unwrap_or(self.xyz_absolute) {
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
        let length = (self.position[0] - old[0]).hypot(self.position[1] - old[1]);
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
        let h = self.height.unwrap_or(if self.layer_index == 0 {
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
            tool: 0,
            layer_index: self.layer_index,
            width_source: if self.width.is_some() {
                "WIDTH metadata"
            } else {
                "extrusion volume and assumed filament diameter"
            }
            .into(),
            height_source: if self.height.is_some() {
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
