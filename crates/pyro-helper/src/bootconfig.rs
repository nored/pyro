//! Mount the freshly written boot (FAT) partition, copy files onto it, and
//! (optionally) keep it mounted so the user can edit files before eject.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// A mounted boot partition. Call [`unmount`] when done.
pub struct Mount {
    pub dir: String,
    // Read on macOS (diskutil unmount <partition>); unused on Linux.
    #[allow(dead_code)]
    pub partition: String,
}

/// A mountable partition on the freshly written device, offered to the user so
/// they can pick which one to open (rather than us guessing by name).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionInfo {
    /// Device path, e.g. /dev/sdb1 or /dev/disk4s1.
    pub path: String,
    /// Filesystem label, if any.
    pub label: Option<String>,
    /// Filesystem type, e.g. vfat, ext4.
    pub fstype: String,
    /// Size in bytes.
    pub size: u64,
}

/// Enumerate the mountable partitions of `device` (those with a filesystem).
pub fn list_partitions(device: &str) -> Vec<PartitionInfo> {
    #[cfg(target_os = "linux")]
    {
        list_partitions_linux(device)
    }
    #[cfg(target_os = "macos")]
    {
        list_partitions_macos(device)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = device;
        Vec::new()
    }
}

/// Mount a specific partition `path` (chosen by the user). On Linux a FAT volume
/// is mounted owned by `uid` so the unprivileged GUI can edit it.
pub fn mount_partition(path: &str, fstype: &str, uid: Option<u32>) -> Result<Mount, String> {
    #[cfg(target_os = "linux")]
    {
        mount_partition_linux(path, fstype, uid)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (fstype, uid);
        mount_partition_macos(path)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, fstype, uid);
        Err("partition mounting is not supported on this OS".into())
    }
}

/// Mount the boot partition of `device`. On Linux, if `uid` is given the FAT
/// volume is mounted owned by that user so the unprivileged GUI can edit it.
pub fn mount_boot(device: &str, uid: Option<u32>) -> Result<Mount, String> {
    #[cfg(target_os = "linux")]
    {
        mount_linux(device, uid)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = uid;
        mount_macos(device)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (device, uid);
        Err("boot-partition mounting is not supported on this OS".into())
    }
}

/// Copy each file to the root of the mounted partition (overwriting same names).
pub fn copy_files(mount: &Mount, files: &[String]) -> Result<(), String> {
    for f in files {
        if !Path::new(f).exists() {
            return Err(format!("config file not found: {f}"));
        }
        let name = Path::new(f)
            .file_name()
            .ok_or("invalid config file name")?;
        let dest = Path::new(&mount.dir).join(name);
        std::fs::copy(f, &dest).map_err(|e| e.to_string())?;
    }
    let _ = Command::new("sync").status();
    Ok(())
}

/// Unmount and clean up (best effort).
pub fn unmount(mount: Mount) {
    let _ = Command::new("sync").status();
    #[cfg(target_os = "linux")]
    let _ = Command::new("umount").arg(&mount.dir).status();
    #[cfg(target_os = "macos")]
    let _ = Command::new("diskutil")
        .args(["unmount", &mount.partition])
        .status();
    let _ = std::fs::remove_dir(&mount.dir);
}

#[cfg(target_os = "linux")]
fn mount_linux(device: &str, uid: Option<u32>) -> Result<Mount, String> {
    let _ = Command::new("partprobe").arg(device).status();
    std::thread::sleep(std::time::Duration::from_millis(800));

    let partition = find_fat_partition_linux(device)?;
    let dir = format!("/tmp/pyro-boot-{}", std::process::id());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut cmd = Command::new("mount");
    if let Some(uid) = uid {
        // Make the FAT volume owned by the GUI user so it can edit files.
        cmd.arg("-o").arg(format!("uid={uid},gid={uid}"));
    }
    cmd.arg(&partition).arg(&dir);
    let ok = cmd.status().map_err(|e| e.to_string())?.success();
    if !ok {
        let _ = std::fs::remove_dir(&dir);
        return Err(format!("failed to mount {partition}"));
    }
    Ok(Mount { dir, partition })
}

#[cfg(target_os = "linux")]
fn list_partitions_linux(device: &str) -> Vec<PartitionInfo> {
    // Re-read the partition table that we just wrote, then enumerate.
    let _ = Command::new("partprobe").arg(device).status();
    std::thread::sleep(std::time::Duration::from_millis(800));

    let out = match Command::new("lsblk")
        .args(["-bJ", "-o", "PATH,LABEL,FSTYPE,SIZE,TYPE", device])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    fn collect(node: &serde_json::Value, out: &mut Vec<PartitionInfo>) {
        if node.get("type").and_then(|t| t.as_str()) == Some("part") {
            let path = node.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let fstype = node
                .get("fstype")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            // Only offer partitions that carry a mountable filesystem.
            if !path.is_empty() && !fstype.is_empty() {
                let label = node
                    .get("label")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                out.push(PartitionInfo {
                    path: path.to_string(),
                    label,
                    fstype,
                    size: node.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                });
            }
        }
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            for c in children {
                collect(c, out);
            }
        }
    }

    let mut parts = Vec::new();
    if let Some(devs) = v.get("blockdevices").and_then(|b| b.as_array()) {
        for d in devs {
            collect(d, &mut parts);
        }
    }
    parts
}

