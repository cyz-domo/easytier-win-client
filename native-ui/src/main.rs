#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, Vec2,
};
use serde_json::json;

mod config;
mod public_nodes;
mod service;
mod settings;
mod theme;
use theme::*;

const VERSION: &str = concat!("v", env!("EASYTIER_APP_VERSION"));

fn app_icon() -> (u32, u32, Vec<u8>) {
    static ICON: &[u8] = include_bytes!("../assets/icon.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICON))
        .read_info()
        .expect("read icon png");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("decode icon png");
    buf.truncate(info.buffer_size());
    if info.color_type == png::ColorType::Rgba {
        (info.width, info.height, buf)
    } else {
        let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
        for px in buf.chunks_exact(3) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        (info.width, info.height, rgba)
    }
}

#[cfg(windows)]
mod tray {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    static SHOW: AtomicBool = AtomicBool::new(false);
    static QUIT: AtomicBool = AtomicBool::new(false);
    static TOGGLE: AtomicBool = AtomicBool::new(false);

    pub fn take_show() -> bool {
        SHOW.swap(false, Ordering::Relaxed)
    }

    pub fn take_quit() -> bool {
        QUIT.swap(false, Ordering::Relaxed)
    }

    pub fn take_toggle() -> bool {
        TOGGLE.swap(false, Ordering::Relaxed)
    }

    pub struct TrayMenu {
        pub icon: TrayIcon,
        pub status: MenuItem,
        pub toggle: MenuItem,
        pub key: String,
    }

    fn icon() -> Icon {
        let (w, h, rgba) = super::app_icon();
        Icon::from_rgba(rgba, w, h).expect("valid tray icon")
    }

    pub fn setup(ctx: &eframe::egui::Context) -> Option<TrayMenu> {
        let menu = Menu::new();
        let status = MenuItem::new("EasyTier · 未选择网络", false, None);
        let toggle = MenuItem::new("启动当前网络", false, None);
        let show = MenuItem::new("显示主界面", true, None);
        let quit = MenuItem::new("退出", true, None);
        menu.append(&status).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&toggle).ok()?;
        menu.append(&show).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&quit).ok()?;
        let show_id = show.id().clone();
        let quit_id = quit.id().clone();
        let toggle_id = toggle.id().clone();

        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(Arc::new(move |e: MenuEvent| {
            if e.id == show_id {
                SHOW.store(true, Ordering::Relaxed);
                menu_ctx.request_repaint();
            } else if e.id == quit_id {
                QUIT.store(true, Ordering::Relaxed);
                menu_ctx.request_repaint();
            } else if e.id == toggle_id {
                TOGGLE.store(true, Ordering::Relaxed);
                menu_ctx.request_repaint();
            }
        })));

        let tray_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                SHOW.store(true, Ordering::Relaxed);
                tray_ctx.request_repaint();
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_tooltip("EasyTier · 原生客户端")
            .with_menu(Box::new(menu))
            .with_icon(icon())
            .build()
            .ok()?;
        Some(TrayMenu {
            icon,
            status,
            toggle,
            key: String::new(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Status,
    Traffic,
    Peers,
    Routes,
    Config,
    Logs,
    Settings,
}

const TABS: [(Tab, &str); 7] = [
    (Tab::Status, "状态"),
    (Tab::Traffic, "流量"),
    (Tab::Peers, "成员"),
    (Tab::Routes, "路由"),
    (Tab::Config, "配置"),
    (Tab::Logs, "日志"),
    (Tab::Settings, "设置"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogFilter {
    All,
    Info,
    Warn,
    Error,
}

impl LogFilter {
    fn label(self) -> &'static str {
        match self {
            LogFilter::All => "ALL",
            LogFilter::Info => "INFO",
            LogFilter::Warn => "WARN",
            LogFilter::Error => "ERROR",
        }
    }

    fn color(self) -> Color32 {
        match self {
            LogFilter::All => TEXT,
            LogFilter::Info => RX,
            LogFilter::Warn => WARN,
            LogFilter::Error => ERR,
        }
    }

    fn matches(self, level: &str) -> bool {
        match self {
            LogFilter::All => true,
            LogFilter::Info => level == "INFO",
            LogFilter::Warn => level == "WARN",
            LogFilter::Error => level == "ERROR",
        }
    }
}

struct LogLine {
    time: String,
    level: String,
    text: String,
}

fn classify_log(line: &str) -> LogLine {
    let trimmed = line.trim();
    let mut parts = trimmed.splitn(3, char::is_whitespace);
    if let (Some(stamp), Some(level), Some(rest)) = (parts.next(), parts.next(), parts.next()) {
        if stamp.len() >= 19 && stamp.contains(['T', '-']) && level.chars().all(|c| c.is_ascii_uppercase()) {
            let time = local_hms(stamp).unwrap_or_else(|| {
                stamp.get(11..19).unwrap_or(stamp).to_owned()
            });
            let mut level = level.to_owned();
            if level == "INFO" && looks_like_error(rest) {
                level = "ERROR".to_owned();
            }
            let level = if level == "TRACE" {
                "DEBUG".to_owned()
            } else {
                level
            };
            return LogLine {
                time,
                level,
                text: rest.trim().to_owned(),
            };
        }
    }
    let level = if looks_like_error(trimmed) {
        "ERROR"
    } else if trimmed.to_lowercase().contains("warn") {
        "WARN"
    } else {
        ""
    };
    LogLine {
        time: String::new(),
        level: level.to_owned(),
        text: trimmed.to_owned(),
    }
}

fn local_hms(stamp: &str) -> Option<String> {
    let body = stamp.trim_end_matches('Z');
    let naive = chrono::NaiveDateTime::parse_from_str(body, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    let utc = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc);
    Some(
        utc.with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string(),
    )
}

fn start_public_fetch(picker: &mut PublicPicker) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(public_nodes::fetch());
    });
    picker.rx = Some(rx);
}

fn render_public_node(
    ui: &mut egui::Ui,
    node: &public_nodes::PublicNode,
    existing: &[String],
    add: &mut Option<String>,
) {
    let already = existing.iter().any(|u| u == &node.address);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(truncate(&node.name, 26))
                .size(11.5)
                .color(if node.online { TEXT } else { DIM }),
        );
        ui.label(RichText::new(&node.address).monospace().size(10.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if already {
                ui.label(RichText::new("已添加").size(10.0).color(DIM));
            } else if ui.small_button("添加").clicked() {
                *add = Some(node.address.clone());
            }
            let ping = match node.local_ping {
                Some(ms) if ms >= 2 => format!("{ms} ms"),
                _ => "—".to_owned(),
            };
            let ping_color = match node.local_ping {
                Some(ms) if ms < 100 => RX,
                Some(ms) if ms < 250 => WARN,
                _ => MUTED,
            };
            ui.label(RichText::new(ping).size(10.0).color(ping_color));
            if node.official {
                ui.label(RichText::new("官方").size(9.5).color(ACCENT));
            }
            if !node.online {
                ui.label(RichText::new("离线").size(9.5).color(DIM));
            }
            if node.masked {
                ui.label(RichText::new("掩码").size(9.5).color(DIM));
            }
            if node.can_relay {
                ui.label(RichText::new("中转").size(9.5).color(MUTED));
            }
        });
    });
}

fn is_public_peer(p: &service::Peer) -> bool {
    let h = p.hostname.to_lowercase();
    h.starts_with("publicserver") || h.starts_with("public_") || h.contains("公共节点")
}

fn looks_like_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("error") || lower.contains("fail") || lower.contains("panic") || text.contains("TunnelError")
}

struct NewDialog {
    name: String,
    secret: String,
    method: u8,
    dhcp: bool,
}

struct PublicPicker {
    rx: Option<Receiver<Result<Vec<public_nodes::PublicNode>, String>>>,
    nodes: Vec<public_nodes::PublicNode>,
    error: Option<String>,
    query: String,
}

struct App {
    tab: Tab,
    search: String,
    peer_query: String,
    show_public_peers: bool,
    config: config::NetworkConfig,
    config_dirty: bool,
    config_source: String,
    config_error: Option<String>,
    show_advanced: bool,
    peer_urls_open: bool,
    secret_visible: bool,
    instance_name: String,
    rpc_port_text: String,
    auto_start: bool,
    remote_manage: bool,
    whitelist_text: String,
    console_level: String,
    new_dialog: Option<NewDialog>,
    public_picker: Option<PublicPicker>,
    confirm_delete: Option<(String, String)>,
    pending_active: Option<String>,
    peers: Vec<service::Peer>,
    selected_peer: Option<usize>,
    history: VecDeque<(f32, f32)>,
    node: Option<service::NodeInfo>,
    routes: Vec<service::Route>,
    route_query: String,
    kernel: Option<service::KernelProgress>,
    kernel_proxy: String,
    custom_proxy: String,
    selected_version: String,
    releases: Option<settings::ReleaseInfo>,
    releases_error: Option<String>,
    releases_loading: bool,
    proxies: Vec<settings::LocalProxy>,
    proxies_scanned: bool,
    autostart: Option<settings::Autostart>,
    autostart_min: bool,
    status_error: Option<String>,
    instances: Vec<service::Instance>,
    active: Option<String>,
    service_running: bool,
    service_error: Option<String>,
    session_rx: f64,
    session_tx: f64,
    last_counters: Option<(f64, f64, Instant)>,
    prev_running: bool,
    logs: Vec<LogLine>,
    log_filter: LogFilter,
    toast: Option<(String, f64, bool)>,
    busy: Option<&'static str>,
    font_loaded: bool,
    icon_tex: egui::TextureHandle,
    style: egui::Style,
    quitting: bool,
    hidden: bool,
    pending_hide: bool,
    visible_sent: Option<bool>,
    actions: Sender<service::Action>,
    events: Receiver<service::Event>,
    #[cfg(windows)]
    tray: Option<tray::TrayMenu>,
}

