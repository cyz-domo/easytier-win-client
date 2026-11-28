#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, Vec2,
};

mod theme;
use theme::*;

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
            .with_tooltip("EasyTier · 原生 UI 预览")
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

#[derive(Clone, Copy)]
enum Route {
    Local,
    P2P,
    Relay,
}

impl Route {
    fn label(self) -> &'static str {
        match self {
            Route::Local => "本机",
            Route::P2P => "P2P",
            Route::Relay => "中继",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Route::Local => ACCENT,
            Route::P2P => OK,
            Route::Relay => WARN,
        }
    }
}

struct Peer {
    name: &'static str,
    id: &'static str,
    ip: &'static str,
    route: Route,
    latency: u32,
    up: f32,
    down: f32,
    loss: f32,
}

fn sample_at(t: f32) -> (f32, f32) {
    let rx = 0.9 + (t * 0.61).sin().abs() * 2.4 + (t * 2.7).sin().abs() * 0.35;
    let tx = 0.12 + (t * 0.37 + 1.7).sin().abs() * 0.55 + (t * 3.3).sin().abs() * 0.08;
    (rx, tx)
}

fn format_speed(mbps: f32) -> String {
    if mbps >= 1.0 {
        format!("{mbps:.1} MB/s")
    } else {
        format!("{:.0} KB/s", mbps * 1024.0)
    }
}

fn lat_color(ms: u32) -> Color32 {
    if ms < 20 {
        OK
    } else if ms < 60 {
        WARN
    } else {
        ERR
    }
}

struct App {
    tab: Tab,
    search: String,
    peer_query: String,
    name: String,
    secret: String,
    ipv4: String,
    port: String,
    dhcp: bool,
    peers: Vec<Peer>,
    selected_peer: Option<usize>,
    history: VecDeque<(f32, f32)>,
    next_sample: f64,
    font_loaded: bool,
    style: egui::Style,
    quitting: bool,
    #[cfg(windows)]
    tray: Option<tray_icon::TrayIcon>,
    logs: Vec<(String, &'static str, &'static str)>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (font_loaded, style) = theme::setup(&cc.egui_ctx);

        let mut history = VecDeque::with_capacity(120);
        for i in 0..90 {
            history.push_back(sample_at(i as f32 * 0.5));
        }

        let peers = vec![
            Peer { name: "客厅-NAS", id: "et-3f8a", ip: "10.126.126.2", route: Route::Local, latency: 0, up: 1.24, down: 3.42, loss: 0.0 },
            Peer { name: "书房-工作站", id: "et-8f3a", ip: "10.126.126.7", route: Route::P2P, latency: 12, up: 0.86, down: 2.10, loss: 0.0 },
            Peer { name: "笔记本-ThinkPad", id: "et-c21d", ip: "10.126.126.9", route: Route::P2P, latency: 28, up: 0.41, down: 1.87, loss: 0.1 },
            Peer { name: "iPhone-Cyz", id: "et-77b0", ip: "10.126.126.12", route: Route::Relay, latency: 87, up: 0.12, down: 0.65, loss: 1.4 },
            Peer { name: "树莓派-网关", id: "et-19ce", ip: "10.126.126.3", route: Route::P2P, latency: 6, up: 0.05, down: 0.11, loss: 0.0 },
            Peer { name: "卧室-电视", id: "et-4dd1", ip: "10.126.126.15", route: Route::P2P, latency: 18, up: 0.02, down: 4.60, loss: 0.0 },
            Peer { name: "平板-iPad", id: "et-a530", ip: "10.126.126.18", route: Route::Relay, latency: 132, up: 0.08, down: 0.42, loss: 2.2 },
            Peer { name: "Mac-mini", id: "et-6b9f", ip: "10.126.126.21", route: Route::P2P, latency: 33, up: 0.67, down: 1.03, loss: 0.0 },
            Peer { name: "家里-主路由", id: "et-0c47", ip: "10.126.126.1", route: Route::Local, latency: 0, up: 0.94, down: 2.88, loss: 0.0 },
            Peer { name: "旧本-Ubuntu", id: "et-e812", ip: "10.126.126.26", route: Route::P2P, latency: 51, up: 0.15, down: 0.28, loss: 0.6 },
        ];

