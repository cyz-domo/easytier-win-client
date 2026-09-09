#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod autostart;
mod config_store;
mod ipc;
mod kernel_updater;
pub mod power_monitor;
mod portal_args;
mod remote_rpc;
mod runtime_manager;
use kernel_updater::KernelUpdateInfo;
#[cfg(windows)]
use named_pipe::PipeClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Emitter;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
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
    #[serde(default)]
    remote_manage_enabled: bool,
    #[serde(default)]
    rpc_whitelist_cidrs: Vec<String>,
}

#[derive(Default)]
struct KernelUpdateLock(tokio::sync::Mutex<()>);

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
            candidates.push(dir.join("resources"));
            candidates.push(dir.join("../resources"));
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
            .creation_flags(0x08000000)
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
fn get_client_autostart() -> Result<autostart::AutoStartStatus, String> {
    autostart::get_status()
}

#[tauri::command]
fn set_client_autostart(enabled: bool, start_minimized: bool) -> Result<(), String> {
    autostart::set_status(enabled, start_minimized)
}

#[tauri::command]
async fn query_service_installation() -> Result<ServiceInstallation, String> {
    tokio::task::spawn_blocking(service_query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn detect_runtime(runtime_dir: Option<String>) -> Result<RuntimeInfo, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())
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
    remote_manage_enabled: Option<bool>,
    rpc_whitelist_cidrs: Option<Vec<String>>,
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
    if remote_manage_enabled == Some(true) {
        // Exposed portal replaces the loopback one; reuse the caller-supplied
        // port. Loopback plus user CIDRs are merged by the shared builder.
        let port = rpc_portal
            .as_deref()
            .and_then(|s| s.rsplit(':').next())
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(15888);
        let args = remote_rpc::build_rpc_portal_args(
            true,
            port,
            rpc_whitelist_cidrs.as_deref().unwrap_or(&[]),
        );
        cmd.args(args);
    } else if let Some(portal) = rpc_portal.as_deref() {
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
                if let Some(mut c) = p.children.remove(&id) {
                    let _ = c.wait();
                }
                std::thread::sleep(std::time::Duration::from_millis(60));
                let log_text = p
                    .logs
                    .get(&id)
                    .and_then(|l| l.lock().ok())
                    .map(|guard| guard.text())
                    .unwrap_or_default();
                let filtered: Vec<&str> = log_text
                    .lines()
                    .filter(|line| {
                        let l = line.to_lowercase();
                        l.contains("error")
                            || l.contains("fail")
                            || l.contains("os error")
                            || l.contains("caused by")
                            || l.contains("10048")
                            || l.contains("refused")
                            || l.contains("denied")
                            || l.contains("warn")
                    })
                    .collect();
                let detail = if !filtered.is_empty() {
                    filtered.join("\n")
                } else if !log_text.trim().is_empty() {
                    log_text
                        .lines()
                        .rev()
                        .take(5)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                };

                let err = if !detail.is_empty() {
                    format!(
                        "core 进程启动后立即退出（exit code: {}）。错误详情：\n{}",
                        status.code().unwrap_or(-1),
                        detail
                    )
                } else {
                    format!(
                        "core 进程启动后立即退出（exit code: {}）。常见原因：监听器端口被占用（多实例需使用不同 listener 端口）、配置校验失败或缺少管理员权限。",
                        status.code().unwrap_or(-1)
                    )
                };
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
    // The staged config is rewritten on every start; dropping it here keeps
    // the temp dir from accumulating one file per stopped instance.
    let _ = fs::remove_file(std::env::temp_dir().join(format!("easytier-{id}.toml")));
    Ok(InstanceState {
        id,
        status: InstanceStatus::Stopped,
        pid: None,
        error: None,
    })
}

#[tauri::command]
fn drop_status_endpoint(port: u16) {
    remote_rpc::drop_local_endpoint(port);
}

/// Release per-instance backend state (log buffer, last error) after the
/// frontend deletes the instance; stopped instances keep their logs so a
/// crash can still be diagnosed, but a deleted one should cost nothing.
#[tauri::command]
fn drop_instance_state(
    id: String,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
) -> Result<(), String> {
    let mut p = state
        .lock()
        .map_err(|_| "runtime state unavailable".to_string())?;
    p.logs.remove(&id);
    p.last_error.remove(&id);
    let _ = fs::remove_file(std::env::temp_dir().join(format!("easytier-{id}.toml")));
    Ok(())
}
#[tauri::command]
async fn check_kernel_update(proxy: Option<String>) -> Result<KernelUpdateInfo, String> {
    tokio::task::spawn_blocking(move || {
        let (core, _) = paths(None);
        kernel_updater::check(proxy.as_deref().unwrap_or("direct"), Some(&core))
    })
    .await
    .map_err(|e| format!("检查更新任务异常：{e}"))?
}

#[tauri::command]
async fn list_kernel_versions(proxy: Option<String>) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        kernel_updater::list_available_versions(proxy.as_deref().unwrap_or("direct"))
    })
    .await
    .map_err(|e| format!("获取版本列表任务异常：{e}"))?
}

#[tauri::command]
fn cancel_kernel_update() {
    kernel_updater::cancel_update();
}