fn zero_history() -> VecDeque<(f32, f32)> {
    (0..30).map(|_| (0.0, 0.0)).collect()
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Self {
        let (font_loaded, style) = theme::setup(&cc.egui_ctx);
        let (icon_w, icon_h, icon_rgba) = app_icon();
        let icon_tex = cc.egui_ctx.load_texture(
            "app-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon_w as usize, icon_h as usize],
                &icon_rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let (actions, events) = service::spawn(cc.egui_ctx.clone());
        let mut app = Self {
            tab: Tab::Status,
            search: String::new(),
            peer_query: String::new(),
            show_public_peers: false,
            config: config::NetworkConfig::default(),
            config_dirty: false,
            config_source: String::new(),
            config_error: None,
            show_advanced: false,
            peer_urls_open: false,
            secret_visible: false,
            instance_name: String::new(),
            rpc_port_text: String::new(),
            auto_start: false,
            remote_manage: false,
            whitelist_text: String::new(),
            console_level: String::new(),
            new_dialog: None,
            public_picker: None,
            confirm_delete: None,
            pending_active: None,
            peers: Vec::new(),
            selected_peer: None,
            history: zero_history(),
            node: None,
            routes: Vec::new(),
            route_query: String::new(),
            kernel: None,
            kernel_proxy: "direct".to_owned(),
            custom_proxy: String::new(),
            selected_version: String::new(),
            releases: None,
            releases_error: None,
            releases_loading: false,
            proxies: Vec::new(),
            proxies_scanned: false,
            autostart: None,
            autostart_min: true,
            status_error: None,
            instances: Vec::new(),
            active: None,
            service_running: false,
            service_error: None,
            session_rx: 0.0,
            session_tx: 0.0,
            last_counters: None,
            prev_running: false,
            logs: Vec::new(),
            log_filter: LogFilter::All,
            toast: None,
            busy: None,
            font_loaded,
            icon_tex,
            style,
            quitting: false,
            hidden: start_hidden,
            pending_hide: start_hidden,
            visible_sent: None,
            actions,
            events,
            #[cfg(windows)]
            tray: tray::setup(&cc.egui_ctx),
        };
        let _ = app.actions.send(service::Action::AutostartGet);
        app
    }

    fn active_instance(&self) -> Option<&service::Instance> {
        let id = self.active.as_deref()?;
        self.instances.iter().find(|i| i.id == id)
    }

    fn active_name(&self) -> String {
        self.active_instance()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "未选择网络".to_owned())
    }

    fn active_running(&self) -> bool {
        self.active_instance().map(|i| i.running()).unwrap_or(false)
    }

    fn set_active(&mut self, id: String) {
        if self.active.as_deref() == Some(id.as_str()) {
            return;
        }
        self.active = Some(id.clone());
        let _ = self.actions.send(service::Action::SetActive(id));
        self.peers.clear();
        self.node = None;
        self.routes.clear();
        self.route_query.clear();
        self.status_error = None;
        self.selected_peer = None;
        self.history = zero_history();
        self.session_rx = 0.0;
        self.session_tx = 0.0;
        self.last_counters = None;
        self.prev_running = false;
        self.logs.clear();
        self.config_dirty = false;
        self.config_source.clear();
        self.instance_name.clear();
        self.rpc_port_text.clear();
        self.auto_start = false;
        self.remote_manage = false;
        self.whitelist_text.clear();
        self.console_level.clear();
        self.config_error = None;
    }

    fn ensure_config(&mut self) {
        if self.config_dirty {
            return;
        }
        let Some(inst) = self.active_instance().cloned() else {
            return;
        };
        if inst.config_toml.is_empty() || inst.config_toml == self.config_source {
            return;
        }
        match config::decode_toml(&inst.config_toml) {
            Ok(mut cfg) => {
                cfg.instance_id = inst.id.clone();
                self.config = cfg;
                self.config_error = None;
            }
            Err(e) => {
                self.config_error = Some(e);
                self.config = config::NetworkConfig {
                    instance_id: inst.id.clone(),
                    ..Default::default()
                };
            }
        }
        self.instance_name = inst.name.clone();
        self.rpc_port_text = inst.rpc_port.to_string();
        self.auto_start = inst.auto_start;
        self.remote_manage = inst.remote_manage;
        self.whitelist_text = inst.whitelist.join(", ");
        self.console_level = inst.console_log_level.clone().unwrap_or_default();
        self.config_source = inst.config_toml.clone();
    }

    fn update_traffic(&mut self) {
        let rx_total: f64 = self.peers.iter().map(|p| p.rx).sum();
        let tx_total: f64 = self.peers.iter().map(|p| p.tx).sum();
        let now = Instant::now();
        if let Some((prev_rx, prev_tx, at)) = self.last_counters {
            let dt = now.duration_since(at).as_secs_f64().max(0.05);
            let drx = (rx_total - prev_rx).max(0.0);
            let dtx = (tx_total - prev_tx).max(0.0);
            self.session_rx += drx;
            self.session_tx += dtx;
            let rx_speed = (drx / dt / (1024.0 * 1024.0)) as f32;
            let tx_speed = (dtx / dt / (1024.0 * 1024.0)) as f32;
            self.history.push_back((rx_speed, tx_speed));
            while self.history.len() > 30 {
                self.history.pop_front();
            }
        }
        self.last_counters = Some((rx_total, tx_total, now));
    }

    fn reset_session(&mut self) {
        self.session_rx = 0.0;
        self.session_tx = 0.0;
        self.last_counters = None;
        self.history = zero_history();
    }

    fn toast(&mut self, ctx: &egui::Context, message: impl Into<String>, ok: bool) {
        self.toast = Some((message.into(), ctx.input(|i| i.time), ok));
    }

    fn rpc_port(&self) -> Option<u16> {
        self.rpc_port_text
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
    }

    fn validate_all(&self) -> Result<(), String> {
        if let Some(err) = &self.config_error {
            return Err(format!("TOML 解析失败：{err}"));
        }
        config::validate(&self.config)?;
        if self.rpc_port().is_none() {
            return Err("RPC 端口必须在 1-65535 之间".to_owned());
        }
        if self.instance_name.trim().is_empty() {
            return Err("实例名称不能为空".to_owned());
        }
        if self.remote_manage && config::parse_whitelist(&self.whitelist_text).is_empty() {
            return Err(
                "开启远程管理必须填写 RPC 白名单（填 EasyTier 虚拟网段，如 10.126.126.0/24）"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn effective_name(&self) -> String {
        let name = self.instance_name.trim();
        if name.is_empty() {
            self.config.network_name.trim().to_owned()
        } else {
            name.to_owned()
        }
    }

    fn build_payload(
        &self,
        inst: &service::Instance,
        toml_text: String,
        desired: &str,
    ) -> serde_json::Value {
        json!({
            "instance_id": inst.id,
            "name": self.effective_name(),
            "config_toml": toml_text,
            "rpc_port": self.rpc_port().unwrap_or(inst.rpc_port),
            "auto_start": self.auto_start,
            "desired_state": desired,
            "remote_manage_enabled": self.remote_manage,
            "rpc_whitelist_cidrs": config::parse_whitelist(&self.whitelist_text),
            "console_log_level": if self.console_level.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(self.console_level.clone())
            },
        })
    }

    fn serialize_config(&self, inst: &service::Instance) -> Result<String, String> {
        if self.config_dirty || self.config_source.is_empty() {
            let mut cfg = self.config.clone();
            cfg.instance_id = inst.id.clone();
            Ok(config::encode_toml(&cfg, true))
        } else {
            Ok(inst.config_toml.clone())
        }
    }

    fn mark_saved(&mut self, toml_text: &str) {
        self.config_dirty = false;
        self.config_source = toml_text.to_owned();
        let Some(id) = self.active.clone() else {
            return;
        };
        let name = self.effective_name();
        let rpc_port = self.rpc_port().unwrap_or(0);
        let whitelist = config::parse_whitelist(&self.whitelist_text);
        let level = if self.console_level.is_empty() {
            None
        } else {
            Some(self.console_level.clone())
        };
        if let Some(inst) = self.instances.iter_mut().find(|i| i.id == id) {
            inst.config_toml = toml_text.to_owned();
            inst.name = name;
            if rpc_port > 0 {
                inst.rpc_port = rpc_port;
            }
            inst.auto_start = self.auto_start;
            inst.remote_manage = self.remote_manage;
            inst.whitelist = whitelist;
            inst.console_log_level = level;
        }
    }

    fn toggle_running(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() {
            return;
        }
        self.ensure_config();
        let Some(inst) = self.active_instance().cloned() else {
            self.toast(ctx, "没有可操作的网络实例", false);
            return;
        };
        let start = !inst.running();
        if start {
            if let Err(e) = self.validate_all() {
                self.toast(ctx, format!("请先修正配置：{e}"), false);
                self.tab = Tab::Config;
                return;
            }
        }
        let toml_text = match self.serialize_config(&inst) {
            Ok(text) => text,
            Err(e) => {
                self.toast(ctx, e, false);
                return;
            }
        };
        let payload = self.build_payload(
            &inst,
            toml_text.clone(),
            if start { "running" } else { "stopped" },
        );
        self.mark_saved(&toml_text);
        self.busy = Some(if start { "启动中…" } else { "停止中…" });
        let _ = self.actions.send(service::Action::SetRunning {
            id: inst.id,
            start,
            payload: Some(payload),
        });
    }

    fn save_config(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() {
            return;
        }
        self.ensure_config();
        let Some(inst) = self.active_instance().cloned() else {
            self.toast(ctx, "没有可保存的实例", false);
            return;
        };
        if let Err(e) = self.validate_all() {
            self.toast(ctx, e, false);
            return;
        }
        let toml_text = match self.serialize_config(&inst) {
            Ok(text) => text,
            Err(e) => {
                self.toast(ctx, e, false);
                return;
            }
        };
        let desired = if inst.running() { "running" } else { "stopped" };
        let payload = self.build_payload(&inst, toml_text.clone(), desired);
        self.busy = Some("保存中…");
        self.mark_saved(&toml_text);
        let _ = self.actions.send(service::Action::Sync {
            payload,
            label: "保存配置".to_owned(),
        });
    }

    fn reset_config(&mut self, ctx: &egui::Context) {
        let Some(inst) = self.active_instance().cloned() else {
            return;
        };
        self.config_source.clear();
        self.config_dirty = false;
        self.ensure_config();
        self.toast(ctx, "已恢复为服务端保存的配置", true);
    }

    fn open_new_dialog(&mut self) {
        self.new_dialog = Some(NewDialog {
            name: "新网络".to_owned(),
            secret: String::new(),
            method: 2,
            dhcp: true,
        });
    }

    fn open_public_picker(&mut self) {
        let mut picker = PublicPicker {
            rx: None,
            nodes: Vec::new(),
            error: None,
            query: String::new(),
        };
        start_public_fetch(&mut picker);
        self.public_picker = Some(picker);
    }

    fn refresh_public_picker(&mut self) {
        if let Some(picker) = self.public_picker.as_mut() {
            picker.error = None;
            start_public_fetch(picker);
        }
    }

    fn next_rpc_port(&self) -> Option<u16> {
        let used: Vec<u16> = self.instances.iter().map(|i| i.rpc_port).collect();
        for port in 15888u16..16888 {
            if used.contains(&port) {
                continue;
            }
            if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return Some(port);
            }
        }
        None
    }

    fn create_instance(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.new_dialog.take() else {
            return;
        };
        let name = dlg.name.trim().to_owned();
        if name.is_empty() {
            self.toast(ctx, "实例名称不能为空", false);
            self.new_dialog = Some(dlg);
            return;
        }
        let Some(rpc_port) = self.next_rpc_port() else {
            self.toast(ctx, "没有可用的 RPC 端口（15888-16887）", false);
            self.new_dialog = Some(dlg);
            return;
        };
        let id = uuid::Uuid::new_v4().to_string();
        let mut cfg = config::NetworkConfig::default();
        cfg.instance_id = id.clone();
        cfg.network_name = name.clone();
        cfg.network_secret = dlg.secret;
        cfg.networking_method = dlg.method;
        cfg.dhcp = dlg.dhcp;
        cfg.listener_urls = if self.instances.is_empty() {
            vec![
                "tcp://0.0.0.0:11010".to_owned(),
                "udp://0.0.0.0:11010".to_owned(),
                "wg://0.0.0.0:11011".to_owned(),
            ]
        } else {
            vec![
                "tcp://0.0.0.0:0".to_owned(),
                "udp://0.0.0.0:0".to_owned(),
                "wg://0.0.0.0:0".to_owned(),
            ]
        };
        let toml_text = config::encode_toml(&cfg, true);
        let payload = json!({
            "instance_id": id.clone(),
            "name": name,
            "config_toml": toml_text,
            "rpc_port": rpc_port,
            "auto_start": false,
            "desired_state": "stopped",
            "remote_manage_enabled": false,
            "rpc_whitelist_cidrs": [],
            "console_log_level": serde_json::Value::Null,
        });
        self.busy = Some("新建中…");
        self.pending_active = Some(id);
        let _ = self.actions.send(service::Action::Create { payload });
    }

    fn clone_instance(&mut self, ctx: &egui::Context) {
        let Some(inst) = self.active_instance().cloned() else {
            self.toast(ctx, "没有可克隆的网络实例", false);
            return;
        };
        let Some(rpc_port) = self.next_rpc_port() else {
            self.toast(ctx, "没有可用的 RPC 端口（15888-16887）", false);
            return;
        };
        let mut cfg = match config::decode_toml(&inst.config_toml) {
            Ok(cfg) => cfg,
            Err(e) => {
                self.toast(ctx, format!("克隆失败：{e}"), false);
                return;
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        cfg.instance_id = id.clone();
        cfg.listener_urls = vec![
            "tcp://0.0.0.0:0".to_owned(),
            "udp://0.0.0.0:0".to_owned(),
            "wg://0.0.0.0:0".to_owned(),
        ];
        let name = format!("{} 副本", inst.name);
        let toml_text = config::encode_toml(&cfg, true);
        let payload = json!({
            "instance_id": id.clone(),
            "name": name,
            "config_toml": toml_text,
            "rpc_port": rpc_port,
            "auto_start": false,
            "desired_state": "stopped",
            "remote_manage_enabled": false,
            "rpc_whitelist_cidrs": [],
            "console_log_level": serde_json::Value::Null,
        });
        self.busy = Some("克隆中…");
        self.pending_active = Some(id);
        let _ = self.actions.send(service::Action::Create { payload });
    }

    fn confirm_remove(&mut self) {
        let Some(inst) = self.active_instance().cloned() else {
            return;
        };
        if self.instances.len() <= 1 {
            return;
        }
        self.confirm_delete = Some((inst.id, inst.name));
    }

    fn delete_instance(&mut self, _ctx: &egui::Context) {
        let Some((id, _name)) = self.confirm_delete.take() else {
            return;
        };
        if self.instances.len() <= 1 {
            return;
        }
        self.busy = Some("删除中…");
        let _ = self.actions.send(service::Action::Remove { id });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.new_dialog.is_some() {
            let mut create = false;
            let mut cancel = false;
            {
                let dlg = self.new_dialog.as_mut().expect("checked");
                egui::Window::new("新建网络实例")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.set_width(360.0);
                        ui.add_space(4.0);
                        ui.label(RichText::new("实例名称").size(11.5).color(MUTED));
                        ui.add_sized(
                            [ui.available_width(), 28.0],
                            egui::TextEdit::singleline(&mut dlg.name)
                                .hint_text("例如：home-net"),
                        );
                        ui.add_space(6.0);
                        ui.label(RichText::new("网络密钥（可留空）").size(11.5).color(MUTED));
                        ui.add_sized(
                            [ui.available_width(), 28.0],
                            egui::TextEdit::singleline(&mut dlg.secret)
                                .password(true)
                                .hint_text("用于验证节点归属"),
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("组网方式").size(11.5).color(MUTED));
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut dlg.method, 2, "独立网络");
                            ui.selectable_value(&mut dlg.method, 1, "指定节点");
                        });
                        if dlg.method == 2 {
                            ui.label(
                                RichText::new("本机作为根节点监听，其他节点通过监听地址加入。")
                                    .size(10.5)
                                    .color(DIM),
                            );
                        } else {
                            ui.label(
                                RichText::new("创建后可在配置页填写对等节点地址。")
                                    .size(10.5)
                                    .color(DIM),
                            );
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if switch(ui, dlg.dhcp) {
                                dlg.dhcp = !dlg.dhcp;
                            }
                            ui.label(
                                RichText::new("DHCP 自动分配虚拟 IP")
                                    .size(11.5)
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new(
                                    RichText::new("创建").color(Color32::WHITE),
                                ).fill(ACCENT))
                                .clicked()
                            {
                                create = true;
                            }
                            if ui.button("取消").clicked() {
                                cancel = true;
                            }
                        });
                    });
            }
            if create {
                self.create_instance(ctx);
            } else if cancel {
                self.new_dialog = None;
            }
        }

        if let Some((_id, name)) = self.confirm_delete.clone() {
            let mut confirmed = false;
            let mut cancel = false;
            egui::Window::new("删除网络实例")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("确定要删除网络实例「{name}」吗？")).size(12.5));
                    ui.label(
                        RichText::new("该实例的本地配置将被移除，若网络正在运行将先自动断开。")
                            .size(10.5)
                            .color(DIM),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("删除").color(Color32::WHITE))
                                    .fill(ERR.gamma_multiply(0.75)),
                            )
                            .clicked()
                        {
                            confirmed = true;
                        }
                        if ui.button("取消").clicked() {
                            cancel = true;
                        }
                    });
                });
            if confirmed {
                self.delete_instance(ctx);
            } else if cancel {
                self.confirm_delete = None;
            }
        }

        if self.public_picker.is_some() {
            let mut refresh = false;
            let mut close = false;
            let mut add_addr: Option<String> = None;
            {
                let picker = self.public_picker.as_mut().expect("checked");
                if let Some(rx) = &picker.rx {
                    if let Ok(result) = rx.try_recv() {
                        picker.rx = None;
                        match result {
                            Ok(nodes) => {
                                picker.nodes = nodes;
                                picker.error = None;
                            }
                            Err(e) => picker.error = Some(e),
                        }
                    }
                }
                egui::Window::new("添加公共节点")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.set_width(560.0);
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [260.0, 26.0],
                                egui::TextEdit::singleline(&mut picker.query)
                                    .hint_text("搜索节点名称 / 地址"),
                            );
                            if ui.button("刷新").clicked() {
                                refresh = true;
                            }
                            if picker.rx.is_some() {
                                ui.spinner();
                                ui.label(RichText::new("正在拉取并测速…").size(10.5).color(DIM));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("关闭").clicked() {
                                        close = true;
                                    }
                                },
                            );
                        });
                        if let Some(err) = &picker.error {
                            ui.label(RichText::new(format!("拉取失败：{err}")).size(10.5).color(WARN));
                        }
                        ui.add_space(6.0);
                        let q = picker.query.trim().to_lowercase();
                        let filtered: Vec<usize> = picker
                            .nodes
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| {
                                q.is_empty()
                                    || n.name.to_lowercase().contains(&q)
                                    || n.address.to_lowercase().contains(&q)
                            })
                            .map(|(i, _)| i)
                            .collect();
                        if picker.nodes.is_empty() && picker.rx.is_none() && picker.error.is_none() {
                            ui.label(RichText::new("暂无节点列表").size(11.0).color(DIM));
                        }
                        ScrollArea::vertical()
                            .id_salt("public-node-list")
                            .max_height(360.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for section in [false, true] {
                                    let label = if section { "海外节点" } else { "国内节点" };
                                    let mut section_nodes = filtered
                                        .iter()
                                        .map(|&i| &picker.nodes[i])
                                        .filter(|n| n.overseas == section);
                                    let first = section_nodes.next();
                                    let Some(first) = first else { continue };
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(label).size(11.0).color(DIM));
                                    render_public_node(ui, first, &self.config.peer_urls, &mut add_addr);
                                    for node in section_nodes {
                                        render_public_node(ui, node, &self.config.peer_urls, &mut add_addr);
                                    }
                                }
                            });
                    });
            }
            if refresh {
                self.refresh_public_picker();
            }
            if close {
                self.public_picker = None;
            }
            if let Some(addr) = add_addr {
                if !self.config.peer_urls.contains(&addr) {
                    self.config.peer_urls.push(addr);
                    self.config_dirty = true;
                    self.peer_urls_open = true;
                }
            }
        }
    }

    fn display_ip(&self) -> String {
        if let Some(node) = &self.node {
            let ip = node.ipv4.split('/').next().unwrap_or("");
            if !ip.is_empty() {
                return ip.to_owned();
            }
        }
        if !self.config.dhcp {
            self.config.virtual_ipv4.trim().to_owned()
        } else {
            String::new()
        }
    }

    fn copy_ip(&mut self, ctx: &egui::Context) {
        let ip = self.display_ip();
        if !ip.is_empty() {
            ctx.copy_text(ip.clone());
            self.toast(ctx, format!("已复制虚拟 IP：{ip}"), true);
        }
    }

    fn process_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                service::Event::Service(state) => {
                    self.service_running = state.running;
                    self.service_error = state.error;
                    if state.running {
                        self.instances = state.instances;
                        if self.active.is_none()
                            || !self
                                .instances
                                .iter()
                                .any(|i| Some(i.id.as_str()) == self.active.as_deref())
                        {
                            if let Some(first) = self.instances.first() {
                                let id = first.id.clone();
                                self.set_active(id);
                            } else {
                                self.active = None;
                            }
                        }
                        if let Some(pending) = self.pending_active.clone() {
                            if self.instances.iter().any(|i| i.id == pending) {
                                self.pending_active = None;
                                self.set_active(pending);
                            }
                        }
                        self.ensure_config();
                        let running = self.active_running();
                        if self.prev_running && !running {
                            self.reset_session();
                        }
                        if !running {
                            self.peers.clear();
                            self.node = None;
                            self.routes.clear();
                            self.status_error = None;
                        }
                        self.prev_running = running;
                    } else {
                        self.instances.clear();
                        self.peers.clear();
                        self.node = None;
                        self.routes.clear();
                        self.prev_running = false;
                    }
                }
                service::Event::Status(update) => {
                    if Some(update.instance_id.as_str()) != self.active.as_deref() {
                        continue;
                    }
                    if let Some(node) = update.node {
                        self.node = Some(node);
                    }
                    if update.error.is_some() {
                        self.status_error = update.error;
                    } else {
                        self.status_error = None;
                        self.routes = update.routes;
                        self.peers = update.peers;
                        if self.selected_peer.map(|i| i >= self.peers.len()).unwrap_or(false) {
                            self.selected_peer = None;
                        }
                        self.busy = None;
                        self.update_traffic();
                    }
                }
                service::Event::Logs { instance_id, lines } => {
                    if Some(instance_id.as_str()) != self.active.as_deref() {
                        continue;
                    }
                    self.logs = lines.iter().rev().map(|l| classify_log(l)).collect();
                }
                service::Event::Action(result) => {
                    self.busy = None;
                    if !result.ok && result.label == "保存配置" {
                        self.config_dirty = true;
                    }
                    if !result.ok && result.label == "新建网络" {
                        self.pending_active = None;
                    }
                    if !result.ok && result.label == "内核更新" {
                        self.kernel = None;
                    }
                    self.toast(ctx, format!("{}：{}", result.label, result.message), result.ok);
                }
                service::Event::Kernel(progress) => {
                    let was_terminal = self.kernel.as_ref().map(|k| k.terminal()).unwrap_or(false);
                    if progress.terminal() && !was_terminal {
                        let ok = progress.status == "completed";
                        let message = if progress.message.is_empty() {
                            progress.phase_label().to_owned()
                        } else {
                            progress.message.clone()
                        };
                        self.toast(ctx, format!("内核更新：{message}"), ok);
                    }
                    self.kernel = Some(progress);
                }
                service::Event::Proxies(list) => {
                    self.proxies_scanned = true;
                    let count = list.len();
                    self.proxies = list;
                    if let Some(first) = self.proxies.first() {
                        if self.kernel_proxy == "direct" || self.kernel_proxy.is_empty() {
                            self.kernel_proxy = format!("local:{}", first.addr);
                        }
                        self.toast(ctx, format!("发现 {count} 个可用本地代理"), true);
                    } else {
                        self.toast(ctx, "未发现可用的本地代理，将使用直连", false);
                    }
                }
                service::Event::Releases(result) => {
                    self.releases_loading = false;
                    match result {
                        Ok(info) => {
                            let latest = info.latest.clone();
                            self.releases = Some(info);
                            self.releases_error = None;
                            if latest.is_none() {
                                self.toast(ctx, "未找到包含 Windows 资源包的最新版本", false);
                            }
                        }
                        Err(e) => {
                            self.releases_error = Some(e);
                        }
                    }
                }
                service::Event::Autostart(state) => {
                    let changed = self.autostart.is_none();
                    if changed || self.autostart.as_ref().map(|a| a.enabled != state.enabled).unwrap_or(false) {
                        self.autostart_min = state.minimized;
                    }
                    self.autostart = Some(state);
                }
            }
        }
    }

    fn status_text(&self) -> (&'static str, Color32) {
        if self.service_error.is_some() {
            return ("后台服务不可用", ERR);
        }
        let Some(inst) = self.active_instance() else {
            return ("未选择网络", MUTED);
        };
        match inst.observed.as_str() {
            "running" => ("已连接", OK),
            "starting" => ("启动中", WARN),
            "stopping" => ("停止中", WARN),
            "failed" => ("启动失败", ERR),
            _ => ("未运行", MUTED),
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let w = ui.available_width();
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();

        let (brand, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
        let icon_rect = Rect::from_center_size(
            Pos2::new(brand.left() + 9.0, brand.center().y),
            Vec2::splat(18.0),
        );
        p.image(
            self.icon_tex.id(),
            icon_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        txt(&p, Pos2::new(brand.left() + 24.0, brand.center().y), Align2::LEFT_CENTER, "EasyTier", 14.5, TEXT);
        txt(&p, Pos2::new(brand.left() + 26.0 + text_w("EasyTier", 14.5), brand.center().y), Align2::LEFT_CENTER, "native", 10.0, DIM);
        let vw = text_w(VERSION, 10.5) + 14.0;
        p.rect_filled(Rect::from_min_size(Pos2::new(brand.right() - vw, brand.center().y - 9.0), Vec2::new(vw, 18.0)), CornerRadius::same(9), CARD);
        txt(&p, Pos2::new(brand.right() - vw / 2.0, brand.center().y), Align2::CENTER_CENTER, VERSION, 10.5, MUTED);

        ui.add_space(10.0);
        ui.add_sized([w, 30.0], egui::TextEdit::singleline(&mut self.search).hint_text("搜索网络"));
        ui.add_space(12.0);

        let (lab, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
        let query = self.search.trim().to_lowercase();
        let instances = self.instances.clone();
        let visible: Vec<&service::Instance> = instances
            .iter()
            .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query))
            .collect();
        let count_label = if query.is_empty() {
            format!("{} 个", instances.len())
        } else {
            format!("{}/{} 个", visible.len(), instances.len())
        };
        txt(&p, Pos2::new(lab.left() + 2.0, lab.center().y), Align2::LEFT_CENTER, "网络", 11.0, DIM);
        txt(&p, Pos2::new(lab.right() - 2.0, lab.center().y), Align2::RIGHT_CENTER, count_label, 10.5, DIM);

        let active = self.active.clone();
        let mut clicked: Option<String> = None;
        for inst in visible {
            let selected = active.as_deref() == Some(inst.id.as_str());
            let (item, resp) = ui.allocate_exact_size(Vec2::new(w, 58.0), Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if selected || resp.hovered() {
                p.rect_filled(item, CornerRadius::same(9), CARD);
                p.rect_stroke(item, CornerRadius::same(9), Stroke::new(1.0, BORDER), StrokeKind::Inside);
            }
            if selected {
                p.rect_filled(Rect::from_min_size(Pos2::new(item.left(), item.top() + 12.0), Vec2::new(3.0, item.height() - 24.0)), CornerRadius::same(2), ACCENT);
            }
            txt(&p, Pos2::new(item.left() + 16.0, item.top() + 12.0), Align2::LEFT_TOP, truncate(&inst.name, 14), 13.5, if selected { TEXT } else { MUTED });
            let sub = inst.status_label();
            txt(&p, Pos2::new(item.left() + 16.0, item.bottom() - 10.0), Align2::LEFT_BOTTOM, sub, 11.0, MUTED);
            let dot = match inst.observed.as_str() {
                "running" => OK,
                "starting" | "stopping" => WARN,
                "failed" => ERR,
                _ => DIM,
            };
            p.circle_filled(Pos2::new(item.right() - 16.0, item.center().y), 4.0, dot);
            if resp.clicked() {
                clicked = Some(inst.id.clone());
            }
            ui.add_space(6.0);
        }
        if let Some(id) = clicked {
            self.set_active(id);
        }

        if self.instances.is_empty() {
            let (empty, _) = ui.allocate_exact_size(Vec2::new(w, 44.0), Sense::hover());
            let message = if let Some(err) = &self.service_error {
                format!("服务不可用\n{err}")
            } else if self.service_running {
                "暂无网络实例".to_owned()
            } else {
                "正在连接后台服务…".to_owned()
            };
            txt(&p, Pos2::new(empty.left() + 4.0, empty.top()), Align2::LEFT_TOP, message, 11.0, DIM);
        }

        let rest = (ui.available_height() - 100.0).max(0.0);
        ui.allocate_space(Vec2::new(w, rest));

        let (row, _) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::hover());
        let bw = (w - 16.0) / 3.0;
        let has_active = self.active_instance().is_some();
        let actions = [("+ 新建", true), ("克隆", has_active), ("刷新", true)];
        for (i, (label, enabled)) in actions.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(row.left() + i as f32 * (bw + 8.0), row.top()), Vec2::new(bw, 30.0));
            let resp = ui.interact(r, egui::Id::new(("sidebar-action", i)), (if *enabled { Sense::click() } else { Sense::hover() }));
            p.rect_filled(r, CornerRadius::same(8), if resp.hovered() && *enabled { CARD_HI } else { CARD });
            p.rect_stroke(r, CornerRadius::same(8), Stroke::new(1.0, BORDER), StrokeKind::Inside);
            let color = if *enabled { if resp.hovered() { TEXT } else { MUTED } } else { DIM };
            txt(&p, r.center(), Align2::CENTER_CENTER, *label, 11.5, color);
            if resp.hovered() && *enabled {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() && *enabled {
                if i == 0 {
                    self.open_new_dialog();
                } else if i == 1 {
                    self.clone_instance(&ctx);
                } else {
                    let _ = self.actions.send(service::Action::Refresh);
                    self.toast(&ctx, "已请求刷新", true);
                }
            }
        }

        ui.add_space(10.0);
        let rows = if self.font_loaded { 1.0 } else { 2.0 };
        let (foot, _) = ui.allocate_exact_size(Vec2::new(w, 16.0 + rows * 16.0), Sense::hover());
        let (foot_text, foot_color) = if let Some(err) = &self.service_error {
            (format!("服务不可用 · {}", truncate(err, 24)), ERR)
        } else if self.service_running {
            ("已连接后台服务".to_owned(), OK)
        } else {
            ("正在连接后台服务…".to_owned(), DIM)
        };
        txt(&p, Pos2::new(foot.left() + 2.0, foot.top()), Align2::LEFT_TOP, foot_text, 10.5, foot_color);
        if !self.font_loaded {
            txt(&p, Pos2::new(foot.left() + 2.0, foot.top() + 16.0), Align2::LEFT_TOP, "CJK 字体缺失，中文将显示为方块", 10.5, ERR);
        }
    }

    fn topbar(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::hover());
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();

        let name = self.active_name();
        let (status, dot) = self.status_text();
        txt(&p, Pos2::new(rect.left() + 2.0, rect.center().y), Align2::LEFT_CENTER, &name, 15.0, TEXT);
        p.circle_filled(Pos2::new(rect.left() + 12.0 + text_w(&name, 15.0), rect.center().y), 3.5, dot);
        txt(&p, Pos2::new(rect.left() + 22.0 + text_w(&name, 15.0), rect.center().y), Align2::LEFT_CENTER, status, 11.5, MUTED);

        let seg_w = 54.0;
        let gap = 2.0;
        let total = TABS.len() as f32 * seg_w + (TABS.len() as f32 - 1.0) * gap + 8.0;
        let tabs = Rect::from_min_size(Pos2::new(rect.right() - total - 2.0, rect.center().y - 15.0), Vec2::new(total, 30.0));
        p.rect_filled(tabs, CornerRadius::same(9), CARD);
        p.rect_stroke(tabs, CornerRadius::same(9), Stroke::new(1.0, BORDER), StrokeKind::Inside);

        for (i, (tab, label)) in TABS.iter().enumerate() {
            let seg = Rect::from_min_size(Pos2::new(tabs.left() + 4.0 + i as f32 * (seg_w + gap), tabs.top() + 3.0), Vec2::new(seg_w, 24.0));
            let resp = ui.interact(seg, egui::Id::new(("tab", i)), Sense::click());
            let active = self.tab == *tab;
            if active {
                ui.painter().rect_filled(seg, CornerRadius::same(7), CARD_HI);
            }
            ui.painter().text(seg.center(), Align2::CENTER_CENTER, *label, FontId::proportional(12.5), if active { TEXT } else { MUTED });
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.tab = *tab;
            }
        }

        let button = Rect::from_min_size(Pos2::new(tabs.left() - 108.0, rect.center().y - 14.0), Vec2::new(98.0, 28.0));
        let observed = self
            .active_instance()
            .map(|i| i.observed.clone())
            .unwrap_or_default();
        let transitioning = matches!(observed.as_str(), "starting" | "stopping");
        let has_instance = self.active_instance().is_some();
        let enabled = has_instance && self.busy.is_none() && !transitioning;
        let running = self.active_running();
        let resp = ui.interact(button, egui::Id::new("run-toggle"), if enabled { Sense::click() } else { Sense::hover() });
        let label = if let Some(busy) = self.busy {
            busy
        } else if observed == "starting" {
            "启动中…"
        } else if observed == "stopping" {
            "停止中…"
        } else if running {
            "停止网络"
        } else {
            "启动网络"
        };
        let (fill, fg, stroke) = if !enabled {
            if observed == "stopping" {
                (ERR.gamma_multiply(0.16), ERR, ERR.gamma_multiply(0.55))
            } else {
                (CARD, DIM, BORDER)
            }
        } else if running {
            let red = if resp.hovered() { ERR.gamma_multiply(0.82) } else { ERR };
            (red, Color32::WHITE, red)
        } else {
            let blue = if resp.hovered() { ACCENT.gamma_multiply(0.85) } else { ACCENT };
            (blue, Color32::WHITE, blue)
        };
        p.rect_filled(button, CornerRadius::same(8), fill);
        p.rect_stroke(button, CornerRadius::same(8), Stroke::new(1.0, stroke), StrokeKind::Inside);
        txt(&p, button.center(), Align2::CENTER_CENTER, label, 11.5, fg);
        if resp.hovered() && enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && enabled {
            self.toggle_running(&ctx);
        }

        p.line_segment([Pos2::new(rect.left(), rect.bottom() - 0.5), Pos2::new(rect.right(), rect.bottom() - 0.5)], Stroke::new(1.0, BORDER));
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, p: &egui::Painter, title: &str, message: &str, action: Option<&str>) {
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 180.0), Sense::hover());
        card(p, r);
        txt(p, Pos2::new(r.left() + 24.0, r.top() + 28.0), Align2::LEFT_TOP, title, 16.0, TEXT);
        txt(p, Pos2::new(r.left() + 24.0, r.top() + 60.0), Align2::LEFT_TOP, message, 12.0, MUTED);
        if let Some(label) = action {
            let button = Rect::from_min_size(Pos2::new(r.left() + 24.0, r.top() + 100.0), Vec2::new(110.0, 32.0));
            let ctx = ui.ctx().clone();
            let resp = ui.interact(button, egui::Id::new(("empty-action", label)), Sense::click());
            p.rect_filled(button, CornerRadius::same(8), if resp.hovered() { ACCENT.gamma_multiply(0.9) } else { ACCENT });
            txt(p, button.center(), Align2::CENTER_CENTER, label, 12.0, Color32::WHITE);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                if !self.service_running {
                    let _ = self.actions.send(service::Action::Refresh);
                    self.toast(&ctx, "正在重试连接后台服务…", true);
                } else {
                    self.toggle_running(&ctx);
                }
            }
        }
    }

    fn effective_proxy(&self) -> String {
        let value = self.kernel_proxy.trim();
        if value == "mirror" {
            self.custom_proxy.trim().to_owned()
        } else if value.is_empty() {
            "direct".to_owned()
        } else {
            value.to_owned()
        }
    }

    fn start_kernel_update(&mut self, ctx: &egui::Context) {
        if !self.service_running {
            self.toast(ctx, "后台服务不可用，无法更新内核", false);
            return;
        }
        if self.kernel.as_ref().map(|k| !k.terminal()).unwrap_or(false) {
            return;
        }
        let proxy = self.effective_proxy();
        if proxy.is_empty() {
            self.toast(ctx, "请填写镜像前缀，或改用直连 / 本地代理", false);
            return;
        }
        let target = if self.selected_version.trim().is_empty() {
            None
        } else {
            Some(self.selected_version.trim().to_owned())
        };
        self.kernel = Some(service::KernelProgress {
            status: "queued".to_owned(),
            phase: "queued".to_owned(),
            message: "正在提交更新任务…".to_owned(),
            ..Default::default()
        });
        let _ = self.actions.send(service::Action::KernelStart { proxy, target });
    }

    #[cfg(windows)]
    fn sync_tray(&mut self) {
        let name = self.active_name();
        let (status, _) = self.status_text();
        let has_instance = self.active_instance().is_some();
        let running = self.active_running();
        let toggle_label = if running {
            "停止当前网络"
        } else {
            "启动当前网络"
        };
        let enabled = has_instance && self.busy.is_none();
        let tooltip = format!("EasyTier · {name} · {status}");
        let status_text = format!("{name} · {status}");
        let key = format!("{name}|{status}|{toggle_label}|{enabled}");
        let Some(tray) = self.tray.as_mut() else {
            return;
        };
        if tray.key == key {
            return;
        }
        tray.key = key;
        let _ = tray.status.set_text(status_text);
        let _ = tray.toggle.set_text(toggle_label);
        tray.toggle.set_enabled(enabled);
        let _ = tray.icon.set_tooltip(Some(tooltip));
    }

    fn status_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();

        if let Some(err) = self.service_error.clone() {
            self.empty_state(ui, &p, "后台服务不可用", &err, Some("重试连接"));
            return;
        }
        if self.active_instance().is_none() {
            if !self.service_running && self.service_error.is_none() {
                self.empty_state(ui, &p, "正在读取后台服务", "首次连接服务需要一到两秒", None);
            } else {
                self.empty_state(ui, &p, "没有可用的网络实例", "点击左下角「+ 新建」创建第一个网络", None);
            }
            return;
        }

        let name = self.active_name();
        let running = self.active_running();
        let (status, dot) = self.status_text();
        let (head, _) = ui.allocate_exact_size(Vec2::new(w, 74.0), Sense::hover());
        txt(&p, Pos2::new(head.left(), head.top() + 2.0), Align2::LEFT_TOP, &name, 26.0, TEXT);
        p.circle_filled(Pos2::new(head.left() + 6.0, head.top() + 48.0), 4.0, dot);
        let node_version = self.node.as_ref().map(|n| n.version.clone()).unwrap_or_default();
        let line = if running {
            let extra = if node_version.is_empty() {
                String::new()
            } else {
                format!(" · 内核 {node_version}")
            };
            format!("{status} · {} 台设备在线{extra}", self.peers.len())
        } else {
            status.to_owned()
        };
        txt(&p, Pos2::new(head.left() + 15.0, head.top() + 48.0), Align2::LEFT_CENTER, line, 12.0, MUTED);

        let ip_clean = self.display_ip();
        let dhcp_label = if self.config.dhcp {
            "DHCP 已启用".to_owned()
        } else {
            "固定地址".to_owned()
        };
        let ip_chip = if running && !ip_clean.is_empty() {
            format!("虚拟 IPv4 {ip_clean}")
        } else {
            "虚拟 IPv4 —".to_owned()
        };
        let chips: [(&str, Color32); 3] = [
            (&dhcp_label, if self.config.dhcp { OK } else { WARN }),
            (&ip_chip, TEXT),
            ("TUN 模式", RX),
        ];
        let mut x = head.right() - 2.0;
        for (label, fg) in chips.iter().rev() {
            let cw = text_w(label, 11.5) + 16.0;
            x -= cw;
            let bg = if *fg == TEXT { CARD } else { fg.gamma_multiply(0.14) };
            if *fg == MUTED {
                pill(&p, Pos2::new(x, head.top() + 37.0), 22.0, label, MUTED, CARD);
            } else {
                pill(&p, Pos2::new(x, head.top() + 37.0), 22.0, label, *fg, bg);
            }
            x -= 8.0;
        }

        ui.add_space(6.0);
        let gap = 12.0;
        let cw = (w - gap * 3.0) / 4.0;
        let (mrow, _) = ui.allocate_exact_size(Vec2::new(w, 88.0), Sense::hover());
        let total = service::format_bytes(self.session_rx + self.session_tx);
        let cards: [(&str, String, String); 4] = [
            (
                "本机虚拟地址",
                if ip_clean.is_empty() { "—".to_owned() } else { ip_clean.clone() },
                "点击复制".to_owned(),
            ),
            (
                "组网成员",
                if running { self.peers.len().to_string() } else { "—".to_owned() },
                "在线节点".to_owned(),
            ),
            (
                "路由条目",
                if running { self.routes.len().to_string() } else { "—".to_owned() },
                "已知网段".to_owned(),
            ),
            ("本次累计", total, "会话累计".to_owned()),
        ];
        let ctx = ui.ctx().clone();
        for (i, (title, value, sub)) in cards.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(mrow.left() + i as f32 * (cw + gap), mrow.top()), Vec2::new(cw, 88.0));
            card(&p, r);
            txt(&p, Pos2::new(r.left() + 14.0, r.top() + 14.0), Align2::LEFT_TOP, *title, 11.0, MUTED);
            txt(&p, Pos2::new(r.left() + 14.0, r.top() + 40.0), Align2::LEFT_TOP, value, 19.0, TEXT);
            txt(&p, Pos2::new(r.left() + 14.0, r.bottom() - 14.0), Align2::LEFT_BOTTOM, sub, 10.5, DIM);
            if i == 0 {
                let resp = ui.interact(r, egui::Id::new("copy-ip"), Sense::click());
                if resp.hovered() && !ip_clean.is_empty() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    self.copy_ip(&ctx);
                }
            }
        }

        ui.add_space(20.0);
        let (shead, _) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::hover());
        txt(&p, Pos2::new(shead.left(), shead.center().y), Align2::LEFT_CENTER, "组网成员", 14.0, TEXT);
        let n_label = format!("{}", self.visible_peers().len());
        let nx = shead.left() + text_w("组网成员", 14.0) + 8.0;
        let nw = text_w(&n_label, 10.5) + 12.0;
        p.rect_filled(Rect::from_min_size(Pos2::new(nx, shead.center().y - 9.0), Vec2::new(nw, 18.0)), CornerRadius::same(9), CARD_HI);
        txt(&p, Pos2::new(nx + nw / 2.0, shead.center().y), Align2::CENTER_CENTER, &n_label, 10.5, MUTED);

        let box_w = 200.0;
        let box_r = Rect::from_min_size(Pos2::new(shead.right() - box_w, shead.center().y - 14.0), Vec2::new(box_w, 28.0));
        ui.put(box_r, egui::TextEdit::singleline(&mut self.peer_query).hint_text("搜索成员 / IP"));
        self.public_peer_toggle(ui, box_r.left() - 10.0, shead.center().y);

        ui.add_space(8.0);
        let (th, _) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::hover());
        let cols: [(&str, f32); 7] = [
            ("成员", 0.00),
            ("虚拟 IPv4", 0.28),
            ("连接", 0.44),
            ("延迟", 0.56),
            ("上传", 0.66),
            ("下载", 0.76),
            ("丢包", 0.88),
        ];
        for (label, fx) in cols.iter() {
            let dx = if *fx == 0.0 { 14.0 } else { 0.0 };
            txt(&p, Pos2::new(th.left() + th.width() * fx + dx, th.center().y), Align2::LEFT_CENTER, *label, 10.5, DIM);
        }
        p.hline(th.x_range(), th.bottom() - 0.5, Stroke::new(1.0, BORDER));

        if !running {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, "网络未运行，点击右上角「启动网络」后显示成员", 11.5, DIM);
            return;
        }

        if let Some(err) = &self.status_error {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, format!("内核数据暂不可用：{err}"), 11.5, WARN);
        }

        let idxs = self.visible_peers();
        let peers = self.peers.clone();
        for i in idxs {
            let peer = &peers[i];
            let (row, resp) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::click());
            let hovered = resp.hovered();
            let selected = self.selected_peer == Some(i);
            if hovered || selected {
                p.rect_filled(row, CornerRadius::same(8), CARD);
            }
            if selected {
                p.rect_stroke(row, CornerRadius::same(8), Stroke::new(1.0, ACCENT.gamma_multiply(0.55)), StrokeKind::Inside);
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            let name = if peer.hostname.is_empty() {
                if peer.ip.is_empty() {
                    peer.id.clone()
                } else {
                    peer.ip.clone()
                }
            } else {
                peer.hostname.clone()
            };
            let (route_label, route_color) = route_badge(&peer.cost);
            let accent = if peer.cost == "local" || peer.cost == "0" { ACCENT } else { route_color };
            let av = Pos2::new(row.left() + 26.0, row.center().y);
            p.circle_filled(av, 13.0, CARD_HI);
            let initial = name.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".to_owned());
            txt(&p, av, Align2::CENTER_CENTER, &initial, 12.0, accent);
            txt(&p, Pos2::new(row.left() + 46.0, row.top() + 11.0), Align2::LEFT_TOP, truncate(&name, 22), 12.5, TEXT);
            let id_text = if peer.id.is_empty() {
                peer.version.clone()
            } else {
                format!("et-{}", truncate(&peer.id, 8))
            };
            mono(&p, Pos2::new(row.left() + 46.0, row.bottom() - 10.0), Align2::LEFT_BOTTOM, id_text, 10.0, DIM);
            mono(&p, Pos2::new(row.left() + row.width() * 0.28, row.center().y), Align2::LEFT_CENTER, &peer.ip, 11.5, MUTED);
            pill(&p, Pos2::new(row.left() + row.width() * 0.44, row.center().y - 10.0), 20.0, route_label, route_color, route_color.gamma_multiply(0.16));
            let latency = if peer.cost == "local" || peer.cost == "0" {
                "0 ms".to_owned()
            } else if peer.latency > 0.0 {
                format!("{:.0} ms", peer.latency)
            } else {
                "—".to_owned()
            };
            txt(&p, Pos2::new(row.left() + row.width() * 0.56, row.center().y), Align2::LEFT_CENTER, latency, 11.5, lat_color(peer.latency as u32));
            txt(&p, Pos2::new(row.left() + row.width() * 0.66, row.center().y), Align2::LEFT_CENTER, format!("↑ {}", service::format_bytes(peer.tx)), 11.0, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.76, row.center().y), Align2::LEFT_CENTER, format!("↓ {}", service::format_bytes(peer.rx)), 11.0, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.88, row.center().y), Align2::LEFT_CENTER, format!("{:.1}%", peer.loss), 11.0, if peer.loss > 1.0 { WARN } else { MUTED });

            p.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, Color32::from_rgb(0x26, 0x2A, 0x30)));
            if resp.clicked() {
                self.selected_peer = Some(i);
            }
        }

        if running && self.status_error.is_none() && self.visible_peers().is_empty() {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            let message = if self.peers.is_empty() {
                "等待内核返回成员数据…"
            } else if !self.show_public_peers {
                "仅剩公共节点（已隐藏），点击右侧「显示公共节点」查看"
            } else {
                "当前筛选条件下没有成员"
            };
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, message, 11.5, DIM);
        }
    }

    fn visible_peers(&self) -> Vec<usize> {
        let q = self.peer_query.trim().to_lowercase();
        self.peers
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                (self.show_public_peers || !is_public_peer(p))
                    && (q.is_empty()
                        || p.hostname.to_lowercase().contains(&q)
                        || p.ip.contains(&q)
                        || p.id.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn public_peer_toggle(&mut self, ui: &mut egui::Ui, right_x: f32, center_y: f32) {
        let n = self.peers.iter().filter(|p| is_public_peer(p)).count();
        if n == 0 {
            return;
        }
        let label = if self.show_public_peers {
            format!("隐藏公共节点 ({n})")
        } else {
            format!("显示公共节点 ({n})")
        };
        let tw = text_w(&label, 10.5) + 14.0;
        let r = Rect::from_min_size(Pos2::new(right_x - tw, center_y - 12.0), Vec2::new(tw, 24.0));
        let resp = ui.put(
            r,
            egui::Button::new(RichText::new(label).size(10.5).color(MUTED)).frame(false),
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.show_public_peers = !self.show_public_peers;
        }
    }

    fn peers_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();
        let running = self.active_running();

        let (head, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
        txt(&p, Pos2::new(head.left() + 2.0, head.center().y), Align2::LEFT_CENTER, "组网成员", 15.0, TEXT);
        let count_label = self.peers.len().to_string();
        let nx = head.left() + 2.0 + text_w("组网成员", 15.0) + 8.0;
        let nw = text_w(&count_label, 10.5) + 12.0;
        p.rect_filled(Rect::from_min_size(Pos2::new(nx, head.center().y - 9.0), Vec2::new(nw, 18.0)), CornerRadius::same(9), CARD_HI);
        txt(&p, Pos2::new(nx + nw / 2.0, head.center().y), Align2::CENTER_CENTER, &count_label, 10.5, MUTED);

        let box_w = 220.0;
        let box_r = Rect::from_min_size(Pos2::new(head.right() - box_w, head.center().y - 14.0), Vec2::new(box_w, 28.0));
        ui.put(box_r, egui::TextEdit::singleline(&mut self.peer_query).hint_text("搜索成员 / IP / 节点 ID"));
        self.public_peer_toggle(ui, box_r.left() - 10.0, head.center().y);

        ui.add_space(10.0);
        if !running {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, "网络未运行，启动后显示成员与链路详情", 11.5, DIM);
            return;
        }
        if let Some(err) = &self.status_error {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, format!("内核数据暂不可用：{err}"), 11.5, WARN);
        }

        let (th, _) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::hover());
        let cols: [(&str, f32); 9] = [
            ("成员", 0.00),
            ("虚拟 IPv4", 0.19),
            ("连接", 0.32),
            ("延迟", 0.40),
            ("丢包", 0.48),
            ("上传", 0.56),
            ("下载", 0.65),
            ("隧道", 0.74),
            ("版本", 0.85),
        ];
        for (label, fx) in cols.iter() {
            let dx = if *fx == 0.0 { 14.0 } else { 0.0 };
            txt(&p, Pos2::new(th.left() + th.width() * fx + dx, th.center().y), Align2::LEFT_CENTER, *label, 10.5, DIM);
        }
        p.hline(th.x_range(), th.bottom() - 0.5, Stroke::new(1.0, BORDER));

        let idxs = self.visible_peers();
        let shown = idxs.len();
        let peers = self.peers.clone();
        for i in idxs {
            let peer = &peers[i];
            let (row, resp) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::click());
            if resp.hovered() {
                p.rect_filled(row, CornerRadius::same(8), CARD);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let name = if peer.hostname.is_empty() {
                if peer.ip.is_empty() { peer.id.clone() } else { peer.ip.clone() }
            } else {
                peer.hostname.clone()
            };
            let (route_label, route_color) = route_badge(&peer.cost);
            let accent = if peer.cost == "local" || peer.cost == "0" { ACCENT } else { route_color };
            let av = Pos2::new(row.left() + 26.0, row.center().y);
            p.circle_filled(av, 13.0, CARD_HI);
            let initial = name.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".to_owned());
            txt(&p, av, Align2::CENTER_CENTER, &initial, 12.0, accent);
            txt(&p, Pos2::new(row.left() + 46.0, row.top() + 11.0), Align2::LEFT_TOP, truncate(&name, 16), 12.5, TEXT);
            let id_text = if peer.id.is_empty() { peer.version.clone() } else { format!("et-{}", truncate(&peer.id, 8)) };
            mono(&p, Pos2::new(row.left() + 46.0, row.bottom() - 10.0), Align2::LEFT_BOTTOM, id_text, 10.0, DIM);
            mono(&p, Pos2::new(row.left() + row.width() * 0.19, row.center().y), Align2::LEFT_CENTER, &peer.ip, 11.5, MUTED);
            pill(&p, Pos2::new(row.left() + row.width() * 0.32, row.center().y - 10.0), 20.0, route_label, route_color, route_color.gamma_multiply(0.16));
            let latency = if peer.cost == "local" || peer.cost == "0" {
                "0 ms".to_owned()
            } else if peer.latency > 0.0 {
                format!("{:.0} ms", peer.latency)
            } else {
                "—".to_owned()
            };
            txt(&p, Pos2::new(row.left() + row.width() * 0.40, row.center().y), Align2::LEFT_CENTER, latency, 11.5, lat_color(peer.latency as u32));
            txt(&p, Pos2::new(row.left() + row.width() * 0.48, row.center().y), Align2::LEFT_CENTER, format!("{:.1}%", peer.loss), 11.0, if peer.loss > 1.0 { WARN } else { MUTED });
            txt(&p, Pos2::new(row.left() + row.width() * 0.56, row.center().y), Align2::LEFT_CENTER, format!("↑ {}", service::format_bytes(peer.tx)), 11.0, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.65, row.center().y), Align2::LEFT_CENTER, format!("↓ {}", service::format_bytes(peer.rx)), 11.0, MUTED);
            let tunnel = if peer.tunnel.is_empty() { "—".to_owned() } else { peer.tunnel.clone() };
            txt(&p, Pos2::new(row.left() + row.width() * 0.74, row.center().y), Align2::LEFT_CENTER, truncate(&tunnel, 9), 10.5, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.85, row.center().y), Align2::LEFT_CENTER, truncate(&peer.version, 9), 10.5, DIM);
            p.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, Color32::from_rgb(0x26, 0x2A, 0x30)));
        }

        if self.status_error.is_none() && (self.peers.is_empty() || shown == 0) {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            let message = if self.peers.is_empty() {
                "等待内核返回成员数据…"
            } else if !self.show_public_peers {
                "仅剩公共节点（已隐藏），点击右侧「显示公共节点」查看"
            } else {
                "当前筛选条件下没有成员"
            };
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, message, 11.5, DIM);
        }
    }

    fn routes_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();
        let running = self.active_running();

        let (head, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
        txt(&p, Pos2::new(head.left() + 2.0, head.center().y), Align2::LEFT_CENTER, "路由表", 15.0, TEXT);
        let box_w = 240.0;
        let box_r = Rect::from_min_size(Pos2::new(head.right() - box_w, head.center().y - 14.0), Vec2::new(box_w, 28.0));
        ui.put(box_r, egui::TextEdit::singleline(&mut self.route_query).hint_text("搜索网段 / 主机 / 下一跳"));

        ui.add_space(10.0);
        if !running {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, "网络未运行，启动后显示内核路由表", 11.5, DIM);
            return;
        }

        let (th, _) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::hover());
        let cols: [(&str, f32); 6] = [
            ("目标", 0.00),
            ("主机名", 0.18),
            ("下一跳", 0.36),
            ("跳数", 0.56),
            ("路径延迟", 0.63),
            ("代理网段", 0.75),
        ];
        for (label, fx) in cols.iter() {
            let dx = if *fx == 0.0 { 14.0 } else { 0.0 };
            txt(&p, Pos2::new(th.left() + th.width() * fx + dx, th.center().y), Align2::LEFT_CENTER, *label, 10.5, DIM);
        }
        p.hline(th.x_range(), th.bottom() - 0.5, Stroke::new(1.0, BORDER));

        let q = self.route_query.trim().to_lowercase();
        let routes = self.routes.clone();
        let mut shown = 0;
        for route in routes.iter() {
            let proxy = route.proxy_cidrs.join(", ");
            if !q.is_empty()
                && !route.ipv4.contains(&q)
                && !route.hostname.to_lowercase().contains(&q)
                && !route.next_hop.to_lowercase().contains(&q)
                && !proxy.to_lowercase().contains(&q)
            {
                continue;
            }
            shown += 1;
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
            let target = if route.ipv4.is_empty() { "—".to_owned() } else { route.ipv4.clone() };
            mono(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, &target, 11.5, TEXT);
            txt(&p, Pos2::new(row.left() + row.width() * 0.18, row.center().y), Align2::LEFT_CENTER, truncate(&route.hostname, 14), 11.5, MUTED);
            let hop = if route.next_hop.is_empty() { "本机" } else { route.next_hop.as_str() };
            txt(&p, Pos2::new(row.left() + row.width() * 0.36, row.center().y), Align2::LEFT_CENTER, truncate(hop, 16), 11.0, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.56, row.center().y), Align2::LEFT_CENTER, format!("{:.0}", route.hops), 11.0, MUTED);
            let latency = if route.latency > 0.0 { format!("{:.1} ms", route.latency) } else { "—".to_owned() };
            txt(&p, Pos2::new(row.left() + row.width() * 0.63, row.center().y), Align2::LEFT_CENTER, latency, 11.0, lat_color(route.latency.max(0.0) as u32));
            let proxy = if proxy.is_empty() { "—".to_owned() } else { proxy };
            txt(&p, Pos2::new(row.left() + row.width() * 0.75, row.center().y), Align2::LEFT_CENTER, truncate(&proxy, 20), 10.5, DIM);
            p.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, Color32::from_rgb(0x26, 0x2A, 0x30)));
        }

        if shown == 0 {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            let message = if self.routes.is_empty() { "等待内核心路由数据…" } else { "当前筛选条件下没有路由" };
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, message, 11.5, DIM);
        }
    }

    fn proxy_label(&self) -> String {
        let value = self.kernel_proxy.as_str();
        if value == "direct" || value.is_empty() {
            "直连 GitHub".to_owned()
        } else if value == "mirror" {
            if self.custom_proxy.trim().is_empty() {
                "自定义镜像前缀（未填写）".to_owned()
            } else {
                format!("镜像：{}", truncate(&self.custom_proxy, 36))
            }
        } else if let Some(addr) = value.strip_prefix("local:") {
            match self.proxies.iter().find(|p| p.addr == addr) {
                Some(proxy) => format!("{} · {}", proxy.label, proxy.addr),
                None => format!("本地代理 {addr}"),
            }
        } else {
            value.to_owned()
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("后台服务").size(14.0).color(TEXT));
                ui.add_space(8.0);
                let (color, text) = if self.service_running { (OK, "运行中") } else { (ERR, "不可用") };
                ui.label(RichText::new(format!("● {text}")).size(10.5).color(color));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("重试连接").clicked() {
                    let _ = self.actions.send(service::Action::Refresh);
                    self.toast(&ctx, "正在重新连接后台服务…", true);
                }
                ui.label(RichText::new(format!("{} 个网络实例", self.instances.len())).size(10.5).color(MUTED));
                if let Some(err) = self.service_error.clone() {
                    ui.label(RichText::new(truncate(&err, 48)).size(10.5).color(ERR));
                }
            });
        });

        ui.add_space(8.0);
        let current_kernel = self.node.as_ref().map(|n| n.version.clone()).unwrap_or_default();
        let kernel_running = self.active_running();
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("内核版本管理").size(14.0).color(TEXT));
                ui.add_space(8.0);
                let current = if current_kernel.is_empty() {
                    if kernel_running {
                        "当前内核：读取中…".to_owned()
                    } else {
                        "当前内核：未知（启动网络后读取）".to_owned()
                    }
                } else {
                    format!("当前内核：{current_kernel}")
                };
                ui.label(RichText::new(current).size(10.5).color(MUTED));
            });
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("下载方式").size(11.5).color(MUTED));
                egui::ComboBox::from_id_salt("kernel-proxy")
                    .selected_text(self.proxy_label())
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.kernel_proxy, "direct".to_owned(), "直连 GitHub");
                        let proxies = self.proxies.clone();
                        for proxy in proxies {
                            let value = format!("local:{}", proxy.addr);
                            let label = format!("{} · {}", proxy.label, proxy.addr);
                            ui.selectable_value(&mut self.kernel_proxy, value, label);
                        }
                        ui.selectable_value(&mut self.kernel_proxy, "mirror".to_owned(), "自定义镜像前缀");
                    });
                if ui.button("检测本地代理").clicked() {
                    let _ = self.actions.send(service::Action::DetectProxies);
                }
                if self.proxies_scanned && self.proxies.is_empty() {
                    ui.label(RichText::new("未发现可用代理").size(10.0).color(DIM));
                }
            });
            if self.kernel_proxy == "mirror" {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("镜像前缀").size(11.5).color(MUTED));
                    let width = (ui.available_width() - 4.0).max(160.0);
                    ui.add_sized(
                        [width, 26.0],
                        egui::TextEdit::singleline(&mut self.custom_proxy)
                            .hint_text("例如 https://ghfast.top"),
                    );
                });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("目标版本").size(11.5).color(MUTED));
                let selected = if self.selected_version.is_empty() {
                    "最新版本（自动选择）".to_owned()
                } else {
                    self.selected_version.clone()
                };
                egui::ComboBox::from_id_salt("kernel-version")
                    .selected_text(selected)
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.selected_version, String::new(), "最新版本（自动选择）");
                        let versions = self.releases.as_ref().map(|r| r.versions.clone()).unwrap_or_default();
                        for version in versions {
                            ui.selectable_value(&mut self.selected_version, version.clone(), version);
                        }
                    });
                if ui
                    .add_enabled(!self.releases_loading, egui::Button::new(if self.releases_loading { "获取中…" } else { "获取版本列表" }))
                    .clicked()
                {
                    let proxy = self.effective_proxy();
                    self.releases_loading = true;
                    self.releases_error = None;
                    let _ = self.actions.send(service::Action::CheckReleases { proxy });
                }
            });
            if let Some(info) = &self.releases {
                if let Some(latest) = &info.latest {
                    ui.label(
                        RichText::new(format!("GitHub 最新版本：{latest} · 共 {} 个可用版本", info.versions.len()))
                            .size(10.5)
                            .color(OK),
                    );
                }
            }
            if let Some(err) = self.releases_error.clone() {
                ui.label(RichText::new(truncate(&err, 96)).size(10.5).color(ERR));
            }

            ui.add_space(8.0);
            let busy = self.kernel.as_ref().map(|k| !k.terminal()).unwrap_or(false);
            ui.horizontal(|ui| {
                let can_update = self.service_running && !busy;
                if ui
                    .add_enabled(
                        can_update,
                        egui::Button::new(RichText::new("下载并安装").color(Color32::WHITE)).fill(ACCENT),
                    )
                    .clicked()
                {
                    self.start_kernel_update(&ctx);
                }
                if busy && ui.button("取消更新").clicked() {
                    let _ = self.actions.send(service::Action::KernelCancel);
                }
                if let Some(kernel) = &self.kernel {
                    ui.label(RichText::new(format!("{} · {:.0}%", kernel.phase_label(), kernel.percent)).size(10.5).color(MUTED));
                }
            });

            if let Some(kernel) = self.kernel.clone() {
                ui.add_space(4.0);
                let fraction = (kernel.percent as f32 / 100.0).clamp(0.0, 1.0);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(ui.available_width().min(540.0))
                        .text(format!("{:.0}%", kernel.percent)),
                );
                let detail = if kernel.total > 0 {
                    format!(
                        "{} / {}",
                        service::format_bytes(kernel.downloaded as f64),
                        service::format_bytes(kernel.total as f64)
                    )
                } else {
                    service::format_bytes(kernel.downloaded as f64)
                };
                let color = if kernel.status == "failed" {
                    ERR
                } else if kernel.status == "completed" {
                    OK
                } else {
                    MUTED
                };
                ui.label(
                    RichText::new(format!("{} · {} · {}", kernel.phase_label(), detail, truncate(&kernel.message, 56)))
                        .size(10.5)
                        .color(color),
                );
            }
        });

        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            ui.label(RichText::new("开机自启").size(14.0).color(TEXT));
            ui.add_space(4.0);
            let current_enabled = self.autostart.as_ref().map(|a| a.enabled).unwrap_or(false);
            let mut enabled = current_enabled;
            let mut minimized = self.autostart_min;
            ui.horizontal(|ui| {
                if switch(ui, enabled) {
                    enabled = !enabled;
                }
                ui.label(RichText::new("登录时自动启动").size(11.5).color(MUTED));
            });
            ui.horizontal(|ui| {
                if switch(ui, minimized) {
                    minimized = !minimized;
                }
                ui.label(RichText::new("启动后最小化到托盘").size(11.5).color(MUTED));
            });
            if enabled != current_enabled || (enabled && minimized != self.autostart_min) {
                self.autostart_min = minimized;
                if let Some(state) = self.autostart.as_mut() {
                    state.enabled = enabled;
                    state.minimized = minimized;
                }
                let _ = self.actions.send(service::Action::AutostartSet { enabled, minimized });
            }

            let registration = self
                .autostart
                .as_ref()
                .filter(|a| a.enabled)
                .map(|a| (a.path.clone(), a.minimized));
            if let Some((path, registered_min)) = registration {
                let current = std::env::current_exe().ok();
                let mismatch = match &current {
                    Some(exe) => {
                        !path.is_empty()
                            && exe.to_string_lossy().to_lowercase() != path.to_lowercase()
                    }
                    None => false,
                };
                ui.add_space(2.0);
                let text = if path.is_empty() {
                    "注册表值缺失".to_owned()
                } else {
                    format!("注册路径：{path}")
                };
                ui.label(RichText::new(truncate(&text, 96)).size(10.5).color(if mismatch { WARN } else { MUTED }));
                if mismatch {
                    ui.label(RichText::new("注册路径与当前程序位置不一致，自启动将无法生效。").size(10.5).color(WARN));
                    if ui.button("修复自启路径").clicked() {
                        let _ = self.actions.send(service::Action::AutostartSet { enabled: true, minimized: registered_min });
                        self.toast(&ctx, "正在重新注册自启动…", true);
                    }
                }
            }
        });

        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            ui.label(RichText::new(format!("EasyTier 原生客户端 {VERSION}")).size(12.0).color(MUTED));
        });
    }

    fn traffic_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();
        let running = self.active_running();

        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 340.0), Sense::hover());
        card(&p, r);
        txt(&p, Pos2::new(r.left() + 18.0, r.top() + 16.0), Align2::LEFT_TOP, "实时网络流量", 14.0, TEXT);

        let (last_rx, last_tx) = *self.history.back().unwrap_or(&(0.0, 0.0));
        let peak = self.history.iter().map(|(a, b)| a.max(*b)).fold(0.0f32, f32::max);

        let legend: [(String, Color32); 3] = [
            (format!("↓ {}", service::format_speed(last_rx)), RX),
            (format!("↑ {}", service::format_speed(last_tx)), TX),
            (format!("★ 峰值 {}", service::format_speed(peak)), WARN),
        ];
        let mut x = r.right() - 18.0;
        for (label, col) in legend.iter().rev() {
            let lw = text_w(label, 10.5) + 22.0;
            x -= lw;
            p.rect_filled(Rect::from_min_size(Pos2::new(x, r.top() + 16.0), Vec2::new(lw, 20.0)), CornerRadius::same(10), col.gamma_multiply(0.14));
            p.circle_filled(Pos2::new(x + 10.0, r.top() + 26.0), 3.0, *col);
            txt(&p, Pos2::new(x + 18.0, r.top() + 26.0), Align2::LEFT_CENTER, label, 10.5, *col);
            x -= 6.0;
        }

        let plot = Rect::from_min_max(Pos2::new(r.left() + 78.0, r.top() + 58.0), Pos2::new(r.right() - 18.0, r.bottom() - 40.0));
        let max = nice_speed_max(peak);
        for i in 0..=4 {
            let frac = i as f32 / 4.0;
            let y = plot.bottom() - plot.height() * frac;
            p.hline(plot.x_range(), y, Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2F, 0x37)));
            txt(&p, Pos2::new(plot.left() - 8.0, y), Align2::RIGHT_CENTER, service::format_speed(max * frac), 9.5, MUTED);
        }

        if !running {
            txt(&p, Pos2::new(plot.center().x, plot.center().y), Align2::CENTER_CENTER, "网络未运行", 12.0, DIM);
        } else {
            let n = self.history.len();
            if n >= 2 {
                let step = plot.width() / (n - 1) as f32;
                for (i, (rx, _)) in self.history.iter().enumerate() {
                    let x = plot.left() + step * i as f32;
                    let y = plot.bottom() - (rx / max).clamp(0.0, 1.0) * plot.height();
                    p.vline(x, y..=plot.bottom(), Stroke::new(step * 1.4, RX.gamma_multiply(0.10)));
                }
                for (i, (_, tx)) in self.history.iter().enumerate() {
                    let x = plot.left() + step * i as f32;
                    let y = plot.bottom() - (tx / max).clamp(0.0, 1.0) * plot.height();
                    p.vline(x, y..=plot.bottom(), Stroke::new(step * 1.4, TX.gamma_multiply(0.12)));
                }

                let mut rx_pts = Vec::with_capacity(n);
                let mut tx_pts = Vec::with_capacity(n);
                for (i, (rx, tx)) in self.history.iter().enumerate() {
                    let x = plot.left() + step * i as f32;
                    rx_pts.push(Pos2::new(x, plot.bottom() - (rx / max).clamp(0.0, 1.0) * plot.height()));
                    tx_pts.push(Pos2::new(x, plot.bottom() - (tx / max).clamp(0.0, 1.0) * plot.height()));
                }
                for seg in rx_pts.windows(2) {
                    p.line_segment([seg[0], seg[1]], Stroke::new(1.8, RX));
                }
                for seg in tx_pts.windows(2) {
                    p.line_segment([seg[0], seg[1]], Stroke::new(1.4, TX.gamma_multiply(0.9)));
                }
                if let Some(last) = rx_pts.last() {
                    p.circle_filled(*last, 3.5, RX);
                }
                if let Some(last) = tx_pts.last() {
                    p.circle_filled(*last, 3.0, TX);
                }
            }
        }

        txt(&p, Pos2::new(plot.left(), plot.bottom() + 10.0), Align2::LEFT_TOP, "60 秒前", 10.0, DIM);
        txt(&p, Pos2::new(plot.right(), plot.bottom() + 10.0), Align2::RIGHT_TOP, "现在", 10.0, DIM);

        ui.add_space(12.0);
        let gap = 12.0;
        let cw = (w - gap * 2.0) / 3.0;
        let (srow, _) = ui.allocate_exact_size(Vec2::new(w, 80.0), Sense::hover());
        let stats: [(&str, String, Color32); 3] = [
            ("当前下行", service::format_speed(last_rx), RX),
            ("当前上行", service::format_speed(last_tx), TX),
            ("近 60 秒峰值", service::format_speed(peak), WARN),
        ];
        for (i, (title, value, col)) in stats.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(srow.left() + i as f32 * (cw + gap), srow.top()), Vec2::new(cw, 80.0));
            card(&p, r);
            txt(&p, Pos2::new(r.left() + 14.0, r.top() + 14.0), Align2::LEFT_TOP, *title, 11.0, MUTED);
            txt(&p, Pos2::new(r.left() + 14.0, r.top() + 38.0), Align2::LEFT_TOP, value, 18.0, *col);
        }
    }

    fn config_tab(&mut self, ui: &mut egui::Ui) {
        let Some(inst) = self.active_instance().cloned() else {
            let p = ui.painter().clone();
            self.empty_state(ui, &p, "未选择网络", "选择左侧网络后编辑配置", None);
            return;
        };
        if inst.config_toml.is_empty() {
            let p = ui.painter().clone();
            self.empty_state(
                ui,
                &p,
                "配置不可用",
                "服务端没有返回该实例的配置内容",
                None,
            );
            return;
        }

        let ctx = ui.ctx().clone();
        let mut cfg = self.config.clone();
        let mut instance_name = self.instance_name.clone();
        let mut rpc_text = self.rpc_port_text.clone();
        let mut auto_start = self.auto_start;
        let mut remote = self.remote_manage;
        let mut whitelist = self.whitelist_text.clone();
        let mut level = self.console_level.clone();
        let mut show_advanced = self.show_advanced;
        let mut secret_visible = self.secret_visible;
        let mut changed = false;
        let mut open_picker = false;

        if let Some(err) = self.config_error.clone() {
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("配置解析失败").size(13.0).color(ERR));
                ui.label(RichText::new(err).size(11.0).color(MUTED));
                ui.label(
                    RichText::new("可点击「重置」重新读取服务端保存的配置。")
                        .size(10.5)
                        .color(DIM),
                );
            });
            ui.add_space(8.0);
        }

        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("实例配置").size(14.0).color(TEXT));
                ui.add_space(8.0);
                if self.config_dirty {
                    ui.label(RichText::new("有未保存的修改").size(10.5).color(WARN));
                }
            });
            ui.add_space(10.0);

            changed |= text_row(ui, "实例名称", &mut instance_name, "用于区分多个网络实例");
            changed |= text_row(ui, "网络名称", &mut cfg.network_name, "同一网络名称 + 密钥才能互联");

            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("网络密钥").size(11.5).color(MUTED)),
                );
                let w = (ui.available_width() - 60.0).max(120.0);
                let resp = ui.add_sized(
                    [w, 26.0],
                    egui::TextEdit::singleline(&mut cfg.network_secret)
                        .password(!secret_visible)
                        .hint_text("用于验证节点归属"),
                );
                changed |= resp.changed();
                if ui
                    .small_button(if secret_visible { "隐藏" } else { "显示" })
                    .clicked()
                {
                    secret_visible = !secret_visible;
                }
            });
            changed |= text_row(ui, "主机名", &mut cfg.hostname, "本机在虚拟网中的名称");

            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("组网方式").size(11.5).color(MUTED)),
                );
                if ui
                    .selectable_label(cfg.networking_method == 2, "独立网络")
                    .clicked()
                {
                    cfg.networking_method = 2;
                    changed = true;
                }
                if ui
                    .selectable_label(cfg.networking_method != 2, "指定节点")
                    .clicked()
                {
                    cfg.networking_method = 1;
                    changed = true;
                }
            });
            if cfg.networking_method != 2 {
                let toggle = if self.peer_urls_open { "收起" } else { "展开" };
                ui.horizontal(|ui| {
                    let header = ui.add(
                        egui::Button::new(
                            RichText::new(format!("公共服务器（{}） · {toggle}", cfg.peer_urls.len()))
                                .size(11.5)
                                .color(MUTED),
                        )
                        .frame(false),
                    );
                    if header.clicked() {
                        self.peer_urls_open = !self.peer_urls_open;
                    }
                    ui.add_space(4.0);
                    if ui.small_button("浏览公共节点").clicked() {
                        open_picker = true;
                    }
                });
                if self.peer_urls_open {
                    changed |= string_list(ui, &mut cfg.peer_urls, "tcp://192.168.1.10:11010");
                }
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("虚拟 IPv4").size(11.5).color(MUTED)),
                );
                let w = (ui.available_width() - 150.0).max(110.0);
                let resp = ui.add_sized(
                    [w, 26.0],
                    egui::TextEdit::singleline(&mut cfg.virtual_ipv4)
                        .interactive(!cfg.dhcp)
                        .hint_text(if cfg.dhcp { "由 DHCP 自动分配" } else { "10.126.126.1" }),
                );
                changed |= resp.changed();
                ui.label(RichText::new("/").size(12.0).color(MUTED));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut cfg.network_length)
                            .range(1..=32)
                            .speed(1.0),
                    )
                    .changed();
                if switch(ui, cfg.dhcp) {
                    cfg.dhcp = !cfg.dhcp;
                    changed = true;
                }
                ui.label(RichText::new("DHCP").size(11.5).color(MUTED));
            });

            ui.add_space(10.0);
            ui.label(RichText::new("实例与运行").size(12.0).color(TEXT));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("RPC 端口").size(11.5).color(MUTED)),
                );
                let resp = ui.add_sized(
                    [90.0, 26.0],
                    egui::TextEdit::singleline(&mut rpc_text).hint_text("15888"),
                );
                changed |= resp.changed();
            });
            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("开机自启").size(11.5).color(MUTED)),
                );
                if switch(ui, auto_start) {
                    auto_start = !auto_start;
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("远程管理").size(11.5).color(MUTED)),
                );
                if switch(ui, remote) {
                    remote = !remote;
                    changed = true;
                }
            });
            if remote {
                changed |= text_row(
                    ui,
                    "RPC 白名单",
                    &mut whitelist,
                    "EasyTier 虚拟网段，如 10.126.126.0/24（逗号分隔）",
                );
            }
            ui.horizontal(|ui| {
                ui.add_sized(
                    [96.0, 26.0],
                    egui::Label::new(RichText::new("内核日志").size(11.5).color(MUTED)),
                );
                let levels = [
                    ("默认", ""),
                    ("off", "off"),
                    ("error", "error"),
                    ("warn", "warn"),
                    ("info", "info"),
                    ("debug", "debug"),
                    ("trace", "trace"),
                ];
                for (label, value) in levels.iter() {
                    if ui.selectable_label(level == *value, *label).clicked() {
                        level = (*value).to_owned();
                        changed = true;
                    }
                }
            });
        });

        ui.add_space(8.0);
        if ui
            .button(if show_advanced {
                "收起高级设置"
            } else {
                "展开高级设置"
            })
            .clicked()
        {
            show_advanced = !show_advanced;
        }

        if show_advanced {
            ui.add_space(8.0);
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("监听与转发").size(14.0).color(TEXT));
                ui.add_space(2.0);
                ui.label(RichText::new("监听器").size(11.0).color(MUTED));
                changed |= string_list(ui, &mut cfg.listener_urls, "tcp://0.0.0.0:11010");
                ui.add_space(6.0);
                ui.label(
                    RichText::new("映射监听器（公网地址/NAT 场景，可留空）")
                        .size(11.0)
                        .color(MUTED),
                );
                changed |= string_list(ui, &mut cfg.mapped_listeners, "tcp://1.2.3.4:11223");

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    changed |= opt_toggle_row(ui, "SOCKS5 服务器", &mut cfg.enable_socks5);
                    if cfg.enable_socks5 == Some(true) {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut cfg.socks5_port)
                                    .range(1..=65535)
                                    .speed(1.0),
                            )
                            .changed();
                    }
                });
                ui.horizontal(|ui| {
                    if bool_toggle_row(ui, "VPN 门户（WireGuard）", &mut cfg.enable_vpn_portal) {
                        changed = true;
                    }
                    if cfg.enable_vpn_portal {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut cfg.vpn_portal_listen_port)
                                    .range(1..=65535)
                                    .speed(1.0),
                            )
                            .changed();
                    }
                });
                if cfg.enable_vpn_portal {
                    changed |= text_row(
                        ui,
                        "客户端网段",
                        &mut cfg.vpn_portal_client_network_addr,
                        "10.14.14.0",
                    );
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [96.0, 26.0],
                            egui::Label::new(
                                RichText::new("网段前缀").size(11.5).color(MUTED),
                            ),
                        );
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut cfg.vpn_portal_client_network_len)
                                    .range(1..=32)
                                    .speed(1.0),
                            )
                            .changed();
                    });
                }

                ui.add_space(8.0);
                ui.label(RichText::new("端口转发").size(11.0).color(MUTED));
                changed |= port_forwards_editor(ui, &mut cfg.port_forwards);
            });

            ui.add_space(8.0);
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("路由与子网").size(14.0).color(TEXT));
                ui.label(RichText::new("子网代理").size(11.0).color(MUTED));
                changed |= string_list(ui, &mut cfg.proxy_cidrs, "10.0.0.0/24");
                ui.add_space(6.0);
                if bool_toggle_row(ui, "手动路由", &mut cfg.enable_manual_routes) {
                    changed = true;
                }
                if cfg.enable_manual_routes {
                    changed |= string_list(ui, &mut cfg.routes, "192.168.0.0/16");
                }
                ui.label(RichText::new("出口节点").size(11.0).color(MUTED));
                changed |= string_list(ui, &mut cfg.exit_nodes, "10.144.144.1");
                changed |= opt_toggle_row(ui, "启用出口节点", &mut cfg.enable_exit_node);
                changed |= opt_toggle_row(
                    ui,
                    "中继白名单",
                    &mut cfg.enable_relay_network_whitelist,
                );
                if cfg.enable_relay_network_whitelist == Some(true) {
                    changed |= string_list(ui, &mut cfg.relay_network_whitelist, "net1");
                }
            });

            ui.add_space(8.0);
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("协议与性能").size(14.0).color(TEXT));
                ui.add_space(4.0);
                ui.columns(2, |cols| {
                    changed |= bool_toggle_row(&mut cols[0], "延迟优先", &mut cfg.latency_first);
                    changed |= opt_toggle_row(&mut cols[1], "多线程", &mut cfg.multi_thread);
                    changed |= opt_toggle_row(&mut cols[0], "禁用 P2P", &mut cfg.disable_p2p);
                    changed |= opt_toggle_row(&mut cols[1], "仅 P2P", &mut cfg.p2p_only);
                    changed |= opt_toggle_row(&mut cols[0], "懒建立 P2P", &mut cfg.lazy_p2p);
                    changed |= opt_toggle_row(&mut cols[1], "需要 P2P", &mut cfg.need_p2p);
                    changed |= opt_toggle_row(
                        &mut cols[0],
                        "禁用 TCP 打洞",
                        &mut cfg.disable_tcp_hole_punching,
                    );
                    changed |= opt_toggle_row(
                        &mut cols[1],
                        "禁用 UDP 打洞",
                        &mut cfg.disable_udp_hole_punching,
                    );
                    changed |= opt_toggle_row(
                        &mut cols[0],
                        "禁用对称 NAT 打洞",
                        &mut cfg.disable_sym_hole_punching,
                    );
                    changed |= opt_toggle_row(&mut cols[1], "禁用 UPnP", &mut cfg.disable_upnp);
                    changed |= opt_toggle_row(&mut cols[0], "KCP 代理", &mut cfg.enable_kcp_proxy);
                    changed |= opt_toggle_row(
                        &mut cols[1],
                        "禁用 KCP 入站",
                        &mut cfg.disable_kcp_input,
                    );
                    changed |= opt_toggle_row(&mut cols[0], "QUIC 代理", &mut cfg.enable_quic_proxy);
                    changed |= opt_toggle_row(
                        &mut cols[1],
                        "禁用 QUIC 入站",
                        &mut cfg.disable_quic_input,
                    );
                    changed |= opt_toggle_row(&mut cols[0], "使用 smoltcp", &mut cfg.use_smoltcp);
                    changed |= opt_toggle_row(&mut cols[1], "无 TUN 模式", &mut cfg.no_tun);
                    changed |= opt_toggle_row(&mut cols[0], "绑定物理设备", &mut cfg.bind_device);
                    changed |= opt_toggle_row(
                        &mut cols[1],
                        "系统转发",
                        &mut cfg.proxy_forward_by_system,
                    );
                    changed |= opt_toggle_row(&mut cols[0], "禁用加密", &mut cfg.disable_encryption);
                    changed |= opt_toggle_row(&mut cols[1], "禁用 IPv6", &mut cfg.disable_ipv6);
                    changed |= opt_toggle_row(&mut cols[0], "私有模式", &mut cfg.enable_private_mode);
                    changed |= opt_toggle_row(
                        &mut cols[1],
                        "转发全部 Peer RPC",
                        &mut cfg.relay_all_peer_rpc,
                    );
                    changed |= opt_toggle_row(
                        &mut cols[0],
                        "UDP 广播中继",
                        &mut cfg.enable_udp_broadcast_relay,
                    );
                    changed |= opt_toggle_row(&mut cols[1], "Magic DNS", &mut cfg.enable_magic_dns);
                });
                ui.label(
                    RichText::new("UDP 广播中继仅 Windows；Magic DNS 会修改系统 DNS")
                        .size(10.5)
                        .color(DIM),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [96.0, 26.0],
                        egui::Label::new(RichText::new("MTU").size(11.5).color(MUTED)),
                    );
                    let mut mtu = cfg.mtu.unwrap_or(0);
                    if ui
                        .add(egui::DragValue::new(&mut mtu).range(0..=9000).speed(1.0))
                        .changed()
                    {
                        cfg.mtu = if mtu == 0 { None } else { Some(mtu) };
                        changed = true;
                    }
                    ui.label(RichText::new("0 = 内核默认").size(10.5).color(DIM));
                });
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [96.0, 26.0],
                        egui::Label::new(
                            RichText::new("入站限速 B/s").size(11.5).color(MUTED),
                        ),
                    );
                    let mut limit = cfg.instance_recv_bps_limit.unwrap_or(0);
                    if ui
                        .add(
                            egui::DragValue::new(&mut limit)
                                .range(0..=1_000_000_000u64)
                                .speed(1024.0),
                        )
                        .changed()
                    {
                        cfg.instance_recv_bps_limit = if limit == 0 { None } else { Some(limit) };
                        changed = true;
                    }
                    ui.label(RichText::new("0 = 不限速").size(10.5).color(DIM));
                });
                changed |= text_row(ui, "TUN 接口名", &mut cfg.dev_name, "留空自动生成");
                changed |= text_row(ui, "凭据文件", &mut cfg.credential_file, "留空使用默认");
            });
        }

        ui.add_space(10.0);
        let mut save_clicked = false;
        let mut reset_clicked = false;
        let mut clone_clicked = false;
        let mut delete_clicked = false;
        let mut copy_clicked = false;
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("保存配置").color(Color32::WHITE)).fill(ACCENT))
                .clicked()
            {
                save_clicked = true;
            }
            if ui.button("重置").clicked() {
                reset_clicked = true;
            }
            if ui.button("复制 TOML").clicked() {
                copy_clicked = true;
            }
            if ui.button("克隆实例").clicked() {
                clone_clicked = true;
            }
            if ui
                .add(egui::Button::new(RichText::new("删除实例").color(ERR)))
                .clicked()
            {
                delete_clicked = true;
            }
            if let Some(busy) = self.busy {
                ui.label(RichText::new(busy).size(11.0).color(MUTED));
            }
        });
        ui.add_space(20.0);

        self.config = cfg;
        self.instance_name = instance_name;
        self.rpc_port_text = rpc_text;
        self.auto_start = auto_start;
        self.remote_manage = remote;
        self.whitelist_text = whitelist;
        self.console_level = level;
        self.show_advanced = show_advanced;
        self.secret_visible = secret_visible;
        if changed {
            self.config_dirty = true;
        }
        if open_picker {
            self.open_public_picker();
        }
        if copy_clicked {
            let mut copy = self.config.clone();
            copy.instance_id = inst.id.clone();
            ctx.copy_text(config::encode_toml(&copy, true));
            self.toast(&ctx, "TOML 已复制到剪贴板", true);
        }
        if save_clicked {
            self.save_config(&ctx);
        }
        if reset_clicked {
            self.reset_config(&ctx);
        }
        if clone_clicked {
            self.clone_instance(&ctx);
        }
        if delete_clicked {
            if self.instances.len() <= 1 {
                self.toast(&ctx, "至少保留一个网络实例", false);
            } else {
                self.confirm_remove();
            }
        }
    }

    fn logs_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();

        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 540.0), Sense::hover());
        card(&p, r);
        txt(&p, Pos2::new(r.left() + 18.0, r.top() + 16.0), Align2::LEFT_TOP, "运行日志", 14.0, TEXT);
        let running = self.active_running();
        if !running {
            let tag_x = r.left() + 18.0 + text_w("运行日志", 14.0) + 10.0;
            txt(&p, Pos2::new(tag_x, r.top() + 19.0), Align2::LEFT_TOP, "(网络未运行)", 10.5, DIM);
        }

        let filters = [LogFilter::All, LogFilter::Info, LogFilter::Warn, LogFilter::Error];
        let mut x = r.right() - 18.0;
        for filter in filters.iter().rev() {
            let label = filter.label();
            let cw = text_w(label, 10.0) + 16.0;
            x -= cw;
            let rect = Rect::from_min_size(Pos2::new(x, r.top() + 14.0), Vec2::new(cw, 20.0));
            let resp = ui.interact(rect, egui::Id::new(("log-filter", label)), Sense::click());
            let active = self.log_filter == *filter;
            p.rect_filled(rect, CornerRadius::same(10), if active { filter.color().gamma_multiply(0.16) } else { CARD_HI });
            txt(&p, rect.center(), Align2::CENTER_CENTER, label, 10.0, if active { filter.color() } else { MUTED });
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.log_filter = *filter;
            }
            x -= 6.0;
        }

        p.line_segment([Pos2::new(r.left() + 18.0, r.top() + 46.0), Pos2::new(r.right() - 18.0, r.top() + 46.0)], Stroke::new(1.0, BORDER));

        let list_rect = Rect::from_min_max(
            Pos2::new(r.left() + 18.0, r.top() + 56.0),
            Pos2::new(r.right() - 10.0, r.bottom() - 12.0),
        );
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        list_ui.spacing_mut().item_spacing.y = 4.0;
        let wrap_w = list_rect.width() - 14.0;
        ScrollArea::vertical()
            .id_salt(("logs", self.active.as_deref().unwrap_or("")))
            .auto_shrink([false, false])
            .show(&mut list_ui, |ui| {
                ui.set_width(wrap_w);
                let mut shown = 0;
                for line in self.logs.iter().filter(|l| self.log_filter.matches(&l.level)) {
                    let lc = match line.level.as_str() {
                        "INFO" => RX,
                        "WARN" => WARN,
                        "ERROR" => ERR,
                        "DEBUG" => DIM,
                        _ => MUTED,
                    };
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (pre, _) = ui.allocate_exact_size(Vec2::new(92.0, 15.0), Sense::hover());
                        let painter = ui.painter();
                        painter.text(pre.left_top(), Align2::LEFT_TOP, &line.time, FontId::monospace(10.5), DIM);
                        painter.text(Pos2::new(pre.left() + 56.0, pre.top()), Align2::LEFT_TOP, &line.level, FontId::monospace(10.5), lc);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&line.text).monospace().size(11.0).color(TEXT),
                            )
                            .wrap(),
                        );
                    });
                    shown += 1;
                }
                if shown == 0 {
                    let message = if self.logs.is_empty() {
                        "暂无日志；启动网络后将显示内核日志"
                    } else {
                        "当前筛选条件下没有日志"
                    };
                    ui.label(RichText::new(message).size(11.0).color(DIM));
                }
            });
    }
}

