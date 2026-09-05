#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod config_store;
mod ipc;
mod kernel_updater;
mod runtime_manager;
use kernel_updater::KernelUpdateInfo;
#[cfg(windows)]
use named_pipe::PipeClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
};
use tauri::{image::Image, Manager, WindowEvent};
use wait_timeout::ChildExt;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}
#[derive(Clone, Serialize)]
pub struct InstanceState {
    pub id: String,
    pub status: InstanceStatus,
    pub pid: Option<u32>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct RuntimeInfo {
    pub core_path: String,
    pub cli_path: String,
    pub version: String,
    pub available: bool,
}
#[derive(Default)]
pub struct RuntimeProcesses {
    pub children: HashMap<String, Child>,
    pub last_error: HashMap<String, String>,
    pub logs: HashMap<String, runtime_manager::SharedLog>,
}

#[derive(Serialize)]
struct NetworkLogs {
    instance_id: String,
    text: String,
}

#[derive(Clone, Deserialize)]
struct RestartInstance {
    id: String,
    config: String,
    rpc_port: u16,
}

#[derive(Default)]
struct KernelUpdateLock(Mutex<()>);

fn runtime_dir(runtime_dir: Option<String>) -> PathBuf {
    if let Some(d) = runtime_dir {
        return PathBuf::from(d);
    }
    let mut candidates = vec![
        PathBuf::from("core"),
        PathBuf::from("../core"),
        PathBuf::from("../../core"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("core"));
            candidates.push(dir.join("../core"));
            candidates.push(dir.join("../../../core"));
            candidates.push(dir.join("resources/core"));
            candidates.push(dir.join("../resources/core"));
        }
    }
    for c in candidates {
        if c.join("easytier-core.exe").exists() {
            return c;
        }
    }
    PathBuf::from("core")
}
fn paths(dir_override: Option<String>) -> (PathBuf, PathBuf) {
    let d = runtime_dir(dir_override);
    (d.join("easytier-core.exe"), d.join("easytier-cli.exe"))
}
#[derive(Serialize)]
struct ServiceInstallation {
    installed: bool,
    running: bool,
    message: Option<String>,
}

fn service_query() -> ServiceInstallation {
    #[cfg(windows)]
    {
        let output = Command::new("sc.exe")
            .args(["query", "EasyTierService"])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                let running = text.contains("RUNNING") || text.contains("START_PENDING");
                ServiceInstallation {
                    installed: true,
                    running,
                    message: None,
                }
            }
            Ok(_) => ServiceInstallation {
                installed: false,
                running: false,
                message: None,
            },
            Err(error) => ServiceInstallation {
                installed: false,
                running: false,
                message: Some(format!("无法查询服务：{error}")),
            },
        }
    }
    #[cfg(not(windows))]
    {
        ServiceInstallation {
            installed: false,
            running: false,
            message: Some("Windows 服务不可用".into()),
        }
    }
}

#[tauri::command]
fn query_service_installation() -> ServiceInstallation {
    service_query()
}

