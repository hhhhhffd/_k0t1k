use serde::{Deserialize, Serialize};

/// One component of an instance (e.g. `net.minecraft` 1.21.4, `net.fabricmc.fabric-loader` 0.16.9),
/// identified by its Prism meta uid. The list order of an instance's components is the apply order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentSpec {
    pub uid: String,
    pub version: String,
    /// Added automatically to satisfy another component's `requires` (LWJGL, intermediary).
    #[serde(default)]
    pub dependency_only: bool,
}

pub const MINECRAFT_UID: &str = "net.minecraft";
