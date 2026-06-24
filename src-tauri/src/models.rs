use serde::{Deserialize, Serialize};

/// A storage device that may be a flash target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub device: String,
    pub description: String,
    pub size: u64,
    pub is_removable: bool,
    pub is_system: bool,
    pub is_mounted: bool,
    pub bus_type: Option<String>,
    pub mountpoints: Vec<String>,
}

/// A source image selected by the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub path: String,
    pub name: String,
    pub file_size: u64,
    pub uncompressed_size: Option<u64>,
    /// "none" | "gzip" | "xz" | "zstd" | "bzip2" | "zip"
    pub compression: String,
    /// Path to an auto-detected sibling .bmap file, if any.
    #[serde(default)]
    pub bmap_path: Option<String>,
}

/// A request to flash an image to one or more devices.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashRequest {
    pub image: ImageInfo,
    pub devices: Vec<String>,
    pub validate: bool,
    #[serde(default)]
    pub boot_config_files: Vec<String>,
    /// Keep the boot partition mounted for in-app editing before eject.
    #[serde(default)]
    pub edit_boot: bool,
}

// Progress is forwarded to the UI as raw JSON tailed from the helper's progress
// file (see flash.rs / pyro-helper), so no Rust progress struct is needed here.

/// Outcome of flashing one device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashResult {
    pub ok: bool,
    pub device: String,
    pub bytes_written: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
