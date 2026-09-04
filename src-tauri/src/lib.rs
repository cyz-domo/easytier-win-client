#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, process::{Child, Command, Stdio}, sync::Mutex};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus { Stopped, Starting, Running, Stopping, Failed }
#[derive(Clone, Serialize)]
pub struct InstanceState { pub id: String, pub status: InstanceStatus, pub pid: Option<u32>, pub error: Option<String> }
#[derive(Serialize)]
pub struct RuntimeInfo { pub core_path: String, pub cli_path: String, pub version: String, pub available: bool }
#[derive(Default)]
pub struct RuntimeProcesses { children: HashMap<String, Child> }

fn runtime_dir(runtime_dir: Option<String>) -> PathBuf {
    if let Some(d) = runtime_dir { return PathBuf::from(d); }
    let mut candidates = vec![PathBuf::from("core"), PathBuf::from("../core"), PathBuf::from("../../core")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("core"));
            candidates.push(dir.join("../core"));
            candidates.push(dir.join("../../../core"));
        }
    }
    for c in candidates { if c.join("easytier-core.exe").exists() { return c; } }
    PathBuf::from("core")
}
fn paths(dir_override: Option<String>) -> (PathBuf, PathBuf) { let d = runtime_dir(dir_override); (d.join("easytier-core.exe"), d.join("easytier-cli.exe")) }
#[tauri::command]
fn detect_runtime(runtime_dir: Option<String>) -> RuntimeInfo { let (core, cli) = paths(runtime_dir); let version = Command::new(&core).arg("--version").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_else(|| "unknown".into()).trim().to_string(); RuntimeInfo { core_path: core.display().to_string(), cli_path: cli.display().to_string(), version, available: core.exists() && cli.exists() } }
#[tauri::command]
fn get_instance_state(id: String, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> InstanceState { let running = state.lock().unwrap().children.contains_key(&id); InstanceState { id, status: if running { InstanceStatus::Running } else { InstanceStatus::Stopped }, pid: None, error: None } }
#[tauri::command]
fn start_instance(id: String, config: String, runtime_dir: Option<String>, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> Result<InstanceState, String> { let (core, _) = paths(runtime_dir); if !core.exists() { return Err(format!("EasyTier core not found: {}", core.display())); } let config_path = std::env::temp_dir().join(format!("easytier-{}.toml", id)); fs::write(&config_path, config).map_err(|e| e.to_string())?; let mut p = state.lock().map_err(|_| "runtime state unavailable".to_string())?; if p.children.contains_key(&id) { return Ok(InstanceState { id, status: InstanceStatus::Running, pid: None, error: None }); } let child = Command::new(core).arg("--config-file").arg(&config_path).stdin(Stdio::null()).spawn().map_err(|e| e.to_string())?; let pid = child.id(); p.children.insert(id.clone(), child); Ok(InstanceState { id, status: InstanceStatus::Running, pid: Some(pid), error: None }) }
#[tauri::command]
fn stop_instance(id: String, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> Result<InstanceState, String> { let mut p = state.lock().map_err(|_| "runtime state unavailable".to_string())?; if let Some(mut child) = p.children.remove(&id) { child.kill().map_err(|e| e.to_string())?; let _ = child.wait(); } Ok(InstanceState { id, status: InstanceStatus::Stopped, pid: None, error: None }) }
#[tauri::command]
fn run_cli(args: Vec<String>, runtime_dir: Option<String>) -> Result<String, String> { let (_, cli) = paths(runtime_dir); let o = Command::new(cli).args(args).output().map_err(|e| e.to_string())?; if o.status.success() { String::from_utf8(o.stdout).map_err(|e| e.to_string()) } else { Err(String::from_utf8_lossy(&o.stderr).to_string()) } }
pub fn run() { tauri::Builder::default().manage(Mutex::new(RuntimeProcesses::default())).invoke_handler(tauri::generate_handler![detect_runtime, get_instance_state, start_instance, stop_instance, run_cli]).run(tauri::generate_context!()).expect("error while running tauri application"); }
