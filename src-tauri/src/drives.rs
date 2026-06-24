//! Enumerate storage devices that may be flash targets.
//!
//! Linux uses `lsblk --json`; macOS uses `diskutil`. Each whole disk is mapped
//! to a `DriveInfo`, with safety flags so the UI can hide/guard system disks.

use crate::models::DriveInfo;

#[cfg(target_os = "linux")]
pub fn list() -> Vec<DriveInfo> {
    linux::list()
}

#[cfg(target_os = "macos")]
pub fn list() -> Vec<DriveInfo> {
    macos::list()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn list() -> Vec<DriveInfo> {
    Vec::new()
}

/// Mountpoints that, if present on a disk, mark it as a system disk we must
/// never offer as a casual flash target.
fn is_system_mountpoint(mp: &str) -> bool {
    matches!(
        mp,
        "/" | "/boot"
            | "/boot/efi"
            | "/efi"
            | "/home"
            | "/var"
            | "/usr"
            | "/nix"
            | "/recovery"
    ) || mp.starts_with("/boot")
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{is_system_mountpoint, DriveInfo};
    use serde_json::Value;
    use std::process::Command;

    fn as_u64(v: &Value) -> u64 {
        match v {
            Value::Number(n) => n.as_u64().unwrap_or(0),
            Value::String(s) => s.parse().unwrap_or(0),
            _ => 0,
        }
    }

    fn as_bool(v: &Value) -> bool {
        match v {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_u64().unwrap_or(0) != 0,
            Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
            _ => false,
        }
    }

    fn str_field<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
        node.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
    }

    /// Collect every mountpoint on a node and its descendants.
    fn collect_mountpoints(node: &Value, out: &mut Vec<String>) {
        // newer lsblk: "mountpoints" array; older: "mountpoint" string
        if let Some(arr) = node.get("mountpoints").and_then(|v| v.as_array()) {
            for m in arr {
                if let Some(s) = m.as_str() {
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                }
            }
        }
        if let Some(s) = str_field(node, "mountpoint") {
            out.push(s.to_string());
        }
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for c in children {
                collect_mountpoints(c, out);
            }
        }
    }

    pub fn list() -> Vec<DriveInfo> {
        let output = Command::new("lsblk")
            .args([
                "--json",
                "--bytes",
                "-o",
                "NAME,PATH,SIZE,TYPE,RM,HOTPLUG,RO,MODEL,VENDOR,TRAN,MOUNTPOINT,MOUNTPOINTS",
            ])
            .output();

        let output = match output {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };

        let root: Value = match serde_json::from_slice(&output.stdout) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let devices = root
            .get("blockdevices")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut drives = Vec::new();
        for dev in &devices {
            if str_field(dev, "type") != Some("disk") {
                continue;
            }
            let path = str_field(dev, "path")
                .map(str::to_string)
                .or_else(|| str_field(dev, "name").map(|n| format!("/dev/{n}")))
                .unwrap_or_default();
            if path.is_empty() {
                continue;
            }

            let vendor = str_field(dev, "vendor").unwrap_or("").trim();
            let model = str_field(dev, "model").unwrap_or("").trim();
            let description = match (vendor, model) {
                ("", "") => path.clone(),
                ("", m) => m.to_string(),
                (v, "") => v.to_string(),
                (v, m) => format!("{v} {m}"),
            };

            let removable = dev.get("rm").map(as_bool).unwrap_or(false)
                || dev.get("hotplug").map(as_bool).unwrap_or(false);
            let bus_type = str_field(dev, "tran").map(str::to_string);

            let mut mountpoints = Vec::new();
            collect_mountpoints(dev, &mut mountpoints);
            mountpoints.sort();
            mountpoints.dedup();

            let has_system_mount = mountpoints.iter().any(|m| is_system_mountpoint(m));
            // Treat internal (non-removable) disks as system to keep them safe.
            let is_system = has_system_mount || !removable;

            drives.push(DriveInfo {
                device: path,
                description,
                size: dev.get("size").map(as_u64).unwrap_or(0),
                is_removable: removable,
                is_system,
                is_mounted: !mountpoints.is_empty(),
                bus_type,
                mountpoints,
            });
        }
        drives
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{is_system_mountpoint, DriveInfo};
    use plist::Value;
    use std::process::Command;

    fn diskutil_plist(args: &[&str]) -> Option<Value> {
        let out = Command::new("diskutil").args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Value::from_reader_xml(std::io::Cursor::new(out.stdout)).ok()
    }

    fn as_str(v: &Value) -> Option<&str> {
        v.as_string()
    }

    pub fn list() -> Vec<DriveInfo> {
        // Whole physical disks and their partitions (with mountpoints).
        let listing = match diskutil_plist(&["list", "-plist", "physical"]) {
            Some(v) => v,
            None => return Vec::new(),
        };
        let disks = listing
            .as_dictionary()
            .and_then(|d| d.get("AllDisksAndPartitions"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut drives = Vec::new();
        for disk in &disks {
            let dd = match disk.as_dictionary() {
                Some(d) => d,
                None => continue,
            };
            let id = match dd.get("DeviceIdentifier").and_then(as_str) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let device = format!("/dev/{id}");

            // Mountpoints from this disk's partitions.
            let mut mountpoints = Vec::new();
            if let Some(parts) = dd.get("Partitions").and_then(|v| v.as_array()) {
                for p in parts {
                    if let Some(mp) = p
                        .as_dictionary()
                        .and_then(|pd| pd.get("MountPoint"))
                        .and_then(as_str)
                    {
                        if !mp.is_empty() {
                            mountpoints.push(mp.to_string());
                        }
                    }
                }
            }

            // Per-disk details.
            let info = diskutil_plist(&["info", "-plist", device.as_str()]);
            let info = info.as_ref().and_then(|v| v.as_dictionary());
            let get_bool = |k: &str| {
                info.and_then(|d| d.get(k))
                    .and_then(|v| v.as_boolean())
                    .unwrap_or(false)
            };
            let size = info
                .and_then(|d| d.get("Size").or_else(|| d.get("TotalSize")))
                .and_then(|v| v.as_unsigned_integer())
                .unwrap_or(0);
            let description = info
                .and_then(|d| {
                    d.get("MediaName")
                        .or_else(|| d.get("IORegistryEntryName"))
                })
                .and_then(as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(id.as_str())
                .to_string();
            let bus_type = info
                .and_then(|d| d.get("BusProtocol"))
                .and_then(as_str)
                .map(|s| s.to_string());

            let internal = get_bool("Internal");
            let removable =
                get_bool("Ejectable") || get_bool("RemovableMedia") || !internal;
            let has_system_mount = mountpoints.iter().any(|m| is_system_mountpoint(m));
            let is_system = internal || has_system_mount;

            drives.push(DriveInfo {
                device,
                description,
                size,
                is_removable: removable,
                is_system,
                is_mounted: !mountpoints.is_empty(),
                bus_type,
                mountpoints,
            });
        }
        drives
    }
}
