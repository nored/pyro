//! Drive the privileged `pyro-helper`: write a job, launch it elevated
//! (pkexec/polkit on Linux, Touch ID on macOS), tail its progress file and
//! emit `flash-progress` events, then return the per-device results.

use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::models::{FlashRequest, FlashResult};

/// Path of the cancel-flag file for the in-flight flash, if any.
static ACTIVE_CANCEL: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Path of the "editing done" flag file for the in-flight flash, if any.
static ACTIVE_EDIT_DONE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobOut {
    image_path: String,
    compression: String,
    file_size: u64,
    devices: Vec<String>,
    validate: bool,
    boot_config_files: Vec<String>,
    edit_boot: bool,
    bmap_path: Option<String>,
}

fn helper_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("cannot locate executable directory")?;
    let name = if cfg!(windows) {
        "pyro-helper.exe"
    } else {
        "pyro-helper"
    };
    let candidate = dir.join(name);
    if candidate.exists() {
        Ok(candidate)
    } else {
        Err(format!(
            "privileged helper not found at {}",
            candidate.display()
        ))
    }
}

#[tauri::command]
pub fn start_flash(app: AppHandle, req: FlashRequest) -> Result<Vec<FlashResult>, String> {
    let helper = helper_path()?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let work = std::env::temp_dir().join(format!("pyro-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let job_path = work.join("job.json");
    let prog_path = work.join("progress.jsonl");
    let res_path = work.join("result.json");
    let cancel_path = work.join("cancel.flag");
    let edit_done_path = work.join("edit-done.flag");

    let job = JobOut {
        image_path: req.image.path.clone(),
        compression: req.image.compression.clone(),
        file_size: req.image.file_size,
        devices: req.devices.clone(),
        validate: req.validate,
        boot_config_files: req.boot_config_files.clone(),
        edit_boot: req.edit_boot,
        bmap_path: req.image.bmap_path.clone(),
    };
    fs::write(
        &job_path,
        serde_json::to_string(&job).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(&prog_path, b"").map_err(|e| e.to_string())?;

    *ACTIVE_CANCEL.lock().unwrap() = Some(cancel_path.clone());
    *ACTIVE_EDIT_DONE.lock().unwrap() = Some(edit_done_path.clone());

    // Tail the progress file and forward events to the UI.
    let stop = Arc::new(AtomicBool::new(false));
    let tail = spawn_progress_tail(app.clone(), prog_path.clone(), stop.clone());

    let status = run_elevated(
        &helper,
        &job_path,
        &prog_path,
        &res_path,
        &cancel_path,
        &edit_done_path,
    );

    // Let the tail flush the final lines, then stop it.
    thread::sleep(Duration::from_millis(250));
    stop.store(true, Ordering::Relaxed);
    let _ = tail.join();
    *ACTIVE_CANCEL.lock().unwrap() = None;
    *ACTIVE_EDIT_DONE.lock().unwrap() = None;

    let results: Vec<FlashResult> = fs::read_to_string(&res_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let _ = fs::remove_dir_all(&work);

    match status {
        Ok(true) => Ok(results),
        Ok(false) if !results.is_empty() => Ok(results),
        Ok(false) => Err("Flashing failed or was cancelled".into()),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub fn cancel_flash() {
    if let Some(path) = ACTIVE_CANCEL.lock().unwrap().clone() {
        let _ = fs::write(path, b"1");
    }
}

/// Signal the helper that boot-file editing is finished, so it unmounts/ejects.
#[tauri::command]
pub fn finish_edit() {
    if let Some(path) = ACTIVE_EDIT_DONE.lock().unwrap().clone() {
        let _ = fs::write(path, b"1");
    }
}

fn spawn_progress_tail(
    app: AppHandle,
    prog_path: PathBuf,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pos: u64 = 0;
        loop {
            if let Ok(file) = fs::File::open(&prog_path) {
                let mut reader = BufReader::new(file);
                if reader.seek(SeekFrom::Start(pos)).is_ok() {
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => break,
                            Ok(n) => {
                                pos += n as u64;
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                                        let _ = app.emit("flash-progress", v);
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_millis(120));
        }
    })
}

#[cfg(target_os = "linux")]
fn run_elevated(
    helper: &Path,
    job: &Path,
    prog: &Path,
    res: &Path,
    cancel: &Path,
    edit_done: &Path,
) -> Result<bool, String> {
    let status = Command::new("pkexec")
        .arg(helper)
        .arg(job)
        .arg(prog)
        .arg(res)
        .arg(cancel)
        .arg(edit_done)
        .status()
        .map_err(|e| format!("could not launch pkexec: {e}"))?;
    match status.code() {
        Some(0) => Ok(true),
        // pkexec: 126 = user dismissed/not authorized, 127 = auth failed
        Some(126) | Some(127) => Err("Authentication was cancelled or failed".into()),
        _ => Ok(false),
    }
}

#[cfg(target_os = "macos")]
fn run_elevated(
    helper: &Path,
    job: &Path,
    prog: &Path,
    res: &Path,
    cancel: &Path,
    edit_done: &Path,
) -> Result<bool, String> {
    // Build a /bin/sh command line (paths single-quoted) and run it via
    // AppleScript's privileged shell, which offers Touch ID on supported Macs.
    let cmd = format!(
        "{} {} {} {} {} {}",
        sh_quote(helper),
        sh_quote(job),
        sh_quote(prog),
        sh_quote(res),
        sh_quote(cancel),
        sh_quote(edit_done),
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_escape(&cmd)
    );
    let status = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| format!("could not launch osascript: {e}"))?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Err("Authentication was cancelled".into()),
        _ => Ok(false),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn run_elevated(
    _helper: &Path,
    _job: &Path,
    _prog: &Path,
    _res: &Path,
    _cancel: &Path,
    _edit_done: &Path,
) -> Result<bool, String> {
    Err("Elevated flashing is not supported on this platform yet".into())
}

#[cfg(target_os = "macos")]
fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
