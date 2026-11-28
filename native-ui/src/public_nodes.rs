use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const STATUS_URL: &str = "https://ruixuan.online/uptime/api/status-page/easytier";
const HEARTBEAT_URL: &str = "https://ruixuan.online/uptime/api/status-page/heartbeat/easytier";

#[derive(Clone, Debug)]
pub struct PublicNode {
    pub name: String,
    pub address: String,
    pub overseas: bool,
    pub online: bool,
    pub local_ping: Option<i64>,
    pub uptime_pct: Option<f64>,
    pub can_relay: bool,
    pub masked: bool,
    pub description: String,
    pub official: bool,
}

pub fn fetch() -> Result<Vec<PublicNode>, String> {
    let mut nodes = Vec::new();
    let official = PublicNode {
        name: "官方默认公共节点 (EasyTier Official)".to_owned(),
        address: "tcp://public.easytier.top:11010".to_owned(),
        overseas: false,
        online: true,
        local_ping: None,
        uptime_pct: None,
        can_relay: true,
        masked: false,
        description: "EasyTier 官方推荐公共服务器 / 稳定高可用".to_owned(),
        official: true,
    };
    nodes.push(official);

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(6))
        .build();
    let status = agent
        .get(STATUS_URL)
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok());
    if let Some(status) = status {
        let heartbeat = agent
            .get(HEARTBEAT_URL)
            .call()
            .ok()
            .and_then(|r| r.into_json::<serde_json::Value>().ok())
            .unwrap_or(serde_json::Value::Null);
        if let Some(groups) = status.get("publicGroupList").and_then(|g| g.as_array()) {
            for group in groups {
                let group_name = group.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if !group_name.contains("公共节点") {
                    continue;
                }
                let overseas = group_name.contains("海外");
                let Some(monitors) = group.get("monitorList").and_then(|m| m.as_array()) else {
                    continue;
                };
                for mon in monitors {
                    let id = mon.get("id").and_then(|i| i.as_i64()).unwrap_or(0);
                    let raw_name = mon.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let Some(address) = extract_node_address(raw_name) else {
                        continue;
                    };
                    let masked = address.contains('*');
                    let can_relay = !raw_name.contains("禁中转");
                    let mut description = String::new();
                    if let Some((_, after_open)) =
                        raw_name.split_once('（').or_else(|| raw_name.split_once('('))
                    {
                        if let Some((inner, _)) = after_open
                            .rsplit_once('）')
                            .or_else(|| after_open.rsplit_once(')'))
                        {
                            description = inner.trim().to_owned();
                        }
                    }
                    if description.is_empty() {
                        description = raw_name.replace(&address, "").trim().to_owned();
                    }

                    let mut online = false;
                    let mut monitor_ping = None;
                    let id_str = id.to_string();
                    if let Some(hb_list) = heartbeat
                        .get("heartbeatList")
                        .and_then(|h| h.get(&id_str))
                        .and_then(|l| l.as_array())
                    {
                        if let Some(latest) = hb_list.last() {
                            online = latest.get("status").and_then(|s| s.as_i64()) == Some(1);
                            monitor_ping = latest.get("ping").and_then(|p| p.as_i64());
                        }
                    }
                    let uptime_key = format!("{}_24", id);
                    let uptime_pct = heartbeat
                        .get("uptimeList")
                        .and_then(|u| u.get(&uptime_key))
                        .and_then(|v| v.as_f64())
                        .map(|val| (val * 1000.0).round() / 10.0);

                    nodes.push(PublicNode {
                        name: raw_name.to_owned(),
                        address,
                        overseas,
                        online,
                        local_ping: monitor_ping,
                        uptime_pct,
                        can_relay,
                        masked,
                        description,
                        official: false,
                    });
                }
            }
        }
    }

    if nodes.len() == 1 {
        nodes.extend(fallback_nodes());
    }

    batch_ping(&mut nodes);
    let mut seen = std::collections::HashSet::new();
    nodes.retain(|n| seen.insert(n.address.clone()));
    sort_nodes(&mut nodes);
    Ok(nodes)
}

fn batch_ping(nodes: &mut [PublicNode]) {
    let mut handles = Vec::new();
    for (idx, node) in nodes.iter().enumerate() {
        if node.masked || node.address.contains('*') || node.address.starts_with("txt://") {
            continue;
        }
        let Some((host, port)) = parse_host_port(&node.address) else {
            continue;
        };
        handles.push(std::thread::spawn(move || {
            let ping = tcp_ping(&host, port, Duration::from_millis(1500));
            (idx, ping)
        }));
    }
    for h in handles {
        if let Ok((idx, ping)) = h.join() {
            if let Some(node) = nodes.get_mut(idx) {
                node.local_ping = ping.or(node.local_ping);
            }
        }
    }
}

