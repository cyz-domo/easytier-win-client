//! NetworkConfig model and TOML codec, mirroring src/network-config.ts and
//! src/toml-codec.ts so configs stay interchangeable with the Tauri client.

#[derive(Clone, Debug)]
pub struct PortForward {
    pub bind_ip: String,
    pub bind_port: u16,
    pub dst_ip: String,
    pub dst_port: u16,
    pub proto: String,
}

impl Default for PortForward {
    fn default() -> Self {
        Self {
            bind_ip: "0.0.0.0".to_owned(),
            bind_port: 12345,
            dst_ip: "10.126.126.1".to_owned(),
            dst_port: 23456,
            proto: "tcp".to_owned(),
        }
    }
}

impl PortForward {
    fn bind_addr(&self) -> String {
        format!("{}:{}", self.bind_ip, self.bind_port)
    }

    fn dst_addr(&self) -> String {
        format!("{}:{}", self.dst_ip, self.dst_port)
    }
}

#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub instance_id: String,
    pub dhcp: bool,
    pub virtual_ipv4: String,
    pub network_length: u8,
    pub hostname: String,
    pub network_name: String,
    pub network_secret: String,
    pub credential_file: String,
    pub networking_method: u8,
    pub public_server_url: String,
    pub peer_urls: Vec<String>,
    pub proxy_cidrs: Vec<String>,
    pub enable_vpn_portal: bool,
    pub vpn_portal_listen_port: u16,
    pub vpn_portal_client_network_addr: String,
    pub vpn_portal_client_network_len: u8,
    pub listener_urls: Vec<String>,
    pub mapped_listeners: Vec<String>,
    pub latency_first: bool,
    pub use_smoltcp: Option<bool>,
    pub disable_ipv6: Option<bool>,
    pub ipv6_public_addr_auto: Option<bool>,
    pub enable_kcp_proxy: Option<bool>,
    pub disable_kcp_input: Option<bool>,
    pub enable_quic_proxy: Option<bool>,
    pub disable_quic_input: Option<bool>,
    pub disable_p2p: Option<bool>,
    pub p2p_only: Option<bool>,
    pub lazy_p2p: Option<bool>,
    pub bind_device: Option<bool>,
    pub no_tun: Option<bool>,
    pub enable_exit_node: Option<bool>,
    pub relay_all_peer_rpc: Option<bool>,
    pub need_p2p: Option<bool>,
    pub multi_thread: Option<bool>,
    pub proxy_forward_by_system: Option<bool>,
    pub disable_encryption: Option<bool>,
    pub disable_tcp_hole_punching: Option<bool>,
    pub disable_udp_hole_punching: Option<bool>,
    pub disable_upnp: Option<bool>,
    pub enable_udp_broadcast_relay: Option<bool>,
    pub disable_sym_hole_punching: Option<bool>,
    pub enable_relay_network_whitelist: Option<bool>,
    pub relay_network_whitelist: Vec<String>,
    pub enable_manual_routes: bool,
    pub routes: Vec<String>,
    pub exit_nodes: Vec<String>,
    pub enable_socks5: Option<bool>,
    pub socks5_port: u16,
    pub mtu: Option<u32>,
    pub instance_recv_bps_limit: Option<u64>,
    pub enable_magic_dns: Option<bool>,
    pub enable_private_mode: Option<bool>,
    pub dev_name: String,
    pub port_forwards: Vec<PortForward>,
    pub magic_dns_zone: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            instance_id: String::new(),
            dhcp: true,
            virtual_ipv4: String::new(),
            network_length: 24,
            hostname: "localhost".to_owned(),
            network_name: "easytier".to_owned(),
            network_secret: String::new(),
            credential_file: String::new(),
            networking_method: 1,
            public_server_url: String::new(),
            peer_urls: Vec::new(),
            proxy_cidrs: Vec::new(),
            enable_vpn_portal: false,
            vpn_portal_listen_port: 22022,
            vpn_portal_client_network_addr: String::new(),
            vpn_portal_client_network_len: 24,
            listener_urls: vec![
                "tcp://0.0.0.0:11010".to_owned(),
                "udp://0.0.0.0:11010".to_owned(),
                "wg://0.0.0.0:11011".to_owned(),
            ],
            mapped_listeners: Vec::new(),
            latency_first: false,
            use_smoltcp: None,
            disable_ipv6: None,
            ipv6_public_addr_auto: None,
            enable_kcp_proxy: None,
            disable_kcp_input: None,
            enable_quic_proxy: None,
            disable_quic_input: None,
            disable_p2p: None,
            p2p_only: None,
            lazy_p2p: None,
            bind_device: None,
            no_tun: None,
            enable_exit_node: None,
            relay_all_peer_rpc: None,
            need_p2p: None,
            multi_thread: None,
            proxy_forward_by_system: None,
            disable_encryption: None,
            disable_tcp_hole_punching: None,
            disable_udp_hole_punching: None,
            disable_upnp: None,
            enable_udp_broadcast_relay: None,
            disable_sym_hole_punching: None,
            enable_relay_network_whitelist: None,
            relay_network_whitelist: Vec::new(),
            enable_manual_routes: false,
            routes: Vec::new(),
            exit_nodes: Vec::new(),
            enable_socks5: None,
            socks5_port: 1080,
            mtu: None,
            instance_recv_bps_limit: None,
            enable_magic_dns: None,
            enable_private_mode: None,
            dev_name: String::new(),
            port_forwards: Vec::new(),
            magic_dns_zone: String::new(),
        }
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|v| quote(v)).collect();
    format!("[{}]", items.join(", "))
}

