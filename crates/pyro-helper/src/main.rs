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
mod format;
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
/// Flush the page cache to the device this often during a write. Without it, a
/// fast source (a local file) fills the kernel with gigabytes of dirty pages,
/// progress races to 99% while the data is still in RAM, and the final sync then
/// stalls for a long time — the classic "stuck at 99%". Syncing periodically
/// caps in-flight data so progress tracks real USB throughput. (A slow source
/// like a streamed URL is naturally paced and never hits this.)
const SYNC_EVERY: u64 = 64 * 1024 * 1024;
/// Alignment for O_DIRECT I/O: buffer address, transfer length, and file offset
/// must all be multiples of the device's logical block size. 4096 is a multiple
/// of both common sizes (512 and 4096), so it satisfies either. CHUNK is already
/// a multiple of this.
#[cfg(target_os = "linux")]
const ALIGN: usize = 4096;

/// A heap buffer whose start address is aligned to `ALIGN`, as O_DIRECT requires.
/// A plain `Vec<u8>` has no such guarantee.
#[cfg(target_os = "linux")]
struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
    layout: std::alloc::Layout,
}

#[cfg(target_os = "linux")]
impl AlignedBuf {
    fn new(len: usize) -> Self {
        let layout = std::alloc::Layout::from_size_align(len, ALIGN).unwrap();
        // SAFETY: layout has non-zero size; we check for null below.
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        AlignedBuf { ptr, len, layout }
    }
    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: ptr is a valid allocation of `len` bytes for this buffer's life.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
    fn as_ref(&self) -> &[u8] {
        // SAFETY: as above, shared borrow.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

#[cfg(target_os = "linux")]
impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: ptr/layout are exactly what alloc returned.
        unsafe { std::alloc::dealloc(self.ptr, self.layout) }
    }
}

/// Open the target device for writing. On Linux, try O_DIRECT first (bypasses the
/// page cache → real device-speed progress and no end-of-write stall, the way
/// rpi-imager does it). Returns `(file, is_direct)`; falls back to a buffered
/// handle (paired with periodic syncing) if O_DIRECT isn't available — some
/// filesystems/devices reject it. `want_direct` is false for write paths that
/// can't satisfy alignment (zip, bmap), forcing the buffered handle.
fn open_device(device: &str, want_direct: bool) -> Result<(File, bool), String> {
    #[cfg(target_os = "linux")]
    if want_direct {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(f) = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_DIRECT)
            .open(device)
        {
            return Ok((f, true));
        }
    }
    let _ = want_direct;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(device)
        .map_err(|e| format!("cannot open {device}: {e}"))?;
    Ok((f, false))
}

/// Optional path to a cancellation flag file; if it appears, abort.
static CANCEL_FILE: OnceLock<Option<String>> = OnceLock::new();
/// Optional path to an "editing finished" flag file (boot-file editor).
static EDIT_DONE_FILE: OnceLock<Option<String>> = OnceLock::new();
/// Optional path to a file the GUI writes with the partition the user chose to
/// mount (or "__skip__"), in response to a "choose" event.
static CHOICE_FILE: OnceLock<Option<String>> = OnceLock::new();

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
    /// HTTP Basic Auth credentials for a URL source (streamed directly).
    #[serde(default)]
    http_username: Option<String>,
    #[serde(default)]
    http_password: Option<String>,
    /// If set, erase & format each device instead of writing an image.
    #[serde(default)]
    format: Option<FormatJob>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormatJob {
    filesystem: String,
    label: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
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
    /// Emit an arbitrary JSON event (used for the "choose" event, which carries a
    /// partition list that doesn't fit the fixed Progress shape).
    fn emit_value(&mut self, v: &serde_json::Value) {
        let _ = writeln!(self.file, "{v}");
        let _ = self.file.flush();
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
    // Optional 6th arg: the partition-choice answer file.
    CHOICE_FILE.set(args.get(6).cloned()).ok();

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

    // Erase/format path: no image write, no verify, no boot step.
    if let Some(fmt) = &job.format {
        return format_one(device, fmt, emitter);
    }

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
                warning: None,
            };
        }
    };

    // Post-write boot-partition step: copy files and/or hold it mounted for
    // editing. This is a BONUS step — the image is already written & verified,
    // so anything here (no FAT partition, mount failure) is a non-fatal warning,
    // never a flash failure. Many images (e.g. plain ISOs) have no editable
    // boot partition, and that's fine.
    let warning: Option<String> = if job.edit_boot {
        // The user wants to edit files: let them pick which partition to mount
        // from the device we just wrote, rather than guessing by name.
        run_partition_chooser(job, device, bytes_written, emitter)
    } else if !job.boot_config_files.is_empty() {
        // Drop-only (no interactive edit): auto-detect the FAT boot partition.
        copy_to_boot(job, device, bytes_written, emitter)
    } else {
        None
    };

    DeviceResult {
        ok: true,
        device: device.to_string(),
        bytes_written,
        checksum: Some(checksum),
        error: None,
        warning,
    }
}