#[tauri::command]
fn detect_runtime(runtime_dir: Option<String>) -> RuntimeInfo {
    let (core, cli) = paths(runtime_dir);
    let mut command = Command::new(&core);
    command
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let version = command
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into())
        .trim()
        .to_string();
    RuntimeInfo {
        core_path: core.display().to_string(),
        cli_path: cli.display().to_string(),
        version,
        available: core.exists() && cli.exists(),
    }
}
#[tauri::command]
fn get_instance_state(
    id: String,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
) -> InstanceState {
    let mut p = state.lock().unwrap();
    let running = p.children.contains_key(&id);
    if !running {
        // Child exited on its own (e.g. port conflict): surface the failure.
        if let Some(err) = p.last_error.remove(&id) {
            return InstanceState {
                id,
                status: InstanceStatus::Failed,
                pid: None,
                error: Some(err),
            };
        }
        if let Some(mut child) = p.children.remove(&id) {
            let _ = child.wait();
        }
    }
    InstanceState {
        id,
        status: if running {
            InstanceStatus::Running
        } else {
            InstanceStatus::Stopped
        },
        pid: None,
        error: None,
    }
}
#[tauri::command]
fn get_network_logs(id: String, state: tauri::State<'_, Mutex<RuntimeProcesses>>) -> NetworkLogs {
    let p = state.lock().unwrap();
    NetworkLogs {
        instance_id: id.clone(),
        text: p
            .logs
            .get(&id)
            .map(|log| log.lock().unwrap().text())
            .unwrap_or_default(),
    }
}
#[tauri::command]
fn start_instance(
    id: String,
    config: String,
    rpc_portal: Option<String>,
    dir_override: Option<String>,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
) -> Result<InstanceState, String> {
    let (core, _) = paths(dir_override);
    if !core.exists() {
        return Err(format!("EasyTier core not found: {}", core.display()));
    }
    let config_path = std::env::temp_dir().join(format!("easytier-{}.toml", id));
    fs::write(&config_path, config).map_err(|e| e.to_string())?;
    let mut p = state
        .lock()
        .map_err(|_| "runtime state unavailable".to_string())?;
    if p.children.contains_key(&id) {
        return Ok(InstanceState {
            id,
            status: InstanceStatus::Running,
            pid: None,
            error: None,
        });
    }
    let mut cmd = Command::new(core);
    cmd.arg("--config-file")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(portal) = rpc_portal {
        cmd.arg("--rpc-portal").arg(portal);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    let log = p
        .logs
        .entry(id.clone())
        .or_insert_with(|| std::sync::Arc::new(Mutex::new(runtime_manager::LogBuffer::default())))
        .clone();
    if let Some(stdout) = child.stdout.take() {
        runtime_manager::spawn_log_reader(stdout, log.clone(), false);
    }
    if let Some(stderr) = child.stderr.take() {
        runtime_manager::spawn_log_reader(stderr, log, true);
    }
    p.children.insert(id.clone(), child);
    Ok(InstanceState {
        id,
        status: InstanceStatus::Running,
        pid: Some(pid),
        error: None,
    })
}
#[tauri::command]
fn wait_for_exit(
    id: String,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
) -> Result<InstanceState, String> {
    // Blocks until the child exits or a short grace period passes; used by the
    // UI right after start to detect instant failures like port conflicts.
    let mut p = state
        .lock()
        .map_err(|_| "runtime state unavailable".to_string())?;
    if let Some(child) = p.children.get_mut(&id) {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut child = p.children.remove(&id).unwrap();
                let _ = child.wait();
                let err = format!("core 进程启动后立即退出（exit code: {}）。常见原因：监听器端口被占用（多实例需使用不同 listener 端口）、配置校验失败或缺少管理员权限。", status.code().unwrap_or(-1));
                p.last_error.insert(id.clone(), err.clone());
                return Ok(InstanceState {
                    id,
                    status: InstanceStatus::Failed,
                    pid: None,
                    error: Some(err),
                });
            }
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(InstanceState {
        id,
        status: InstanceStatus::Running,
        pid: None,
        error: None,
    })
}
#[tauri::command]
fn stop_instance(
    id: String,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
) -> Result<InstanceState, String> {
    let mut p = state
        .lock()
        .map_err(|_| "runtime state unavailable".to_string())?;
    if let Some(mut child) = p.children.remove(&id) {
        child.kill().map_err(|e| e.to_string())?;
        let _ = child.wait();
    }
    Ok(InstanceState {
        id,
        status: InstanceStatus::Stopped,
        pid: None,
        error: None,
    })
}
#[tauri::command]
fn check_kernel_update(proxy: Option<String>) -> Result<KernelUpdateInfo, String> {
    kernel_updater::check(proxy.as_deref().unwrap_or("direct"))
}

#[tauri::command]
fn update_kernel(
    app: tauri::AppHandle,
    proxy: String,
    instances: Vec<RestartInstance>,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
    update_lock: tauri::State<'_, KernelUpdateLock>,
) -> Result<KernelUpdateInfo, String> {
    let _guard = update_lock
        .0
        .lock()
        .map_err(|_| "内核更新锁不可用".to_string())?;
    let runtime = runtime_dir(None);
    let parent = runtime.parent().ok_or("无法确定 core 目录")?.to_path_buf();
    kernel_updater::emit_progress(&app, "checking", "正在准备内核更新", 0, None, None, None);
    let staged = match kernel_updater::download_and_stage(&app, proxy.as_str(), &runtime) {
        Ok(path) => path,
        Err(error) => {
            kernel_updater::emit_progress(
                &app,
                "failed",
                "内核下载或校验失败",
                0,
                None,
                None,
                Some(error.clone()),
            );
            return Err(error);
        }
    };
    kernel_updater::emit_progress(
        &app,
        "stopping",
        "正在停止运行中的网络",
        0,
        None,
        None,
        None,
    );
    {
        let mut processes = state
            .lock()
            .map_err(|_| "runtime state unavailable".to_string())?;
        for instance in &instances {
            if let Some(mut child) = processes.children.remove(&instance.id) {
                child.kill().map_err(|e| e.to_string())?;
                let _ = child.wait();
            }
        }
    }
    kernel_updater::emit_progress(
        &app,
        "installing",
        "正在替换 EasyTier 内核",
        0,
        None,
        None,
        None,
    );
    let backup = match kernel_updater::install(&runtime, &staged) {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&parent.join(staged.file_name().unwrap_or_default()));
            kernel_updater::emit_progress(
                &app,
                "failed",
                "内核替换失败",
                0,
                None,
                None,
                Some(error.clone()),
            );
            return Err(error);
        }
    };
    kernel_updater::emit_progress(&app, "restarting", "正在恢复原有网络", 0, None, None, None);
    for instance in instances {
        let config_path = std::env::temp_dir().join(format!("easytier-{}.toml", instance.id));
        if let Err(error) = std::fs::write(&config_path, &instance.config).and_then(|_| {
            let core = runtime.join("easytier-core.exe");
            let mut cmd = Command::new(core);
            cmd.arg("--config-file")
                .arg(config_path)
                .arg("--rpc-portal")
                .arg(format!("127.0.0.1:{}", instance.rpc_port))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(0x08000000);
            let mut child = cmd.spawn().map_err(std::io::Error::other)?;
            let mut processes = state
                .lock()
                .map_err(|_| std::io::Error::other("runtime state unavailable"))?;
            let log = processes
                .logs
                .entry(instance.id.clone())
                .or_insert_with(|| {
                    std::sync::Arc::new(Mutex::new(runtime_manager::LogBuffer::default()))
                })
                .clone();
            if let Some(stdout) = child.stdout.take() {
                runtime_manager::spawn_log_reader(stdout, log.clone(), false);
            }
            if let Some(stderr) = child.stderr.take() {
                runtime_manager::spawn_log_reader(stderr, log, true);
            }
            processes.children.insert(instance.id.clone(), child);
            Ok(())
        }) {
            kernel_updater::emit_progress(
                &app,
                "failed",
                "部分网络恢复失败",
                0,
                None,
                None,
                Some(error.to_string()),
            );
        }
    }
    let _ = backup;
    kernel_updater::emit_progress(
        &app,
        "completed",
        "EasyTier 内核更新完成",
        1,
        Some(1),
        None,
        None,
    );
    Ok(KernelUpdateInfo {
        current_version: kernel_updater::CURRENT_VERSION.to_string(),
        latest_version: None,
        asset_name: None,
        update_available: false,
        error: None,
    })
}

