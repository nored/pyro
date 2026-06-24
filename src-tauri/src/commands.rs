use std::fs;
use std::io::Read;
use std::path::Path;

use crate::drives;
use crate::models::{DriveInfo, ImageInfo};

/// Detect compression from the leading magic bytes.
pub fn detect_compression_bytes(b: &[u8]) -> String {
    match b {
        [0x1f, 0x8b, ..] => "gzip",
        [0xfd, b'7', b'z', b'X', b'Z', 0x00, ..] => "xz",
        [0x28, 0xb5, 0x2f, 0xfd, ..] => "zstd",
        [0x42, 0x5a, 0x68, ..] => "bzip2",
        [0x50, 0x4b, 0x03, 0x04, ..] | [0x50, 0x4b, 0x05, 0x06, ..] => "zip",
        _ => "none",
    }
    .to_string()
}

/// Detect compression by reading a file's magic bytes (extension-independent).
pub fn detect_compression(path: &Path) -> String {
    let mut buf = [0u8; 8];
    let read = fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    detect_compression_bytes(&buf[..read])
}

/// Inspect a remote image (HTTP) without downloading it: read Content-Length and
/// the leading magic bytes, so the UI can show size/format and the flasher can
/// stream it directly to the device.
#[tauri::command]
pub fn inspect_url(url: String) -> Result<ImageInfo, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("URL must start with http:// or https://".into());
    }
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("could not reach URL: {e}"))?;
    let file_size = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let mut head = [0u8; 8];
    let n = resp.into_reader().read(&mut head).unwrap_or(0);
    let compression = detect_compression_bytes(&head[..n]);
    let name = url
        .split('?')
        .next()
        .unwrap_or(&url)
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("image")
        .to_string();
    Ok(ImageInfo {
        path: url,
        name,
        file_size,
        uncompressed_size: if compression == "none" {
            Some(file_size)
        } else {
            None
        },
        compression,
        bmap_path: None,
    })
}

#[tauri::command]
pub fn list_drives() -> Vec<DriveInfo> {
    drives::list()
}

/// Look for a sibling .bmap next to `path` (e.g. foo.img.xz -> foo.img.xz.bmap
/// or foo.img.bmap).
fn find_bmap(path: &Path) -> Option<String> {
    let candidates = [
        format!("{}.bmap", path.to_string_lossy()),
        path.with_extension("bmap").to_string_lossy().to_string(),
    ];
    candidates
        .into_iter()
        .find(|c| Path::new(c).is_file())
}

/// Build image metadata for a path (size + magic-byte compression detection).
fn build_image_info(path: &Path) -> Option<ImageInfo> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let compression = detect_compression(path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_size = meta.len();
    Some(ImageInfo {
        path: path.to_string_lossy().to_string(),
        name,
        file_size,
        uncompressed_size: if compression == "none" {
            Some(file_size)
        } else {
            None
        },
        compression,
        bmap_path: find_bmap(path),
    })
}

#[tauri::command]
pub async fn select_image() -> Option<ImageInfo> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Select an image")
        .add_filter(
            "OS images",
            &["img", "iso", "dmg", "raw", "gz", "xz", "zst", "zstd", "bz2", "zip"],
        )
        .add_filter("All files", &["*"])
        .pick_file()
        .await?;
    build_image_info(file.path())
}

/// Inspect a path provided by drag-and-drop.
#[tauri::command]
pub fn inspect_image(path: String) -> Option<ImageInfo> {
    build_image_info(Path::new(&path))
}

#[tauri::command]
pub fn notify(title: String, body: String) {
    let _ = notify_rust::Notification::new()
        .summary(&title)
        .body(&body)
        .show();
}

#[tauri::command]
pub async fn select_boot_config_files() -> Vec<String> {
    rfd::AsyncFileDialog::new()
        .set_title("Select file(s) to copy onto the boot partition")
        .add_filter("All files", &["*"])
        .pick_files()
        .await
        .map(|files| {
            files
                .iter()
                .map(|f| f.path().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Remove a temp file we created (e.g. a downloaded image), with a safety check
/// that it lives under the OS temp dir.
#[tauri::command]
pub fn forget_temp(path: String) {
    let p = Path::new(&path);
    if p.starts_with(std::env::temp_dir()) {
        let _ = fs::remove_file(p);
    }
}

#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";

    std::process::Command::new(program)
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