fn str_value(table: &toml::Table, key: &str) -> Option<String> {
    table.get(key).and_then(toml::Value::as_str).map(str::to_owned)
}

fn bool_value(table: &toml::Table, key: &str) -> Option<bool> {
    table.get(key).and_then(toml::Value::as_bool)
}

fn string_list(table: &toml::Table, key: &str) -> Option<Vec<String>> {
    table.get(key).and_then(toml::Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(toml::Value::as_str)
            .map(str::to_owned)
            .collect()
    })
}

fn table_array<'a>(table: &'a toml::Table, key: &str) -> &'a [toml::Value] {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn split_last_colon(text: &str) -> (String, u16) {
    match text.rsplit_once(':') {
        Some((head, tail)) => (head.to_owned(), tail.trim().parse().unwrap_or(0)),
        None => (text.to_owned(), 0),
    }
}

pub fn listener_port(url: &str) -> Option<u16> {
    url.rsplit_once(':')?.1.parse().ok()
}

pub fn decode_toml(text: &str) -> Result<NetworkConfig, String> {
    let doc: toml::Table = text
        .parse()
        .map_err(|e| format!("配置解析失败：{e}"))?;
    let mut cfg = NetworkConfig::default();

    if let Some(id) = str_value(&doc, "instance_id") {
        cfg.instance_id = id;
    }
    cfg.dhcp = bool_value(&doc, "dhcp").unwrap_or(true);
    if let Some(ipv4) = str_value(&doc, "ipv4") {
        if !ipv4.trim().is_empty() {
            if let Some((addr, len)) = ipv4.split_once('/') {
                cfg.virtual_ipv4 = addr.trim().to_owned();
                if let Ok(len) = len.trim().parse::<u8>() {
                    cfg.network_length = len;
                }
            } else {
                cfg.virtual_ipv4 = ipv4.trim().to_owned();
            }
            cfg.dhcp = false;
        }
    }
    cfg.ipv6_public_addr_auto = bool_value(&doc, "ipv6_public_addr_auto");
    if let Some(host) = str_value(&doc, "hostname") {
        cfg.hostname = host;
    }
    if let Some(list) = string_list(&doc, "listeners") {
        if !list.is_empty() {
            cfg.listener_urls = list;
        }
    }
    if let Some(list) = string_list(&doc, "mapped_listeners") {
        cfg.mapped_listeners = list;
    }
    if let Some(list) = string_list(&doc, "routes") {
        if !list.is_empty() {
            cfg.routes = list;
            cfg.enable_manual_routes = true;
        }
    }
    if let Some(list) = string_list(&doc, "exit_nodes") {
        cfg.exit_nodes = list;
    }
    if let Some(mtu) = doc.get("mtu").and_then(toml::Value::as_integer) {
        if mtu > 0 {
            cfg.mtu = Some(mtu as u32);
        }
    }
    if let Some(file) = str_value(&doc, "credential_file") {
        cfg.credential_file = file;
    }
    if let Some(socks) = str_value(&doc, "socks5_proxy") {
        if !socks.trim().is_empty() {
            cfg.enable_socks5 = Some(true);
            let port = socks
                .rsplit_once(':')
                .and_then(|(_, p)| p.trim().parse::<u16>().ok())
                .unwrap_or(1080);
            cfg.socks5_port = port;
        }
    }

    if let Some(identity) = doc.get("network_identity").and_then(toml::Value::as_table) {
        if let Some(name) = str_value(identity, "network_name") {
            if !name.is_empty() {
                cfg.network_name = name;
            }
        }
        if let Some(secret) = str_value(identity, "network_secret") {
            cfg.network_secret = secret;
        }
    } else if let Some(name) = str_value(&doc, "instance_name") {
        if !name.is_empty() {
            cfg.network_name = name;
        }
    }

    for entry in table_array(&doc, "peer") {
        if let Some(table) = entry.as_table() {
            if let Some(uri) = str_value(table, "uri") {
                if !uri.is_empty() {
                    cfg.peer_urls.push(uri);
                }
            }
        }
    }
    for entry in table_array(&doc, "proxy_network") {
        if let Some(table) = entry.as_table() {
            if let Some(cidr) = str_value(table, "cidr") {
                if !cidr.is_empty() {
                    cfg.proxy_cidrs.push(cidr);
                }
            }
        }
    }
    if let Some(vpn) = doc.get("vpn_portal_config").and_then(toml::Value::as_table) {
        cfg.enable_vpn_portal = true;
        if let Some(cidr) = str_value(vpn, "client_cidr") {
            if let Some((addr, len)) = cidr.split_once('/') {
                cfg.vpn_portal_client_network_addr = addr.to_owned();
                if let Ok(len) = len.trim().parse::<u8>() {
                    cfg.vpn_portal_client_network_len = len;
                }
            } else if !cidr.is_empty() {
                cfg.vpn_portal_client_network_addr = cidr;
            }
        }
        if let Some(addr) = str_value(vpn, "client_network_addr") {
            if !addr.is_empty() {
                cfg.vpn_portal_client_network_addr = addr;
            }
        }
        if let Some(len) = vpn.get("client_network_len").and_then(toml::Value::as_integer) {
            if len > 0 && len <= 32 {
                cfg.vpn_portal_client_network_len = len as u8;
            }
        }
        if let Some(listen) = str_value(vpn, "wireguard_listen") {
            if let Some((_, port)) = listen.rsplit_once(':') {
                if let Ok(port) = port.trim().parse::<u16>() {
                    cfg.vpn_portal_listen_port = port;
                }
            }
        }
    }
    for entry in table_array(&doc, "port_forward") {
        if let Some(table) = entry.as_table() {
            let (bind_ip, bind_port) = split_last_colon(&str_value(table, "bind_addr").unwrap_or_default());
            let (dst_ip, dst_port) = split_last_colon(&str_value(table, "dst_addr").unwrap_or_default());
            let proto = str_value(table, "proto").unwrap_or_else(|| "tcp".to_owned());
            cfg.port_forwards.push(PortForward {
                bind_ip,
                bind_port,
                dst_ip,
                dst_port,
                proto,
            });
        }
    }

    if let Some(flags) = doc.get("flags").and_then(toml::Value::as_table) {
        cfg.latency_first = bool_value(flags, "latency_first").unwrap_or(false);
        cfg.use_smoltcp = bool_value(flags, "use_smoltcp");
        cfg.disable_ipv6 = bool_value(flags, "enable_ipv6").map(|v| !v);
        cfg.enable_kcp_proxy = bool_value(flags, "enable_kcp_proxy");
        cfg.disable_kcp_input = bool_value(flags, "disable_kcp_input");
        cfg.enable_quic_proxy = bool_value(flags, "enable_quic_proxy");
        cfg.disable_quic_input = bool_value(flags, "disable_quic_input");
        cfg.disable_p2p = bool_value(flags, "disable_p2p");
        cfg.p2p_only = bool_value(flags, "p2p_only");
        cfg.lazy_p2p = bool_value(flags, "lazy_p2p");
        cfg.bind_device = bool_value(flags, "bind_device");
        cfg.no_tun = bool_value(flags, "no_tun");
        cfg.enable_exit_node = bool_value(flags, "enable_exit_node");
        cfg.relay_all_peer_rpc = bool_value(flags, "relay_all_peer_rpc");
        cfg.need_p2p = bool_value(flags, "need_p2p");
        cfg.multi_thread = bool_value(flags, "multi_thread");
        cfg.proxy_forward_by_system = bool_value(flags, "proxy_forward_by_system");
        cfg.disable_encryption = bool_value(flags, "enable_encryption").map(|v| !v);
        cfg.disable_tcp_hole_punching = bool_value(flags, "disable_tcp_hole_punching");
        cfg.disable_udp_hole_punching = bool_value(flags, "disable_udp_hole_punching");
        cfg.disable_upnp = bool_value(flags, "disable_upnp");
        cfg.enable_udp_broadcast_relay = bool_value(flags, "enable_udp_broadcast_relay");
        cfg.disable_sym_hole_punching = bool_value(flags, "disable_sym_hole_punching");
        cfg.enable_magic_dns = bool_value(flags, "accept_dns");
        cfg.enable_private_mode = bool_value(flags, "private_mode");
        if let Some(zone) = str_value(flags, "tld_dns_zone") {
            if !zone.is_empty() {
                cfg.magic_dns_zone = zone;
            }
        }
        if let Some(whitelist) = str_value(flags, "relay_network_whitelist") {
            if whitelist == "*" {
                cfg.enable_relay_network_whitelist = Some(false);
                cfg.relay_network_whitelist.clear();
            } else {
                cfg.enable_relay_network_whitelist = Some(true);
                cfg.relay_network_whitelist = whitelist
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
            }
        }
        if let Some(dev) = str_value(flags, "dev_name") {
            if !dev.is_empty() {
                cfg.dev_name = dev;
            }
        }
        if let Some(limit) = flags.get("instance_recv_bps_limit").and_then(toml::Value::as_integer) {
            if limit >= 0 {
                cfg.instance_recv_bps_limit = Some(limit as u64);
            }
        }
    }

    Ok(cfg)
}

