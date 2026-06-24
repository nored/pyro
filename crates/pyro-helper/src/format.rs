//! Erase a whole device and lay down a single fresh filesystem — the "Erase"
//! utility (like Raspberry Pi Imager). exFAT/FAT32 are cross-platform; ext4 is
//! Linux-only. Progress is reported through a `(message, fraction)` callback.

use std::process::Command;

/// Run the erase+format for `device` as `filesystem` ("exfat"|"fat32"|"ext4")
/// with volume `label`. Calls `progress(message, fraction)` as it advances.
pub fn run(
    device: &str,
    filesystem: &str,
    label: &str,
    progress: &mut dyn FnMut(&str, f64),
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        run_linux(device, filesystem, label, progress)
    }
    #[cfg(target_os = "macos")]
    {
        run_macos(device, filesystem, label, progress)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (device, filesystem, label, progress);
        Err("erase/format is not supported on this OS".into())
    }
}

/// Run a command, mapping a missing binary or a non-zero exit to a clear error.
fn run_cmd(bin: &str, args: &[&str], missing_hint: &str) -> Result<(), String> {
    let out = match Command::new(bin).args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("`{bin}` is not installed — {missing_hint}"));
        }
        Err(e) => return Err(format!("could not run {bin}: {e}")),
    };
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{bin} failed: {}", err.trim()))
    }
}

#[cfg(target_os = "linux")]
fn run_linux(
    device: &str,
    filesystem: &str,
    label: &str,
    progress: &mut dyn FnMut(&str, f64),
) -> Result<(), String> {
    progress("unmounting", 0.1);
    unmount_all(device);

    progress("wiping existing signatures", 0.25);
    run_cmd("wipefs", &["-a", device], "install util-linux")?;

    // MBR/msdos table + one primary partition spanning the device. msdos is the
    // most universally readable scheme for exFAT/FAT removable media.
    progress("creating partition", 0.45);
    let part_hint = if filesystem == "ext4" { "ext4" } else { "fat32" };
    run_cmd(
        "parted",
        &["-s", device, "mklabel", "msdos", "mkpart", "primary", part_hint, "1MiB", "100%"],
        "install parted",
    )?;

    // Let the kernel pick up the new partition.
    let _ = Command::new("partprobe").arg(device).status();
    std::thread::sleep(std::time::Duration::from_millis(1000));

    let part = partition_path(device);
    progress(&format!("formatting as {filesystem}"), 0.7);
    match filesystem {
        "exfat" => run_cmd(
            "mkfs.exfat",
            &["-n", &exfat_label(label), &part],
            "install exfatprogs",
        )?,
        "fat32" => run_cmd(
            "mkfs.vfat",
            &["-F", "32", "-n", &fat_label(label), &part],
            "install dosfstools",
        )?,
        "ext4" => run_cmd(
            "mkfs.ext4",
            &["-F", "-L", &ext_label(label), &part],
            "install e2fsprogs",
        )?,
        other => return Err(format!("unsupported filesystem: {other}")),
    }

    let _ = Command::new("sync").status();
    progress("done", 1.0);
    Ok(())
}

/// On Linux a partition suffix is `p1` after a digit-terminated device
/// (mmcblk0 -> mmcblk0p1, nvme0n1 -> nvme0n1p1) else `1` (sdb -> sdb1).
#[cfg(target_os = "linux")]
fn partition_path(device: &str) -> String {
    match device.chars().last() {
        Some(c) if c.is_ascii_digit() => format!("{device}p1"),
        _ => format!("{device}1"),
    }
}

#[cfg(target_os = "linux")]
fn unmount_all(device: &str) {
    if let Ok(out) = Command::new("lsblk")
        .args(["-nro", "PATH,MOUNTPOINT", device])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let path = parts.next().unwrap_or("");
            let mountpoint = parts.next().unwrap_or("");
            if !mountpoint.is_empty() && !path.is_empty() {
                let _ = Command::new("umount").arg(path).status();
            }
        }
    }
}

/// FAT labels are at most 11 chars, uppercased.
#[cfg(target_os = "linux")]
fn fat_label(label: &str) -> String {
    let l = if label.is_empty() { "PYRO" } else { label };
    l.to_uppercase().chars().take(11).collect()
}

/// exFAT labels are at most 15 chars.
#[cfg(target_os = "linux")]
fn exfat_label(label: &str) -> String {
    let l = if label.is_empty() { "PYRO" } else { label };
    l.chars().take(15).collect()
}

/// ext4 labels are at most 16 chars.
#[cfg(target_os = "linux")]
fn ext_label(label: &str) -> String {
    let l = if label.is_empty() { "PYRO" } else { label };
    l.chars().take(16).collect()
}

#[cfg(target_os = "macos")]
fn run_macos(
    device: &str,
    filesystem: &str,
    label: &str,
    progress: &mut dyn FnMut(&str, f64),
) -> Result<(), String> {
    // diskutil eraseDisk lays down the partition table and filesystem in one go.
    let dfs = match filesystem {
        "exfat" => "ExFAT",
        "fat32" => "FAT32",
        "ext4" => return Err("ext4 is not supported on macOS — choose exFAT".into()),
        other => return Err(format!("unsupported filesystem: {other}")),
    };
    let name = mac_label(label);
    progress(&format!("erasing & formatting as {filesystem}"), 0.3);
    run_cmd(
        "diskutil",
        &["eraseDisk", dfs, &name, device],
        "diskutil should be present on macOS",
    )?;
    progress("done", 1.0);
    Ok(())
}

/// diskutil volume names: keep it short and free of spaces/odd chars.
#[cfg(target_os = "macos")]
fn mac_label(label: &str) -> String {
    let l = if label.is_empty() { "PYRO" } else { label };
    let cleaned: String = l
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(15)
        .collect();
    if cleaned.is_empty() {
        "PYRO".into()
    } else {
        cleaned
    }
}
