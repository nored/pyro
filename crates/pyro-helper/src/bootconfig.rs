//! Mount the freshly written boot (FAT) partition, copy files onto it, and
//! (optionally) keep it mounted so the user can edit files before eject.

use std::path::Path;
use std::process::Command;

/// A mounted boot partition. Call [`unmount`] when done.
pub struct Mount {
    pub dir: String,
    // Read on macOS (diskutil unmount <partition>); unused on Linux.
    #[allow(dead_code)]
    pub partition: String,
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