fn switch(ui: &mut egui::Ui, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(11),
        if on { OK.gamma_multiply(0.85) } else { CARD_HI },
    );
    let knob_x = if on {
        rect.right() - 11.0
    } else {
        rect.left() + 11.0
    };
    p.circle_filled(Pos2::new(knob_x, rect.center().y), 8.0, Color32::WHITE);
    resp.clicked()
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            [96.0, 26.0],
            egui::Label::new(RichText::new(label).size(11.5).color(MUTED)),
        );
        let w = ui.available_width().max(120.0);
        changed = ui
            .add_sized([w, 26.0], egui::TextEdit::singleline(value).hint_text(hint))
            .changed();
    });
    changed
}

fn bool_toggle_row(ui: &mut egui::Ui, label: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        if switch(ui, *value) {
            *value = !*value;
            changed = true;
        }
        ui.label(RichText::new(label).size(11.5).color(MUTED));
    });
    changed
}

fn opt_toggle_row(ui: &mut egui::Ui, label: &str, value: &mut Option<bool>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        if switch(ui, *value == Some(true)) {
            *value = Some(*value != Some(true));
            changed = true;
        }
        ui.label(RichText::new(label).size(11.5).color(MUTED));
    });
    changed
}

fn string_list(ui: &mut egui::Ui, values: &mut Vec<String>, placeholder: &str) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (i, value) in values.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 32.0).max(100.0);
            changed |= ui
                .add_sized([w, 26.0], egui::TextEdit::singleline(value).hint_text(placeholder))
                .changed();
            if ui.small_button("−").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        values.remove(i);
        changed = true;
    }
    if ui.small_button("+ 添加").clicked() {
        values.push(String::new());
        changed = true;
    }
    changed
}

