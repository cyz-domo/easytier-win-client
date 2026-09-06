#![cfg_attr(not(windows), allow(dead_code))]
mod config_store;
mod ipc;
mod kernel_updater;
// NOTE: intentionally NOT compiling `remote_rpc` here — the service never
// issues remote RPC calls, and skipping the easytier stack keeps this binary
// (and its memory footprint) small.
mod portal_args;
mod runtime_manager;

#[cfg(windows)]
mod windows_service {
    use super::*;
    use ::windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    #[derive(Clone)]
    struct TaskState {
        phase: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        percent: Option<u8>,
        message: String,
        result: Option<serde_json::Value>,
        error: Option<String>,
        updated_at: u128,
    }
    type Tasks = Arc<Mutex<HashMap<String, TaskState>>>;

    /// Terminal (completed/failed) tasks are kept only so the frontend can
    /// fetch their final status; prune them once they age out.
    const TASK_RETENTION_MS: u128 = 10 * 60 * 1000;

    fn task_timestamp() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    }

    fn prune_finished_tasks(tasks: &Tasks) {
        let now = task_timestamp();
        tasks
            .lock()
            .unwrap()
            .retain(|_, t| {
                !matches!(t.phase.as_str(), "completed" | "failed")
                    || now.saturating_sub(t.updated_at) < TASK_RETENTION_MS
            });
    }

    define_windows_service!(ffi_service_main, service_main);

    pub fn run() -> Result<(), String> {
        service_dispatcher::start("EasyTierService", ffi_service_main).map_err(|e| e.to_string())
    }
    fn service_main(args: Vec<std::ffi::OsString>) {
        let stopped = Arc::new(Mutex::new(false));
        let stop_flag = stopped.clone();
        let status_handle =
            match service_control_handler::register("EasyTierService", move |event| match event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    *stop_flag.lock().unwrap() = true;
                    ServiceControlHandlerResult::NoError
                }
                _ => ServiceControlHandlerResult::NotImplemented,
            }) {
                Ok(handle) => handle,
                Err(error) => {
                    write_startup_error(&format!(
                        "service control handler registration failed: {error}"
                    ));
                    return;
                }
            };
        let result = run_service(args, stopped, &status_handle);
        if let Err(error) = result {
            write_startup_error(&error);
            let _ = status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: std::time::Duration::default(),
                process_id: None,
            });
        }
    }

    fn write_startup_error(error: &str) {
        let _ = std::fs::create_dir_all(config_store::data_dir());
        let _ = std::fs::write(
            config_store::data_dir().join("service-startup.log"),
            format!("{error}\n"),
        );
    }

    fn run_service(
        args: Vec<std::ffi::OsString>,
        stopped: Arc<Mutex<bool>>,
        status_handle: &::windows_service::service_control_handler::ServiceStatusHandle,
    ) -> Result<(), String> {
        let stopped_for_ipc = stopped.clone();
        let interactive_sid = args
            .iter()
            .filter_map(|arg| arg.to_str())
            .find_map(|arg| arg.strip_prefix("--interactive-user-sid="))
            .filter(|sid| !sid.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(|dir| dir.join("interactive-user.sid")))
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .map(|sid| sid.trim().to_string())
                    .filter(|sid| !sid.is_empty())
            })
            .or_else(|| {
                std::fs::read_to_string(config_store::data_dir().join("interactive-user.sid"))
                    .ok()
                    .map(|sid| sid.trim().to_string())
                    .filter(|sid| !sid.is_empty())
            })
            .ok_or_else(|| {
                format!(
                    "missing trusted interactive user SID; refusing to start IPC; service args: {:?}",
                    args.iter().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>()
                )
            })?;
        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: std::time::Duration::default(),
                process_id: None,
            })
            .map_err(|e| e.to_string())?;
        let state_path = config_store::state_path();
        let state = Arc::new(Mutex::new(config_store::load(&state_path)?));
        let runtime = Arc::new(Mutex::new(runtime_manager::RuntimeManager::new(
            service_core_path(),
        )));
        {
            let configs = state.lock().unwrap().instances.clone();
            let mut manager = runtime.lock().unwrap();
            for config in configs
                .iter()
                .filter(|c| c.auto_start && c.desired_state == config_store::DesiredState::Running)
            {
                if let Err(error) = manager.start(config) {
                    state
                        .lock()
                        .unwrap()
                        .instances
                        .iter_mut()
                        .find(|c| c.id == config.id)
                        .map(|c| c.last_error = Some(error));
                }
            }
        }
        let tasks: Tasks = Arc::new(Mutex::new(HashMap::new()));
        let update_lock = Arc::new(Mutex::new(false));
        let state2 = state.clone();
        let runtime2 = runtime.clone();
        let tasks2 = tasks.clone();
        let lock2 = update_lock.clone();
        let stopped2 = stopped_for_ipc.clone();
        std::thread::spawn(move || {
            let _ = ipc::serve(
                move |request| {
                    handle(
                        request,
                        &state2,
                        &runtime2,
                        &state_path,
                        &tasks2,
                        &lock2,
                        &stopped2,
                    )
                },
                &interactive_sid,
            );
        });
        while !*stopped.lock().unwrap() {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        runtime.lock().unwrap().stop_all();
        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: std::time::Duration::default(),
                process_id: None,
            })
            .map_err(|e| e.to_string())
    }

    fn service_core_path() -> std::path::PathBuf {
        // Mirror the GUI's runtime_dir candidates: the service CWD is
        // System32, so relative lookups must be anchored at the exe dir.
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("core"));
                candidates.push(dir.join("../core"));
                candidates.push(dir.join("../../../core"));
                candidates.push(dir.join("resources/core"));
                candidates.push(dir.join("../resources/core"));
            }
        }
        candidates.push(std::path::PathBuf::from("core"));
        for c in &candidates {
            let core = c.join("easytier-core.exe");
            if core.exists() {
                return core;
            }
        }
        std::path::PathBuf::from("core/easytier-core.exe")
    }

    fn handle(
        req: ipc::Request,
        state: &Arc<Mutex<config_store::ServiceState>>,
        runtime: &Arc<Mutex<runtime_manager::RuntimeManager>>,
        path: &std::path::Path,
        tasks: &Tasks,
        update_lock: &Arc<Mutex<bool>>,
        stopped_opt: &Arc<Mutex<bool>>,
    ) -> ipc::Response {
        if req.protocol_version != ipc::PROTOCOL_VERSION {
            return ipc::error(&req, "invalid_request", "unsupported protocol version");
        }
        let result = dispatch(&req, state, runtime, tasks, update_lock, stopped_opt);
        let response = match result {
            Ok(v) => ipc::response(&req, v),
            Err((c, m)) => ipc::error(&req, c, m),
        };
        if response.ok {
            let _ = config_store::save(path, &state.lock().unwrap());
        }
        response
    }

    fn dispatch(
        req: &ipc::Request,
        state: &Arc<Mutex<config_store::ServiceState>>,
        runtime: &Arc<Mutex<runtime_manager::RuntimeManager>>,
        tasks: &Tasks,
        update_lock: &Arc<Mutex<bool>>,
        stopped_opt: &Arc<Mutex<bool>>,
    ) -> Result<serde_json::Value, (&'static str, String)> {
        use config_store::DesiredState;
        let mut s = state.lock().unwrap();
        if *update_lock.lock().unwrap()
            && matches!(
                req.command.as_str(),
                "sync_instance"
                    | "remove_instance"
                    | "start_instance"
                    | "stop_instance"
                    | "set_auto_start"
            )
        {
            return Err(("busy", "kernel update is running".into()));
        }
        match req.command.as_str() {
            "service_status" => {
                Ok(serde_json::json!({"running": true, "protocol_version": ipc::PROTOCOL_VERSION}))
            }
            "list_instances" => {
                let mut r = runtime.lock().unwrap();
                Ok(serde_json::to_value(
                    s.instances
                        .iter()
                        .map(|x| r.snapshot(x))
                        .collect::<Vec<_>>(),
                )
                .unwrap())
            }
            "get_network_logs" => {
                let id = req
                    .payload
                    .get("instance_id")
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_request", "missing instance_id".into()))?;
                if !s.instances.iter().any(|c| c.id == id) {
                    return Err(("instance_not_found", "instance not found".into()));
                }
                Ok(serde_json::json!({"instance_id": id, "text": runtime.lock().unwrap().logs(id)}))
            }
            "sync_instance" => {
                let mut payload = req.payload.clone();
                if let Some(object) = payload.as_object_mut() {
                    if let Some(instance_id) = object.remove("instance_id") {
                        if let Some(existing) = object.get("id") {
                            if existing != &instance_id {
                                return Err((
                                    "invalid_request",
                                    "id and instance_id do not match".into(),
                                ));
                            }
                        } else {
                            object.insert("id".into(), instance_id);
                        }
                    }
                }
                let c: config_store::InstanceConfig = serde_json::from_value(payload)
                    .map_err(|e| ("invalid_request", e.to_string()))?;
                if let Some(old) = s.instances.iter_mut().find(|x| x.id == c.id) {
                    *old = c.clone();
                } else {
                    s.instances.push(c.clone());
                }
                Ok(serde_json::to_value(c).unwrap())
            }
            "remove_instance" => {
                let id = req
                    .payload
                    .get("instance_id")
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_request", "missing instance_id".into()))?;
                if let Some(pos) = s.instances.iter().position(|x| x.id == id) {
                    if runtime.lock().unwrap().children.contains_key(id) {
                        return Err(("busy", "instance is running".into()));
                    }
                    s.instances.remove(pos);
                    // Drop the retained log/error buffers (up to MAX_LOG_BYTES
                    // each) so deleted instances stop costing memory.
                    let mut r = runtime.lock().unwrap();
                    r.logs.remove(id);
                    r.errors.remove(id);
                    Ok(serde_json::json!({"removed": true}))
                } else {
                    Err(("instance_not_found", "instance not found".into()))
                }
            }
            "start_instance" | "stop_instance" => {
                let id = req
                    .payload
                    .get("instance_id")
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_request", "missing instance_id".into()))?;
                let c = s
                    .instances
                    .iter_mut()
                    .find(|x| x.id == id)
                    .ok_or(("instance_not_found", "instance not found".into()))?;
                if req.command == "start_instance" {
                    c.desired_state = DesiredState::Running;
                    runtime
                        .lock()
                        .unwrap()
                        .start(c)
                        .map_err(|e| ("core_not_found", e))?;
                    // Brief readiness probe; the lock is released between
                    // probes so other IPC requests are not blocked.
                    for _ in 0..8 {
                        if runtime.lock().unwrap().portal_ready(c) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                } else {
                    c.desired_state = DesiredState::Stopped;
                    runtime
                        .lock()
                        .unwrap()
                        .stop(c)
                        .map_err(|e| ("operation_timeout", e))?;
                }
                Ok(serde_json::to_value(runtime.lock().unwrap().snapshot(c)).unwrap())
            }
            "stop_all_instances" => {
                for c in s.instances.iter_mut() {
                    if c.desired_state == DesiredState::Running
                        || runtime.lock().unwrap().children.contains_key(&c.id)
                    {
                        c.desired_state = DesiredState::Stopped;
                        let _ = runtime.lock().unwrap().stop(c);
                    }
                }
                Ok(serde_json::json!({"stopped": true}))
            }
            // GUI quit asks the resident service to shut itself down so the
            // whole stack goes away with the client. The GUI auto-starts the
            // service again on next launch.
            "shutdown_service" => {
                for c in s.instances.iter_mut() {
                    if c.desired_state == DesiredState::Running
                        || runtime.lock().unwrap().children.contains_key(&c.id)
                    {
                        c.desired_state = DesiredState::Stopped;
                        let _ = runtime.lock().unwrap().stop(c);
                    }
                }
                *stopped_opt.lock().unwrap() = true;
                Ok(serde_json::json!({"shutdown": true}))
            }
            "set_auto_start" => {
                let id = req
                    .payload
                    .get("instance_id")
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_request", "missing instance_id".into()))?;
                let enabled = req
                    .payload
                    .get("auto_start")
                    .and_then(|v| v.as_bool())
                    .ok_or(("invalid_request", "missing auto_start".into()))?;
                let c = s
                    .instances
                    .iter_mut()
                    .find(|x| x.id == id)
                    .ok_or(("instance_not_found", "instance not found".into()))?;
                c.auto_start = enabled;
                Ok(serde_json::json!({"auto_start": enabled}))
            }
            "run_cli" => {
                let args: Vec<String> = serde_json::from_value(
                    req.payload
                        .get("args")
                        .cloned()
                        .ok_or(("invalid_request", "missing args".into()))?,
                )
                .map_err(|e| ("invalid_request", e.to_string()))?;
                if args.len() > 16 || args.iter().any(|arg| arg.len() > 256 || arg.contains('\0')) {
                    return Err(("invalid_request", "invalid CLI arguments".into()));
                }
                let cli = service_core_path()
                    .parent()
                    .map(|dir| dir.join("easytier-cli.exe"))
                    .ok_or(("core_not_found", "CLI path unavailable".into()))?;
                let mut command = std::process::Command::new(cli);
                command
                    .args(args)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                #[cfg(windows)]
                std::os::windows::process::CommandExt::creation_flags(&mut command, 0x08000000);
                let mut child = command
                    .spawn()
                    .map_err(|e| ("core_not_found", e.to_string()))?;
                match wait_timeout::ChildExt::wait_timeout(
                    &mut child,
                    std::time::Duration::from_secs(5),
                )
                .map_err(|e| ("operation_timeout", e.to_string()))?
                {
                    Some(status) => {
                        let output = child
                            .wait_with_output()
                            .map_err(|e| ("operation_timeout", e.to_string()))?;
                        if status.success() {
                            String::from_utf8(output.stdout)
                                .map(serde_json::Value::String)
                                .map_err(|e| ("invalid_response", e.to_string()))
                        } else {
                            Err((
                                "operation_failed",
                                String::from_utf8_lossy(&output.stderr).to_string(),
                            ))
                        }
                    }
                    None => {
                        let _ = child.kill();
                        let _ = child.wait();
                        Err(("operation_timeout", "CLI 查询超时".into()))
                    }
                }
            }
            "get_task_status" => {
                let id = req
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_request", "missing task_id".into()))?;
                let task = tasks
                    .lock()
                    .unwrap()
                    .get(id)
                    .cloned()
                    .ok_or(("invalid_request", "task not found".into()))?;
                Ok(
                    serde_json::json!({"task_id": id, "phase": task.phase, "downloaded_bytes": task.downloaded_bytes, "total_bytes": task.total_bytes, "percent": task.percent, "message": task.message, "result": task.result, "error": task.error}),
                )
            }
            "update_kernel" => {
                let proxy = req
                    .payload
                    .get("proxy")
                    .and_then(|v| v.as_str())
                    .unwrap_or("direct")
                    .to_string();
                let mut busy = update_lock.lock().unwrap();
                if *busy {
                    return Err(("busy", "kernel update already running".into()));
                }
                *busy = true;
                let task_id = format!("kernel-{}", task_timestamp());
                prune_finished_tasks(&tasks);
                tasks.lock().unwrap().insert(
                    task_id.clone(),
                    TaskState {
                        phase: "queued".into(),
                        downloaded_bytes: 0,
                        total_bytes: None,
                        percent: Some(0),
                        message: "内核更新已排队".into(),
                        result: None,
                        error: None,
                        updated_at: task_timestamp(),
                    },
                );
                let tasks2 = tasks.clone();
                let state2 = state.clone();
                let runtime2 = runtime.clone();
                let lock2 = update_lock.clone();
                let id2 = task_id.clone();
                std::thread::spawn(move || {
                    run_kernel_update(id2, proxy, state2, runtime2, tasks2, lock2)
                });
                Ok(serde_json::json!({"task_id": task_id}))
            }
            _ => Err(("invalid_request", "unknown command".into())),
        }
    }

    fn set_task(
        tasks: &Tasks,
        id: &str,
        phase: &str,
        message: impl Into<String>,
        downloaded: u64,
        total: Option<u64>,
        error: Option<String>,
        result: Option<serde_json::Value>,
    ) {
        if let Some(task) = tasks.lock().unwrap().get_mut(id) {
            task.phase = phase.into();
            task.message = message.into();
            task.downloaded_bytes = downloaded;
            task.total_bytes = total;
            task.percent = total
                .filter(|v| *v > 0)
                .map(|v| ((downloaded.saturating_mul(100) / v).min(100)) as u8);
            task.error = error;
            task.result = result;
            task.updated_at = task_timestamp();
        }
    }

    fn run_kernel_update(
        task_id: String,
        proxy: String,
        state: Arc<Mutex<config_store::ServiceState>>,
        runtime: Arc<Mutex<runtime_manager::RuntimeManager>>,
        tasks: Tasks,
        update_lock: Arc<Mutex<bool>>,
    ) {
        let finish = |phase: &str,
                      message: String,
                      error: Option<String>,
                      result: Option<serde_json::Value>| {
            set_task(&tasks, &task_id, phase, message, 0, None, error, result);
            *update_lock.lock().unwrap() = false;
        };
        let core = service_core_path();
        let runtime_dir = match core.parent() {
            Some(path) => path.to_path_buf(),
            None => {
                finish(
                    "failed",
                    "无法确定 core 目录".into(),
                    Some("invalid core path".into()),
                    None,
                );
                return;
            }
        };
        let staged = match kernel_updater::download_and_stage_with_progress(
            &proxy,
            &runtime_dir,
            |phase, message, downloaded, total, file, error| {
                set_task(
                    &tasks, &task_id, phase, message, downloaded, total, error, None,
                );
                if let Some(name) = file {
                    if let Some(task) = tasks.lock().unwrap().get_mut(&task_id) {
                        task.message = format!("{}: {}", task.message, name);
                    }
                }
            },
        ) {
            Ok(path) => path,
            Err(error) => {
                finish("failed", "内核下载或校验失败".into(), Some(error), None);
                return;
            }
        };
        let configs = state.lock().unwrap().instances.clone();
        let running: Vec<_> = configs
            .iter()
            .filter(|c| runtime.lock().unwrap().children.contains_key(&c.id))
            .cloned()
            .collect();
        set_task(
            &tasks,
            &task_id,
            "stopping",
            "正在停止运行中的网络",
            0,
            None,
            None,
            None,
        );
        for config in &running {
            if let Err(error) = runtime.lock().unwrap().stop(config) {
                let _ = std::fs::remove_dir_all(staged.parent().unwrap_or(&staged));
                finish("failed", "停止网络失败".into(), Some(error), None);
                return;
            }
        }
        set_task(
            &tasks,
            &task_id,
            "installing",
            "正在替换 EasyTier 内核",
            0,
            None,
            None,
            None,
        );
        if let Err(error) = kernel_updater::install(&runtime_dir, &staged) {
            finish("failed", "内核替换失败".into(), Some(error), None);
            return;
        }
        set_task(
            &tasks,
            &task_id,
            "restarting",
            "正在恢复原有网络",
            0,
            None,
            None,
            None,
        );
        let mut failures = Vec::new();
        for config in &running {
            if let Err(error) = runtime.lock().unwrap().start(config) {
                failures.push(format!("{}: {}", config.id, error));
            } else {
                for _ in 0..8 {
                    if runtime.lock().unwrap().portal_ready(config) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
        let result =
            serde_json::json!({"restarted": running.len() - failures.len(), "failed": failures});
        if !failures.is_empty() {
            finish(
                "completed",
                "内核更新完成，但部分网络恢复失败".into(),
                Some("部分实例重启失败".into()),
                Some(result),
            );
        } else {
            set_task(
                &tasks,
                &task_id,
                "completed",
                "EasyTier 内核更新完成",
                1,
                Some(1),
                None,
                Some(result),
            );
            *update_lock.lock().unwrap() = false;
        }
    }
}

#[cfg(windows)]
fn main() {
    if let Err(e) = windows_service::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("easytier-service is only supported on Windows");
}
