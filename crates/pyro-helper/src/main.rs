//! Pyro privileged flashing helper.
//!
//! Usage: `pyro-helper <job.json> <progress.jsonl> <result.json>`
//!
//! Reads a flash job, writes (and optionally verifies) the image to each target
//! device, optionally drops a config file on the boot partition, streams
//! newline-delimited JSON progress to the progress file, and writes the final
//! per-device results to the result file. Exit code 0 iff every device succeeded.

mod bmap;
mod bootconfig;
mod source;

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CHUNK: usize = 4 * 1024 * 1024;

/// Optional path to a cancellation flag file; if it appears, abort.
static CANCEL_FILE: OnceLock<Option<String>> = OnceLock::new();
/// Optional path to an "editing finished" flag file (boot-file editor).
static EDIT_DONE_FILE: OnceLock<Option<String>> = OnceLock::new();

fn flag_exists(cell: &OnceLock<Option<String>>) -> bool {
    cell.get()
        .and_then(|o| o.as_ref())
        .map(|p| Path::new(p).exists())
        .unwrap_or(false)
}

fn is_cancelled() -> bool {
    flag_exists(&CANCEL_FILE)
}

fn edit_done() -> bool {
    flag_exists(&EDIT_DONE_FILE)
}

/// The uid of the user who invoked us via pkexec/sudo, for user-owned mounts.
fn invoking_uid() -> Option<u32> {
    std::env::var("PKEXEC_UID")
        .or_else(|_| std::env::var("SUDO_UID"))
        .ok()
        .and_then(|s| s.parse().ok())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    image_path: String,
    compression: String,
    file_size: u64,
    devices: Vec<String>,
    validate: bool,
    #[serde(default)]
    boot_config_files: Vec<String>,
    /// Keep the boot partition mounted for in-app editing before eject.
    #[serde(default)]
    edit_boot: bool,
    /// Optional path to a .bmap file to skip blank regions when writing.
    #[serde(default)]
    bmap_path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    phase: String,
    fraction: f64,
    bytes: u64,
    total_bytes: Option<u64>,
    speed: f64,
    eta: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceResult {
    ok: bool,
    device: String,
    bytes_written: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Appends newline-delimited JSON progress events to the progress file.
struct Emitter {
    file: File,
}

impl Emitter {
    fn open(path: &str) -> io::Result<Self> {
        Ok(Self {
            file: OpenOptions::new().create(true).append(true).open(path)?,
        })
    }
    fn emit(&mut self, p: &Progress) {
        if let Ok(line) = serde_json::to_string(p) {
            let _ = writeln!(self.file, "{line}");
            let _ = self.file.flush();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: pyro-helper <job.json> <progress.jsonl> <result.json>");
        std::process::exit(2);
    }
    let job_path = &args[1];
    let progress_path = &args[2];
    let result_path = &args[3];
    // Optional 4th arg: a cancellation flag file watched during the flash.
    CANCEL_FILE.set(args.get(4).cloned()).ok();
    // Optional 5th arg: an "editing done" flag file for the boot-file editor.
    EDIT_DONE_FILE.set(args.get(5).cloned()).ok();

    let job: Job = match std::fs::read_to_string(job_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(j) => j,
        None => {
            eprintln!("could not read job file: {job_path}");
            std::process::exit(2);
        }
    };

    let mut emitter = match Emitter::open(progress_path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("could not open progress file: {e}");
            std::process::exit(2);
        }
    };

    let mut results = Vec::new();
    let mut all_ok = true;
    for device in &job.devices {
        let result = flash_one(&job, device, &mut emitter);
        if !result.ok {
            all_ok = false;
        }
        results.push(result);
    }

    let _ = std::fs::write(
        result_path,
        serde_json::to_string(&results).unwrap_or_else(|_| "[]".into()),
    );

    emitter.emit(&Progress {
        phase: if all_ok { "finished" } else { "failed" }.into(),
        fraction: 1.0,
        bytes: 0,
        total_bytes: None,
        speed: 0.0,
        eta: None,
        message: None,
        device: None,
    });

    std::process::exit(if all_ok { 0 } else { 1 });
}

fn flash_one(job: &Job, device: &str, emitter: &mut Emitter) -> DeviceResult {
    emitter.emit(&Progress {
        phase: "starting".into(),
        fraction: 0.0,
        bytes: 0,
        total_bytes: Some(job.file_size),
        speed: 0.0,
        eta: None,
        message: Some(format!("Preparing {device}")),
        device: Some(device.to_string()),
    });

    let (bytes_written, checksum) = match flash_one_inner(job, device, emitter) {
        Ok(v) => v,
        Err(e) => {
            emitter.emit(&Progress {
                phase: "failed".into(),
                fraction: 0.0,
                bytes: 0,
                total_bytes: None,
                speed: 0.0,
                eta: None,
                message: Some(e.clone()),
                device: Some(device.to_string()),
            });
            return DeviceResult {
                ok: false,
                device: device.to_string(),
                bytes_written: 0,
                checksum: None,
                error: Some(e),
            };
        }
    };

    // Post-write boot-partition step: copy files and/or hold it mounted for
    // editing. Any failure here is reported, but the image is still written &
    // verified (bytes_written/checksum preserved).
    let need_boot = !job.boot_config_files.is_empty() || job.edit_boot;
    if need_boot {
        let fail = |emitter: &mut Emitter, reason: String| -> DeviceResult {
            let msg = format!("Image written & verified, but {reason}");
            emitter.emit(&Progress {
                phase: "failed".into(),
                fraction: 1.0,
                bytes: bytes_written,
                total_bytes: Some(bytes_written),
                speed: 0.0,
                eta: None,
                message: Some(msg.clone()),
                device: Some(device.to_string()),
            });
            DeviceResult {
                ok: false,
                device: device.to_string(),
                bytes_written,
                checksum: Some(checksum.clone()),
                error: Some(msg),
            }
        };

        if !job.boot_config_files.is_empty() {
            emitter.emit(&Progress {
                phase: "configuring".into(),
                fraction: 1.0,
                bytes: bytes_written,
                total_bytes: Some(bytes_written),
                speed: 0.0,
                eta: None,
                message: Some(format!(
                    "Copying {} file(s) to boot partition",
                    job.boot_config_files.len()
                )),
                device: Some(device.to_string()),
            });
        }

        let uid = if job.edit_boot { invoking_uid() } else { None };
        let mount = match bootconfig::mount_boot(device, uid) {
            Ok(m) => m,
            Err(e) => return fail(emitter, format!("could not mount boot partition: {e}")),
        };

        if !job.boot_config_files.is_empty() {
            if let Err(e) = bootconfig::copy_files(&mount, &job.boot_config_files) {
                bootconfig::unmount(mount);
                return fail(emitter, format!("copying boot files failed: {e}"));
            }
        }

        if job.edit_boot {
            // Hand the mountpoint to the GUI and wait until the user is done.
            emitter.emit(&Progress {
                phase: "editing".into(),
                fraction: 1.0,
                bytes: bytes_written,
                total_bytes: Some(bytes_written),
                speed: 0.0,
                eta: None,
                message: Some(mount.dir.clone()),
                device: Some(device.to_string()),
            });
            wait_for_edit_done();
        }

        bootconfig::unmount(mount);
    }

    DeviceResult {
        ok: true,
        device: device.to_string(),
        bytes_written,
        checksum: Some(checksum),
        error: None,
    }
}

/// Block until the GUI signals editing is finished (or cancels), with a cap.
fn wait_for_edit_done() {
    let deadline = Instant::now() + std::time::Duration::from_secs(3600);
    while !edit_done() && !is_cancelled() {
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

fn flash_one_inner(
    job: &Job,
    device: &str,
    emitter: &mut Emitter,
) -> Result<(u64, String), String> {
    unmount_device(device);

    let mut dev = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(device)
        .map_err(|e| format!("cannot open {device}: {e}"))?;

    let mut hasher = Sha256::new();
    let started = Instant::now();
    let mut last_emit = Instant::now();
    let mut last_written = 0u64;
    let file_size = job.file_size.max(1);

    let mut emit_flashing = |fraction: f64, written: u64, emitter: &mut Emitter| {
        let now = Instant::now();
        let dt = now.duration_since(last_emit).as_secs_f64();
        if dt < 0.15 {
            return;
        }
        let speed = (written.saturating_sub(last_written)) as f64 / dt;
        last_written = written;
        last_emit = now;
        let frac = fraction.clamp(0.0, 0.999);
        let elapsed = now.duration_since(started).as_secs_f64();
        let eta = if frac > 0.01 {
            Some(elapsed * (1.0 - frac) / frac)
        } else {
            None
        };
        emitter.emit(&Progress {
            phase: "flashing".into(),
            fraction: frac,
            bytes: written,
            total_bytes: Some(job.file_size),
            speed,
            eta,
            message: None,
            device: Some(device.to_string()),
        });
    };

    // A .bmap lets us skip blank blocks (faster). Only for the streaming path.
    let bmap = if job.compression != "zip" {
        match job.bmap_path.as_deref() {
            Some(p) => match bmap::parse(p) {
                Ok(b) => Some(b),
                Err(e) => {
                    emitter.emit(&Progress {
                        phase: "flashing".into(),
                        fraction: 0.0,
                        bytes: 0,
                        total_bytes: Some(job.file_size),
                        speed: 0.0,
                        eta: None,
                        message: Some(format!("ignoring bmap ({e})")),
                        device: Some(device.to_string()),
                    });
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };

    let is_url = job.image_path.starts_with("http://") || job.image_path.starts_with("https://");
    let bytes_written = if job.compression == "zip" {
        if is_url {
            // Zip needs random access, so it can't be streamed from a URL; the
            // GUI downloads zip URLs to a temp file first.
            return Err("zip images cannot be streamed from a URL".into());
        }
        write_from_zip(&job.image_path, &mut dev, &mut hasher, &mut |fraction, written| {
            emit_flashing(fraction, written, emitter)
        })?
    } else {
        // Source is a local file OR an http(s) URL streamed directly to the
        // device (write-while-download), then decompressed on the fly.
        let raw = source::open_raw(&job.image_path).map_err(|e| e.to_string())?;
        let (mut reader, counter) = source::streaming_decoder(raw, &job.compression)
            .map_err(|e| e.to_string())?;
        let progress = &mut |written: u64| {
            let fraction = counter.load(Ordering::Relaxed) as f64 / file_size as f64;
            emit_flashing(fraction, written, emitter)
        };
        if let Some(bm) = &bmap {
            write_with_bmap(&mut reader, &mut dev, bm, &mut hasher, progress)
                .map_err(|e| format!("write error: {e}"))?
        } else {
            pump(&mut reader, &mut dev, &mut hasher, progress)
                .map_err(|e| format!("write error: {e}"))?
        }
    };

    dev.flush().map_err(|e| e.to_string())?;
    dev.sync_all().map_err(|e| e.to_string())?;

    let checksum = hex(&hasher.finalize());

    if job.validate {
        if let Some(bm) = &bmap {
            verify_with_bmap(device, &mut dev, bm, &checksum, emitter)?;
        } else {
            verify(device, &mut dev, bytes_written, &checksum, emitter)?;
        }
    }

    // Boot-config copy is handled in flash_one() so that a post-write failure
    // still reports the image as written & verified.
    Ok((bytes_written, checksum))
}

/// Stream `reader` to `dev`, hashing as we go. Calls `on_progress(bytes_written)`
/// after each chunk. Returns total bytes written.
fn pump(
    reader: &mut dyn Read,
    dev: &mut File,
    hasher: &mut Sha256,
    on_progress: &mut dyn FnMut(u64),
) -> io::Result<u64> {
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled by user"));
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dev.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        total += n as u64;
        on_progress(total);
    }
    Ok(total)
}

/// Read exactly `buf.len()` bytes (or fewer at EOF), looping over short reads.
fn read_block(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Like `pump`, but consults a bmap: blocks outside the mapped ranges are read
/// from the decompressed stream but not written to the device. Returns the
/// number of bytes actually written (mapped data).
fn write_with_bmap(
    reader: &mut dyn Read,
    dev: &mut File,
    bmap: &bmap::Bmap,
    hasher: &mut Sha256,
    on_progress: &mut dyn FnMut(u64),
) -> io::Result<u64> {
    let bs = bmap.block_size as usize;
    let mut buf = vec![0u8; bs];
    let mut block: u64 = 0;
    let mut written = 0u64;
    let mut ri = 0;
    loop {
        if is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled by user"));
        }
        let n = read_block(reader, &mut buf)?;
        if n == 0 {
            break;
        }
        while ri < bmap.ranges.len() && block > bmap.ranges[ri].1 {
            ri += 1;
        }
        let mapped = ri < bmap.ranges.len()
            && block >= bmap.ranges[ri].0
            && block <= bmap.ranges[ri].1;
        if mapped {
            dev.seek(SeekFrom::Start(block * bs as u64))?;
            dev.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            written += n as u64;
            on_progress(written);
        }
        block += 1;
    }
    Ok(written)
}

/// Validate a bmap write by reading back only the mapped ranges and hashing
/// them in the same order they were written.
fn verify_with_bmap(
    device: &str,
    dev: &mut File,
    bmap: &bmap::Bmap,
    expected: &str,
    emitter: &mut Emitter,
) -> Result<(), String> {
    let bs = bmap.block_size;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; bs as usize];
    let total_blocks: u64 = bmap.ranges.iter().map(|(a, b)| b - a + 1).sum();
    let mut done_blocks = 0u64;
    let mut last_emit = Instant::now();

    for &(start, end) in &bmap.ranges {
        for blk in start..=end {
            if is_cancelled() {
                return Err("cancelled by user".into());
            }
            let want = bmap.block_len(blk);
            dev.seek(SeekFrom::Start(blk * bs))
                .map_err(|e| e.to_string())?;
            let n = read_block(dev, &mut buf[..want]).map_err(|e| e.to_string())?;
            if n != want {
                return Err(format!("validation read short at block {blk}"));
            }
            hasher.update(&buf[..n]);
            done_blocks += 1;
            if last_emit.elapsed().as_secs_f64() > 0.15 {
                last_emit = Instant::now();
                emitter.emit(&Progress {
                    phase: "validating".into(),
                    fraction: (done_blocks as f64 / total_blocks.max(1) as f64).min(0.999),
                    bytes: done_blocks * bs,
                    total_bytes: Some(total_blocks * bs),
                    speed: 0.0,
                    eta: None,
                    message: None,
                    device: Some(device.to_string()),
                });
            }
        }
    }
    if hex(&hasher.finalize()) != expected {
        return Err("validation failed: written data does not match image".into());
    }
    Ok(())
}

fn write_from_zip(
    image_path: &str,
    dev: &mut File,
    hasher: &mut Sha256,
    on_progress: &mut dyn FnMut(f64, u64),
) -> Result<u64, String> {
    let file = File::open(image_path).map_err(|e| format!("cannot open image: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    let mut index = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_file() {
            index = Some(i);
            break;
        }
    }
    let index = index.ok_or("zip archive contains no files")?;
    let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
    let total = entry.size().max(1);

    let mut buf = vec![0u8; CHUNK];
    let mut written = 0u64;
    loop {
        if is_cancelled() {
            return Err("cancelled by user".into());
        }
        let n = entry.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        dev.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        written += n as u64;
        on_progress(written as f64 / total as f64, written);
    }
    Ok(written)
}

fn verify(
    device: &str,
    dev: &mut File,
    bytes_written: u64,
    expected: &str,
    emitter: &mut Emitter,
) -> Result<(), String> {
    dev.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut read_total = 0u64;
    let mut last_emit = Instant::now();
    while read_total < bytes_written {
        if is_cancelled() {
            return Err("cancelled by user".into());
        }
        let want = ((bytes_written - read_total) as usize).min(CHUNK);
        let n = dev.read(&mut buf[..want]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read_total += n as u64;
        if last_emit.elapsed().as_secs_f64() > 0.15 {
            last_emit = Instant::now();
            emitter.emit(&Progress {
                phase: "validating".into(),
                fraction: (read_total as f64 / bytes_written.max(1) as f64).min(0.999),
                bytes: read_total,
                total_bytes: Some(bytes_written),
                speed: 0.0,
                eta: None,
                message: None,
                device: Some(device.to_string()),
            });
        }
    }
    if read_total != bytes_written {
        return Err(format!(
            "validation read short: {read_total} of {bytes_written} bytes"
        ));
    }
    let actual = hex(&hasher.finalize());
    if actual != expected {
        return Err("validation failed: written data does not match image".into());
    }
    Ok(())
}

/// Unmount any mounted partitions of `device` so we can write the whole disk.
fn unmount_device(device: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = std::process::Command::new("lsblk")
            .args(["-nro", "PATH,MOUNTPOINT", device])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let mut parts = line.split_whitespace();
                let path = parts.next().unwrap_or("");
                let mountpoint = parts.next().unwrap_or("");
                if !mountpoint.is_empty() && !path.is_empty() {
                    let _ = std::process::Command::new("umount").arg(path).status();
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("diskutil")
            .args(["unmountDisk", device])
            .status();
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