/// Erase a device and lay down a fresh filesystem (the "Erase" utility).
fn format_one(device: &str, fmt: &FormatJob, emitter: &mut Emitter) -> DeviceResult {
    let result = format::run(device, &fmt.filesystem, &fmt.label, &mut |msg, frac| {
        emitter.emit(&Progress {
            phase: "formatting".into(),
            fraction: frac,
            bytes: 0,
            total_bytes: None,
            speed: 0.0,
            eta: None,
            message: Some(msg.to_string()),
            device: Some(device.to_string()),
        });
    });
    match result {
        Ok(()) => DeviceResult {
            ok: true,
            device: device.to_string(),
            bytes_written: 0,
            checksum: None,
            error: None,
            warning: None,
        },
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
            DeviceResult {
                ok: false,
                device: device.to_string(),
                bytes_written: 0,
                checksum: None,
                error: Some(e),
                warning: None,
            }
        }
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

/// Block until the GUI writes the partition-choice file. Returns the chosen
/// device path, or `None` if the user skipped/cancelled (or it timed out).
fn wait_for_choice() -> Option<String> {
    let path = CHOICE_FILE.get().and_then(|o| o.as_ref())?;
    let deadline = Instant::now() + std::time::Duration::from_secs(3600);
    loop {
        if is_cancelled() {
            return None;
        }
        if Path::new(path).exists() {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            let trimmed = content.trim();
            if trimmed.is_empty() || trimmed == "__skip__" {
                return None;
            }
            return Some(trimmed.to_string());
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Auto-detect the FAT boot partition and copy the drop files onto it. Returns a
/// non-fatal warning if there's nothing to mount or the copy fails.
fn copy_to_boot(
    job: &Job,
    device: &str,
    bytes_written: u64,
    emitter: &mut Emitter,
) -> Option<String> {
    match bootconfig::mount_boot(device, None) {
        Ok(mount) => {
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
            let warning = bootconfig::copy_files(&mount, &job.boot_config_files)
                .err()
                .map(|e| format!("could not copy boot files: {e}"));
            bootconfig::unmount(mount);
            warning
        }
        Err(e) => Some(format!("no editable boot partition found ({e})")),
    }
}

/// Offer the device's partitions to the user, mount the one they pick, copy any
/// drop files, then hold it mounted for editing until they're done.
fn run_partition_chooser(
    job: &Job,
    device: &str,
    bytes_written: u64,
    emitter: &mut Emitter,
) -> Option<String> {
    let parts = bootconfig::list_partitions(device);
    if parts.is_empty() {
        return Some("no mountable partitions found on the written device".into());
    }

    // Ask the GUI to present the choices.
    emitter.emit_value(&serde_json::json!({
        "phase": "choose",
        "fraction": 1.0,
        "bytes": bytes_written,
        "totalBytes": bytes_written,
        "device": device,
        "partitions": parts,
    }));

    let chosen = match wait_for_choice() {
        Some(p) => p,
        // Skipped or cancelled — the image is already written & verified.
        None => return None,
    };

    let fstype = parts
        .iter()
        .find(|p| p.path == chosen)
        .map(|p| p.fstype.clone())
        .unwrap_or_default();

    match bootconfig::mount_partition(&chosen, &fstype, invoking_uid()) {
        Ok(mount) => {
            if !job.boot_config_files.is_empty() {
                emitter.emit(&Progress {
                    phase: "configuring".into(),
                    fraction: 1.0,
                    bytes: bytes_written,
                    total_bytes: Some(bytes_written),
                    speed: 0.0,
                    eta: None,
                    message: Some(format!(
                        "Copying {} file(s) to {chosen}",
                        job.boot_config_files.len()
                    )),
                    device: Some(device.to_string()),
                });
                if let Err(e) = bootconfig::copy_files(&mount, &job.boot_config_files) {
                    bootconfig::unmount(mount);
                    return Some(format!("could not copy files: {e}"));
                }
            }
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
            bootconfig::unmount(mount);
            None
        }
        Err(e) => Some(format!("could not mount {chosen}: {e}")),
    }
}

fn flash_one_inner(
    job: &Job,
    device: &str,
    emitter: &mut Emitter,
) -> Result<(u64, String), String> {
    unmount_device(device);

    // O_DIRECT needs block-aligned, sequential writes — only the plain streaming
    // path can guarantee that. zip (random-access reads) and bmap (scattered
    // seeks) use the buffered handle with periodic syncing instead.
    let want_direct = job.compression != "zip" && job.bmap_path.is_none();
    let (mut dev, is_direct) = open_device(device, want_direct)?;

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
        let raw = source::open_raw(
            &job.image_path,
            job.http_username.as_deref(),
            job.http_password.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        let (mut reader, counter) = source::streaming_decoder(raw, &job.compression)
            .map_err(|e| e.to_string())?;
        let progress = &mut |written: u64| {
            let fraction = counter.load(Ordering::Relaxed) as f64 / file_size as f64;
            emit_flashing(fraction, written, emitter)
        };
        if let Some(bm) = &bmap {
            write_with_bmap(&mut reader, &mut dev, bm, &mut hasher, progress)
                .map_err(|e| format!("write error: {e}"))?
        } else if is_direct {
            // Direct I/O: aligned, page-cache-bypassing writes (rpi-imager's fast
            // path). No periodic sync needed — nothing is buffered in RAM.
            pump_direct(&mut reader, &mut dev, &mut hasher, progress)
                .map_err(|e| format!("write error: {e}"))?
        } else {
            pump(&mut reader, &mut dev, &mut hasher, progress)
                .map_err(|e| format!("write error: {e}"))?
        }
    };

    // Any data still in the page cache gets flushed here. Periodic syncing during
    // the write keeps this small, but label it so the bar never looks frozen.
    emitter.emit(&Progress {
        phase: "finalizing".into(),
        fraction: 0.999,
        bytes: bytes_written,
        total_bytes: Some(bytes_written),
        speed: 0.0,
        eta: None,
        message: None,
        device: Some(device.to_string()),
    });
    dev.flush().map_err(|e| e.to_string())?;
    dev.sync_all().map_err(|e| e.to_string())?;

    let checksum = hex(&hasher.finalize());

    if job.validate {
        if let Some(bm) = &bmap {
            verify_with_bmap(device, &mut dev, bm, &checksum, emitter)?;
        } else if is_direct {
            verify_direct(device, &mut dev, bytes_written, &checksum, emitter)?;
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
    let mut since_sync = 0u64;
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
        since_sync += n as u64;
        if since_sync >= SYNC_EVERY {
            dev.sync_data()?;
            since_sync = 0;
        }
        on_progress(total);
    }
    Ok(total)
}

/// Direct-I/O variant of `pump` (Linux O_DIRECT). Writes must be aligned and a
/// multiple of the device block size, so we fill a whole aligned buffer before
/// writing and zero-pad the final short block up to `ALIGN`. We hash only the
/// real image bytes (not the padding) so readback verification still matches.
/// No periodic sync: O_DIRECT already bypasses the page cache.
#[cfg(target_os = "linux")]
fn pump_direct(
    reader: &mut dyn Read,
    dev: &mut File,
    hasher: &mut Sha256,
    on_progress: &mut dyn FnMut(u64),
) -> io::Result<u64> {
    let mut buf = AlignedBuf::new(CHUNK);
    let mut total = 0u64;
    loop {
        if is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled by user"));
        }
        // Fill the whole buffer so each write is a full aligned CHUNK; a short
        // read mid-stream would otherwise produce an unaligned write length.
        let n = read_block(reader, buf.as_mut())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf.as_ref()[..n]);
        // Round the write length up to ALIGN (<= CHUNK, since CHUNK is a multiple
        // of ALIGN), zeroing the pad bytes. Only the final block is ever padded.
        let write_len = if n % ALIGN == 0 {
            n
        } else {
            let pad = ALIGN - (n % ALIGN);
            buf.as_mut()[n..n + pad].fill(0);
            n + pad
        };
        dev.write_all(&buf.as_ref()[..write_len])?;
        total += n as u64;
        on_progress(total);
    }
    Ok(total)
}

/// Direct-I/O readback verification (Linux O_DIRECT). Reads are aligned too: we
/// read full aligned blocks and hash only up to `bytes_written`, ignoring any
/// padding written past the image's end.
#[cfg(target_os = "linux")]
fn verify_direct(
    device: &str,
    dev: &mut File,
    bytes_written: u64,
    expected: &str,
    emitter: &mut Emitter,
) -> Result<(), String> {
    dev.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = AlignedBuf::new(CHUNK);
    let mut read_total = 0u64;
    let mut last_emit = Instant::now();
    while read_total < bytes_written {
        if is_cancelled() {
            return Err("cancelled by user".into());
        }
        let remaining = bytes_written - read_total;
        let to_read = if remaining >= CHUNK as u64 {
            CHUNK
        } else {
            // Round up to ALIGN so the read length stays block-aligned.
            (((remaining as usize) + ALIGN - 1) / ALIGN) * ALIGN
        };
        let n = read_block(dev, &mut buf.as_mut()[..to_read]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let use_n = (n as u64).min(remaining) as usize;
        hasher.update(&buf.as_ref()[..use_n]);
        read_total += use_n as u64;
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
    if hex(&hasher.finalize()) != expected {
        return Err("validation failed: written data does not match image".into());
    }
    Ok(())
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
    let mut since_sync = 0u64;
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
            since_sync += n as u64;
            if since_sync >= SYNC_EVERY {
                dev.sync_data()?;
                since_sync = 0;
            }
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
    let mut since_sync = 0u64;
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
        since_sync += n as u64;
        if since_sync >= SYNC_EVERY {
            dev.sync_data().map_err(|e| e.to_string())?;
            since_sync = 0;
        }
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