#[cfg(target_os = "linux")]
fn mount_partition_linux(path: &str, fstype: &str, uid: Option<u32>) -> Result<Mount, String> {
    let dir = format!("/tmp/pyro-boot-{}", std::process::id());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let fat = ["vfat", "fat", "fat12", "fat16", "fat32", "msdos"];
    let mut cmd = Command::new("mount");
    if let Some(uid) = uid {
        // uid=/gid= only applies to FAT; other filesystems carry their own
        // ownership, so we mount them as-is.
        if fat.contains(&fstype.to_lowercase().as_str()) {
            cmd.arg("-o").arg(format!("uid={uid},gid={uid}"));
        }
    }
    cmd.arg(path).arg(&dir);
    let ok = cmd.status().map_err(|e| e.to_string())?.success();
    if !ok {
        let _ = std::fs::remove_dir(&dir);
        return Err(format!("failed to mount {path}"));
    }
    Ok(Mount {
        dir,
        partition: path.to_string(),
    })
}

#[cfg(target_os = "linux")]
fn find_fat_partition_linux(device: &str) -> Result<String, String> {
    let out = Command::new("lsblk")
        .args(["-nro", "PATH,FSTYPE", device])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let fat = ["vfat", "fat", "fat12", "fat16", "fat32", "msdos"];
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let path = parts.next().unwrap_or("");
        let fstype = parts.next().unwrap_or("");
        if path != device
            && !path.is_empty()
            && fat.contains(&fstype.to_lowercase().as_str())
        {
            return Ok(path.to_string());
        }
    }
    Err(format!("no FAT/boot partition found on {device}"))
}

#[cfg(target_os = "macos")]
fn mount_macos(device: &str) -> Result<Mount, String> {
    let partition = find_fat_partition_macos(device)?;
    let dir = format!("/tmp/pyro-boot-{}", std::process::id());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ok = Command::new("diskutil")
        .args(["mount", "-mountPoint", &dir, &partition])
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        let _ = std::fs::remove_dir(&dir);
        return Err(format!("failed to mount {partition}"));
    }
    Ok(Mount { dir, partition })
}

#[cfg(target_os = "macos")]
fn list_partitions_macos(device: &str) -> Vec<PartitionInfo> {
    let out = match Command::new("diskutil").args(["list", device]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = Vec::new();
    for line in text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let id = match toks.last() {
            Some(t) => *t,
            None => continue,
        };
        // Partition identifiers look like disk4s1; whole disks are disk4.
        if !(id.starts_with("disk") && id.contains('s')) {
            continue;
        }
        let fstype = toks.get(1).copied().unwrap_or("").to_string();
        parts.push(PartitionInfo {
            path: format!("/dev/{id}"),
            label: None,
            fstype,
            size: parse_size_macos(&toks),
        });
    }
    parts
}

#[cfg(target_os = "macos")]
fn parse_size_macos(toks: &[&str]) -> u64 {
    for i in 1..toks.len() {
        let mult: u64 = match toks[i] {
            "B" => 1,
            "KB" => 1_000,
            "MB" => 1_000_000,
            "GB" => 1_000_000_000,
            "TB" => 1_000_000_000_000,
            _ => continue,
        };
        let num = toks[i - 1].trim_start_matches('*');
        if let Ok(v) = num.parse::<f64>() {
            return (v * mult as f64) as u64;
        }
    }
    0
}

#[cfg(target_os = "macos")]
fn mount_partition_macos(path: &str) -> Result<Mount, String> {
    let dir = format!("/tmp/pyro-boot-{}", std::process::id());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ok = Command::new("diskutil")
        .args(["mount", "-mountPoint", &dir, path])
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        let _ = std::fs::remove_dir(&dir);
        return Err(format!("failed to mount {path}"));
    }
    Ok(Mount {
        dir,
        partition: path.to_string(),
    })
}

#[cfg(target_os = "macos")]
fn find_fat_partition_macos(device: &str) -> Result<String, String> {
    let out = Command::new("diskutil")
        .args(["list", device])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let markers = [
        "Windows_FAT_32",
        "Windows_FAT_16",
        "DOS_FAT_32",
        "DOS_FAT_16",
        "Microsoft Basic Data",
        "EFI",
    ];
    for line in text.lines() {
        if markers.iter().any(|m| line.contains(m)) {
            if let Some(id) = line.split_whitespace().last() {
                if id.starts_with("disk") {
                    return Ok(format!("/dev/{id}"));
                }
            }
        }
    }
    Err(format!("no FAT/boot partition found on {device}"))
}
