//! Download a remote image to a temp file (unprivileged), so it can then be
//! flashed through the normal file path. Emits `download-progress` events.

use std::fs::File;
use std::io::{Read, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::commands::{basic_auth_header, detect_compression, filename_from_disposition};
use crate::models::{HttpAuth, ImageInfo};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    fraction: f64,
    bytes: u64,
    total_bytes: Option<u64>,
    speed: f64,
    eta: Option<f64>,
}

fn url_basename(url: &str) -> String {
    let no_query = url.split('?').next().unwrap_or(url);
    no_query
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("image")
        .to_string()
}

#[tauri::command]
pub fn download_image(
    app: AppHandle,
    url: String,
    auth: Option<HttpAuth>,
) -> Result<ImageInfo, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("URL must start with http:// or https://".into());
    }
    let mut req = ureq::get(&url);
    if let Some(a) = &auth {
        req = req.set("Authorization", &basic_auth_header(a));
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(401, _) | ureq::Error::Status(403, _) => {
            "authentication required or failed for this URL".to_string()
        }
        other => format!("download failed: {other}"),
    })?;
    let total: Option<u64> = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let resolved_name = resp
        .header("Content-Disposition")
        .and_then(filename_from_disposition)
        .unwrap_or_else(|| url_basename(&url));

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("pyro-dl-{}-{}", std::process::id(), nonce));

    let mut reader = resp.into_reader();
    let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    let mut downloaded = 0u64;
    let mut last = Instant::now();
    let mut last_bytes = 0u64;

    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;

        let dt = last.elapsed().as_secs_f64();
        if dt > 0.15 {
            let speed = (downloaded - last_bytes) as f64 / dt;
            last = Instant::now();
            last_bytes = downloaded;
            let fraction = total
                .map(|t| (downloaded as f64 / t.max(1) as f64).min(0.999))
                .unwrap_or(0.0);
            let eta = if speed > 1.0 {
                total.map(|t| (t.saturating_sub(downloaded)) as f64 / speed)
            } else {
                None
            };
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    fraction,
                    bytes: downloaded,
                    total_bytes: total,
                    speed,
                    eta,
                },
            );
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            fraction: 1.0,
            bytes: downloaded,
            total_bytes: total,
            speed: 0.0,
            eta: Some(0.0),
        },
    );

    let compression = detect_compression(&tmp);
    let file_size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(downloaded);
    Ok(ImageInfo {
        path: tmp.to_string_lossy().to_string(),
        name: resolved_name,
        file_size,
        uncompressed_size: if compression == "none" {
            Some(file_size)
        } else {
            None
        },
        compression,
        bmap_path: None,
        // The download lands as a local temp file; no auth needed downstream.
        auth: None,
    })
}
