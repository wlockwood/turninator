//! Physical camera timelines shared by native playback and authoritative export.
use crate::{Result, viewport::OrbitCamera};
use serde::{Deserialize, Serialize};

/// Basic turntable timing. Output length is rounded to the nearest 20 fps frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurntableTiming {
    pub seconds_per_turn: f64,
    pub duration_seconds: f64,
}
impl Default for TurntableTiming {
    fn default() -> Self {
        Self {
            seconds_per_turn: 4.,
            duration_seconds: 4.,
        }
    }
}
impl TurntableTiming {
    pub fn validate(&self) -> Result<()> {
        if [self.seconds_per_turn, self.duration_seconds]
            .into_iter()
            .any(|value| !value.is_finite() || !(0.05..=500.).contains(&value))
        {
            return Err(
                "turntable speed and duration must be finite, from 0.05 to 500 seconds".into(),
            );
        }
        Ok(())
    }
    pub fn frame_count(&self) -> Result<u32> {
        self.validate()?;
        Ok((self.duration_seconds * 20.).round() as u32)
    }
    pub fn total_turns(&self) -> Result<f64> {
        Ok(f64::from(self.frame_count()?) / 20. / self.seconds_per_turn)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transition {
    #[default]
    Orbit,
    Spiral,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    #[default]
    Linear,
    SmoothStep,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraKeyframe {
    pub time_seconds: f64,
    pub camera: OrbitCamera,
    /// Applies to the segment leaving this key, including a loop's closing segment.
    pub transition: Transition,
    pub easing: Easing,
    pub spiral_turns: f64,
}
impl Default for CameraKeyframe {
    fn default() -> Self {
        Self {
            time_seconds: 0.,
            camera: OrbitCamera::default(),
            transition: Transition::Orbit,
            easing: Easing::Linear,
            spiral_turns: 0.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Timeline {
    pub duration_seconds: f64,
    pub looping: bool,
    pub keyframes: Vec<CameraKeyframe>,
    pub turntable: bool,
    pub buildup: bool,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            duration_seconds: 4.,
            looping: false,
            keyframes: vec![CameraKeyframe::default()],
            turntable: false,
            buildup: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct AnimationSample {
    pub camera: OrbitCamera,
    pub turntable_degrees: f64,
    /// Cumulative whole-layer visibility, not machine/toolhead time.
    pub buildup_fraction: Option<f64>,
}
impl AnimationSample {
    pub fn worker_state(&self) -> serde_json::Value {
        serde_json::json!({"camera":self.camera.camera_spec(),"angle_degrees":self.turntable_degrees,"buildup_fraction":self.buildup_fraction})
    }
}
fn yaw_delta(a: f64, b: f64) -> f64 {
    (b - a + 180.).rem_euclid(360.) - 180.
}
fn same_camera(a: &OrbitCamera, b: &OrbitCamera) -> bool {
    yaw_delta(a.yaw_degrees, b.yaw_degrees).abs() < 1e-8
        && (a.pitch_degrees - b.pitch_degrees).abs() < 1e-8
        && (a.distance_mm - b.distance_mm).abs() < 1e-8
        && (a.ortho_span_mm - b.ortho_span_mm).abs() < 1e-8
        && (a.vertical_fov_degrees - b.vertical_fov_degrees).abs() < 1e-8
        && a.target_mm
            .iter()
            .zip(b.target_mm)
            .all(|(x, y)| (*x - y).abs() < 1e-8)
        && a.orthographic == b.orthographic
}
impl Timeline {
    pub fn validate(&self) -> Result<()> {
        if !self.duration_seconds.is_finite()
            || !(0.05..=500.).contains(&self.duration_seconds)
            || self.keyframes.is_empty()
            || self.keyframes.len() > 256
        {
            return Err("timeline needs 1..256 keys and a finite 0.05..500 second duration".into());
        }
        let mut previous = -1.;
        for key in &self.keyframes {
            key.camera.validate()?;
            if !key.time_seconds.is_finite()
                || key.time_seconds < 0.
                || key.time_seconds > self.duration_seconds
                || key.time_seconds <= previous
                || !key.spiral_turns.is_finite()
                || key.spiral_turns.abs() > 100.
                || key.spiral_turns.fract() != 0.
                || key.camera.yaw_degrees.abs() > 1e9
            {
                return Err("timeline times must increase within duration; spiral turns must be whole turns from -100 to 100".into());
            }
            if key.camera.orthographic != self.keyframes[0].camera.orthographic {
                return Err(
                    "timeline keys must use one projection; change all keys together".into(),
                );
            }
            previous = key.time_seconds;
        }
        if self.keyframes[0].time_seconds != 0. {
            return Err("first timeline key must be at zero seconds".into());
        }
        let last = self.keyframes.last().unwrap();
        if self.looping
            && last.time_seconds == self.duration_seconds
            && !same_camera(&last.camera, &self.keyframes[0].camera)
        {
            return Err("loop endpoint camera must match the first key, or leave time for a closing transition".into());
        }
        Ok(())
    }
    pub fn frame_count(&self) -> Result<u32> {
        self.validate()?;
        Ok((self.duration_seconds * 20.).round().clamp(1., 10000.) as u32)
    }
    pub fn sample(&self, time_seconds: f64) -> Result<AnimationSample> {
        self.validate()?;
        if !time_seconds.is_finite() {
            return Err("timeline sample time must be finite".into());
        }
        let time = if self.looping {
            time_seconds.rem_euclid(self.duration_seconds)
        } else {
            time_seconds.clamp(0., self.duration_seconds)
        };
        let mut camera = self.keyframes.last().unwrap().camera.clone();
        let pair = self
            .keyframes
            .windows(2)
            .find(|pair| time >= pair[0].time_seconds && time <= pair[1].time_seconds);
        let closing;
        let (start, end, end_time) = if let Some(pair) = pair {
            (&pair[0], &pair[1], pair[1].time_seconds)
        } else if self.looping
            && self.keyframes.last().unwrap().time_seconds < self.duration_seconds
        {
            closing = self.keyframes[0].clone();
            (
                self.keyframes.last().unwrap(),
                &closing,
                self.duration_seconds,
            )
        } else {
            (&self.keyframes[0], &self.keyframes[0], 0.)
        };
        if end_time > start.time_seconds {
            let raw = ((time - start.time_seconds) / (end_time - start.time_seconds)).clamp(0., 1.);
            let f = match start.easing {
                Easing::Linear => raw,
                Easing::SmoothStep => raw * raw * (3. - 2. * raw),
            };
            let lerp = |a: f64, b: f64| a + (b - a) * f;
            let zoom = |a: f64, b: f64| (a.ln() + (b.ln() - a.ln()) * f).exp();
            let turns = if start.transition == Transition::Spiral {
                start.spiral_turns * 360.
            } else {
                0.
            };
            camera = OrbitCamera {
                yaw_degrees: start.camera.yaw_degrees
                    + (yaw_delta(start.camera.yaw_degrees, end.camera.yaw_degrees) + turns) * f,
                pitch_degrees: lerp(start.camera.pitch_degrees, end.camera.pitch_degrees),
                distance_mm: zoom(start.camera.distance_mm, end.camera.distance_mm),
                target_mm: std::array::from_fn(|i| {
                    lerp(start.camera.target_mm[i], end.camera.target_mm[i])
                }),
                orthographic: start.camera.orthographic,
                vertical_fov_degrees: lerp(
                    start.camera.vertical_fov_degrees,
                    end.camera.vertical_fov_degrees,
                ),
                ortho_span_mm: zoom(start.camera.ortho_span_mm, end.camera.ortho_span_mm),
            };
        }
        camera.validate()?;
        Ok(AnimationSample {
            camera,
            turntable_degrees: if self.turntable {
                360. * time / self.duration_seconds
            } else {
                0.
            },
            buildup_fraction: self.buildup.then_some(time / self.duration_seconds),
        })
    }
    pub fn sample_frame(&self, index: u32) -> Result<AnimationSample> {
        let count = self.frame_count()?;
        if index >= count {
            return Err("timeline frame index outside duration".into());
        }
        let progress = if count <= 1 {
            1.
        } else {
            f64::from(index) / f64::from(count - 1)
        };
        let camera_time = if self.looping {
            f64::from(index) / f64::from(count) * self.duration_seconds
        } else {
            progress * self.duration_seconds
        };
        let mut sample = self.sample(camera_time)?;
        // A cumulative build intentionally resets between loops; its last frame is complete.
        sample.buildup_fraction = self.buildup.then_some(progress);
        Ok(sample)
    }
    pub fn worker_frames(&self) -> Result<Vec<serde_json::Value>> {
        (0..self.frame_count()?)
            .map(|index| Ok(self.sample_frame(index)?.worker_state()))
            .collect()
    }
}