#[tauri::command]
fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        use std::mem::size_of;
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = 0;
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut size = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            );
            let _ = windows_sys::Win32::Foundation::CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[tauri::command]
fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("0.0.0.0", port)).is_err()
}

#[tauri::command]
fn run_cli(args: Vec<String>, runtime_dir: Option<String>) -> Result<String, String> {
    let (_, cli) = paths(runtime_dir);
    let mut child = Command::new(cli);
    child
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    child.creation_flags(0x08000000);
    let mut child = child.spawn().map_err(|e| e.to_string())?;
    match child.wait_timeout(std::time::Duration::from_secs(5)) {
        Ok(Some(status)) => {
            let o = child.wait_with_output().map_err(|e| e.to_string())?;
            if status.success() {
                String::from_utf8(o.stdout).map_err(|e| e.to_string())
            } else {
                Err(String::from_utf8_lossy(&o.stderr).to_string())
            }
        }
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            Err("状态查询超时（CLI 5 秒内未响应，核心可能已退出）".into())
        }
        Err(e) => Err(e.to_string()),
    }
}
#[tauri::command]
fn service_request(request: Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(&request).map_err(|e| format!("invalid_request: {e}"))?;
    if bytes.len() > ipc::MAX_MESSAGE {
        return Err("message_too_large".into());
    }
    #[cfg(windows)]
    {
        let mut pipe = PipeClient::connect(r"\\.\pipe\EasyTierService")
            .map_err(|e| format!("service_unavailable: {e}"))?;
        pipe.write_all(&bytes)
            .and_then(|_| pipe.write_all(b"\n"))
            .map_err(|e| format!("service_unavailable: {e}"))?;
        let mut line = Vec::new();
        BufReader::new(pipe)
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("service_unavailable: {e}"))?;
        if line.len() > ipc::MAX_MESSAGE {
            return Err("message_too_large".into());
        }
        serde_json::from_slice(&line).map_err(|e| format!("invalid_response: {e}"))
    }
    #[cfg(not(windows))]
    {
        let _ = request;
        Err("service_unavailable: Windows service is unavailable on this platform".into())
    }
}

