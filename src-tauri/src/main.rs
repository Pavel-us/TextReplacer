#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod flow;

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

/// Runs search/replace on a worker thread; per-file results are streamed as events.
#[tauri::command]
async fn run_replace(app: AppHandle, req: engine::Request) -> Result<engine::Stats, String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        engine::run(&req, |ev| {
            let _ = app.emit("file-result", ev);
        })
    });
    let result = handle.await.map_err(|e| e.to_string())?;
    result
}

#[tauri::command]
fn load_rules(path: String, regex: bool) -> Result<Vec<engine::Rule>, String> {
    engine::load_rules(&path, regex)
}

#[derive(Serialize)]
struct PathInfo {
    dir: String,
    file: Option<String>,
}

/// For drag-and-drop: folder -> (folder, -), file -> (its folder, file name).
#[tauri::command]
fn inspect_path(path: String) -> PathInfo {
    let p = Path::new(&path);
    if p.is_dir() {
        return PathInfo { dir: path, file: None };
    }
    PathInfo {
        dir: p
            .parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default(),
        file: p.file_name().map(|f| f.to_string_lossy().to_string()),
    }
}

/// <config dir>/dev.textreplacer.app/locales — where users put their own translations.
fn locales_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|d| d.join("locales"))
        .map_err(|e| e.to_string())
}

/// Contents of user translations (*.json; files starting with "_" are ignored).
#[tauri::command]
fn user_locales(app: AppHandle) -> Vec<String> {
    let Ok(dir) = locales_dir(&app) else { return vec![] };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('_') || p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&p) {
            out.push(text);
        }
    }
    out
}

/// In-app updating works on Windows, macOS and AppImage only:
/// .deb / .rpm installations are managed by the system package manager.
#[tauri::command]
fn can_self_update() -> bool {
    if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE").is_some()
    } else {
        true
    }
}

const LOCALE_TEMPLATE: &str = include_str!("../../src/locales/en.json");

/// Creates the translations folder and `_template.json`; returns the template path.
#[tauri::command]
fn prepare_locales_dir(app: AppHandle) -> Result<String, String> {
    let dir = locales_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let template = dir.join("_template.json");
    if !template.exists() {
        std::fs::write(&template, LOCALE_TEMPLATE).map_err(|e| e.to_string())?;
    }
    Ok(template.to_string_lossy().to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            run_replace,
            load_rules,
            inspect_path,
            user_locales,
            prepare_locales_dir,
            can_self_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Tauri application");
}
