use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;
use serde_json::{json, Value};

const PIPE: &str = r"\\.\pipe\EasyTierService";

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

fn request_id() -> String {
    let n = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("native-{t}-{n}")
}

#[derive(Clone, Debug, Default)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub rpc_port: u16,
    pub observed: String,
    pub desired: String,
    pub last_error: Option<String>,
    pub pid: Option<u32>,
    pub config_toml: String,
    pub auto_start: bool,
    pub remote_manage: bool,
    pub whitelist: Vec<String>,
    pub console_log_level: Option<String>,
}

impl Instance {
    pub fn running(&self) -> bool {
        self.observed == "running"
    }

    pub fn status_label(&self) -> &'static str {
        match self.observed.as_str() {
            "running" => "在线",
            "starting" => "启动中",
            "stopping" => "停止中",
            "failed" => "启动失败",
            _ => "未运行",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Peer {
    pub hostname: String,
    pub ip: String,
    pub cost: String,
    pub latency: f64,
    pub loss: f64,
    pub rx: f64,
    pub tx: f64,
    pub tunnel: String,
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Default)]
pub struct NodeInfo {
    pub ipv4: String,
    pub hostname: String,
    pub version: String,
    pub peer_id: String,
}

pub struct ServiceState {
    pub running: bool,
    pub error: Option<String>,
    pub instances: Vec<Instance>,
}

#[derive(Clone, Debug, Default)]
pub struct Route {
    pub ipv4: String,
    pub hostname: String,
    pub proxy_cidrs: Vec<String>,
    pub next_hop: String,
    pub hops: f64,
    pub latency: f64,
    pub version: String,
}

#[derive(Clone, Debug, Default)]
pub struct KernelProgress {
    pub task_id: String,
    pub status: String,
    pub phase: String,
    pub percent: f64,
    pub downloaded: u64,
    pub total: u64,
    pub message: String,
}

impl KernelProgress {
    pub fn terminal(&self) -> bool {
        matches!(self.status.as_str(), "completed" | "failed" | "cancelled")
    }

    pub fn phase_label(&self) -> &str {
        match self.phase.as_str() {
            "queued" => "排队中",
            "downloading" => "下载中",
            "verifying" => "校验文件",
            "extracting" => "解压",
            "stopping" => "停止内核",
            "installing" => "安装",
            "completed" => "已完成",
            "failed" => "失败",
            "cancelled" => "已取消",
            _ => "准备中",
        }
    }
}

pub struct StatusUpdate {
    pub instance_id: String,
    pub node: Option<NodeInfo>,
    pub peers: Vec<Peer>,
    pub routes: Vec<Route>,
    pub error: Option<String>,
}

pub struct ActionResult {
    pub label: String,
    pub ok: bool,
    pub message: String,
}

pub enum Event {
    Service(ServiceState),
    Status(StatusUpdate),
    Logs { instance_id: String, lines: Vec<String> },
    Action(ActionResult),
    Kernel(KernelProgress),
    Proxies(Vec<crate::settings::LocalProxy>),
    Releases(Result<crate::settings::ReleaseInfo, String>),
    Autostart(crate::settings::Autostart),
}

pub enum Action {
    SetVisible(bool),
    SetActive(String),
    Refresh,
    SetRunning {
        id: String,
        start: bool,
        payload: Option<Value>,
    },
    Sync {
        payload: Value,
        label: String,
    },
    Create {
        payload: Value,
    },
    Remove {
        id: String,
    },
    KernelStart {
        proxy: String,
        target: Option<String>,
    },
    KernelCancel,
    DetectProxies,
    CheckReleases {
        proxy: String,
    },
    AutostartGet,
    AutostartSet {
        enabled: bool,
        minimized: bool,
    },
}

pub fn spawn(ctx: egui::Context) -> (Sender<Action>, Receiver<Event>) {
    let (action_tx, action_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || run(ctx, action_rx, event_tx));
    (action_tx, event_rx)
}

struct LoopState {
    visible: bool,
    active: Option<String>,
    instances: Vec<Instance>,
    cfg_cache: Option<(SystemTime, Vec<InstanceCfg>)>,
    node: Option<NodeInfo>,
    routes: Vec<Route>,
    kernel_task: Option<String>,
    kernel_misses: u32,
    last_service: Option<Instant>,
    last_node: Option<Instant>,
    last_peers: Option<Instant>,
    last_routes: Option<Instant>,
    last_logs: Option<Instant>,
    last_kernel: Option<Instant>,
}

impl LoopState {
    fn new() -> Self {
        Self {
            visible: true,
            active: None,
            instances: Vec::new(),
            cfg_cache: None,
            node: None,
            routes: Vec::new(),
            kernel_task: None,
            kernel_misses: 0,
            last_service: None,
            last_node: None,
            last_peers: None,
            last_routes: None,
            last_logs: None,
            last_kernel: None,
        }
    }