#[cfg(windows)]
fn scm_command(args: &[&str], code: &str) -> Result<String, String> {
    let output = Command::new("sc.exe")
        .args(args)
        .output()
        .map_err(|e| format!("{code}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "{code}: {}",
            if detail.is_empty() {
                "SCM operation failed"
            } else {
                &detail
            }
        ))
    }
}

#[tauri::command]
fn install_service() -> Result<String, String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| format!("service_path_invalid: {e}"))?;
        let dir = exe
            .parent()
            .ok_or_else(|| "service_path_invalid: executable has no parent".to_string())?;
        let candidates = [
            dir.join("easytier-service.exe"),
            dir.join("resources/easytier-service.exe"),
        ];
        let service = candidates
            .iter()
            .find(|path| path.exists())
            .ok_or_else(|| {
                format!(
                    "service_exe_not_found: {}",
                    candidates
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        let current_sid = std::env::var("USERNAME").ok().and_then(|name| {
            let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "(New-Object System.Security.Principal.NTAccount($env:USERNAME)).Translate([System.Security.Principal.SecurityIdentifier]).Value"]).output().ok()?;
            if output.status.success() { Some(String::from_utf8_lossy(&output.stdout).trim().to_string()) } else { let _ = name; None }
        }).filter(|sid| sid.starts_with("S-"));
        let trusted_sid = current_sid.ok_or_else(|| {
            "service_install_failed: cannot resolve interactive user SID".to_string()
        })?;
        let service_path = service.display().to_string().replace('"', "\\\"");
        let command = format!("$ErrorActionPreference='Stop'; $p='{}'; $bin='\\\"'+$p+'\\\" --interactive-user-sid={}' ; & sc.exe stop EasyTierService 2>$null; & sc.exe create EasyTierService binPath= $bin start= auto DisplayName= 'EasyTier Service'; if ($LASTEXITCODE -ne 0) {{ & sc.exe config EasyTierService binPath= $bin start= auto; if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }} }}; & sc.exe description EasyTierService 'EasyTier background service'; & sc.exe start EasyTierService; exit $LASTEXITCODE", service_path, trusted_sid);
        let status = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &format!("Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList '-NoProfile','-NonInteractive','-Command','{}'", command.replace('\'', "''"))])
            .status().map_err(|e| format!("service_install_failed: {e}"))?;
        if status.success() {
            Ok("后台服务已安装并启动".into())
        } else {
            Err("service_install_failed: UAC 被拒绝或 SCM 操作失败".into())
        }
    }
    #[cfg(not(windows))]
    {
        Err("service_unavailable: Windows SCM is unavailable on this platform".into())
    }
}

#[tauri::command]
fn start_service() -> Result<String, String> {
    #[cfg(windows)]
    {
        let command = "Start-Service -Name EasyTierService";
        let status = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &format!("Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList '-NoProfile','-NonInteractive','-Command','{}'", command)]).status().map_err(|e| format!("service_start_failed: {e}"))?;
        if status.success() {
            Ok("后台服务已启动".into())
        } else {
            Err("service_start_failed: UAC 被拒绝或服务启动失败".into())
        }
    }
    #[cfg(not(windows))]
    {
        Err("service_unavailable: Windows SCM is unavailable on this platform".into())
    }
}

#[tauri::command]
fn repair_service() -> Result<String, String> {
    install_service()
}

pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(RuntimeProcesses::default()))
        .manage(KernelUpdateLock::default())
        .setup(|app| {
            let show =
                tauri::menu::MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
            let quit =
                tauri::menu::MenuItem::with_id(app, "quit", "退出 EasyTier", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            let tray_icon = Image::from_bytes(include_bytes!("../icons/icon.png"))
                .map_err(|e| e.to_string())?;
            tauri::tray::TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .tooltip("EasyTier")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            query_service_installation,
            detect_runtime,
            get_instance_state,
            get_network_logs,
            is_elevated,
            start_instance,
            wait_for_exit,
            check_kernel_update,
            update_kernel,
            stop_instance,
            is_port_in_use,
            run_cli,
            service_request,
            install_service,
            start_service,
            repair_service
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