        let logs = vec![
            ("23:58:12.104".to_owned(), "INFO", "kernel v2.6.4 started, pid=8124"),
            ("23:58:12.118".to_owned(), "INFO", "tun device ready: easytier0 mtu=1380"),
            ("23:58:12.221".to_owned(), "INFO", "dhcp lease acquired: 10.126.126.1/24"),
            ("23:58:12.640".to_owned(), "INFO", "listener ready on udp://0.0.0.0:11010"),
            ("23:58:13.002".to_owned(), "INFO", "peer handshake ok: et-8f3a (10.126.126.7)"),
            ("23:58:13.004".to_owned(), "INFO", "route update: 10.126.126.0/24 via et-8f3a"),
            ("23:58:13.310".to_owned(), "INFO", "peer handshake ok: et-3f8a (10.126.126.2)"),
            ("23:58:13.512".to_owned(), "WARN", "relay fallback: et-77b0 latency 87ms"),
            ("23:58:14.020".to_owned(), "INFO", "peer handshake ok: et-19ce (10.126.126.3)"),
            ("23:58:14.431".to_owned(), "ERROR", "probe timeout: et-a530, retry in 5s"),
            ("23:58:19.442".to_owned(), "INFO", "probe recovered: et-a530 via relay"),
            ("23:58:20.003".to_owned(), "INFO", "traffic sampler enabled, window=60s"),
        ];