#[tauri::command]
async fn update_kernel(
    app: tauri::AppHandle,
    proxy: String,
    target_version: Option<String>,
    instances: Vec<RestartInstance>,
    state: tauri::State<'_, Mutex<RuntimeProcesses>>,
    update_lock: tauri::State<'_, KernelUpdateLock>,
) -> Result<KernelUpdateInfo, String> {
    let _guard = update_lock
        .0
        .try_lock()
        .map_err(|_| "内核更新正在进行中，请勿重复操作".to_string())?;
    let runtime = runtime_dir(None);
    let parent = runtime.parent().ok_or("无法确定 core 目录")?.to_path_buf();
    kernel_updater::emit_progress(&app, "checking", "正在准备内核更新", 0, None, None, None);

    let app_handle = app.clone();
    let proxy_arg = proxy.clone();
    let target_ver_arg = target_version.clone();
    let runtime_arg = runtime.clone();
    let staged_res = tokio::task::spawn_blocking(move || {
        kernel_updater::download_and_stage(&app_handle, proxy_arg.as_str(), target_ver_arg.as_deref(), &runtime_arg)
    })
    .await
    .map_err(|e| format!("下载线程执行异常：{e}"))?;

    let staged = match staged_res {
        Ok(path) => path,
        Err(error) => {
            let is_cancelled = kernel_updater::is_cancelled() || error.contains("取消");
            kernel_updater::emit_progress(
                &app,
                if is_cancelled { "cancelled" } else { "failed" },
                if is_cancelled { "内核更新已取消" } else { "内核下载或校验失败" },
                0,
                None,
                None,
                Some(error.clone()),
            );
            return Err(error);
        }
    };

    if kernel_updater::is_cancelled() {
        let _ = std::fs::remove_dir_all(&parent.join(staged.file_name().unwrap_or_default()));
        kernel_updater::emit_progress(&app, "cancelled", "内核更新已取消", 0, None, None, None);
        return Err("用户取消了内核更新".into());
    }

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
            remote_rpc::drop_local_endpoint(instance.rpc_port);
            let _ = fs::remove_file(std::env::temp_dir().join(format!("easytier-{}.toml", instance.id)));
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
                .args(remote_rpc::build_rpc_portal_args(
                    instance.remote_manage_enabled,
                    instance.rpc_port,
                    &instance.rpc_whitelist_cidrs,
                ))
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
    let updated_version = kernel_updater::detect_current_version(Some(&runtime.join("easytier-core.exe")));
    Ok(KernelUpdateInfo {
        current_version: updated_version,
        latest_version: None,
        asset_name: None,
        update_available: false,
        available_versions: None,
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
        || std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
        || std::net::UdpSocket::bind(("0.0.0.0", port)).is_err()
}

#[tauri::command]
fn restart_as_admin(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;

        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_wide: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let verb_wide: Vec<u16> = OsStr::new("runas").encode_wide().chain(std::iter::once(0)).collect();

        unsafe {
            let res = ShellExecuteW(
                0,
                verb_wide.as_ptr(),
                exe_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1, // SW_SHOWNORMAL
            );
            if (res as isize) <= 32 {
                return Err(format!("无法以管理员权限启动（错误码: {}）", res as isize));
            }
        }
        app.exit(0);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅支持 Windows 平台".to_string())
    }
}

#[cfg(windows)]
mod job_object {
    //! Puts short-lived child processes (easytier-cli) into a kill-on-close
    //! job so a GUI crash/exit can never leave them behind.
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub fn kill_on_close(process_handle: windows_sys::Win32::Foundation::HANDLE) {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job == 0 {
                return;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0 {
                windows_sys::Win32::Foundation::CloseHandle(job);
                return;
            }
            let _ = AssignProcessToJobObject(job, process_handle);
            // Intentionally leak the job handle: it is closed when our process
            // exits, which kills every assigned child.
        }
    }

    /// Assigns the current process to a kill-on-close Job Object so all child
    /// processes (WebView2 browser, renderers, GPU process, CLI) are guaranteed
    /// to be terminated by Windows kernel when this process terminates.
    pub fn init_global_job() {
        unsafe {
            use windows_sys::Win32::System::Threading::GetCurrentProcess;
            kill_on_close(GetCurrentProcess());
        }
    }
}

#[tauri::command]
async fn run_cli(args: Vec<String>, runtime_dir: Option<String>) -> Result<String, String> {
    let (_, cli) = paths(runtime_dir);
    let mut child = Command::new(cli);
    child
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    child.creation_flags(0x08000000);
    let mut child = child.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    job_object::kill_on_close(child.as_raw_handle() as _);
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
async fn service_request(request: Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(&request).map_err(|e| format!("invalid_request: {e}"))?;
    if bytes.len() > ipc::MAX_MESSAGE {
        return Err("message_too_large".into());
    }
    #[cfg(windows)]
    {
        let task = tokio::task::spawn_blocking(move || {
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
        });
        match tokio::time::timeout(std::time::Duration::from_millis(15000), task).await {
            Ok(Ok(res)) => res,
            Ok(Err(join_err)) => Err(format!("service_request error: {join_err}")),
            Err(_) => Err("service_unavailable: request timed out".into()),
        }
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
        .creation_flags(0x08000000)
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

fn get_current_user_sid() -> Option<String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
        use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
        use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut token = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut needed = 0;
        let _ = GetTokenInformation(token, TokenUser, core::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        if GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as _, needed, &mut needed) == 0 {
            CloseHandle(token);
            return None;
        }
        CloseHandle(token);
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = core::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return None;
        }
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text as *mut _);
        Some(sid)
    }
    #[cfg(not(windows))]
    None
}