fn port_forwards_editor(ui: &mut egui::Ui, forwards: &mut Vec<config::PortForward>) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (i, fwd) in forwards.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui
                .add_sized(
                    [56.0, 26.0],
                    egui::TextEdit::singleline(&mut fwd.proto).hint_text("tcp"),
                )
                .changed();
            changed |= ui
                .add_sized(
                    [120.0, 26.0],
                    egui::TextEdit::singleline(&mut fwd.bind_ip).hint_text("0.0.0.0"),
                )
                .changed();
            changed |= ui
                .add_sized(
                    [64.0, 26.0],
                    egui::DragValue::new(&mut fwd.bind_port).range(1..=65535),
                )
                .changed();
            ui.label(RichText::new("→").size(12.0).color(MUTED));
            changed |= ui
                .add_sized(
                    [120.0, 26.0],
                    egui::TextEdit::singleline(&mut fwd.dst_ip).hint_text("10.126.126.1"),
                )
                .changed();
            changed |= ui
                .add_sized(
                    [64.0, 26.0],
                    egui::DragValue::new(&mut fwd.dst_port).range(1..=65535),
                )
                .changed();
            if ui.small_button("−").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        forwards.remove(i);
        changed = true;
    }
    if ui.small_button("+ 添加转发").clicked() {
        forwards.push(config::PortForward::default());
        changed = true;
    }
    changed
}

