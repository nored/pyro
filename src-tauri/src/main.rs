// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bootedit;
mod commands;
mod download;
mod drives;
mod flash;
mod models;
mod settings;

use tauri::Emitter;

fn main() {
    // WebKitGTK's DMABUF renderer is broken under Wayland on several drivers
    // (most notably the NVIDIA proprietary driver), crashing with
    // "Error 71 (Protocol error) dispatching to Wayland display". Disabling it
    // is a universally safe fallback — every GPU still renders correctly via the
    // standard path; only a GPU optimisation (irrelevant for this UI) is skipped.
    // Linux-only, and we honour an explicit override so power users can opt out.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .setup(|app| {
            // Poll for drive changes and notify the UI.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last = String::new();
                loop {
                    let list = drives::list();
                    let snapshot = serde_json::to_string(&list).unwrap_or_default();
                    if snapshot != last {
                        last = snapshot;
                        let _ = handle.emit("drives-changed", list);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::select_image,
            commands::inspect_image,
            commands::inspect_url,
            commands::select_boot_config_files,
            commands::notify,
            commands::forget_temp,
            settings::get_settings,
            settings::set_settings,
            settings::add_recent_url,
            download::download_image,
            flash::start_flash,
            flash::cancel_flash,
            flash::finish_edit,
            flash::choose_partition,
            bootedit::boot_list,
            bootedit::boot_read_text,
            bootedit::boot_write_text,
            bootedit::boot_rename,
            bootedit::boot_delete,
            bootedit::boot_add,
            commands::open_external,
            commands::os_platform,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pyro");
}
