#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, process::{Child, Command, Stdio}, sync::Mutex};
use wait_timeout::ChildExt;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus { Stopped, Starting, Running, Stopping, Failed }
#[derive(Clone, Serialize)]
pub struct InstanceState { pub id: String, pub status: InstanceStatus, pub pid: Option<u32>, pub error: Option<String> }
#[derive(Serialize)]
pub struct RuntimeInfo { pub core_path: String, pub cli_path: String, pub version: String, pub available: bool }
#[derive(Default)]
pub struct RuntimeProcesses { children: HashMap<String, Child>, last_error: HashMap<String, String> }

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
fn get_instance_state(id: String, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> InstanceState {
    let mut p = state.lock().unwrap();
    let running = p.children.contains_key(&id);
    if !running {
        // Child exited on its own (e.g. port conflict): surface the failure.
        if let Some(err) = p.last_error.remove(&id) {
            return InstanceState { id, status: InstanceStatus::Failed, pid: None, error: Some(err) };
        }
        if let Some(mut child) = p.children.remove(&id) {
            let _ = child.wait();
        }
    }
    InstanceState { id, status: if running { InstanceStatus::Running } else { InstanceStatus::Stopped }, pid: None, error: None }
}
#[tauri::command]
fn start_instance(id: String, config: String, rpc_portal: Option<String>, dir_override: Option<String>, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> Result<InstanceState, String> {
    let (core, _) = paths(dir_override);
    if !core.exists() { return Err(format!("EasyTier core not found: {}", core.display())); }
    let config_path = std::env::temp_dir().join(format!("easytier-{}.toml", id));
    fs::write(&config_path, config).map_err(|e| e.to_string())?;
    let mut p = state.lock().map_err(|_| "runtime state unavailable".to_string())?;
    if p.children.contains_key(&id) { return Ok(InstanceState { id, status: InstanceStatus::Running, pid: None, error: None }); }
    let mut cmd = Command::new(core);
    cmd.arg("--config-file").arg(&config_path).stdin(Stdio::null());
    if let Some(portal) = rpc_portal { cmd.arg("--rpc-portal").arg(portal); }
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    p.children.insert(id.clone(), child);
    Ok(InstanceState { id, status: InstanceStatus::Running, pid: Some(pid), error: None })
}
#[tauri::command]
fn wait_for_exit(id: String, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> Result<InstanceState, String> {
    // Blocks until the child exits or a short grace period passes; used by the
    // UI right after start to detect instant failures like port conflicts.
    let mut p = state.lock().map_err(|_| "runtime state unavailable".to_string())?;
    if let Some(child) = p.children.get_mut(&id) {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut child = p.children.remove(&id).unwrap();
                let _ = child.wait();
                let err = format!("core 进程启动后立即退出（exit code: {}）。常见原因：监听器端口被占用（多实例需使用不同 listener 端口）、配置校验失败或缺少管理员权限。", status.code().unwrap_or(-1));
                p.last_error.insert(id.clone(), err.clone());
                return Ok(InstanceState { id, status: InstanceStatus::Failed, pid: None, error: Some(err) });
            }
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(InstanceState { id, status: InstanceStatus::Running, pid: None, error: None })
}
#[tauri::command]
fn stop_instance(id: String, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> Result<InstanceState, String> { let mut p = state.lock().map_err(|_| "runtime state unavailable".to_string())?; if let Some(mut child) = p.children.remove(&id) { child.kill().map_err(|e| e.to_string())?; let _ = child.wait(); } Ok(InstanceState { id, status: InstanceStatus::Stopped, pid: None, error: None }) }
#[tauri::command]
fn is_port_in_use(port: u16) -> bool { std::net::TcpListener::bind(("0.0.0.0", port)).is_err() }
#[tauri::command]
fn run_cli(args: Vec<String>, runtime_dir: Option<String>) -> Result<String, String> {
    let (_, cli) = paths(runtime_dir);
    let mut child = Command::new(cli).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    // The CLI retries internally when the RPC portal is unreachable and can
    // hang for a long time; cap each query so dead cores cannot pile up
    // cli processes.
    match child.wait_timeout(std::time::Duration::from_secs(5)) {
        Ok(Some(status)) => {
            let o = child.wait_with_output().map_err(|e| e.to_string())?;
            if status.success() { String::from_utf8(o.stdout).map_err(|e| e.to_string()) } else { Err(String::from_utf8_lossy(&o.stderr).to_string()) }
        }
        Ok(None) => { let _ = child.kill(); let _ = child.wait(); Err("状态查询超时（CLI 5 秒内未响应，核心可能已退出）".into()) }
        Err(e) => Err(e.to_string()),
    }
}
pub fn run() { tauri::Builder::default().manage(Mutex::new(RuntimeProcesses::default())).invoke_handler(tauri::generate_handler![detect_runtime, get_instance_state, start_instance, wait_for_exit, stop_instance, run_cli, is_port_in_use]).run(tauri::generate_context!()).expect("error while running tauri application"); }