fn fallback_dev_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(6)
        .collect();
    let safe = if safe.is_empty() {
        "default".to_owned()
    } else {
        safe
    };
    format!("et_{safe}")
}

/// Serializes the config in the same document layout as src/toml-codec.ts.
pub fn encode_toml(cfg: &NetworkConfig, include_instance_id: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    let instance_name = if cfg.network_name.trim().is_empty() {
        cfg.instance_id.clone()
    } else {
        cfg.network_name.clone()
    };
    lines.push(format!("instance_name = {}", quote(&instance_name)));
    if include_instance_id && !cfg.instance_id.is_empty() {
        lines.push(format!("instance_id = {}", quote(&cfg.instance_id)));
    }
    lines.push(format!("dhcp = {}", cfg.dhcp));
    if !cfg.dhcp && !cfg.virtual_ipv4.trim().is_empty() {
        lines.push(format!(
            "ipv4 = {}",
            quote(&format!(
                "{}/{}",
                cfg.virtual_ipv4.trim(),
                cfg.network_length
            ))
        ));
    }
    if cfg.ipv6_public_addr_auto == Some(true) {
        lines.push("ipv6_public_addr_auto = true".to_owned());
    }
    if !cfg.hostname.trim().is_empty() {
        lines.push(format!("hostname = {}", quote(cfg.hostname.trim())));
    }
    if !cfg.listener_urls.is_empty() {
        lines.push(format!("listeners = {}", array(&cfg.listener_urls)));
    }
    if !cfg.mapped_listeners.is_empty() {
        lines.push(format!("mapped_listeners = {}", array(&cfg.mapped_listeners)));
    }
    if cfg.enable_manual_routes && !cfg.routes.is_empty() {
        lines.push(format!("routes = {}", array(&cfg.routes)));
    }
    if !cfg.exit_nodes.is_empty() {
        lines.push(format!("exit_nodes = {}", array(&cfg.exit_nodes)));
    }
    if let Some(mtu) = cfg.mtu {
        lines.push(format!("mtu = {mtu}"));
    }
    if !cfg.credential_file.trim().is_empty() {
        lines.push(format!("credential_file = {}", quote(cfg.credential_file.trim())));
    }
    if cfg.enable_socks5 == Some(true) {
        lines.push(format!(
            "socks5_proxy = {}",
            quote(&format!("socks5://127.0.0.1:{}", cfg.socks5_port))
        ));
    }

    lines.push(String::new());
    lines.push("[network_identity]".to_owned());
    lines.push(format!("network_name = {}", quote(&cfg.network_name)));
    if !cfg.network_secret.is_empty() {
        lines.push(format!("network_secret = {}", quote(&cfg.network_secret)));
    }

    for uri in &cfg.peer_urls {
        if uri.trim().is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push("[[peer]]".to_owned());
        lines.push(format!("uri = {}", quote(uri.trim())));
    }
    for cidr in &cfg.proxy_cidrs {
        if cidr.trim().is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push("[[proxy_network]]".to_owned());
        lines.push(format!("cidr = {}", quote(cidr.trim())));
        lines.push("allow = [\"tcp\", \"udp\", \"icmp\"]".to_owned());
    }
    if cfg.enable_vpn_portal {
        lines.push(String::new());
        lines.push("[vpn_portal_config]".to_owned());
        if !cfg.vpn_portal_client_network_addr.trim().is_empty() {
            lines.push(format!(
                "client_cidr = {}",
                quote(&format!(
                    "{}/{}",
                    cfg.vpn_portal_client_network_addr.trim(),
                    cfg.vpn_portal_client_network_len
                ))
            ));
        }
        lines.push(format!(
            "wireguard_listen = {}",
            quote(&format!("0.0.0.0:{}", cfg.vpn_portal_listen_port))
        ));
    }
    for fwd in &cfg.port_forwards {
        if fwd.bind_ip.trim().is_empty() || fwd.dst_ip.trim().is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push("[[port_forward]]".to_owned());
        lines.push(format!("bind_addr = {}", quote(&fwd.bind_addr())));
        lines.push(format!("dst_addr = {}", quote(&fwd.dst_addr())));
        lines.push(format!("proto = {}", quote(&fwd.proto)));
    }

    lines.push(String::new());
    lines.push("[flags]".to_owned());
    lines.push(format!("latency_first = {}", cfg.latency_first));
    let mut push_opt = |key: &str, value: Option<bool>| {
        if let Some(v) = value {
            lines.push(format!("{key} = {v}"));
        }
    };
    push_opt("use_smoltcp", cfg.use_smoltcp);
    push_opt("enable_ipv6", cfg.disable_ipv6.map(|v| !v));
    push_opt("enable_kcp_proxy", cfg.enable_kcp_proxy);
    push_opt("disable_kcp_input", cfg.disable_kcp_input);
    push_opt("enable_quic_proxy", cfg.enable_quic_proxy);
    push_opt("disable_quic_input", cfg.disable_quic_input);
    push_opt("disable_p2p", cfg.disable_p2p);
    push_opt("p2p_only", cfg.p2p_only);
    push_opt("lazy_p2p", cfg.lazy_p2p);
    push_opt("bind_device", cfg.bind_device);
    push_opt("no_tun", cfg.no_tun);
    push_opt("enable_exit_node", cfg.enable_exit_node);
    push_opt("relay_all_peer_rpc", cfg.relay_all_peer_rpc);
    push_opt("need_p2p", cfg.need_p2p);
    push_opt("multi_thread", cfg.multi_thread);
    push_opt("proxy_forward_by_system", cfg.proxy_forward_by_system);
    push_opt("enable_encryption", cfg.disable_encryption.map(|v| !v));
    push_opt("disable_tcp_hole_punching", cfg.disable_tcp_hole_punching);
    push_opt("disable_udp_hole_punching", cfg.disable_udp_hole_punching);
    push_opt("disable_upnp", cfg.disable_upnp);
    push_opt(
        "enable_udp_broadcast_relay",
        cfg.enable_udp_broadcast_relay,
    );
    push_opt("disable_sym_hole_punching", cfg.disable_sym_hole_punching);
    push_opt("accept_dns", cfg.enable_magic_dns);
    push_opt("private_mode", cfg.enable_private_mode);
    drop(push_opt);
    if !cfg.magic_dns_zone.trim().is_empty() {
        lines.push(format!("tld_dns_zone = {}", quote(cfg.magic_dns_zone.trim())));
    }
    if cfg.enable_relay_network_whitelist == Some(true) && !cfg.relay_network_whitelist.is_empty()
    {
        lines.push(format!(
            "relay_network_whitelist = {}",
            quote(&cfg.relay_network_whitelist.join(" "))
        ));
    }
    let dev_name = if cfg.dev_name.trim().is_empty() {
        fallback_dev_name(&cfg.instance_id)
    } else {
        cfg.dev_name.trim().to_owned()
    };
    lines.push(format!("dev_name = {}", quote(&dev_name)));
    if let Some(limit) = cfg.instance_recv_bps_limit {
        lines.push(format!("instance_recv_bps_limit = {limit}"));
    }

    let mut text = lines.join("\n");
    text.push('\n');
    text
}