#[tauri::command]
async fn install_service() -> Result<String, String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| format!("service_path_invalid: {e}"))?;
        let dir = exe
            .parent()
            .ok_or_else(|| "service_path_invalid: executable has no parent".to_string())?;
        let candidates = [
            dir.join("easytier-service.exe"),
            dir.join("service/easytier-service.exe"),
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

        let trusted_sid = get_current_user_sid().unwrap_or_else(|| "S-1-5-32-544".to_string());
        let service_path = service.display().to_string();

        // Persist the SID where the service can read it directly
        let sid_dir = config_store::data_dir();
        let _ = std::fs::create_dir_all(&sid_dir);
        let _ = std::fs::write(sid_dir.join("interactive-user.sid"), &trusted_sid);
        let _ = std::fs::write(dir.join("interactive-user.sid"), &trusted_sid);

        // Also generate 1-click install/uninstall batch scripts in the application directory for convenience
        let install_bat = format!(
            "@echo off\r\n\
             chcp 65001 >nul\r\n\
             cd /d \"%~dp0\"\r\n\
             echo 正在安装 EasyTierService 后台服务...\r\n\
             sc.exe stop EasyTierService >nul 2>&1\r\n\
             sc.exe delete EasyTierService >nul 2>&1\r\n\
             sc.exe create EasyTierService binPath= \"\\\"{}\\\" --interactive-user-sid={}\" start= auto DisplayName= \"EasyTier Service\"\r\n\
             sc.exe description EasyTierService \"EasyTier background service\"\r\n\
             sc.exe start EasyTierService\r\n\
             echo 服务状态：\r\n\
             sc.exe query EasyTierService\r\n\
             pause\r\n",
            service_path, trusted_sid
        );
        let _ = std::fs::write(dir.join("install-service.bat"), install_bat);

        let uninstall_bat =
            "@echo off\r\n\
             chcp 65001 >nul\r\n\
             echo 正在停止并卸载 EasyTierService 后台服务...\r\n\
             sc.exe stop EasyTierService >nul 2>&1\r\n\
             sc.exe delete EasyTierService\r\n\
             echo 卸载完成。\r\n\
             pause\r\n";
        let _ = std::fs::write(dir.join("uninstall-service.bat"), uninstall_bat);

        if is_elevated() {
            // Already elevated: execute directly without UAC prompt or PowerShell nesting
            let _ = Command::new("sc.exe")
                .args(["stop", "EasyTierService"])
                .creation_flags(0x08000000)
                .output();
            let sc_create_cmd = format!(
                "create EasyTierService binPath= \"\\\"{}\\\" --interactive-user-sid={}\" start= auto DisplayName= \"EasyTier Service\"",
                service_path, trusted_sid
            );
            let mut create_cmd = Command::new("sc.exe");
            create_cmd.raw_arg(&sc_create_cmd).creation_flags(0x08000000);
            let create_res = create_cmd.output();
            if let Ok(res) = create_res {
                if !res.status.success() {
                    let sc_config_cmd = format!(
                        "config EasyTierService binPath= \"\\\"{}\\\" --interactive-user-sid={}\" start= auto",
                        service_path, trusted_sid
                    );
                    let mut config_cmd = Command::new("sc.exe");
                    config_cmd.raw_arg(&sc_config_cmd).creation_flags(0x08000000);
                    let _ = config_cmd.output();
                }
            }
            let _ = Command::new("sc.exe")
                .args(["description", "EasyTierService", "EasyTier background service"])
                .creation_flags(0x08000000)
                .output();
            let _ = Command::new("sc.exe")
                .args(["start", "EasyTierService"])
                .creation_flags(0x08000000)
                .output();
        } else {
            // Not elevated: trigger native UAC execution via batch file in %TEMP%
            let temp_bat = std::env::temp_dir().join("install_easytier_service.bat");
            let bat_content = format!(
                "@echo off\r\n\
                 chcp 65001 >nul\r\n\
                 sc.exe stop EasyTierService >nul 2>&1\r\n\
                 sc.exe create EasyTierService binPath= \"\\\"{}\\\" --interactive-user-sid={}\" start= auto DisplayName= \"EasyTier Service\"\r\n\
                 if %ERRORLEVEL% NEQ 0 (\r\n\
                     sc.exe config EasyTierService binPath= \"\\\"{}\\\" --interactive-user-sid={}\" start= auto\r\n\
                 )\r\n\
                 sc.exe description EasyTierService \"EasyTier background service\" >nul 2>&1\r\n\
                 sc.exe start EasyTierService >nul 2>&1\r\n",
                service_path, trusted_sid, service_path, trusted_sid
            );
            std::fs::write(&temp_bat, bat_content).map_err(|e| format!("write temp bat failed: {e}"))?;

            let ps_cmd = format!(
                "$proc = Start-Process cmd.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '/c','\"{}\"'; exit $proc.ExitCode",
                temp_bat.display().to_string().replace('\'', "''")
            );
            let _ = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &ps_cmd])
                .creation_flags(0x08000000)
                .status();
        }

        // Adaptive polling: wait up to 12 seconds for UAC approval and SCM registration
        let mut installed = false;
        for _ in 0..12 {
            let query = service_query();
            if query.installed {
                installed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
        if installed {
            Ok("后台服务已安装并启动".into())
        } else {
            Err("服务安装未能生效。若在虚拟机中受权限限制，请以管理员身份右键运行程序目录下的 install-service.bat 手动安装".into())
        }
    }
    #[cfg(not(windows))]
    {
        Err("service_unavailable: Windows SCM is unavailable on this platform".into())
    }
}

