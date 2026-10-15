//! Stable renderer intent, independent of runtime discovery or device availability.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendId {
    Wgpu,
    Filament,
    #[serde(rename = "luxcore")]
    LuxCore,
    BlenderEevee,
    BlenderCycles,
}
impl BackendId {
    pub const ALL: [Self; 5] = [
        Self::Wgpu,
        Self::Filament,
        Self::LuxCore,
        Self::BlenderEevee,
        Self::BlenderCycles,
    ];
    pub fn parse_legacy(value: &str) -> Result<Self> {
        Ok(match value {
            "eevee" | "blender_eevee" => Self::BlenderEevee,
            "cycles" | "blender_cycles" => Self::BlenderCycles,
            "wgpu" => Self::Wgpu,
            "filament" => Self::Filament,
            "luxcore" => Self::LuxCore,
            _ => return Err(format!("unknown renderer ID: {value}").into()),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wgpu => "wgpu",
            Self::Filament => "filament",
            Self::LuxCore => "luxcore",
            Self::BlenderEevee => "blender_eevee",
            Self::BlenderCycles => "blender_cycles",
        }
    }
    pub fn legacy_blender_engine(self) -> Result<&'static str> {
        match self {
            Self::BlenderEevee => Ok("eevee"),
            Self::BlenderCycles => Ok("cycles"),
            _ => Err(format!("{} renderer is not integrated yet", self.as_str()).into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderUsage {
    Interactive,
    Preview,
    Animation,
    Still,
    Sheet,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AaQuality {
    Low,
    Medium,
    High,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowQuality {
    Low,
    Medium,
    High,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BackendQuality {
    Wgpu {
        aa: AaQuality,
        shadows: ShadowQuality,
    },
    Filament {
        aa: AaQuality,
        shadows: ShadowQuality,
    },
    #[serde(rename = "luxcore")]
    LuxCore {
        samples: u32,
        denoise: bool,
    },
    Eevee {
        samples: u32,
    },
    Cycles {
        samples: u32,
        denoise: bool,
    },
}
impl BackendQuality {
    pub fn backend(&self) -> BackendId {
        match self {
            Self::Wgpu { .. } => BackendId::Wgpu,
            Self::Filament { .. } => BackendId::Filament,
            Self::LuxCore { .. } => BackendId::LuxCore,
            Self::Eevee { .. } => BackendId::BlenderEevee,
            Self::Cycles { .. } => BackendId::BlenderCycles,
        }
    }
    pub fn samples(&self) -> Option<u32> {
        match self {
            Self::Eevee { samples }
            | Self::Cycles { samples, .. }
            | Self::LuxCore { samples, .. } => Some(*samples),
            _ => None,
        }
    }
    pub fn samples_mut(&mut self) -> Option<&mut u32> {
        match self {
            Self::Eevee { samples }
            | Self::Cycles { samples, .. }
            | Self::LuxCore { samples, .. } => Some(samples),
            _ => None,
        }
    }
    pub fn default_for(backend: BackendId, still: bool) -> Self {
        let samples = if still { 64 } else { 32 };
        match backend {
            BackendId::Wgpu => Self::Wgpu {
                aa: AaQuality::High,
                shadows: ShadowQuality::High,
            },
            BackendId::Filament => Self::Filament {
                aa: AaQuality::High,
                shadows: ShadowQuality::High,
            },
            BackendId::LuxCore => Self::LuxCore {
                samples,
                denoise: true,
            },
            BackendId::BlenderEevee => Self::Eevee { samples },
            BackendId::BlenderCycles => Self::Cycles {
                samples,
                denoise: true,
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SelectionWire")]
pub struct RenderSelection {
    pub backend: BackendId,
    pub device: Option<String>,
    pub quality: BackendQuality,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionWire {
    backend: BackendId,
    #[serde(default)]
    device: Option<String>,
    quality: BackendQuality,
}
impl TryFrom<SelectionWire> for RenderSelection {
    type Error = String;
    fn try_from(w: SelectionWire) -> std::result::Result<Self, String> {
        let s = Self {
            backend: w.backend,
            device: w.device,
            quality: w.quality,
        };
        s.validate().map_err(|e| e.to_string())?;
        Ok(s)
    }
}
impl Default for RenderSelection {
    fn default() -> Self {
        Self::for_backend(BackendId::BlenderEevee, false)
    }
}
impl RenderSelection {
    pub fn for_backend(backend: BackendId, still: bool) -> Self {
        Self {
            backend,
            device: None,
            quality: BackendQuality::default_for(backend, still),
        }
    }
    pub fn from_legacy(engine: &str, samples: u32) -> Result<Self> {
        let backend = BackendId::parse_legacy(engine)?;
        if !matches!(backend, BackendId::BlenderEevee | BackendId::BlenderCycles) {
            return Err("legacy requests require a Blender renderer".into());
        }
        let quality = if backend == BackendId::BlenderEevee {
            BackendQuality::Eevee { samples }
        } else {
            BackendQuality::Cycles {
                samples,
                denoise: true,
            }
        };
        let selection = Self {
            backend,
            device: None,
            quality,
        };
        selection.validate()?;
        Ok(selection)
    }
    pub fn validate(&self) -> Result<()> {
        if self.backend != self.quality.backend() {
            return Err("renderer and quality variant do not match".into());
        }
        if self
            .quality
            .samples()
            .is_some_and(|n| !(1..=4096).contains(&n))
        {
            return Err("renderer samples must be 1 to 4096".into());
        }
        if self
            .device
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
        {
            return Err("invalid renderer device ID".into());
        }
        Ok(())
    }
    /// Only for the existing Blender boundary; never translates raster quality into samples.
    pub fn blender_samples(&self) -> Result<u32> {
        self.validate()?;
        self.backend.legacy_blender_engine()?;
        self.quality
            .samples()
            .ok_or_else(|| "Blender samples required".into())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewportRenderer {
    #[default]
    Wgpu,
    Filament,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityProfiles {
    pub animation: BackendQuality,
    pub still: BackendQuality,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererPreferences {
    #[serde(default)]
    pub viewport: ViewportRenderer,
    pub animation: RenderSelection,
    pub still: RenderSelection,
    /// Inactive profiles; when switching, the outgoing active quality is saved here.
    #[serde(default)]
    pub profiles: BTreeMap<BackendId, QualityProfiles>,
}
impl Default for RendererPreferences {
    fn default() -> Self {
        Self {
            viewport: ViewportRenderer::Wgpu,
            animation: RenderSelection::default(),
            still: RenderSelection::for_backend(BackendId::BlenderCycles, true),
            profiles: BackendId::ALL
                .into_iter()
                .map(|b| {
                    (
                        b,
                        QualityProfiles {
                            animation: BackendQuality::default_for(b, false),
                            still: BackendQuality::default_for(b, true),
                        },
                    )
                })
                .collect(),
        }
    }
}
impl RendererPreferences {
    pub fn validate(&self) -> Result<()> {
        self.animation.validate()?;
        self.still.validate()?;
        for (backend, profiles) in &self.profiles {
            for q in [&profiles.animation, &profiles.still] {
                RenderSelection {
                    backend: *backend,
                    device: None,
                    quality: q.clone(),
                }
                .validate()?;
            }
        }
        Ok(())
    }
    pub fn select_backend(&mut self, still: bool, backend: BackendId) {
        let active = if still {
            &mut self.still
        } else {
            &mut self.animation
        };
        if active.backend == backend {
            return;
        }
        let outgoing = self
            .profiles
            .entry(active.backend)
            .or_insert_with(|| QualityProfiles {
                animation: BackendQuality::default_for(active.backend, false),
                still: BackendQuality::default_for(active.backend, true),
            });
        if still {
            outgoing.still = active.quality.clone();
        } else {
            outgoing.animation = active.quality.clone();
        }
        let quality = self
            .profiles
            .get(&backend)
            .map(|p| {
                if still {
                    p.still.clone()
                } else {
                    p.animation.clone()
                }
            })
            .unwrap_or_else(|| BackendQuality::default_for(backend, still));
        *active = RenderSelection {
            backend,
            device: None,
            quality,
        };
    }
}