pub fn validate(cfg: &NetworkConfig) -> Result<(), String> {
    if cfg.network_name.trim().is_empty() {
        return Err("网络名称不能为空".to_owned());
    }
    if !cfg.dhcp {
        if cfg.virtual_ipv4.trim().is_empty() {
            return Err("手动模式下必须填写虚拟 IPv4 地址".to_owned());
        }
        let octets: Vec<&str> = cfg.virtual_ipv4.trim().split('.').collect();
        let valid = octets.len() == 4
            && octets.iter().all(|o| {
                !o.is_empty()
                    && o.len() <= 3
                    && o.chars().all(|c| c.is_ascii_digit())
                    && o.parse::<u16>().map(|v| v <= 255).unwrap_or(false)
            });
        if !valid {
            return Err("虚拟 IPv4 地址格式不正确".to_owned());
        }
        if cfg.network_length == 0 || cfg.network_length > 32 {
            return Err("子网前缀长度必须在 1-32 之间".to_owned());
        }
    }
    if cfg.enable_vpn_portal && cfg.vpn_portal_client_network_addr.trim().is_empty() {
        return Err("VPN 门户客户端网段地址不能为空".to_owned());
    }
    if cfg.enable_socks5 == Some(true) && (cfg.socks5_port == 0) {
        return Err("SOCKS5 端口必须在 1-65535 之间".to_owned());
    }
    if let Some(mtu) = cfg.mtu {
        if !(576..=9000).contains(&mtu) {
            return Err("MTU 必须在 576-9000 之间".to_owned());
        }
    }
    for fwd in &cfg.port_forwards {
        if fwd.bind_ip.trim().is_empty() || fwd.dst_ip.trim().is_empty() {
            return Err("端口转发必须填写完整的绑定与目标地址".to_owned());
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn parse_whitelist(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