    fn due(slot: &Option<Instant>, every: Duration) -> bool {
        slot.map(|t| t.elapsed() >= every).unwrap_or(true)
    }

    fn reset(&mut self) {
        self.last_service = None;
        self.last_node = None;
        self.last_peers = None;
        self.last_routes = None;
        self.last_logs = None;
        self.node = None;
        self.routes.clear();
    }

    fn active_instance(&self) -> Option<&Instance> {
        let id = self.active.as_deref()?;
        self.instances.iter().find(|i| i.id == id)
    }
}

fn run(ctx: egui::Context, action_rx: Receiver<Action>, event_tx: Sender<Event>) {
    let mut st = LoopState::new();
    loop {
        let mut wake = false;
        loop {
            match action_rx.try_recv() {
                Ok(action) => {
                    if !apply_action(&mut st, action, &event_tx, &ctx) {
                        return;
                    }
                    wake = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if st.visible {
            if LoopState::due(&st.last_service, Duration::from_millis(1000)) {
                st.last_service = Some(Instant::now());
                poll_service(&event_tx, &mut st);
            }
            if st.kernel_task.is_some()
                && LoopState::due(&st.last_kernel, Duration::from_millis(1000))
            {
                st.last_kernel = Some(Instant::now());
                poll_kernel(&event_tx, &mut st);
            }
            if let Some(inst) = st.active_instance().cloned() {
                if inst.running() {
                    let portal = format!("127.0.0.1:{}", inst.rpc_port);
                    if LoopState::due(&st.last_node, Duration::from_secs(6)) {
                        st.last_node = Some(Instant::now());
                        if let Ok(node) = cli_json(&portal, "node") {
                            st.node = parse_node(&node);
                        }
                    }
                    if LoopState::due(&st.last_routes, Duration::from_secs(6)) {
                        st.last_routes = Some(Instant::now());
                        if let Ok(routes) = cli_json(&portal, "route") {
                            st.routes = parse_routes(&routes);
                        }
                    }
                    if LoopState::due(&st.last_peers, Duration::from_secs(2)) {
                        st.last_peers = Some(Instant::now());
                        let (peers, error) = match cli_json(&portal, "peer") {
                            Ok(v) => (parse_peers(&v), None),
                            Err(e) => (Vec::new(), Some(e)),
                        };
                        let _ = event_tx.send(Event::Status(StatusUpdate {
                            instance_id: inst.id.clone(),
                            node: st.node.clone(),
                            peers,
                            routes: st.routes.clone(),
                            error,
                        }));
                    }
                    if LoopState::due(&st.last_logs, Duration::from_secs(3)) {
                        st.last_logs = Some(Instant::now());
                        if let Ok(data) =
                            pipe_request("get_network_logs", &json!({ "instance_id": inst.id }))
                        {
                            let text = data.get("text").and_then(Value::as_str).unwrap_or("");
                            let lines: Vec<String> = text
                                .lines()
                                .map(|l| l.trim_end().to_owned())
                                .filter(|l| !l.is_empty())
                                .collect();
                            let tail = if lines.len() > 400 {
                                lines[lines.len() - 400..].to_vec()
                            } else {
                                lines
                            };
                            let _ = event_tx.send(Event::Logs {
                                instance_id: inst.id.clone(),
                                lines: tail,
                            });
                        }
                    }
                }
            }
        }

        if st.visible || wake {
            ctx.request_repaint();
        }
        match action_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(action) => {
                if !apply_action(&mut st, action, &event_tx, &ctx) {
                    return;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn apply_action(
    st: &mut LoopState,
    action: Action,
    event_tx: &Sender<Event>,
    ctx: &egui::Context,
) -> bool {
    match action {
        Action::SetVisible(visible) => {
            if st.visible != visible {
                st.visible = visible;
                st.reset();
            }
        }
        Action::SetActive(id) => {
            if st.active.as_deref() != Some(id.as_str()) {
                st.active = Some(id);
                st.reset();
            }
        }
        Action::Refresh => st.reset(),
        Action::SetRunning { id, start, payload } => {
            handle_set_running(event_tx, st, &id, start, payload)
        }
        Action::Sync { payload, label } => {
            let result = match pipe_request("sync_instance", &payload) {
                Ok(_) => {
                    st.last_service = None;
                    ActionResult {
                        label,
                        ok: true,
                        message: "配置已保存".to_owned(),
                    }
                }
                Err(e) => ActionResult {
                    label,
                    ok: false,
                    message: e,
                },
            };
            let _ = event_tx.send(Event::Action(result));
        }
        Action::Create { payload } => {
            let id = payload
                .get("instance_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let result = match pipe_request("sync_instance", &payload) {
                Ok(_) => {
                    st.last_service = None;
                    if !id.is_empty() {
                        st.active = Some(id);
                        st.reset();
                    }
                    ActionResult {
                        label: "新建网络".to_owned(),
                        ok: true,
                        message: "已创建并写入后台服务".to_owned(),
                    }
                }
                Err(e) => ActionResult {
                    label: "新建网络".to_owned(),
                    ok: false,
                    message: e,
                },
            };
            let _ = event_tx.send(Event::Action(result));
        }
        Action::Remove { id } => {
            // The service refuses to delete a running instance, so stop first
            // and ignore "not running" errors.
            let _ = pipe_request("stop_instance", &json!({ "instance_id": id }));
            let result = match pipe_request("remove_instance", &json!({ "instance_id": id })) {
                Ok(_) => {
                    st.last_service = None;
                    if st.active.as_deref() == Some(id.as_str()) {
                        st.active = None;
                    }
                    ActionResult {
                        label: "删除网络".to_owned(),
                        ok: true,
                        message: "实例已删除".to_owned(),
                    }
                }
                Err(e) => ActionResult {
                    label: "删除网络".to_owned(),
                    ok: false,
                    message: e,
                },
            };
            let _ = event_tx.send(Event::Action(result));
        }
        Action::KernelStart { proxy, target } => {
            let payload = json!({
                "proxy": proxy,
                "target_version": target,
            });
            match pipe_request("update_kernel", &payload) {
                Ok(data) => {
                    let mut progress = parse_kernel(&data);
                    if progress.task_id.is_empty() {
                        progress.task_id = text_field(&data, "task_id");
                    }
                    st.kernel_task = Some(progress.task_id.clone());
                    st.kernel_misses = 0;
                    st.last_kernel = None;
                    let _ = event_tx.send(Event::Kernel(progress));
                }
                Err(e) => {
                    let _ = event_tx.send(Event::Action(ActionResult {
                        label: "内核更新".to_owned(),
                        ok: false,
                        message: e,
                    }));
                }
            }
        }
        Action::KernelCancel => {
            let result = match pipe_request("cancel_kernel_update", &json!({})) {
                Ok(_) => {
                    st.kernel_task = None;
                    let _ = event_tx.send(Event::Kernel(KernelProgress {
                        status: "cancelled".to_owned(),
                        phase: "cancelled".to_owned(),
                        message: "已取消更新".to_owned(),
                        ..Default::default()
                    }));
                    ActionResult {
                        label: "取消更新".to_owned(),
                        ok: true,
                        message: "已请求取消".to_owned(),
                    }
                }
                Err(e) => ActionResult {
                    label: "取消更新".to_owned(),
                    ok: false,
                    message: e,
                },
            };
            let _ = event_tx.send(Event::Action(result));
        }
        Action::DetectProxies => {
            let tx = event_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let proxies = crate::settings::detect_local_proxies();
                let _ = tx.send(Event::Proxies(proxies));
                ctx.request_repaint();
            });
        }
        Action::CheckReleases { proxy } => {
            let tx = event_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = crate::settings::check_releases(&proxy);
                let _ = tx.send(Event::Releases(result));
                ctx.request_repaint();
            });
        }
        Action::AutostartGet => {
            let _ = event_tx.send(Event::Autostart(crate::settings::read_autostart()));
        }
        Action::AutostartSet { enabled, minimized } => {
            let result = match crate::settings::write_autostart(enabled, minimized) {
                Ok(_) => {
                    let _ = event_tx.send(Event::Autostart(crate::settings::read_autostart()));
                    ActionResult {
                        label: "开机自启".to_owned(),
                        ok: true,
                        message: if enabled {
                            "已注册开机自启动".to_owned()
                        } else {
                            "已取消开机自启动".to_owned()
                        },
                    }
                }
                Err(e) => ActionResult {
                    label: "开机自启".to_owned(),
                    ok: false,
                    message: e,
                },
            };
            let _ = event_tx.send(Event::Action(result));
        }
    }
    true
}

fn poll_kernel(event_tx: &Sender<Event>, st: &mut LoopState) {
    let Some(task_id) = st.kernel_task.clone() else {
        return;
    };
    match pipe_request("get_task_status", &json!({ "task_id": task_id })) {
        Ok(data) => {
            st.kernel_misses = 0;
            let progress = parse_kernel(&data);
            if progress.terminal() {
                st.kernel_task = None;
            }
            let _ = event_tx.send(Event::Kernel(progress));
        }
        Err(e) => {
            if e.contains("task not found") || e.contains("不存在") {
                st.kernel_task = None;
                let _ = event_tx.send(Event::Kernel(KernelProgress {
                    task_id,
                    status: "failed".to_owned(),
                    phase: "failed".to_owned(),
                    message: "更新任务已失效（后台服务可能重启过）".to_owned(),
                    ..Default::default()
                }));
                return;
            }
            st.kernel_misses = st.kernel_misses.saturating_add(1);
            if st.kernel_misses >= 10 {
                st.kernel_task = None;
                let _ = event_tx.send(Event::Kernel(KernelProgress {
                    task_id,
                    status: "failed".to_owned(),
                    phase: "failed".to_owned(),
                    message: format!("无法查询更新进度：{e}"),
                    ..Default::default()
                }));
            }
        }
    }
}

fn parse_kernel(value: &Value) -> KernelProgress {
    let progress = value.get("progress");
    let nested_text = |key: &str| -> String {
        progress
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let percent = {
        let top = field_number(value, "percent");
        if top > 0.0 {
            top
        } else {
            progress
                .and_then(|p| p.get("percent"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        }
    };
    let phase = {
        let phase = text_field(value, "phase");
        if phase.is_empty() {
            nested_text("phase")
        } else {
            phase
        }
    };
    let mut status = text_field(value, "status");
    if status.is_empty() {
        status = phase.clone();
    }
    let mut message = text_field(value, "message");
    if message.is_empty() {
        message = nested_text("message");
    }
    KernelProgress {
        task_id: text_field(value, "task_id"),
        status,
        phase,
        percent,
        downloaded: value
            .get("downloaded_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total: value
            .get("total_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        message,
    }
}

fn handle_set_running(
    event_tx: &Sender<Event>,
    st: &mut LoopState,
    id: &str,
    start: bool,
    payload: Option<Value>,
) {
    let label = if start { "启动网络" } else { "停止网络" };
    if let Some(payload) = payload {
        if let Err(e) = pipe_request("sync_instance", &payload) {
            let _ = event_tx.send(Event::Action(ActionResult {
                label: label.to_owned(),
                ok: false,
                message: e,
            }));
            return;
        }
    }
    let command = if start { "start_instance" } else { "stop_instance" };
    match pipe_request(command, &json!({ "instance_id": id })) {
        Ok(_) => {
            st.last_service = None;
            st.last_peers = None;
            st.last_node = None;
            st.last_routes = None;
            st.routes.clear();
            let _ = event_tx.send(Event::Action(ActionResult {
                label: label.to_owned(),
                ok: true,
                message: if start {
                    "网络已启动".to_owned()
                } else {
                    "网络已停止".to_owned()
                },
            }));
        }
        Err(e) => {
            let _ = event_tx.send(Event::Action(ActionResult {
                label: label.to_owned(),
                ok: false,
                message: e,
            }));
        }
    }
}

fn poll_service(event_tx: &Sender<Event>, st: &mut LoopState) {
    let status = pipe_request("service_status", &json!({}));
    if let Err(e) = status {
        let _ = event_tx.send(Event::Service(ServiceState {
            running: false,
            error: Some(e),
            instances: Vec::new(),
        }));
        return;
    }
    match fetch_instances() {
        Ok(mut instances) => {
            merge_configs(&mut instances, &mut st.cfg_cache);
            if st.active.is_none() {
                st.active = instances.first().map(|i| i.id.clone());
            }
            if let Some(active) = st.active.clone() {
                if !instances.iter().any(|i| i.id == active) {
                    st.active = instances.first().map(|i| i.id.clone());
                }
            }
            st.instances = instances.clone();
            let _ = event_tx.send(Event::Service(ServiceState {
                running: true,
                error: None,
                instances,
            }));
        }
        Err(e) => {
            let _ = event_tx.send(Event::Service(ServiceState {
                running: false,
                error: Some(e),
                instances: Vec::new(),
            }));
        }
    }
}

fn pipe_request(command: &str, payload: &Value) -> Result<Value, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match pipe_request_once(command, payload) {
            Ok(value) => return Ok(value),
            Err((true, message)) => {
                last = message;
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(400));
                }
            }
            Err((false, message)) => return Err(message),
        }
    }
    Err(last)
}

fn pipe_request_once(command: &str, payload: &Value) -> Result<Value, (bool, String)> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE)
        .map_err(|e| (true, format!("后台服务不可用：{e}")))?;
    let mut writer = file
        .try_clone()
        .map_err(|e| (false, format!("连接服务失败：{e}")))?;
    let request = json!({
        "protocol_version": 1,
        "request_id": request_id(),
        "command": command,
        "payload": payload,
    });
    let mut line = serde_json::to_vec(&request)
        .map_err(|e| (false, format!("请求序列化失败：{e}")))?;
    line.push(b'\n');
    writer
        .write_all(&line)
        .and_then(|_| writer.flush())
        .map_err(|e| (false, format!("发送请求失败：{e}")))?;
    let mut reader = BufReader::new(file);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| (false, format!("读取服务响应失败：{e}")))?;
    if response.trim().is_empty() {
        return Err((false, "服务未返回响应".to_owned()));
    }
    let value: Value =
        serde_json::from_str(&response).map_err(|e| (false, format!("响应解析失败：{e}")))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = &value["error"];
        let message = error["message"]
            .as_str()
            .or_else(|| error.as_str())
            .or_else(|| error["code"].as_str())
            .unwrap_or("服务请求失败")
            .to_owned();
        return Err((false, message));
    }
    Ok(value.get("data").cloned().unwrap_or(Value::Null))
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn fetch_instances() -> Result<Vec<Instance>, String> {
    let data = pipe_request("list_instances", &json!({}))?;
    let list = data.as_array().cloned().unwrap_or_default();
    Ok(list
        .iter()
        .map(|v| Instance {
            id: text_field(v, "id"),
            name: text_field(v, "name"),
            rpc_port: v.get("rpc_port").and_then(Value::as_u64).unwrap_or(0) as u16,
            observed: text_field(v, "observed_state"),
            desired: text_field(v, "desired_state"),
            last_error: v
                .get("last_error")
                .and_then(Value::as_str)
                .map(str::to_owned),
            pid: v.get("pid").and_then(Value::as_u64).map(|p| p as u32),
            console_log_level: v
                .get("console_log_level")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ..Default::default()
        })
        .collect())
}

#[derive(Clone, Default)]
struct InstanceCfg {
    id: String,
    config_toml: String,
    auto_start: bool,
    remote_manage: bool,
    whitelist: Vec<String>,
}

fn config_store_path() -> PathBuf {
    std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("EasyTier")
        .join("instances.json")
}

fn load_config_file() -> Vec<InstanceCfg> {
    let Ok(bytes) = std::fs::read(config_store_path()) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    value
        .get("instances")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|v| InstanceCfg {
                    id: text_field(v, "id"),
                    config_toml: text_field(v, "config_toml"),
                    auto_start: v.get("auto_start").and_then(Value::as_bool).unwrap_or(false),
                    remote_manage: v
                        .get("remote_manage_enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    whitelist: v
                        .get("rpc_whitelist_cidrs")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn merge_configs(instances: &mut [Instance], cache: &mut Option<(SystemTime, Vec<InstanceCfg>)>) {
    let stamp = std::fs::metadata(config_store_path())
        .and_then(|m| m.modified())
        .ok();
    let stale = cache
        .as_ref()
        .map(|(cached, _)| stamp != Some(*cached))
        .unwrap_or(true);
    if stale {
        match stamp {
            Some(s) => *cache = Some((s, load_config_file())),
            None => *cache = None,
        }
    }
    let Some((_, cfgs)) = cache.as_ref() else {
        return;
    };
    for instance in instances.iter_mut() {
        if let Some(cfg) = cfgs.iter().find(|c| c.id == instance.id) {
            instance.config_toml = cfg.config_toml.clone();
            instance.auto_start = cfg.auto_start;
            instance.remote_manage = cfg.remote_manage;
            instance.whitelist = cfg.whitelist.clone();
        }
    }
}

fn cli_json(portal: &str, sub: &str) -> Result<Value, String> {
    let data = pipe_request(
        "run_cli",
        &json!({ "args": ["-p", portal, "-o", "json", sub] }),
    )?;
    let text = data
        .as_str()
        .ok_or_else(|| "内核未返回数据".to_owned())?;
    let trimmed = text.trim();
    match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => Ok(value),
        Err(e) => {
            let start = trimmed
                .find(['[', '{'])
                .ok_or_else(|| format!("内核数据解析失败：{e}"))?;
            serde_json::from_str::<Value>(&trimmed[start..])
                .map_err(|e| format!("内核数据解析失败：{e}"))
        }
    }
}

fn value_list(value: &Value, key: &str) -> Vec<Value> {
    if let Some(list) = value.as_array() {
        return list.clone();
    }
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn field_number(value: &Value, key: &str) -> f64 {
    match value.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => parse_number(s),
        _ => 0.0,
    }
}

fn parse_number(text: &str) -> f64 {
    let cleaned: String = text
        .chars()
        .skip_while(|c| !c.is_ascii_digit() && *c != '-' && *c != '.')
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    cleaned.parse().unwrap_or(0.0)
}

pub fn parse_bytes(text: &str) -> f64 {
    let lower = text.trim().to_lowercase();
    let mut number = String::new();
    let mut unit = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            if unit.is_empty() {
                number.push(ch);
            }
        } else if !ch.is_whitespace() {
            unit.push(ch);
        }
    }
    let value: f64 = number.parse().unwrap_or(0.0);
    let mult = match unit.as_str() {
        "kb" | "kib" => 1024.0,
        "mb" | "mib" => 1024.0 * 1024.0,
        "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    value * mult
}

fn parse_peers(value: &Value) -> Vec<Peer> {
    value_list(value, "peers")
        .iter()
        .map(|p| Peer {
            hostname: text_field(p, "hostname"),
            ip: text_field(p, "ipv4"),
            cost: match p.get("cost") {
                Some(Value::String(s)) => s.to_lowercase(),
                Some(Value::Number(n)) => n.to_string(),
                _ => String::new(),
            },
            latency: field_number(p, "lat_ms"),
            loss: field_number(p, "loss_rate"),
            rx: match p.get("rx_bytes") {
                Some(Value::String(s)) => parse_bytes(s),
                Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            },
            tx: match p.get("tx_bytes") {
                Some(Value::String(s)) => parse_bytes(s),
                Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
                _ => 0.0,
            },
            tunnel: text_field(p, "tunnel_proto"),
            id: text_field(p, "id"),
            version: text_field(p, "version"),
        })
        .collect()
}

fn parse_routes(value: &Value) -> Vec<Route> {
    value_list(value, "routes")
        .iter()
        .map(|r| {
            let next_host = text_field(r, "next_hop_hostname");
            let next_ip = text_field(r, "next_hop_ipv4");
            Route {
                ipv4: text_field(r, "ipv4"),
                hostname: text_field(r, "hostname"),
                proxy_cidrs: r
                    .get("proxy_cidrs")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                next_hop: if next_host.is_empty() { next_ip } else { next_host },
                hops: field_number(r, "path_len"),
                latency: field_number(r, "path_latency"),
                version: text_field(r, "version"),
            }
        })
        .collect()
}

fn parse_node(value: &Value) -> Option<NodeInfo> {
    let ipv4 = value
        .get("ipv4_addr")
        .or_else(|| value.get("ipv4"))
        .and_then(Value::as_str)?
        .to_owned();
    Some(NodeInfo {
        ipv4,
        hostname: text_field(value, "hostname"),
        version: text_field(value, "version"),
        peer_id: match value.get("peer_id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        },
    })
}

pub fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{:.0} B", value)
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

pub fn format_speed(mb_per_s: f32) -> String {
    if mb_per_s >= 1.0 {
        format!("{mb_per_s:.1} MB/s")
    } else if mb_per_s * 1024.0 >= 10.0 {
        format!("{:.0} KB/s", mb_per_s * 1024.0)
    } else {
        format!("{:.1} KB/s", mb_per_s * 1024.0)
    }
}