fn sort_nodes(nodes: &mut [PublicNode]) {
    nodes.sort_by(|a, b| {
        b.official
            .cmp(&a.official)
            .then(a.masked.cmp(&b.masked))
            .then(b.online.cmp(&a.online))
            .then_with(|| {
                let pa = a.local_ping.filter(|&ms| ms >= 2).unwrap_or(9999);
                let pb = b.local_ping.filter(|&ms| ms >= 2).unwrap_or(9999);
                pa.cmp(&pb)
            })
    });
}

fn parse_host_port(address: &str) -> Option<(String, u16)> {
    let (scheme, after_scheme) = address.split_once("://")?;
    let default_port = match scheme {
        "https" | "wss" => 443,
        "http" | "ws" => 80,
        _ => 11010,
    };
    let clean_after = after_scheme.trim_end_matches('/');
    if let Some((host, port_part)) = clean_after.rsplit_once(':') {
        let p_str: String = port_part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = p_str.parse::<u16>() {
            return Some((host.trim_matches(|c| c == '[' || c == ']').to_owned(), port));
        }
    }
    Some((
        clean_after.trim_matches(|c| c == '[' || c == ']').to_owned(),
        default_port,
    ))
}

fn is_fake_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            (o[0] == 198 && (o[1] == 18 || o[1] == 19)) || o[0] == 127 || o[0] == 0
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn tcp_ping(host: &str, port: u16, timeout: Duration) -> Option<i64> {
    let addrs: Vec<_> = format!("{host}:{port}").to_socket_addrs().ok()?.collect();
    if addrs.is_empty() {
        return None;
    }
    let start = Instant::now();
    for addr in addrs {
        if is_fake_ip(&addr.ip()) {
            return None;
        }
        if let Ok(stream) = TcpStream::connect_timeout(&addr, timeout) {
            drop(stream);
            let elapsed = start.elapsed().as_millis() as i64;
            return if elapsed < 2 { None } else { Some(elapsed) };
        }
    }
    None
}

fn extract_node_address(raw_name: &str) -> Option<String> {
    let (before_scheme, after_scheme) = raw_name.split_once("://")?;
    let mut scheme_chars: Vec<char> = Vec::new();
    for c in before_scheme.chars().rev() {
        if c.is_ascii_alphanumeric() || c == '_' {
            scheme_chars.push(c);
        } else {
            break;
        }
    }
    if scheme_chars.is_empty() {
        return None;
    }
    scheme_chars.reverse();
    let scheme: String = scheme_chars.into_iter().collect();

    let mut host_port = String::new();
    if after_scheme.starts_with('[') {
        if let Some((v6, rest)) = after_scheme.split_once(']') {
            host_port.push_str(v6);
            host_port.push(']');
            for c in rest.chars() {
                if c == ':' || c.is_ascii_digit() {
                    host_port.push(c);
                } else {
                    break;
                }
            }
        }
    } else {
        for c in after_scheme.chars() {
            if c.is_whitespace()
                || ['（', '(', '[', '【', '，', ',', ')', '）', ']', '】'].contains(&c)
            {
                break;
            }
            host_port.push(c);
        }
    }
    let cleaned = host_port.trim_end_matches('/');
    if cleaned.is_empty() {
        None
    } else {
        Some(format!("{}://{}", scheme, cleaned))
    }
}

fn fallback_nodes() -> Vec<PublicNode> {
    let items = [
        ("上海电信公共节点", "tcp://225284.xyz:11010", false, "海波：上海电信 / 支持中转"),
        ("厦门电信公共节点", "tcp://easytier.weiai.org.cn:11010", false, "为爱唯爱：厦门电信 / 支持中转"),
        ("枣庄BGP公共节点", "udp://et.basd1.de:11010", false, "宇智波.希：枣庄BGP跨网 / 支持中转"),
        ("重庆移动专线节点", "tcp://183.230.36.171:11010", false, "重庆移动跨网专线 / 支持中转"),
        ("美国公共节点", "udp://us01.225284.xyz:11010", true, "海波：美国西海岸 / 支持中转"),
        ("美国优质节点", "tcp://107.172.5.203:11010", true, "罐头：美国 / 支持中转"),
    ];
    items
        .iter()
        .map(|(name, address, overseas, description)| PublicNode {
            name: (*name).to_owned(),
            address: (*address).to_owned(),
            overseas: *overseas,
            online: true,
            local_ping: None,
            uptime_pct: None,
            can_relay: true,
            masked: false,
            description: (*description).to_owned(),
            official: false,
        })
        .collect()
}
