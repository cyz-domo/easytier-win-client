use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

const OWNER: &str = "EasyTier";
const REPO: &str = "EasyTier";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "EasyTierNative";
const PROBE_REQUEST: &[u8] =
    b"CONNECT api.github.com:443 HTTP/1.1\r\nHost: api.github.com:443\r\n\r\n";

#[derive(Clone, Debug, Default)]
pub struct ReleaseInfo {
    pub latest: Option<String>,
    pub versions: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct LocalProxy {
    pub addr: String,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct Autostart {
    pub enabled: bool,
    pub path: String,
    pub minimized: bool,
}

const COMMON_PROXY_PORTS: &[(u16, &str)] = &[
    (7890, "Clash / Mihomo"),
    (7897, "Clash Verge"),
    (10808, "v2rayN 混合端口"),
    (10809, "v2rayN HTTP"),
    (1087, "v2rayN"),
    (3128, "Squid"),
    (8080, "HTTP 代理"),
];

pub fn check_releases(proxy: &str) -> Result<ReleaseInfo, String> {
    let raw = format!("https://api.github.com/repos/{OWNER}/{REPO}/releases?per_page=25");
    let url = build_url(proxy, &raw);
    let agent = build_agent(proxy)?;
    let body = agent
        .get(&url)
        .set("User-Agent", "easytier-native-client")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("请求 GitHub Release 失败：{e}"))?
        .into_string()
        .map_err(|e| format!("读取 Release 响应失败：{e}"))?;
    let releases: Vec<serde_json::Value> =
        serde_json::from_str(&body).map_err(|e| format!("解析 Release 列表失败：{e}"))?;
    let stem = "easytier-windows-x86_64";
    let mut info = ReleaseInfo::default();
    for release in &releases {
        if release.get("draft").and_then(serde_json::Value::as_bool) == Some(true) {
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let ver = tag.trim_start_matches('v');
        let expected = format!("{stem}-v{ver}.zip");
        let has_asset = release
            .get("assets")
            .and_then(serde_json::Value::as_array)
            .map(|assets| {
                assets.iter().any(|a| {
                    a.get("name").and_then(serde_json::Value::as_str) == Some(expected.as_str())
                })
            })
            .unwrap_or(false);
        if !has_asset {
            continue;
        }
        let prerelease =
            release.get("prerelease").and_then(serde_json::Value::as_bool) == Some(true);
        if info.latest.is_none() && !prerelease {
            info.latest = Some(tag.to_owned());
        }
        info.versions.push(tag.to_owned());
    }
    Ok(info)
}

fn build_url(proxy: &str, raw: &str) -> String {
    let proxy = proxy.trim().trim_end_matches('/');
    if proxy.is_empty() || proxy == "direct" || proxy.starts_with("local:") {
        raw.to_owned()
    } else {
        format!("{proxy}/{raw}")
    }
}

fn build_agent(proxy: &str) -> Result<ureq::Agent, String> {
    let mut builder = ureq::AgentBuilder::new().timeout(Duration::from_secs(20));
    if let Some(addr) = proxy.trim().strip_prefix("local:") {
        let addr = addr.trim();
        if !addr.is_empty() {
            let proxy = ureq::Proxy::new(addr).map_err(|e| format!("本地代理地址无效：{e}"))?;
            builder = builder.proxy(proxy);
        }
    }
    Ok(builder.build())
}

pub fn detect_local_proxies() -> Vec<LocalProxy> {
    let mut found: Vec<LocalProxy> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(addr) = system_proxy() {
        if probe(&addr) && seen.insert(addr.clone()) {
            found.push(LocalProxy {
                addr,
                label: "系统代理".to_owned(),
            });
        }
    }
    for (port, label) in COMMON_PROXY_PORTS {
        let addr = format!("127.0.0.1:{port}");
        if seen.contains(&addr) {
            continue;
        }
        if probe(&addr) {
            seen.insert(addr.clone());
            found.push(LocalProxy {
                addr,
                label: (*label).to_owned(),
            });
        }
    }
    found
}

fn system_proxy() -> Option<String> {
    let out = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyServer",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("ProxyServer") else {
            continue;
        };
        let Some(idx) = rest.find("REG_SZ") else {
            continue;
        };
        let value = rest[idx + "REG_SZ".len()..].trim();
        let candidate = value
            .split(';')
            .find_map(|part| part.trim().strip_prefix("http="))
            .or_else(|| value.split(';').next())
            .map(str::trim)
            .unwrap_or("");
        let candidate = candidate.trim_start_matches("http://").to_owned();
        if !candidate.is_empty() {
            return Some(candidate);
        }
    }
    None
}

fn probe(addr: &str) -> bool {
    let Ok(mut targets) = addr.to_socket_addrs() else {
        return false;
    };
    let Some(target) = targets.next() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&target, Duration::from_millis(300)) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    if stream.write_all(PROBE_REQUEST).is_err() {
        return false;
    }
    let mut buf = [0u8; 16];
    match stream.read(&mut buf) {
        Ok(n) if n >= 12 => buf.starts_with(b"HTTP/1."),
        _ => false,
    }
}

pub fn read_autostart() -> Autostart {
    let mut state = Autostart::default();
    let Ok(out) = Command::new("reg")
        .args(["query", RUN_KEY, "/v", VALUE_NAME])
        .output()
    else {
        return state;
    };
    if !out.status.success() {
        return state;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix(VALUE_NAME) else {
            continue;
        };
        let Some(idx) = rest.find("REG_SZ") else {
            continue;
        };
        let value = rest[idx + "REG_SZ".len()..].trim();
        state.enabled = true;
        state.minimized = value.to_lowercase().contains("--minimized");
        state.path = if let Some(rest) = value.strip_prefix('"') {
            rest.split('"').next().unwrap_or("").to_owned()
        } else {
            value.split_whitespace().next().unwrap_or("").to_owned()
        };
        break;
    }
    state
}

pub fn write_autostart(enabled: bool, minimized: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe().map_err(|e| format!("获取程序路径失败：{e}"))?;
        let value = if minimized {
            format!("\"{}\" --minimized", exe.display())
        } else {
            format!("\"{}\"", exe.display())
        };
        let out = Command::new("reg")
            .args([
                "add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", value.as_str(), "/f",
            ])
            .output()
            .map_err(|e| format!("写入自启动失败：{e}"))?;
        if !out.status.success() {
            return Err(format!(
                "写入自启动失败：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    } else {
        let _ = Command::new("reg")
            .args(["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])
            .output();
    }
    Ok(())
}
