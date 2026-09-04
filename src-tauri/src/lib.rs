#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde::Serialize;
use std::{path::PathBuf, process::Command};

#[derive(Serialize)]
struct RuntimeInfo { core_path: String, cli_path: String, version: String, available: bool }

#[tauri::command]
fn detect_runtime(runtime_dir: Option<String>) -> RuntimeInfo {
    let dir = runtime_dir.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("core"));
    let core = dir.join("easytier-core.exe"); let cli = dir.join("easytier-cli.exe");
    let version = Command::new(&core).arg("--version").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_else(|| "unknown".into()).trim().to_string();
    RuntimeInfo { core_path: core.display().to_string(), cli_path: cli.display().to_string(), version, available: core.exists() && cli.exists() }
}

pub fn run() { tauri::Builder::default().invoke_handler(tauri::generate_handler![detect_runtime]).run(tauri::generate_context!()).expect("error while running tauri application"); }
