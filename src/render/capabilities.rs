//! Descriptive capability and discovery results. Constructing these never initializes a GPU.
use super::selection::{BackendId, RenderUsage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "support", rename_all = "snake_case")]
pub enum FeatureSupport {
    Supported,
    Approximate { reason: String },
    Unsupported { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BackendAvailability {
    NotProbed,
    Available,
    Unavailable { reason: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub usages: Vec<RenderUsage>,
    pub alpha: FeatureSupport,
    pub material_volume: FeatureSupport,
    pub sections: FeatureSupport,
    pub layer_appearance: FeatureSupport,
    pub maximum_output_side: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendStatus {
    pub backend: BackendId,
    pub availability: BackendAvailability,
    /// None until the installed implementation has been qualified.
    pub capabilities: Option<CapabilityDescriptor>,
}