#[tauri::command]
async fn start_service() -> Result<String, String> {
    #[cfg(windows)]
    {
        // The installer grants the interactive user start/stop rights via
        // sdset, so try the direct path first (no UAC flash).
        let direct = Command::new("sc.exe")
            .args(["start", "EasyTierService"])
            .creation_flags(0x08000000)
            .output();
        if let Ok(output) = &direct {
            if output.status.success() {
                return Ok("后台服务已启动".into());
            }
        }
        let command = "Start-Service -Name EasyTierService";
        let status = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &format!("Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-Command','{}'", command)]).creation_flags(0x08000000).status().map_err(|e| format!("service_start_failed: {e}"))?;
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
async fn repair_service() -> Result<String, String> {
    install_service().await
}

/// Sweep orphaned CLI helpers, stop every running network (GUI children and
/// service instances), then exit. Used by both the tray quit action and the
/// main window close button.
fn quit_and_stop_networks(app: &tauri::AppHandle) {
    // 1. Hide main window immediately so the user sees instant feedback
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    // 2. Stop local processes
    if let Ok(mut p) = app.state::<Mutex<RuntimeProcesses>>().lock() {
        for (id, mut child) in p.children.drain() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(std::env::temp_dir().join(format!("easytier-{}.toml", id)));
        }
    }
    // 3. Fast cleanup of CLI
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "easytier-cli.exe"])
        .creation_flags(0x08000000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // 4. Send shutdown_service to service with short timeout
    #[cfg(windows)]
    {
        use std::io::{BufRead as _, Write as _};
        let _ = std::thread::spawn(|| {
            let request = serde_json::json!({
                "protocol_version": ipc::PROTOCOL_VERSION,
                "request_id": uuid::Uuid::new_v4().to_string(),
                "command": "shutdown_service",
                "payload": {},
            });
            if let Ok(bytes) = serde_json::to_vec(&request) {
                if let Ok(mut pipe) = PipeClient::connect(r"\\.\pipe\EasyTierService") {
                    let _ = pipe.write_all(&bytes).and_then(|_| pipe.write_all(b"\n"));
                    let mut line = Vec::new();
                    let _ = BufReader::new(pipe).read_until(b'\n', &mut line);
                }
            }
        }).join();

        // 5. Terminate any easytier-core.exe belonging to this installation
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "easytier-core.exe"])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // 6. Explicitly terminate any child processes (including WebView2 renderers & GPU processes)
        unsafe {
            use windows_sys::Win32::System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            };
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcessId, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
            };
            let my_pid = GetCurrentProcessId();
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                let mut entry: PROCESSENTRY32W = std::mem::zeroed();
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32FirstW(snapshot, &mut entry) != 0 {
                    loop {
                        if entry.th32ParentProcessID == my_pid {
                            let child_handle = OpenProcess(PROCESS_TERMINATE, 0, entry.th32ProcessID);
                            if child_handle != 0 {
                                TerminateProcess(child_handle, 1);
                                windows_sys::Win32::Foundation::CloseHandle(child_handle);
                            }
                        }
                        if Process32NextW(snapshot, &mut entry) == 0 {
                            break;
                        }
                    }
                }
                windows_sys::Win32::Foundation::CloseHandle(snapshot);
            }
        }
    }
    std::process::exit(0);
}

#[tauri::command]
async fn status_query(port: u16) -> Result<Value, String> {
    remote_rpc::local_status_query(port).await
}

#[tauri::command]
#[cfg(feature = "remote-rpc")]
async fn remote_config_discover(host: String, port: u16, virtual_ip: String) -> Result<Value, String> {
    let info = remote_rpc::discover_remote_instance(&host, port, &virtual_ip).await?;
    serde_json::to_value(info).map_err(|e| e.to_string())
}

#[tauri::command]
#[cfg(feature = "remote-rpc")]
async fn remote_config_load(host: String, port: u16, instance_id: String) -> Result<Value, String> {
    remote_rpc::load_remote_config(&host, port, &instance_id).await
}