fn lat_color(ms: u32) -> Color32 {
    if ms == 0 {
        MUTED
    } else if ms < 60 {
        OK
    } else if ms < 150 {
        WARN
    } else {
        ERR
    }
}

fn route_badge(cost: &str) -> (&'static str, Color32) {
    let value = cost.trim().to_lowercase();
    if value == "local" || value == "0" {
        ("本机", ACCENT)
    } else if value == "p2p" || value == "1" {
        ("P2P", OK)
    } else if value.starts_with("relay") || value.parse::<u32>().is_ok() {
        ("中继", WARN)
    } else if value.is_empty() {
        ("—", MUTED)
    } else {
        ("未知", MUTED)
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn nice_speed_max(peak: f32) -> f32 {
    let raw = (peak * 1.15).max(0.01);
    let exp = 10f32.powf(raw.log10().floor());
    let mantissa = raw / exp;
    let step = if mantissa <= 1.0 {
        1.0
    } else if mantissa <= 2.0 {
        2.0
    } else if mantissa <= 5.0 {
        5.0
    } else {
        10.0
    };
    step * exp
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events(ctx);

        if self.pending_hide {
            self.pending_hide = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        #[cfg(windows)]
        {
            if tray::take_quit() {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if tray::take_show() && !self.quitting {
                self.hidden = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            if tray::take_toggle() && !self.quitting {
                self.toggle_running(ctx);
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            self.hidden = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let visible = !self.hidden;
        if self.visible_sent != Some(visible) {
            self.visible_sent = Some(visible);
            let _ = self.actions.send(service::Action::SetVisible(visible));
        }

        #[cfg(windows)]
        self.sync_tray();

        if visible {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        *ui.style_mut() = self.style.clone();

        let screen = ui.max_rect();
        let sw = 240.0;
        {
            let bg = ui.painter();
            bg.rect_filled(screen, CornerRadius::ZERO, BG);
            bg.rect_filled(
                Rect::from_min_size(screen.min, Vec2::new(sw, screen.height())),
                CornerRadius::ZERO,
                SIDEBAR,
            );
            bg.line_segment(
                [
                    Pos2::new(screen.left() + sw, screen.top()),
                    Pos2::new(screen.left() + sw, screen.bottom()),
                ],
                Stroke::new(1.0, BORDER),
            );
        }

        egui::Panel::left("sidebar")
            .exact_size(sw)
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default().show(ui, |ui| {
            self.topbar(ui);
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let full_w = ui.available_width();
                    ui.set_width(full_w);
                    ui.add_space(6.0);
                    match self.tab {
                        Tab::Status => self.status_tab(ui),
                        Tab::Traffic => self.traffic_tab(ui),
                        Tab::Peers => self.peers_tab(ui),
                        Tab::Routes => self.routes_tab(ui),
                        Tab::Config => self.config_tab(ui),
                        Tab::Logs => self.logs_tab(ui),
                        Tab::Settings => self.settings_tab(ui),
                    }
                    ui.add_space(24.0);
                });
        });

        let ctx = ui.ctx().clone();
        self.dialogs(&ctx);

        if let Some((message, at, ok)) = self.toast.clone() {
            let now = ui.input(|i| i.time);
            if now - at > 3.2 {
                self.toast = None;
            } else {
                let p = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("toast"),
                ));
                let text_w = text_w(&message, 12.0) + 32.0;
                let rect = Rect::from_center_size(
                    Pos2::new(screen.center().x, screen.bottom() - 34.0),
                    Vec2::new(text_w.min(screen.width() - 40.0), 34.0),
                );
                p.rect_filled(rect, CornerRadius::same(8), CARD_HI);
                p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, if ok { OK.gamma_multiply(0.6) } else { ERR.gamma_multiply(0.7) }), StrokeKind::Inside);
                txt(&p, rect.center(), Align2::CENTER_CENTER, truncate(&message, 60), 12.0, if ok { TEXT } else { ERR });
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let start_hidden = std::env::args().any(|a| {
        matches!(a.as_str(), "--minimized" | "--tray" | "--silent")
    });
    let options = eframe::NativeOptions {
        viewport: {
            let (icon_w, icon_h, icon_rgba) = app_icon();
            egui::ViewportBuilder::default()
                .with_inner_size([1120.0, 760.0])
                .with_min_inner_size([960.0, 640.0])
                .with_title("EasyTier · 原生客户端")
                .with_icon(egui::IconData {
                    rgba: icon_rgba,
                    width: icon_w,
                    height: icon_h,
                })
                .with_visible(!start_hidden)
        },
        ..Default::default()
    };
    eframe::run_native(
        "EasyTier Native",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, start_hidden)))),
    )
}