        Self {
            tab: Tab::Status,
            search: String::new(),
            peer_query: String::new(),
            name: "家庭网络".to_owned(),
            secret: "correct-horse-battery".to_owned(),
            ipv4: "10.126.126.1".to_owned(),
            port: "11010".to_owned(),
            dhcp: true,
            peers,
            selected_peer: Some(0),
            history,
            next_sample: 0.0,
            font_loaded,
            style,
            quitting: false,
            #[cfg(windows)]
            tray: tray::setup(&cc.egui_ctx),
            logs,
        }
    }

    fn tick(&mut self, now: f64) {
        let mut guard = 0;
        while now >= self.next_sample && guard < 240 {
            self.history.push_back(sample_at(self.next_sample as f32));
            if self.history.len() > 120 {
                self.history.pop_front();
            }
            self.next_sample += 0.5;
            guard += 1;
        }
        if self.next_sample < now {
            self.next_sample = now + 0.5;
        }
    }

    fn visible_peers(&self) -> Vec<usize> {
        let q = self.peer_query.trim().to_lowercase();
        self.peers
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                q.is_empty()
                    || p.name.to_lowercase().contains(&q)
                    || p.ip.contains(&q)
                    || p.id.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let w = ui.available_width();
        let p = ui.painter().clone();

        let (brand, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
        p.circle_filled(Pos2::new(brand.left() + 9.0, brand.center().y), 7.0, ACCENT);
        txt(&p, Pos2::new(brand.left() + 24.0, brand.center().y), Align2::LEFT_CENTER, "EasyTier", 14.5, TEXT);
        txt(&p, Pos2::new(brand.left() + 26.0 + text_w("EasyTier", 14.5), brand.center().y), Align2::LEFT_CENTER, "native spike", 10.0, DIM);
        let vw = text_w("v0.1.5", 10.5) + 14.0;
        p.rect_filled(Rect::from_min_size(Pos2::new(brand.right() - vw, brand.center().y - 9.0), Vec2::new(vw, 18.0)), CornerRadius::same(9), CARD);
        txt(&p, Pos2::new(brand.right() - vw / 2.0, brand.center().y), Align2::CENTER_CENTER, "v0.1.5", 10.5, MUTED);

        ui.add_space(10.0);
        ui.add_sized([w, 30.0], egui::TextEdit::singleline(&mut self.search).hint_text("搜索网络或设备"));
        ui.add_space(12.0);

        let (lab, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
        txt(&p, Pos2::new(lab.left() + 2.0, lab.center().y), Align2::LEFT_CENTER, "网络", 11.0, DIM);
        txt(&p, Pos2::new(lab.right() - 2.0, lab.center().y), Align2::RIGHT_CENTER, "1 个", 10.5, DIM);

        let (item, resp) = ui.allocate_exact_size(Vec2::new(w, 58.0), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        p.rect_filled(item, CornerRadius::same(9), CARD);
        p.rect_stroke(item, CornerRadius::same(9), Stroke::new(1.0, BORDER), StrokeKind::Inside);
        p.rect_filled(Rect::from_min_size(Pos2::new(item.left(), item.top() + 12.0), Vec2::new(3.0, item.height() - 24.0)), CornerRadius::same(2), ACCENT);
        txt(&p, Pos2::new(item.left() + 16.0, item.top() + 12.0), Align2::LEFT_TOP, "家庭网络", 13.5, TEXT);
        txt(&p, Pos2::new(item.left() + 16.0, item.bottom() - 10.0), Align2::LEFT_BOTTOM, "27 台设备 · 在线", 11.0, MUTED);
        p.circle_filled(Pos2::new(item.right() - 16.0, item.center().y), 4.0, OK);

        let rest = (ui.available_height() - 100.0).max(0.0);
        ui.allocate_space(Vec2::new(w, rest));

        let (row, _) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::hover());
        let bw = (w - 16.0) / 3.0;
        let actions = [("+ 新建", ACCENT), ("克隆", MUTED), ("刷新", MUTED)];
        for (i, (label, col)) in actions.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(row.left() + i as f32 * (bw + 8.0), row.top()), Vec2::new(bw, 30.0));
            let resp = ui.interact(r, egui::Id::new(("sidebar-action", i)), Sense::click());
            p.rect_filled(r, CornerRadius::same(8), if resp.hovered() { CARD_HI } else { CARD });
            p.rect_stroke(r, CornerRadius::same(8), Stroke::new(1.0, BORDER), StrokeKind::Inside);
            txt(&p, r.center(), Align2::CENTER_CENTER, *label, 11.5, *col);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }

        ui.add_space(10.0);
        let (foot, _) = ui.allocate_exact_size(Vec2::new(w, 38.0), Sense::hover());
        if self.font_loaded {
            txt(&p, Pos2::new(foot.left() + 2.0, foot.top()), Align2::LEFT_TOP, "egui 直接绘制 · 系统字体已加载", 10.5, DIM);
        } else {
            txt(&p, Pos2::new(foot.left() + 2.0, foot.top()), Align2::LEFT_TOP, "CJK 字体缺失，中文将显示为方块", 10.5, ERR);
        }
        txt(&p, Pos2::new(foot.left() + 2.0, foot.top() + 16.0), Align2::LEFT_TOP, "演示数据 · 未接入内核", 10.5, DIM);
    }

    fn topbar(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::hover());
        let p = ui.painter().clone();

        txt(&p, Pos2::new(rect.left() + 2.0, rect.center().y), Align2::LEFT_CENTER, "家庭网络", 15.0, TEXT);
        p.circle_filled(Pos2::new(rect.left() + 12.0 + text_w("家庭网络", 15.0), rect.center().y), 3.5, OK);
        txt(&p, Pos2::new(rect.left() + 22.0 + text_w("家庭网络", 15.0), rect.center().y), Align2::LEFT_CENTER, "已连接", 11.5, MUTED);

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

        p.line_segment([Pos2::new(rect.left(), rect.bottom() - 0.5), Pos2::new(rect.right(), rect.bottom() - 0.5)], Stroke::new(1.0, BORDER));
    }

    fn status_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();

        let (head, _) = ui.allocate_exact_size(Vec2::new(w, 74.0), Sense::hover());
        txt(&p, Pos2::new(head.left(), head.top() + 2.0), Align2::LEFT_TOP, "家庭网络", 26.0, TEXT);
        p.circle_filled(Pos2::new(head.left() + 6.0, head.top() + 48.0), 4.0, OK);
        txt(&p, Pos2::new(head.left() + 15.0, head.top() + 48.0), Align2::LEFT_CENTER, "已连接 · 27 台设备在线 · 内核 v2.6.4", 12.0, MUTED);

        let chips: [(&str, Color32); 3] = [("DHCP 已启用", OK), ("虚拟 IPv4 10.126.126.1", TEXT), ("TUN 模式", RX)];
        let mut x = head.right() - 2.0;
        for (label, fg) in chips.iter().rev() {
            let cw = text_w(label, 11.5) + 16.0;
            x -= cw;
            let bg = if *fg == TEXT { CARD } else { fg.gamma_multiply(0.14) };
            pill(&p, Pos2::new(x, head.top() + 37.0), 22.0, label, *fg, bg);
            x -= 8.0;
        }

        ui.add_space(6.0);
        let gap = 12.0;
        let cw = (w - gap * 3.0) / 4.0;
        let (mrow, _) = ui.allocate_exact_size(Vec2::new(w, 88.0), Sense::hover());
        let cards: [(&str, String, &str); 4] = [
            ("本机虚拟地址", "10.126.126.1".to_owned(), "点击复制"),
            ("组网成员", "27".to_owned(), "在线节点"),
            ("路由条目", "18".to_owned(), "已知网段"),
            ("本次累计", "1.24 GB".to_owned(), "会话累计"),
        ];
        for (i, (title, value, sub)) in cards.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(mrow.left() + i as f32 * (cw + gap), mrow.top()), Vec2::new(cw, 88.0));
            card(&p, r);
            txt(&p, Pos2::new(r.left() + 14.0, r.top() + 14.0), Align2::LEFT_TOP, *title, 11.0, MUTED);
            txt(&p, Pos2::new(r.left() + 14.0, r.top() + 40.0), Align2::LEFT_TOP, value, 19.0, TEXT);
            txt(&p, Pos2::new(r.left() + 14.0, r.bottom() - 14.0), Align2::LEFT_BOTTOM, *sub, 10.5, DIM);
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

        let idxs = self.visible_peers();
        for i in idxs {
            let peer = &self.peers[i];
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

            let av = Pos2::new(row.left() + 26.0, row.center().y);
            p.circle_filled(av, 13.0, CARD_HI);
            let initial = peer.name.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".to_owned());
            txt(&p, av, Align2::CENTER_CENTER, &initial, 12.0, ACCENT);
            txt(&p, Pos2::new(row.left() + 46.0, row.top() + 11.0), Align2::LEFT_TOP, peer.name, 12.5, TEXT);
            mono(&p, Pos2::new(row.left() + 46.0, row.bottom() - 10.0), Align2::LEFT_BOTTOM, peer.id, 10.0, DIM);
            mono(&p, Pos2::new(row.left() + row.width() * 0.28, row.center().y), Align2::LEFT_CENTER, peer.ip, 11.5, MUTED);
            pill(&p, Pos2::new(row.left() + row.width() * 0.44, row.center().y - 10.0), 20.0, peer.route.label(), peer.route.color(), peer.route.color().gamma_multiply(0.16));
            txt(&p, Pos2::new(row.left() + row.width() * 0.56, row.center().y), Align2::LEFT_CENTER, format!("{} ms", peer.latency), 11.5, lat_color(peer.latency));
            txt(&p, Pos2::new(row.left() + row.width() * 0.66, row.center().y), Align2::LEFT_CENTER, format!("↑ {}", format_speed(peer.up)), 11.0, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.76, row.center().y), Align2::LEFT_CENTER, format!("↓ {}", format_speed(peer.down)), 11.0, MUTED);
            txt(&p, Pos2::new(row.left() + row.width() * 0.88, row.center().y), Align2::LEFT_CENTER, format!("{:.1}%", peer.loss), 11.0, if peer.loss > 1.0 { WARN } else { MUTED });

            p.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, Color32::from_rgb(0x26, 0x2A, 0x30)));
            if resp.clicked() {
                self.selected_peer = Some(i);
            }
        }
    }

    fn traffic_tab(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let p = ui.painter().clone();

        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 340.0), Sense::hover());
        card(&p, r);
        txt(&p, Pos2::new(r.left() + 18.0, r.top() + 16.0), Align2::LEFT_TOP, "实时网络流量", 14.0, TEXT);
        txt(&p, Pos2::new(r.left() + 18.0, r.top() + 38.0), Align2::LEFT_TOP, "上行 / 下行 · 近 60 秒 · 直接由 egui 绘制", 11.0, DIM);

        let (last_rx, last_tx) = *self.history.back().unwrap_or(&(0.0, 0.0));
        let peak = self.history.iter().map(|(a, b)| a.max(*b)).fold(0.0f32, f32::max);

        let legend: [(String, Color32); 3] = [
            (format!("↓ {}", format_speed(last_rx)), RX),
            (format!("↑ {}", format_speed(last_tx)), TX),
            (format!("★ 峰值 {}", format_speed(peak)), WARN),
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

        txt(&p, Pos2::new(plot.left(), plot.bottom() + 10.0), Align2::LEFT_TOP, "60 秒前", 10.0, DIM);
        txt(&p, Pos2::new(plot.right(), plot.bottom() + 10.0), Align2::RIGHT_TOP, "现在", 10.0, DIM);

        ui.add_space(12.0);
        let gap = 12.0;
        let cw = (w - gap * 2.0) / 3.0;
        let (srow, _) = ui.allocate_exact_size(Vec2::new(w, 80.0), Sense::hover());
        let stats: [(&str, String, Color32); 3] = [
            ("当前下行", format_speed(last_rx), RX),
            ("当前上行", format_speed(last_tx), TX),
            ("近 60 秒峰值", format_speed(peak), WARN),
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

        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 430.0), Sense::hover());
        card(&p, r);
        txt(&p, Pos2::new(r.left() + 20.0, r.top() + 18.0), Align2::LEFT_TOP, "基础配置", 14.0, TEXT);
        txt(&p, Pos2::new(r.left() + 20.0, r.top() + 38.0), Align2::LEFT_TOP, "「网络名称」支持中文，可在这里验证输入法组词", 11.0, WARN);

        let label_x = r.left() + 20.0;
        let field_x = r.left() + 150.0;
        let field_w = 300.0;
        let row_h = 48.0;
        let mut y = r.top() + 76.0;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "网络名称", 12.0, MUTED);
        ui.put(Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)), egui::TextEdit::singleline(&mut self.name).hint_text("例如：home-net"));
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "网络密钥", 12.0, MUTED);
        ui.put(Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)), egui::TextEdit::singleline(&mut self.secret).password(true));
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "虚拟 IPv4", 12.0, MUTED);
        ui.put(Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)), egui::TextEdit::singleline(&mut self.ipv4).hint_text("10.126.126.1"));
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 16.0), Align2::LEFT_CENTER, "监听端口", 12.0, MUTED);
        ui.put(Rect::from_min_size(Pos2::new(field_x, y), Vec2::new(field_w, 32.0)), egui::TextEdit::singleline(&mut self.port).hint_text("11010"));
        y += row_h;

        txt(&p, Pos2::new(label_x, y + 11.0), Align2::LEFT_CENTER, "DHCP", 12.0, MUTED);
        let sw = Rect::from_min_size(Pos2::new(field_x, y + 1.0), Vec2::new(40.0, 22.0));
        let resp = ui.interact(sw, egui::Id::new("dhcp-switch"), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.dhcp = !self.dhcp;
        }
        p.rect_filled(sw, CornerRadius::same(11), if self.dhcp { OK.gamma_multiply(0.85) } else { CARD_HI });
        let knob_x = if self.dhcp { sw.right() - 11.0 } else { sw.left() + 11.0 };
        p.circle_filled(Pos2::new(knob_x, sw.center().y), 8.0, Color32::WHITE);
        txt(&p, Pos2::new(sw.right() + 10.0, sw.center().y), Align2::LEFT_CENTER, if self.dhcp { "已启用 · 自动分配地址" } else { "已关闭 · 使用固定地址" }, 11.5, MUTED);

        let btn_y = r.bottom() - 56.0;
        let save = Rect::from_min_size(Pos2::new(field_x, btn_y), Vec2::new(96.0, 32.0));
        let _ = ui.put(save, egui::Button::new(RichText::new("保存配置").color(Color32::WHITE)).fill(ACCENT));
        let reset = Rect::from_min_size(Pos2::new(field_x + 108.0, btn_y), Vec2::new(80.0, 32.0));
        if ui.put(reset, egui::Button::new("重置")).clicked() {
            self.name = "家庭网络".to_owned();
            self.dhcp = true;
        }

        let side = Rect::from_min_size(Pos2::new(r.right() - 250.0, r.top() + 76.0), Vec2::new(230.0, 220.0));
        p.rect_filled(side, CornerRadius::same(8), INPUT);
        p.rect_stroke(side, CornerRadius::same(8), Stroke::new(1.0, BORDER), StrokeKind::Inside);
        txt(&p, Pos2::new(side.left() + 14.0, side.top() + 14.0), Align2::LEFT_TOP, "配置摘要", 12.0, TEXT);
        let rows: [(&str, String); 6] = [
            ("网络类型", "私有网络".to_owned()),
            ("连接方式", "P2P + 中继回退".to_owned()),
            ("加密算法", "WireGuard".to_owned()),
            ("TUN 设备", "easytier0".to_owned()),
            ("MTU", "1380".to_owned()),
            ("IPv4 分配", if self.dhcp { "DHCP".to_owned() } else { "固定地址".to_owned() }),
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

        let levels: [(&str, Color32); 4] = [("ALL", TEXT), ("INFO", RX), ("WARN", WARN), ("ERROR", ERR)];
        let mut x = r.right() - 18.0;
        for (label, col) in levels.iter().rev() {
            let cw = text_w(label, 10.0) + 16.0;
            x -= cw;
            p.rect_filled(Rect::from_min_size(Pos2::new(x, r.top() + 14.0), Vec2::new(cw, 20.0)), CornerRadius::same(10), CARD_HI);
            txt(&p, Pos2::new(x + cw / 2.0, r.top() + 24.0), Align2::CENTER_CENTER, *label, 10.0, *col);
            x -= 6.0;
        }

        p.line_segment([Pos2::new(r.left() + 18.0, r.top() + 46.0), Pos2::new(r.right() - 18.0, r.top() + 46.0)], Stroke::new(1.0, BORDER));

        let mut y = r.top() + 58.0;
        for (ts, level, msg) in self.logs.iter() {
            let lc = match *level {
                "INFO" => RX,
                "WARN" => WARN,
                "ERROR" => ERR,
                _ => MUTED,
            };
            mono(&p, Pos2::new(r.left() + 18.0, y), Align2::LEFT_TOP, ts, 10.5, DIM);
            mono(&p, Pos2::new(r.left() + 92.0, y), Align2::LEFT_TOP, *level, 10.5, lc);
            mono(&p, Pos2::new(r.left() + 142.0, y), Align2::LEFT_TOP, *msg, 11.0, TEXT);
            y += 18.0;
            if y > r.bottom() - 18.0 {
                break;
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            if tray::take_quit() {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if tray::take_show() && !self.quitting {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let now = ctx.input(|i| i.time);
        self.tick(now);
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
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
            .with_title("EasyTier · 原生 UI 预览")
            .with_visible(!start_hidden),
        ..Default::default()
    };
    eframe::run_native(
        "EasyTier Native Spike",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