#[tauri::command]
#[cfg(feature = "remote-rpc")]
async fn remote_config_patch(host: String, port: u16, instance_id: String, patch: Value) -> Result<Value, String> {
    remote_rpc::patch_remote_config(&host, port, &instance_id, patch).await
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TrayStatusPayload {
    pub running: bool,
    pub instance_id: Option<String>,
    pub network_name: Option<String>,
    pub virtual_ip: Option<String>,
    pub peer_count: usize,
    pub rx_speed: Option<String>,
    pub tx_speed: Option<String>,
}

static CURRENT_RUNNING_INSTANCE: Mutex<Option<String>> = Mutex::new(None);
static CURRENT_RUNNING_STATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
fn update_tray_status(app: tauri::AppHandle, payload: TrayStatusPayload) -> Result<(), String> {
    CURRENT_RUNNING_STATE.store(payload.running, std::sync::atomic::Ordering::SeqCst);
    if payload.running {
        if let Some(ref id) = payload.instance_id {
            if let Ok(mut g) = CURRENT_RUNNING_INSTANCE.lock() {
                *g = Some(id.clone());
            }
        }
    } else if let Ok(mut g) = CURRENT_RUNNING_INSTANCE.lock() {
        *g = None;
    }
    let status_text = if payload.running {
        format!(
            "🟢 状态: 运行中 ({})",
            payload.network_name.as_deref().unwrap_or("默认")
        )
    } else {
        "⚪ 状态: 已停止".to_string()
    };
    let status_item =
        tauri::menu::MenuItem::with_id(&app, "status_info", &status_text, false, None::<&str>)
            .map_err(|e| e.to_string())?;

    let ip_text = if let Some(ref ip) = payload.virtual_ip {
        format!("📋 虚拟 IP: {} (点击复制)", ip)
    } else {
        "📋 虚拟 IP: 未分配".to_string()
    };
    let ip_item = tauri::menu::MenuItem::with_id(
        &app,
        "copy_ip",
        &ip_text,
        payload.virtual_ip.is_some(),
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let peer_text = format!("🌐 在线节点: {} 个", payload.peer_count);
    let peer_item =
        tauri::menu::MenuItem::with_id(&app, "peers_info", &peer_text, false, None::<&str>)
            .map_err(|e| e.to_string())?;

    let speed_text = format!(
        "⚡ 瞬时流量: ↓ {}  ↑ {}",
        payload.rx_speed.as_deref().unwrap_or("0 B/s"),
        payload.tx_speed.as_deref().unwrap_or("0 B/s")
    );
    let speed_item =
        tauri::menu::MenuItem::with_id(&app, "speed_info", &speed_text, false, None::<&str>)
            .map_err(|e| e.to_string())?;

    let sep1 = tauri::menu::PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;

    let toggle_text = if payload.running {
        "⏹ 停止当前网络"
    } else {
        "▶ 启动网络"
    };
    let toggle = tauri::menu::MenuItem::with_id(&app, "toggle_network", toggle_text, true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let show =
        tauri::menu::MenuItem::with_id(&app, "show", "💻 打开主窗口", true, None::<&str>)
            .map_err(|e| e.to_string())?;

    let sep2 = tauri::menu::PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;

    let quit =
        tauri::menu::MenuItem::with_id(&app, "quit", "❌ 退出 EasyTier", true, None::<&str>)
            .map_err(|e| e.to_string())?;

    static LAST_MENU_KEY: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let menu_key = format!(
        "{}:{}:{}:{}",
        payload.running,
        payload.network_name.as_deref().unwrap_or(""),
        payload.virtual_ip.as_deref().unwrap_or(""),
        payload.peer_count
    );

    let mut key_guard = LAST_MENU_KEY.lock().map_err(|e| e.to_string())?;
    let menu_changed = *key_guard != menu_key;

    if let Some(tray) = app.tray_by_id("main-tray") {
        if menu_changed {
            let menu = tauri::menu::Menu::with_items(
                &app,
                &[
                    &status_item,
                    &ip_item,
                    &peer_item,
                    &sep1,
                    &toggle,
                    &show,
                    &sep2,
                    &quit,
                ],
            )
            .map_err(|e| e.to_string())?;
            let _ = tray.set_menu(Some(menu));
            *key_guard = menu_key;
        }

        let tooltip = if payload.running {
            format!(
                "EasyTier 运行中\nIP: {}\n{} 个节点在线\n↓ {}  ↑ {}",
                payload.virtual_ip.as_deref().unwrap_or("-"),
                payload.peer_count,
                payload.rx_speed.as_deref().unwrap_or("0 B/s"),
                payload.tx_speed.as_deref().unwrap_or("0 B/s")
            )
        } else {
            "EasyTier - 已停止".to_string()
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
    Ok(())
}

#[cfg(windows)]
pub fn trim_process_tree_working_set() {
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetCurrentProcessId, OpenProcess, SetProcessWorkingSetSize,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
        };
        // 1. Trim host process
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);

        // 2. Enumerate and trim all child processes (WebView2 browser, GPU, renderer, utility)
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };
        let my_pid = GetCurrentProcessId();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot != 0 && snapshot != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snapshot, &mut entry) != 0 {
                loop {
                    if entry.th32ParentProcessID == my_pid {
                        let child = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                        if child != 0 {
                            SetProcessWorkingSetSize(child, usize::MAX, usize::MAX);
                            windows_sys::Win32::Foundation::CloseHandle(child);
                        }
                    }
                    if Process32NextW(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            windows_sys::Win32::Foundation::CloseHandle(snapshot);
        }
    }
}

#[cfg(not(windows))]
pub fn trim_process_tree_working_set() {}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PublicNodeInfo {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub category: String, // "domestic" | "overseas"
    pub is_online: bool,
    pub ping_ms: Option<i64>,
    pub uptime_pct: Option<f64>,
    pub can_relay: bool,
    pub is_masked: bool,
    pub description: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PublicNodesResponse {
    pub nodes: Vec<PublicNodeInfo>,
    pub is_fallback: bool,
    pub updated_at: u64,
}

static PUBLIC_NODES_CACHE: Mutex<Option<(std::time::Instant, PublicNodesResponse)>> = Mutex::new(None);
const PUBLIC_NODES_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

fn default_fallback_nodes() -> Vec<PublicNodeInfo> {
    vec![
        PublicNodeInfo {
            id: 0,
            name: "官方默认公共节点 (EasyTier Official)".into(),
            address: "tcp://public.easytier.top:11010".into(),
            category: "domestic".into(),
            is_online: true,
            ping_ms: Some(35),
            uptime_pct: Some(99.9),
            can_relay: true,
            is_masked: false,
            description: "EasyTier 官方稳定公共服务器 / 全国低延迟".into(),
        },
        PublicNodeInfo {
            id: 1,
            name: "上海电信公共节点".into(),
            address: "tcp://225284.xyz:11010".into(),
            category: "domestic".into(),
            is_online: true,
            ping_ms: Some(38),
            uptime_pct: Some(100.0),
            can_relay: true,
            is_masked: false,
            description: "海波：上海电信 / 支持中转".into(),
        },
        PublicNodeInfo {
            id: 11,
            name: "厦门电信公共节点".into(),
            address: "tcp://easytier.weiai.org.cn:11010".into(),
            category: "domestic".into(),
            is_online: true,
            ping_ms: Some(42),
            uptime_pct: Some(100.0),
            can_relay: true,
            is_masked: false,
            description: "为爱唯爱：厦门电信 / 支持中转".into(),
        },
        PublicNodeInfo {
            id: 53,
            name: "枣庄BGP公共节点".into(),
            address: "udp://et.basd1.de:11010".into(),
            category: "domestic".into(),
            is_online: true,
            ping_ms: Some(30),
            uptime_pct: Some(99.8),
            can_relay: true,
            is_masked: false,
            description: "宇智波.希：枣庄BGP跨网 / 支持中转".into(),
        },
        PublicNodeInfo {
            id: 47,
            name: "重庆移动专线节点".into(),
            address: "tcp://183.230.36.171:11010".into(),
            category: "domestic".into(),
            is_online: true,
            ping_ms: Some(41),
            uptime_pct: Some(100.0),
            can_relay: true,
            is_masked: false,
            description: "重庆移动跨网专线 / 支持中转".into(),
        },
        PublicNodeInfo {
            id: 46,
            name: "美国公共节点".into(),
            address: "udp://us01.225284.xyz:11010".into(),
            category: "overseas".into(),
            is_online: true,
            ping_ms: Some(165),
            uptime_pct: Some(100.0),
            can_relay: true,
            is_masked: false,
            description: "海波：美国西海岸 / 支持中转".into(),
        },
        PublicNodeInfo {
            id: 25,
            name: "美国优质节点".into(),
            address: "tcp://107.172.5.203:11010".into(),
            category: "overseas".into(),
            is_online: true,
            ping_ms: Some(180),
            uptime_pct: Some(100.0),
            can_relay: true,
            is_masked: false,
            description: "罐头：美国 / 支持中转".into(),
        },
    ]
}

/// Robustly extracts node address from raw monitor names.
/// Correctly handles IPv4, domain names, AND bracketed IPv6 hosts (e.g. tcp://[2400:...]:11010).
fn extract_node_address(raw_name: &str) -> Option<String> {
    let idx = raw_name.find("://")?;
    let scheme_start = raw_name[..idx]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    let scheme = &raw_name[scheme_start..idx];
    let rest = &raw_name[idx + 3..];

    let host_port = if rest.starts_with('[') {
        // IPv6 bracketed host: [2400:...]:11010(...)
        if let Some(bracket_end) = rest.find(']') {
            let after = &rest[bracket_end + 1..];
            let end_offset = after
                .find(|c: char| c.is_whitespace() || c == '（' || c == '(' || c == '【' || c == '[' || c == '，' || c == ',')
                .unwrap_or(after.len());
            &rest[..bracket_end + 1 + end_offset]
        } else {
            let end_offset = rest
                .find(|c: char| c.is_whitespace() || c == '（' || c == '(' || c == '【')
                .unwrap_or(rest.len());
            &rest[..end_offset]
        }
    } else {
        // IPv4 or domain name: 225284.xyz:11010(...)
        let end_offset = rest
            .find(|c: char| c.is_whitespace() || c == '（' || c == '(' || c == '[' || c == '【' || c == '，' || c == ',')
            .unwrap_or(rest.len());
        &rest[..end_offset]
    };

    let cleaned = host_port.trim_end_matches(|c: char| c == '/' || c == ' ' || c == ')' || c == '）' || c == ']' || c == '】');
    if cleaned.is_empty() {
        None
    } else {
        Some(format!("{}://{}", scheme, cleaned))
    }
}

#[tauri::command]
async fn fetch_public_nodes(force_refresh: Option<bool>) -> Result<PublicNodesResponse, String> {
    let force = force_refresh.unwrap_or(false);
    if !force {
        if let Ok(guard) = PUBLIC_NODES_CACHE.lock() {
            if let Some((instant, ref cached)) = *guard {
                if instant.elapsed() < PUBLIC_NODES_CACHE_TTL {
                    return Ok(cached.clone());
                }
            }
        }
    }

    let resp = tokio::task::spawn_blocking(|| {
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let fallback_resp = PublicNodesResponse {
            nodes: default_fallback_nodes(),
            is_fallback: true,
            updated_at: now_ts,
        };

        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
        {
            Ok(c) => c,
            Err(_) => return fallback_resp,
        };

        let status_res = client
            .get("https://ruixuan.online/uptime/api/status-page/easytier")
            .send();
        let status_json: Value = match status_res.and_then(|r| r.json()) {
            Ok(j) => j,
            Err(_) => return fallback_resp,
        };

        let heartbeat_json: Value = client
            .get("https://ruixuan.online/uptime/api/status-page/heartbeat/easytier")
            .send()
            .and_then(|r| r.json())
            .unwrap_or(Value::Null);

        let mut nodes = Vec::new();

        // Always add official default node first
        nodes.push(PublicNodeInfo {
            id: 0,
            name: "官方默认公共节点 (EasyTier Official)".into(),
            address: "tcp://public.easytier.top:11010".into(),
            category: "domestic".into(),
            is_online: true,
            ping_ms: Some(35),
            uptime_pct: Some(99.9),
            can_relay: true,
            is_masked: false,
            description: "EasyTier 官方推荐公共服务器 / 稳定高可用".into(),
        });

        if let Some(groups) = status_json.get("publicGroupList").and_then(|g| g.as_array()) {
            for group in groups {
                let group_name = group.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if !group_name.contains("公共节点") {
                    continue; // Skip websites and webconsole groups
                }
                let category = if group_name.contains("海外") {
                    "overseas"
                } else {
                    "domestic"
                };

                if let Some(monitors) = group.get("monitorList").and_then(|m| m.as_array()) {
                    for mon in monitors {
                        let id = mon.get("id").and_then(|i| i.as_i64()).unwrap_or(0);
                        let raw_name = mon.get("name").and_then(|n| n.as_str()).unwrap_or("");

                        let address = match extract_node_address(raw_name) {
                            Some(addr) => addr,
                            None => continue,
                        };

                        let is_masked = address.contains('*');
                        let can_relay = !raw_name.contains("禁中转");

                        // Extract clean description from parentheses
                        let mut description = String::new();
                        if let Some(start) = raw_name.find('（').or_else(|| raw_name.find('(')) {
                            if let Some(end) = raw_name.rfind('）').or_else(|| raw_name.rfind(')')) {
                                if end > start {
                                    description = raw_name[start + 1..end].trim().to_string();
                                }
                            }
                        }
                        if description.is_empty() {
                            description = raw_name.replace(&address, "").trim().to_string();
                        }

                        // Parse status & ping from heartbeat
                        let mut is_online = false;
                        let mut ping_ms = None;
                        let id_str = id.to_string();
                        if let Some(hb_list) = heartbeat_json
                            .get("heartbeatList")
                            .and_then(|h| h.get(&id_str))
                            .and_then(|l| l.as_array())
                        {
                            if let Some(latest) = hb_list.last() {
                                is_online = latest.get("status").and_then(|s| s.as_i64()) == Some(1);
                                ping_ms = latest.get("ping").and_then(|p| p.as_i64());
                            }
                        }

                        // Parse 24h uptime
                        let uptime_key = format!("{}_24", id);
                        let uptime_pct = heartbeat_json
                            .get("uptimeList")
                            .and_then(|u| u.get(&uptime_key))
                            .and_then(|v| v.as_f64())
                            .map(|val| (val * 1000.0).round() / 10.0);

                        nodes.push(PublicNodeInfo {
                            id,
                            name: raw_name.to_string(),
                            address,
                            category: category.to_string(),
                            is_online,
                            ping_ms,
                            uptime_pct,
                            can_relay,
                            is_masked,
                            description,
                        });
                    }
                }
            }
        }

        // Sort nodes: official node first, unmasked first, online first, then lowest ping
        nodes.sort_by(|a, b| {
            if a.id == 0 {
                return std::cmp::Ordering::Less;
            }
            if b.id == 0 {
                return std::cmp::Ordering::Greater;
            }
            if a.is_masked != b.is_masked {
                return a.is_masked.cmp(&b.is_masked);
            }
            if a.is_online != b.is_online {
                return b.is_online.cmp(&a.is_online);
            }
            let ping_a = a.ping_ms.unwrap_or(9999);
            let ping_b = b.ping_ms.unwrap_or(9999);
            ping_a.cmp(&ping_b)
        });

        PublicNodesResponse {
            nodes,
            is_fallback: false,
            updated_at: now_ts,
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking error: {e}"))?;

    if !resp.is_fallback {
        if let Ok(mut guard) = PUBLIC_NODES_CACHE.lock() {
            *guard = Some((std::time::Instant::now(), resp.clone()));
        }
    }

    Ok(resp)
}

#[tauri::command]
fn trim_memory() {
    trim_process_tree_working_set();
}

pub fn run() {
    tauri::Builder::default()
        // Must be the first registered plugin: while an instance is already
        // running, any new launch (portable or installed — they share the
        // identifier and thus one mutex) exits immediately and this callback
        // surfaces the existing window instead of starting a second client.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("app-window-visibility", true);
            }
        }))
        .manage(Mutex::new(RuntimeProcesses::default()))
        .manage(KernelUpdateLock::default())
        .setup(|app| {
            #[cfg(windows)]
            {
                job_object::init_global_job();
                power_monitor::windows_power::init_power_monitor(app.handle().clone());
            }

            let show =
                tauri::menu::MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
            let quit =
                tauri::menu::MenuItem::with_id(app, "quit", "退出 EasyTier", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            let tray_icon = Image::from_bytes(include_bytes!("../icons/icon.png"))
                .map_err(|e| e.to_string())?;
            tauri::tray::TrayIconBuilder::with_id("main-tray")
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("EasyTier")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                            let _ = window.emit("app-window-visibility", true);
                        }
                    }
                    "copy_ip" => {
                        let _ = app.emit("tray-copy-ip", ());
                    }
                    "toggle_network" => {
                        let _ = app.emit("tray-toggle-network", ());
                        let is_running = CURRENT_RUNNING_STATE.load(std::sync::atomic::Ordering::SeqCst);
                        if is_running {
                            let app_handle = app.clone();
                            std::thread::spawn(move || {
                                // 1. Stop local processes
                                if let Ok(mut p) = app_handle.state::<Mutex<RuntimeProcesses>>().lock() {
                                    for (id, mut child) in p.children.drain() {
                                        let _ = child.kill();
                                        let _ = child.wait();
                                        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("easytier-{}.toml", id)));
                                    }
                                }
                                // 2. Stop service instances if any
                                #[cfg(windows)]
                                {
                                    use std::io::{BufRead as _, Write as _};
                                    let target_id = CURRENT_RUNNING_INSTANCE.lock().ok().and_then(|g| g.clone());
                                    let req_payload = if let Some(id) = target_id {
                                        serde_json::json!({ "instance_id": id })
                                    } else {
                                        serde_json::json!({})
                                    };
                                    let request = serde_json::json!({
                                        "protocol_version": ipc::PROTOCOL_VERSION,
                                        "request_id": uuid::Uuid::new_v4().to_string(),
                                        "command": "stop_all_instances",
                                        "payload": req_payload,
                                    });
                                    if let Ok(bytes) = serde_json::to_vec(&request) {
                                        if let Ok(mut pipe) = PipeClient::connect(r"\\.\pipe\EasyTierService") {
                                            let _ = pipe.write_all(&bytes).and_then(|_| pipe.write_all(b"\n"));
                                            let mut line = Vec::new();
                                            let _ = BufReader::new(pipe).read_until(b'\n', &mut line);
                                        }
                                    }
                                }
                                // 3. Update tray to stopped state immediately
                                let _ = update_tray_status(app_handle.clone(), TrayStatusPayload {
                                    running: false,
                                    instance_id: None,
                                    network_name: None,
                                    virtual_ip: None,
                                    peer_count: 0,
                                    rx_speed: None,
                                    tx_speed: None,
                                });
                            });
                        } else if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                            let _ = window.emit("app-window-visibility", true);
                        }
                    }
                    "quit" => quit_and_stop_networks(&app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                            let _ = window.emit("app-window-visibility", true);
                        }
                    }
                })
                .build(app)?;

            let start_in_tray = std::env::args().any(|arg| arg == "--minimized" || arg == "--tray" || arg == "--silent");
            if start_in_tray {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                    let _ = window.emit("app-window-visibility", false);
                    #[cfg(windows)]
                    trim_process_tree_working_set();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    // Closing the window only minimizes to the tray — networks
                    // keep running. Full teardown happens via the tray quit item.
                    api.prevent_close();
                    let _ = window.emit("app-window-visibility", false);
                    let _ = window.hide();
                    #[cfg(windows)]
                    trim_process_tree_working_set();
                }
                WindowEvent::Focused(focused) => {
                    if *focused {
                        let _ = window.emit("app-window-visibility", true);
                    } else if window.is_minimized().unwrap_or(false) {
                        let _ = window.emit("app-window-visibility", false);
                        #[cfg(windows)]
                        trim_process_tree_working_set();
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            trim_memory,
            fetch_public_nodes,
            query_service_installation,
            detect_runtime,
            get_instance_state,
            get_network_logs,
            is_elevated,
            restart_as_admin,
            start_instance,
            wait_for_exit,
            check_kernel_update,
            list_kernel_versions,
            update_kernel,
            cancel_kernel_update,
            update_tray_status,
            stop_instance,
            drop_status_endpoint,
            drop_instance_state,
            is_port_in_use,
            run_cli,
            status_query,
            service_request,
            install_service,
            start_service,
            repair_service,
            get_client_autostart,
            set_client_autostart,
            #[cfg(feature = "remote-rpc")]
            remote_config_discover,
            #[cfg(feature = "remote-rpc")]
            remote_config_load,
            #[cfg(feature = "remote-rpc")]
            remote_config_patch
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
