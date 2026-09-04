#![cfg_attr(not(windows), allow(dead_code))]
mod config_store;
mod ipc;
mod runtime_manager;

#[cfg(windows)]
mod windows_service {
    use super::*;
    use std::sync::{Arc, Mutex};
    use ::windows_service::{define_windows_service, service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType}, service_control_handler::{self, ServiceControlHandlerResult}, service_dispatcher};
    define_windows_service!(ffi_service_main, service_main);

    pub fn run() -> Result<(), String> { service_dispatcher::start("EasyTierService", ffi_service_main).map_err(|e| e.to_string()) }
    fn service_main(_args: Vec<std::ffi::OsString>) {
        let _ = run_service();
    }
    fn run_service() -> Result<(), String> {
        let stopped = Arc::new(Mutex::new(false));
        let stop_flag = stopped.clone();
        let status_handle = service_control_handler::register("EasyTierService", move |event| {
            match event { ServiceControl::Stop | ServiceControl::Shutdown => { *stop_flag.lock().unwrap() = true; ServiceControlHandlerResult::NoError }, _ => ServiceControlHandlerResult::NotImplemented }
        }).map_err(|e| e.to_string())?;
        status_handle.set_service_status(ServiceStatus { service_type: ServiceType::OWN_PROCESS, current_state: ServiceState::Running, controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN, exit_code: ServiceExitCode::Win32(0), checkpoint: 0, wait_hint: std::time::Duration::default(), process_id: None }).map_err(|e| e.to_string())?;
        let state_path = config_store::state_path();
        let state = Arc::new(Mutex::new(config_store::load(&state_path)?));
        let runtime = Arc::new(Mutex::new(runtime_manager::RuntimeManager::new(service_core_path())));
        // Restore only instances explicitly marked for automatic startup.
        {
            let configs = state.lock().unwrap().instances.clone();
            let mut manager = runtime.lock().unwrap();
            for config in configs.iter().filter(|c| c.auto_start && c.desired_state == config_store::DesiredState::Running) {
                if let Err(error) = manager.start(config) {
                    state.lock().unwrap().instances.iter_mut().find(|c| c.id == config.id).map(|c| c.last_error = Some(error));
                }
            }
        }
        let cache = ipc::new_cache();
        let state2 = state.clone(); let runtime2 = runtime.clone(); let cache2 = cache.clone();
        std::thread::spawn(move || { let _ = ipc::serve(move |request| handle(request, &state2, &runtime2, &state_path, &cache2)); });
        while !*stopped.lock().unwrap() { std::thread::sleep(std::time::Duration::from_millis(250)); }
        runtime.lock().unwrap().stop_all();
        status_handle.set_service_status(ServiceStatus { service_type: ServiceType::OWN_PROCESS, current_state: ServiceState::Stopped, controls_accepted: ServiceControlAccept::empty(), exit_code: ServiceExitCode::Win32(0), checkpoint: 0, wait_hint: std::time::Duration::default(), process_id: None }).map_err(|e| e.to_string())
    }
    fn service_core_path() -> std::path::PathBuf { std::env::current_exe().ok().and_then(|p| p.parent().map(|x| x.join("core").join("easytier-core.exe"))).unwrap_or_else(|| std::path::PathBuf::from("core/easytier-core.exe")) }
    fn handle(req: ipc::Request, state: &Arc<Mutex<config_store::ServiceState>>, runtime: &Arc<Mutex<runtime_manager::RuntimeManager>>, path: &std::path::Path, cache: &ipc::IdempotencyCache) -> ipc::Response {
        if req.protocol_version != ipc::PROTOCOL_VERSION { return ipc::error(&req, "invalid_request", "unsupported protocol version"); }
        if let Some(old) = cache.lock().unwrap().get(&req.request_id).cloned() { return old; }
        let result = dispatch(&req, state, runtime);
        let response = match result { Ok(v) => ipc::response(&req, v), Err((c,m)) => ipc::error(&req,c,m) };
        if response.ok { let _ = config_store::save(path, &state.lock().unwrap()); }
        cache.lock().unwrap().insert(req.request_id.clone(), response.clone()); response
    }
    fn dispatch(req: &ipc::Request, state: &Arc<Mutex<config_store::ServiceState>>, runtime: &Arc<Mutex<runtime_manager::RuntimeManager>>) -> Result<serde_json::Value, (&'static str, String)> {
        use config_store::DesiredState;
        let mut s = state.lock().unwrap();
        match req.command.as_str() {
            "service_status" => Ok(serde_json::json!({"running":true,"protocol_version":ipc::PROTOCOL_VERSION})),
            "list_instances" => { let mut r=runtime.lock().unwrap(); Ok(serde_json::to_value(s.instances.iter().map(|x| r.snapshot(x)).collect::<Vec<_>>()).unwrap()) },
            "sync_instance" => { let c: config_store::InstanceConfig=serde_json::from_value(req.payload.clone()).map_err(|e| ("invalid_request",e.to_string()))?; if let Some(old)=s.instances.iter_mut().find(|x|x.id==c.id){*old=c.clone()}else{s.instances.push(c.clone())}; Ok(serde_json::to_value(c).unwrap()) },
            "remove_instance" => { let id=req.payload.get("instance_id").and_then(|v|v.as_str()).ok_or(("invalid_request","missing instance_id".into()))?; if let Some(pos)=s.instances.iter().position(|x|x.id==id){ if runtime.lock().unwrap().children.contains_key(id){return Err(("busy","instance is running".into()))} s.instances.remove(pos); Ok(serde_json::json!({"removed":true})) } else {Err(("instance_not_found","instance not found".into()))} },
            "start_instance" | "stop_instance" => { let id=req.payload.get("instance_id").and_then(|v|v.as_str()).ok_or(("invalid_request","missing instance_id".into()))?; let c=s.instances.iter_mut().find(|x|x.id==id).ok_or(("instance_not_found","instance not found".into()))?; if req.command=="start_instance" {c.desired_state=DesiredState::Running; runtime.lock().unwrap().start(c).map_err(|e|("core_not_found",e))?;} else {c.desired_state=DesiredState::Stopped; runtime.lock().unwrap().stop(c).map_err(|e|("operation_timeout",e))?;} Ok(serde_json::to_value(runtime.lock().unwrap().snapshot(c)).unwrap()) },
            "set_auto_start" => { let id=req.payload.get("instance_id").and_then(|v|v.as_str()).ok_or(("invalid_request","missing instance_id".into()))?; let enabled=req.payload.get("auto_start").and_then(|v|v.as_bool()).ok_or(("invalid_request","missing auto_start".into()))?; let c=s.instances.iter_mut().find(|x|x.id==id).ok_or(("instance_not_found","instance not found".into()))?; c.auto_start=enabled; Ok(serde_json::json!({"auto_start":enabled})) },
            "get_task_status" => Ok(serde_json::json!({"task_id":req.payload.get("task_id"),"status":"unknown"})),
            "update_kernel" => Err(("update_failed","kernel update is not enabled in this service build".into())),
            _ => Err(("invalid_request","unknown command".into()))
        }
    }
}

#[cfg(windows)] fn main() { if let Err(e)=windows_service::run(){ eprintln!("{e}"); std::process::exit(1); } }
#[cfg(not(windows))] fn main() { eprintln!("easytier-service is only supported on Windows"); }
