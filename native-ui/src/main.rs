#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, Vec2,
};
use serde_json::json;

mod service;
mod theme;
use theme::*;

const VERSION: &str = "v0.1.5";

#[cfg(windows)]
mod tray {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    static SHOW: AtomicBool = AtomicBool::new(false);
    static QUIT: AtomicBool = AtomicBool::new(false);

    pub fn take_show() -> bool {
        SHOW.swap(false, Ordering::Relaxed)
    }

    pub fn take_quit() -> bool {
        QUIT.swap(false, Ordering::Relaxed)
    }

    fn icon() -> Icon {
        const N: u32 = 32;
        let mut rgba = vec![0u8; (N * N * 4) as usize];
        let c = N as f32 / 2.0;
        for y in 0..N {
            for x in 0..N {
                let dx = x as f32 + 0.5 - c;
                let dy = y as f32 + 0.5 - c;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 15.0 {
                    continue;
                }
                let (r, g, b) = if d < 5.5 {
                    (0xE8, 0xEA, 0xEE)
                } else if d < 10.0 {
                    (0x17, 0x19, 0x1D)
                } else {
                    (0x4C, 0x8D, 0xFF)
                };
                let i = ((y * N + x) * 4) as usize;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }
        Icon::from_rgba(rgba, N, N).expect("valid tray icon")
    }

    pub fn setup(ctx: &eframe::egui::Context) -> Option<TrayIcon> {
        let menu = Menu::new();
        let show = MenuItem::new("显示主界面", true, None);
        let quit = MenuItem::new("退出", true, None);
        menu.append(&show).ok()?;
        menu.append(&quit).ok()?;
        let show_id = show.id().clone();
        let quit_id = quit.id().clone();

        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(Arc::new(move |e: MenuEvent| {
            if e.id == show_id {
                SHOW.store(true, Ordering::Relaxed);
                menu_ctx.request_repaint();
            } else if e.id == quit_id {
                QUIT.store(true, Ordering::Relaxed);
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

        TrayIconBuilder::new()
            .with_tooltip("EasyTier · 原生客户端")
            .with_menu(Box::new(menu))
            .with_icon(icon())
            .build()
            .ok()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Status,
    Traffic,
    Config,
    Logs,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Status, "状态"),
    (Tab::Traffic, "流量"),
    (Tab::Config, "配置"),
    (Tab::Logs, "日志"),
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
            let time = if let Some(t) = stamp.get(11..19) {
                t.to_owned()
            } else {
                stamp.to_owned()
            };
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

fn looks_like_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("error") || lower.contains("fail") || lower.contains("panic") || text.contains("TunnelError")
}

struct App {
    tab: Tab,
    search: String,
    peer_query: String,
    config: service::ConfigFields,
    config_dirty: bool,
    config_source: String,
    peers: Vec<service::Peer>,
    selected_peer: Option<usize>,
    history: VecDeque<(f32, f32)>,
    node: Option<service::NodeInfo>,
    routes: usize,
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
    style: egui::Style,
    quitting: bool,
    hidden: bool,
    pending_hide: bool,
    visible_sent: Option<bool>,
    actions: Sender<service::Action>,
    events: Receiver<service::Event>,
    #[cfg(windows)]
    tray: Option<tray_icon::TrayIcon>,
}

fn zero_history() -> VecDeque<(f32, f32)> {
    (0..30).map(|_| (0.0, 0.0)).collect()
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Self {
        let (font_loaded, style) = theme::setup(&cc.egui_ctx);
        let (actions, events) = service::spawn(cc.egui_ctx.clone());
        Self {
            tab: Tab::Status,
            search: String::new(),
            peer_query: String::new(),
            config: service::ConfigFields::default(),
            config_dirty: false,
            config_source: String::new(),
            peers: Vec::new(),
            selected_peer: None,
            history: zero_history(),
            node: None,
            routes: 0,
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
            style,
            quitting: false,
            hidden: start_hidden,
            pending_hide: start_hidden,
            visible_sent: None,
            actions,
            events,
            #[cfg(windows)]
            tray: tray::setup(&cc.egui_ctx),
        }
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
        self.routes = 0;
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
        self.config = service::config_fields(&inst.config_toml);
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

    fn validate_config(&self) -> Result<(), String> {
        if self.config.network_name.trim().is_empty() {
            return Err("网络名称不能为空".to_owned());
        }
        if self.config.port == 0 {
            return Err("监听端口必须在 1-65535 之间".to_owned());
        }
        if !self.config.dhcp {
            let ip = self.config.ipv4.trim();
            let octets: Vec<&str> = ip.split('.').collect();
            let valid = octets.len() == 4
                && octets
                    .iter()
                    .all(|o| !o.is_empty() && o.len() <= 3 && o.chars().all(|c| c.is_ascii_digit()));
            if !valid {
                return Err("虚拟 IPv4 格式不正确".to_owned());
            }
        }
        Ok(())
    }

    fn build_payload(
        &self,
        inst: &service::Instance,
        toml_text: String,
        start: bool,
    ) -> serde_json::Value {
        json!({
            "instance_id": inst.id,
            "name": inst.name,
            "config_toml": toml_text,
            "rpc_port": inst.rpc_port,
            "auto_start": inst.auto_start,
            "desired_state": if start { "running" } else { "stopped" },
            "remote_manage_enabled": inst.remote_manage,
            "rpc_whitelist_cidrs": inst.whitelist,
            "console_log_level": inst.console_log_level,
        })
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
        if inst.rpc_port == 0 {
            self.toast(ctx, "实例 RPC 端口无效", false);
            return;
        }
        let start = !inst.running();
        if start {
            if let Err(e) = self.validate_config() {
                self.toast(ctx, format!("请先修正配置：{e}"), false);
                self.tab = Tab::Config;
                return;
            }
        }
        let fields = if self.config_dirty || self.config_source.is_empty() {
            match service::patch_config(&inst.config_toml, &self.config) {
                Ok(toml_text) => {
                    self.config_dirty = false;
                    self.config_source = toml_text.clone();
                    toml_text
                }
                Err(e) => {
                    self.toast(ctx, e, false);
                    return;
                }
            }
        } else {
            inst.config_toml.clone()
        };
        let payload = self.build_payload(&inst, fields, start);
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
        if let Err(e) = self.validate_config() {
            self.toast(ctx, e, false);
            return;
        }
        let toml_text = match service::patch_config(&inst.config_toml, &self.config) {
            Ok(text) => text,
            Err(e) => {
                self.toast(ctx, e, false);
                return;
            }
        };
        let payload = self.build_payload(&inst, toml_text.clone(), inst.running());
        self.busy = Some("保存中…");
        self.config_dirty = false;
        self.config_source = toml_text;
        let _ = self.actions.send(service::Action::Save { payload });
    }

    fn reset_config(&mut self, ctx: &egui::Context) {
        let Some(inst) = self.active_instance().cloned() else {
            return;
        };
        self.config = service::config_fields(&inst.config_toml);
        self.config_source = inst.config_toml;
        self.config_dirty = false;
        self.toast(ctx, "已恢复为服务端保存的配置", true);
    }

    fn display_ip(&self) -> String {
        if let Some(node) = &self.node {
            let ip = node.ipv4.split('/').next().unwrap_or("");
            if !ip.is_empty() {
                return ip.to_owned();
            }
        }
        if !self.config.dhcp {
            self.config.ipv4.trim().to_owned()
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
                        self.ensure_config();
                        let running = self.active_running();
                        if self.prev_running && !running {
                            self.reset_session();
                        }
                        if !running {
                            self.peers.clear();
                            self.node = None;
                            self.routes = 0;
                            self.status_error = None;
                        }
                        self.prev_running = running;
                    } else {
                        self.instances.clear();
                        self.peers.clear();
                        self.node = None;
                        self.routes = 0;
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
                    self.toast(ctx, format!("{}：{}", result.label, result.message), result.ok);
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
        p.circle_filled(Pos2::new(brand.left() + 9.0, brand.center().y), 7.0, ACCENT);
        txt(&p, Pos2::new(brand.left() + 24.0, brand.center().y), Align2::LEFT_CENTER, "EasyTier", 14.5, TEXT);
        txt(&p, Pos2::new(brand.left() + 26.0 + text_w("EasyTier", 14.5), brand.center().y), Align2::LEFT_CENTER, "native", 10.0, DIM);
        let vw = text_w(VERSION, 10.5) + 14.0;
        p.rect_filled(Rect::from_min_size(Pos2::new(brand.right() - vw, brand.center().y - 9.0), Vec2::new(vw, 18.0)), CornerRadius::same(9), CARD);
        txt(&p, Pos2::new(brand.right() - vw / 2.0, brand.center().y), Align2::CENTER_CENTER, VERSION, 10.5, MUTED);

        ui.add_space(10.0);
        ui.add_sized([w, 30.0], egui::TextEdit::singleline(&mut self.search).hint_text("搜索网络"));
        ui.add_space(12.0);

        let (lab, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
        txt(&p, Pos2::new(lab.left() + 2.0, lab.center().y), Align2::LEFT_CENTER, "网络", 11.0, DIM);
        txt(&p, Pos2::new(lab.right() - 2.0, lab.center().y), Align2::RIGHT_CENTER, format!("{} 个", self.instances.len()), 10.5, DIM);

        let query = self.search.trim().to_lowercase();
        let instances = self.instances.clone();
        let active = self.active.clone();
        let mut clicked: Option<String> = None;
        for inst in instances
            .iter()
            .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query))
        {
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
            txt(&p, Pos2::new(item.left() + 16.0, item.top() + 12.0), Align2::LEFT_TOP, &inst.name, 13.5, if selected { TEXT } else { MUTED });
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
        let actions = [("+ 新建", false), ("克隆", false), ("刷新", true)];
        for (i, (label, enabled)) in actions.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(row.left() + i as f32 * (bw + 8.0), row.top()), Vec2::new(bw, 30.0));
            let resp = ui.interact(r, egui::Id::new(("sidebar-action", i)), Sense::click());
            p.rect_filled(r, CornerRadius::same(8), if resp.hovered() && *enabled { CARD_HI } else { CARD });
            p.rect_stroke(r, CornerRadius::same(8), Stroke::new(1.0, BORDER), StrokeKind::Inside);
            let color = if *enabled { if resp.hovered() { TEXT } else { MUTED } } else { DIM };
            txt(&p, r.center(), Align2::CENTER_CENTER, *label, 11.5, color);
            if resp.hovered() && *enabled {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                if i == 0 || i == 1 {
                    self.toast(&ctx, "新建/克隆将在下一阶段接入", false);
                } else {
                    let _ = self.actions.send(service::Action::Refresh);
                    self.toast(&ctx, "已请求刷新", true);
                }
            }
        }

        ui.add_space(10.0);
        let (foot, _) = ui.allocate_exact_size(Vec2::new(w, 38.0), Sense::hover());
        if self.font_loaded {
            txt(&p, Pos2::new(foot.left() + 2.0, foot.top()), Align2::LEFT_TOP, "egui 直接绘制 · 系统字体已加载", 10.5, DIM);
        } else {
            txt(&p, Pos2::new(foot.left() + 2.0, foot.top()), Align2::LEFT_TOP, "CJK 字体缺失，中文将显示为方块", 10.5, ERR);
        }
        let (foot_text, foot_color) = if let Some(err) = &self.service_error {
            (format!("服务不可用 · {}", truncate(err, 26)), ERR)
        } else if self.service_running {
            ("已连接后台服务 · 真实数据".to_owned(), OK)
        } else {
            ("正在连接后台服务…".to_owned(), DIM)
        };
        txt(&p, Pos2::new(foot.left() + 2.0, foot.top() + 16.0), Align2::LEFT_TOP, foot_text, 10.5, foot_color);
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

        let seg_w = 58.0;
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
        let fill = if !enabled {
            CARD
        } else if running {
            CARD_HI
        } else {
            ACCENT
        };
        p.rect_filled(button, CornerRadius::same(8), fill);
        p.rect_stroke(button, CornerRadius::same(8), Stroke::new(1.0, BORDER), StrokeKind::Inside);
        txt(&p, button.center(), Align2::CENTER_CENTER, label, 11.5, if !enabled { DIM } else if running { MUTED } else { Color32::WHITE });
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
                self.empty_state(ui, &p, "没有可用的网络实例", "创建与管理实例将在下一阶段接入", None);
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
                if running { self.routes.to_string() } else { "—".to_owned() },
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

        if self.peers.is_empty() && running && self.status_error.is_none() {
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::hover());
            txt(&p, Pos2::new(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, "等待内核返回成员数据…", 11.5, DIM);
        }
    }

    fn visible_peers(&self) -> Vec<usize> {
        let q = self.peer_query.trim().to_lowercase();
        self.peers
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                q.is_empty()
                    || p.hostname.to_lowercase().contains(&q)
                    || p.ip.contains(&q)
                    || p.id.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn traffic_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();
        let running = self.active_running();

        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 340.0), Sense::hover());
        card(&p, r);
        txt(&p, Pos2::new(r.left() + 18.0, r.top() + 16.0), Align2::LEFT_TOP, "实时网络流量", 14.0, TEXT);
        txt(&p, Pos2::new(r.left() + 18.0, r.top() + 38.0), Align2::LEFT_TOP, "上行 / 下行 · 近 60 秒 · 直接由 egui 绘制", 11.0, DIM);

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

        let plot = Rect::from_min_max(Pos2::new(r.left() + 18.0, r.top() + 76.0), Pos2::new(r.right() - 18.0, r.bottom() - 40.0));
        let max = peak.max(1.0);
        for i in 0..=4 {
            let y = plot.top() + plot.height() * i as f32 / 4.0;
            p.hline(plot.x_range(), y, Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2F, 0x37)));
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
        let w = ui.available_width();
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();

        let Some(inst) = self.active_instance().cloned() else {
            self.empty_state(ui, &p, "未选择网络", "选择左侧网络后编辑配置", None);
            return;
        };
        if inst.config_toml.is_empty() {
            self.empty_state(ui, &p, "配置不可用", "服务端没有返回该实例的配置内容", None);
            return;
        }

        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 430.0), Sense::hover());
        card(&p, r);
        txt(&p, Pos2::new(r.left() + 20.0, r.top() + 18.0), Align2::LEFT_TOP, "基础配置", 14.0, TEXT);
        let subtitle = if self.config_dirty {
            "有未保存的修改，点击「保存配置」写入服务"
        } else {
            "修改后点击「保存配置」；运行中修改需要重启网络生效"
        };
        txt(&p, Pos2::new(r.left() + 20.0, r.top() + 38.0), Align2::LEFT_TOP, subtitle, 11.0, if self.config_dirty { WARN } else { DIM });

        let label_x = r.left() + 20.0;
        let field_x = r.left() + 150.0;
        let field_w = 300.0;
        let row_h = 48.0;
        let mut y = r.top() + 76.0;
        let mut changed = false;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "网络名称", 12.0, MUTED);
        changed |= ui
            .put(Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)), egui::TextEdit::singleline(&mut self.config.network_name).hint_text("例如：home-net"))
            .changed();
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "网络密钥", 12.0, MUTED);
        changed |= ui
            .put(Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)), egui::TextEdit::singleline(&mut self.config.network_secret).password(true))
            .changed();
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "虚拟 IPv4", 12.0, MUTED);
        let ip_resp = ui.put(
            Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)),
            egui::TextEdit::singleline(&mut self.config.ipv4)
                .hint_text(if self.config.dhcp { "由 DHCP 自动分配" } else { "10.126.126.1" })
                .interactive(!self.config.dhcp),
        );
        changed |= ip_resp.changed();
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "监听端口", 12.0, MUTED);
        let mut port_text = if self.config.port == 0 {
            String::new()
        } else {
            self.config.port.to_string()
        };
        if ui
            .put(
                Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)),
                egui::TextEdit::singleline(&mut port_text).hint_text("11010"),
            )
            .changed()
        {
            if let Ok(port) = port_text.trim().parse::<u16>() {
                if port != self.config.port {
                    self.config.port = port;
                    changed = true;
                }
            } else if port_text.trim().is_empty() {
                if self.config.port != 0 {
                    self.config.port = 0;
                    changed = true;
                }
            }
        }
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 11.0), Align2::LEFT_CENTER, "DHCP", 12.0, MUTED);
        let sw = Rect::from_min_size(Pos2::new(field_x, y + 1.0), Vec2::new(40.0, 22.0));
        let resp = ui.interact(sw, egui::Id::new("dhcp-switch"), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.config.dhcp = !self.config.dhcp;
            changed = true;
        }
        p.rect_filled(sw, CornerRadius::same(11), if self.config.dhcp { OK.gamma_multiply(0.85) } else { CARD_HI });
        let knob_x = if self.config.dhcp { sw.right() - 11.0 } else { sw.left() + 11.0 };
        p.circle_filled(Pos2::new(knob_x, sw.center().y), 8.0, Color32::WHITE);
        txt(&p, Pos2::new(sw.right() + 10.0, sw.center().y), Align2::LEFT_CENTER, if self.config.dhcp { "已启用 · 自动分配地址" } else { "已关闭 · 使用固定地址" }, 11.5, MUTED);

        if changed {
            self.config_dirty = true;
        }

        let btn_y = r.bottom() - 56.0;
        let save = Rect::from_min_size(Pos2::new(field_x, btn_y), Vec2::new(96.0, 32.0));
        let save_resp = ui.put(save, egui::Button::new(RichText::new("保存配置").color(Color32::WHITE)).fill(ACCENT));
        if save_resp.clicked() {
            self.save_config(&ctx);
        }
        let reset = Rect::from_min_size(Pos2::new(field_x + 108.0, btn_y), Vec2::new(80.0, 32.0));
        if ui.put(reset, egui::Button::new("重置")).clicked() {
            self.reset_config(&ctx);
        }
        if let Some(busy) = self.busy {
            txt(&p, Pos2::new(field_x + 200.0, btn_y + 16.0), Align2::LEFT_CENTER, busy, 11.0, MUTED);
        }

        let side = Rect::from_min_size(Pos2::new(r.right() - 250.0, r.top() + 76.0), Vec2::new(230.0, 264.0));
        p.rect_filled(side, CornerRadius::same(8), INPUT);
        p.rect_stroke(side, CornerRadius::same(8), Stroke::new(1.0, BORDER), StrokeKind::Inside);
        txt(&p, Pos2::new(side.left() + 14.0, side.top() + 14.0), Align2::LEFT_TOP, "配置摘要", 12.0, TEXT);
        let rows: [(&str, String); 7] = [
            ("实例 ID", truncate(&inst.id, 12)),
            ("RPC 端口", inst.rpc_port.to_string()),
            ("当前状态", inst.status_label().to_owned()),
            ("开机自启", if inst.auto_start { "已开启" } else { "关闭" }.to_owned()),
            ("远程管理", if inst.remote_manage { "已开启" } else { "关闭" }.to_owned()),
            ("IPv4 分配", if self.config.dhcp { "DHCP" } else { "固定地址" }.to_owned()),
            ("监听端口", self.config.port.to_string()),
        ];
        let mut ry = side.top() + 42.0;
        for (k, v) in rows.iter() {
            txt(&p, Pos2::new(side.left() + 14.0, ry), Align2::LEFT_TOP, *k, 11.0, DIM);
            txt(&p, Pos2::new(side.right() - 14.0, ry), Align2::RIGHT_TOP, v, 11.0, MUTED);
            ry += 28.0;
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
            txt(&p, Pos2::new(r.left() + 70.0, r.top() + 18.0), Align2::LEFT_TOP, "(网络未运行)", 10.5, DIM);
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

        let mut y = r.top() + 58.0;
        let mut shown = 0;
        for line in self.logs.iter().filter(|l| self.log_filter.matches(&l.level)) {
            let lc = match line.level.as_str() {
                "INFO" => RX,
                "WARN" => WARN,
                "ERROR" => ERR,
                "DEBUG" => DIM,
                _ => MUTED,
            };
            mono(&p, Pos2::new(r.left() + 18.0, y), Align2::LEFT_TOP, &line.time, 10.5, DIM);
            mono(&p, Pos2::new(r.left() + 92.0, y), Align2::LEFT_TOP, &line.level, 10.5, lc);
            mono(&p, Pos2::new(r.left() + 142.0, y), Align2::LEFT_TOP, truncate(&line.text, 110), 11.0, TEXT);
            y += 18.0;
            shown += 1;
            if y > r.bottom() - 18.0 {
                break;
            }
        }
        if shown == 0 {
            let message = if self.logs.is_empty() {
                "暂无日志；启动网络后将显示内核日志"
            } else {
                "当前筛选条件下没有日志"
            };
            txt(&p, Pos2::new(r.left() + 18.0, r.top() + 64.0), Align2::LEFT_TOP, message, 11.0, DIM);
        }
    }
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

        if visible {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        *ui.style_mut() = self.style.clone();

        let screen = ui.max_rect();
        let sw = 296.0;
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
                        Tab::Config => self.config_tab(ui),
                        Tab::Logs => self.logs_tab(ui),
                    }
                    ui.add_space(24.0);
                });
        });

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
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("EasyTier · 原生客户端")
            .with_visible(!start_hidden),
        ..Default::default()
    };
    eframe::run_native(
        "EasyTier Native",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, start_hidden)))),
    )
}
