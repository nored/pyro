//! Unprivileged file operations for the boot-partition editor. The helper
//! mounts the FAT partition owned by this user (Linux uid mount / macOS
//! diskutil), so these run without elevation. Every path is restricted to the
//! `/tmp/pyro-boot*` mountpoint as a safety guard.

use std::fs;
use std::path::Path;

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

fn guard(p: &str) -> Result<(), String> {
    if p.starts_with("/tmp/pyro-boot") && !p.contains("..") {
        Ok(())
    } else {
        Err("path is outside the boot partition".into())
    }
}

#[tauri::command]
pub fn boot_list(dir: String) -> Result<Vec<BootEntry>, String> {
    guard(&dir)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // hide dotfiles / volume metadata
        }
        entries.push(BootEntry {
            name,
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| (b.is_dir, a.name.to_lowercase()).cmp(&(a.is_dir, b.name.to_lowercase())));
    Ok(entries)
}

#[tauri::command]
pub fn boot_read_text(path: String) -> Result<String, String> {
    guard(&path)?;
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8 text".into())
}

#[tauri::command]
pub fn boot_write_text(path: String, content: String) -> Result<(), String> {
    guard(&path)?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn boot_rename(from: String, to: String) -> Result<(), String> {
    guard(&from)?;
    guard(&to)?;
    fs::rename(&from, &to).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn boot_delete(path: String) -> Result<(), String> {
    guard(&path)?;
    let p = Path::new(&path);
    if p.is_dir() {
        fs::remove_dir_all(p).map_err(|e| e.to_string())
    } else {
        fs::remove_file(p).map_err(|e| e.to_string())
    }
}

/// Copy external files (e.g. drag-and-dropped) into the boot partition.
#[tauri::command]
pub fn boot_add(dir: String, sources: Vec<String>) -> Result<(), String> {
    guard(&dir)?;
    for src in &sources {
        let name = Path::new(src)
            .file_name()
            .ok_or("invalid source file name")?;
        let dest = Path::new(&dir).join(name);
        fs::copy(src, &dest).map_err(|e| e.to_string())?;
    }
    Ok(())
}
