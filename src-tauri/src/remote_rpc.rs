//! Remote EasyTier node configuration management.
//!
//! Talks to a remote `easytier-core` RPC portal over the EasyTier virtual
//! network using the same `easytier` crate version as the bundled core, so
//! wire formats and instance-identifier selectors match without runtime
//! compat shims.
//!
//! Security model (mirrors easytier-macos `EasyTierGuiFFI`):
//! - only `tcp://<ip>:<port>` literals inside private/virtual ranges;
//! - an explicit service+method whitelist;
//! - per-endpoint and global concurrency limits, connect/call timeouts, and
//!   a cooldown after failed connects.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use easytier::proto::rpc_types::controller::Controller;
use easytier::proto::{
    api::{
        config::{ConfigRpc, ConfigRpcClientFactory, GetConfigRequest, PatchConfigRequest},
        instance::{
            InstanceIdentifier, ListPeerRequest, ListRouteRequest, PeerManageRpc,
            PeerManageRpcClientFactory, ShowNodeInfoRequest,
        },
    },
    rpc_impl::standalone::StandAloneClient,
    rpc_types::controller::BaseController,
};
use easytier::tunnel::tcp::TcpTunnelConnector;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const COOLDOWN: Duration = Duration::from_secs(30);
const RPC_MAX_CONCURRENT_PER_ENDPOINT: usize = 2;
const RPC_MAX_CONCURRENT_TOTAL: usize = 4;

static RPC_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("failed to build RPC runtime")
});

static RPC_TOTAL_LIMIT: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(RPC_MAX_CONCURRENT_TOTAL)));

struct RpcEndpoint {
    #[allow(dead_code)]
    url: String,
    limit: Arc<tokio::sync::Semaphore>,
    client: tokio::sync::Mutex<StandAloneClient<TcpTunnelConnector>>,
    state: Mutex<RpcEndpointState>,
}

#[derive(Default)]
struct RpcEndpointState {
    cooldown_until: Option<Instant>,
    last_error: Option<String>,
}

static RPC_CLIENTS: LazyLock<Mutex<HashMap<String, Arc<RpcEndpoint>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
enum RpcError {
    Validate(String),
    Unavailable(String),
    Busy(String),
    Timeout(String),
    Denied(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Validate(m) | RpcError::Unavailable(m) | RpcError::Busy(m)
            | RpcError::Timeout(m) | RpcError::Denied(m) => write!(f, "{m}"),
        }
    }
}

fn endpoint_for(host: &str, port: u16) -> Result<Arc<RpcEndpoint>, RpcError> {
    let url = validate_rpc_url(host, port)?;
    let client_id = format!("rpc-{host}-{port}");
    let mut clients = RPC_CLIENTS.lock().map_err(|_| RpcError::Busy("RPC endpoint table unavailable".into()))?;
    if let Some(ep) = clients.get(&client_id) {
        return Ok(Arc::clone(ep));
    }
    let connector = TcpTunnelConnector::new(url.clone());
    let endpoint = Arc::new(RpcEndpoint {
        url: url.to_string(),
        limit: Arc::new(tokio::sync::Semaphore::new(RPC_MAX_CONCURRENT_PER_ENDPOINT)),
        client: tokio::sync::Mutex::new(StandAloneClient::new(connector)),
        state: Mutex::new(RpcEndpointState::default()),
    });
    clients.insert(client_id, Arc::clone(&endpoint));
    Ok(endpoint)
}

fn validate_rpc_url(host: &str, port: u16) -> Result<url::Url, RpcError> {
    if port == 0 {
        return Err(RpcError::Validate("RPC URL must include a port".into()));
    }
    let ip: IpAddr = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .map_err(|_| RpcError::Validate("RPC URL host must be an IP address, not a domain name".into()))?;
    if !is_allowed_rpc_ip(ip) {
        return Err(RpcError::Validate(
            "RPC URL host must be a private, loopback, link-local, or EasyTier virtual IP".into(),
        ));
    }
    let s = match ip {
        IpAddr::V4(v4) => format!("tcp://{v4}:{port}"),
        IpAddr::V6(v6) => format!("tcp://[{v6}]:{port}"),
    };
    url::Url::parse(&s).map_err(|e| RpcError::Validate(format!("invalid RPC URL: {e}")))
}

fn is_allowed_rpc_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => is_allowed_ipv4(addr),
        IpAddr::V6(addr) => is_allowed_ipv6(addr),
    }
}

fn is_allowed_ipv4(addr: Ipv4Addr) -> bool {
    let o = addr.octets();
    if matches!(o, [192, 168, ..] | [127, ..] | [169, 254, ..]) {
        return true;
    }
    match o {
        [10, ..] => true,
        [172, second, ..] => (16..=31).contains(&second),
        [100, second, ..] => (64..=127).contains(&second),
        _ => false,
    }
}

fn is_allowed_ipv6(addr: Ipv6Addr) -> bool {
    addr.is_loopback() || (addr.segments()[0] & 0xfe00) == 0xfc00 || (addr.segments()[0] & 0xffc0) == 0xfe80
}

/// Only these service methods may be invoked on a remote node.
fn is_allowed_service_method(service: &str, method: &str) -> bool {
    match service {
        "api.config.ConfigRpcService" => matches!(method, "get_config" | "patch_config"),
        "api.instance.PeerManageRpcService" => matches!(method, "list_peer" | "list_route"),
        _ => false,
    }
}

async fn acquire_rpc_permit(semaphore: Arc<tokio::sync::Semaphore>, label: &str) -> Result<tokio::sync::OwnedSemaphorePermit, RpcError> {
    tokio::time::timeout(CALL_TIMEOUT, semaphore.acquire_owned())
        .await
        .map_err(|_| RpcError::Busy(format!("EasyTier RPC is busy waiting for {label} capacity. Try again shortly.")))?
        .map_err(|_| RpcError::Busy(format!("EasyTier RPC {label} limiter is closed.")))
}

async fn call_with_endpoint<T>(
    endpoint: &RpcEndpoint,
    endpoint_key: &str,
    label: &str,
    f: impl FnOnce(&mut StandAloneClient<TcpTunnelConnector>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send + '_>>,
) -> Result<T, RpcError> {
    {
        let state = endpoint.state.lock().map_err(|_| RpcError::Busy("endpoint state unavailable".into()))?;
        if let Some(until) = state.cooldown_until {
            if Instant::now() < until {
                return Err(RpcError::Unavailable(
                    state.last_error.clone().unwrap_or_else(|| "remote endpoint in cooldown".into()),
                ));
            }
        }
    }
    let _global = acquire_rpc_permit(Arc::clone(&RPC_TOTAL_LIMIT), "global RPC").await?;
    let _per_ep = acquire_rpc_permit(Arc::clone(&endpoint.limit), "endpoint RPC").await?;
    let mut guard = endpoint.client.lock().await;
    match tokio::time::timeout(CALL_TIMEOUT, f(&mut guard)).await {
        Ok(Ok(v)) => Ok(v),
        // A dead tunnel is not always reported by take_error(); drop the whole
        // cached endpoint so the next call rebuilds a fresh TCP + RPC session
        // instead of retrying on a stale connection forever.
        Ok(Err(e)) => {
            if let Ok(mut clients) = RPC_CLIENTS.lock() {
                clients.remove(endpoint_key);
            }
            Err(RpcError::Unavailable(format!("Remote EasyTier RPC failed: {e}")))
        }
        Err(_) => {
            if let Ok(mut clients) = RPC_CLIENTS.lock() {
                clients.remove(endpoint_key);
            }
            let message = format!("EasyTier RPC {label} timed out after {} seconds.", CALL_TIMEOUT.as_secs());
            Err(RpcError::Timeout(message))
        }
    }
}

/// RPC calls use the controller default of 5s; slow links over the VPN need
/// more headroom, so build a controller with a longer deadline.
fn rpc_controller() -> BaseController {
    let mut ctrl = BaseController::default();
    ctrl.set_timeout_ms(CALL_TIMEOUT.as_millis() as i32);
    ctrl
}

/// A portal that accepts TCP but never answers is almost always the remote
/// whitelist rejecting a non-loopback source (default allowlist is loopback
/// only). Translate the raw timeout into an actionable hint.
pub use crate::portal_args::build_rpc_portal_args;

// ---------------------------------------------------------------------------
// Local status queries (peer/route/node) over a persistent RPC connection.
// Replaces spawning three easytier-cli processes per status refresh: the
// endpoint pool keeps one TCP connection per instance and answers in
// milliseconds instead of ~100ms+ of process-creation overhead per call.
// Output JSON mirrors easytier-cli's `peer`/`route`/`node` shapes so the
// frontend parsing stays unchanged.
// ---------------------------------------------------------------------------

fn cost_to_str(cost: i32) -> String {
    if cost == 1 {
        "p2p".to_string()
    } else {
        format!("relay({cost})")
    }
}

/// Mirror humansize::DECIMAL ("1.83 kB") used by easytier-cli.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

fn pair_latency(p: &easytier::proto::api::instance::PeerRoutePair) -> Option<f64> {
    p.get_latency_ms()
}

fn peer_item(
    p: &easytier::proto::api::instance::PeerRoutePair,
) -> serde_json::Value {
    let route = p.route.clone().unwrap_or_default();
    let lat_ms = if route.cost == 1 {
        p.get_latency_ms().unwrap_or(0.0)
    } else {
        route.path_latency_latency_first() as f64
    };
    serde_json::json!({
        "cidr": route.ipv4_addr.as_ref().map(|ip| ip.to_string()).unwrap_or_default(),
        "ipv4": route.ipv4_addr.as_ref().and_then(|ip| ip.address.as_ref()).map(|a| std::net::Ipv4Addr::from(a.addr).to_string()).unwrap_or_default(),
        "hostname": route.hostname,
        "cost": cost_to_str(route.cost),
        "lat_ms": format!("{lat_ms:.2}"),
        "loss_rate": format!("{:.1}%", p.get_loss_rate().unwrap_or(0.0) * 100.0),
        "rx_bytes": human_size(p.get_rx_bytes().unwrap_or(0)),
        "tx_bytes": human_size(p.get_tx_bytes().unwrap_or(0)),
        "tunnel_proto": p.get_conn_protos().unwrap_or_default().join(","),
        "nat_type": p.get_udp_nat_type(),
        "id": route.peer_id.to_string(),
        "version": if route.version.is_empty() { "unknown".to_string() } else { route.version },
    })
}

fn route_item(
    p: &easytier::proto::api::instance::PeerRoutePair,
    peer_routes: &[easytier::proto::api::instance::PeerRoutePair],
    local: &easytier::proto::api::instance::NodeInfo,
) -> serde_json::Value {
    let route = p.route.clone().unwrap_or_default();
    let find_pair = |peer_id: u32| {
        peer_routes
            .iter()
            .find(|pair| pair.route.clone().unwrap_or_default().peer_id == peer_id)
    };
    let next_hop_pair = find_pair(route.next_hop_peer_id);
    let next_hop_lat_first = find_pair(route.next_hop_peer_id_latency_first.unwrap_or_default());

    let nh_ipv4 = |pair: Option<&easytier::proto::api::instance::PeerRoutePair>, direct: bool| -> String {
        if direct {
            "DIRECT".to_string()
        } else {
            pair.and_then(|x| x.route.as_ref())
                .and_then(|r| r.ipv4_addr.as_ref())
                .map(|ip| ip.to_string())
                .unwrap_or_default()
        }
    };
    let nh_host = |pair: Option<&easytier::proto::api::instance::PeerRoutePair>, direct: bool| -> String {
        if direct {
            "DIRECT".to_string()
        } else {
            pair.and_then(|x| x.route.as_ref()).map(|r| r.hostname.clone()).unwrap_or_default()
        }
    };

    serde_json::json!({
        "ipv4": route.ipv4_addr.as_ref().map(|ip| ip.to_string()).unwrap_or_default(),
        "hostname": route.hostname,
        "proxy_cidrs": route.proxy_cidrs.join(","),
        "next_hop_ipv4": nh_ipv4(next_hop_pair, route.cost == 1),
        "next_hop_hostname": nh_host(next_hop_pair, route.cost == 1),
        "next_hop_lat": next_hop_pair.map(|x| x.get_latency_ms().unwrap_or(0.0)).unwrap_or(0.0),
        "path_len": route.cost,
        "path_latency": route.path_latency,
        "next_hop_ipv4_lat_first": nh_ipv4(next_hop_lat_first, route.cost_latency_first.unwrap_or_default() == 1),
        "next_hop_hostname_lat_first": nh_host(next_hop_lat_first, route.cost_latency_first.unwrap_or_default() == 1),
        "path_len_lat_first": route.cost_latency_first.unwrap_or_default(),
        "path_latency_lat_first": route.path_latency_latency_first,
        "version": route.version,
        "_local": false,
        // local-node row (mirrors CLI's first RouteTableItem)
        "local_ipv4": local.ipv4_addr,
        "local_hostname": local.hostname,
        "local_proxy_cidrs": local.proxy_cidrs.join(", "),
        "local_version": local.version,
    })
}

/// Query one local instance's peer/route/node state over the persistent
/// connection and return CLI-compatible JSON.
pub async fn local_status_query(port: u16) -> Result<Value, String> {
    let endpoint = endpoint_for("127.0.0.1", port).map_err(|e| e.to_string())?;
    let result = call_with_endpoint(&endpoint, &format!("rpc-127.0.0.1-{port}"), "status", |client| {
        Box::pin(async move {
            let ctrl = rpc_controller();
            let node_stub = client
                .scoped_client::<PeerManageRpcClientFactory<BaseController>>("".to_string())
                .await
                .map_err(|e| format!("connect failed: {e:#}"))?;
            let node_info = node_stub
                .show_node_info(ctrl.clone(), ShowNodeInfoRequest { instance: None })
                .await
                .map_err(|e| format!("show_node_info failed: {e:#}"))?
                .node_info
                .ok_or_else(|| "node info unavailable".to_string())?;

            let peer_stub = client
                .scoped_client::<PeerManageRpcClientFactory<BaseController>>("".to_string())
                .await
                .map_err(|e| format!("connect failed: {e:#}"))?;
            let peers = peer_stub
                .list_peer(ctrl.clone(), ListPeerRequest { instance: None })
                .await
                .map_err(|e| format!("list_peer failed: {e:#}"))?
                .peer_infos;

            let route_stub = client
                .scoped_client::<PeerManageRpcClientFactory<BaseController>>("".to_string())
                .await
                .map_err(|e| format!("connect failed: {e:#}"))?;
            let routes = route_stub
                .list_route(ctrl, ListRouteRequest { instance: None })
                .await
                .map_err(|e| format!("list_route failed: {e:#}"))?
                .routes;

            Ok((node_info, peers, routes))
        })
    })
    .await;

    let (node_info, peers, routes) = match result {
        Ok(v) => v,
        Err(e) => {
            if matches!(e, RpcError::Unavailable(_) | RpcError::Timeout(_)) {
                if let Ok(mut clients) = RPC_CLIENTS.lock() {
                    clients.remove(&format!("rpc-127.0.0.1-{port}"));
                }
            }
            return Err(e.to_string());
        }
    };

    let pairs = easytier::proto::api::instance::list_peer_route_pair(peers, routes.clone());

    let mut peer_items: Vec<Value> = pairs.iter().map(peer_item).collect();
    // Local-node row, mirroring easytier-cli's PeerTableItem::from(NodeInfo).
    peer_items.insert(
        0,
        serde_json::json!({
            "cidr": node_info.ipv4_addr,
            "ipv4": node_info.ipv4_addr.split('/').next().unwrap_or_default(),
            "hostname": node_info.hostname,
            "cost": "Local",
            "lat_ms": "-",
            "loss_rate": "-",
            "rx_bytes": "-",
            "tx_bytes": "-",
            "tunnel_proto": "-",
            "nat_type": node_info.stun_info.as_ref().map(|s| {
                easytier::proto::common::NatType::try_from(s.udp_nat_type).map(|n| format!("{n:?}")).unwrap_or_else(|_| "Unknown".into())
            }).unwrap_or_else(|| "Unknown".into()),
            "id": node_info.peer_id.to_string(),
            "version": node_info.version,
        }),
    );

    let mut route_items: Vec<Value> = Vec::new();
    route_items.push(serde_json::json!({
        "ipv4": node_info.ipv4_addr,
        "hostname": node_info.hostname,
        "proxy_cidrs": node_info.proxy_cidrs.join(", "),
        "next_hop_ipv4": "-",
        "next_hop_hostname": "Local",
        "next_hop_lat": 0.0,
        "path_len": 0,
        "path_latency": 0,
        "next_hop_ipv4_lat_first": "-",
        "next_hop_hostname_lat_first": "Local",
        "path_len_lat_first": 0,
        "path_latency_lat_first": 0,
        "version": node_info.version,
    }));
    for p in &pairs {
        if p.route.as_ref().map(|r| r.cost).unwrap_or_default() == 0 && p.route.as_ref().map(|r| r.peer_id).unwrap_or_default() == node_info.peer_id {
            continue; // local route row already inserted
        }
        route_items.push(route_item(p, &pairs, &node_info));
    }

    let stun = node_info.stun_info.as_ref();
    Ok(serde_json::json!({
        "peers": peer_items,
        "routes": route_items,
        "node": {
            "peer_id": node_info.peer_id,
            "ipv4_addr": node_info.ipv4_addr,
            "hostname": node_info.hostname,
            "version": node_info.version,
            "stun_info": stun.map(|s| serde_json::json!({
                "udp_nat_type": s.udp_nat_type,
                "public_ip": s.public_ip,
            })),
            "listeners": node_info.listeners,
            "proxy_cidrs": node_info.proxy_cidrs,
        },
    }))
}

fn explain_timeout(method: &str, e: &easytier::proto::rpc_types::error::Error) -> String {
    let raw = format!("{e:#}");
    if raw.contains("Timeout") || raw.contains("deadline") {
        format!(
            "{method} failed: {raw}。已连上对端但对端未应答：请在对端设备的设置里开启「允许远程管理」并确认其白名单包含本机的虚拟 IP，然后在对端重启网络"
        )
    } else {
        format!("{method} failed: {raw}")
    }
}

fn set_connect_cooldown(endpoint: &RpcEndpoint, message: String) {
    if let Ok(mut state) = endpoint.state.lock() {
        state.cooldown_until = Some(Instant::now() + COOLDOWN);
        state.last_error = Some(message);
    }
}

fn instance_identifier(instance_id: &str) -> Result<InstanceIdentifier, RpcError> {
    let uuid = uuid::Uuid::parse_str(instance_id)
        .map_err(|_| RpcError::Validate(format!("invalid remote instance id: {instance_id}")))?;
    Ok(InstanceIdentifier {
        selector: Some(easytier::proto::api::instance::instance_identifier::Selector::Id(
            easytier::proto::common::Uuid::from(uuid),
        )),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteInstanceInfo {
    pub instance_id: String,
    pub hostname: String,
    pub virtual_ip: String,
}

/// Discover the remote instance UUID by matching a peer's virtual IP in its
/// peer/route listing (same data source as macOS `peer_route_pairs`).
pub async fn discover_remote_instance(host: &str, port: u16, virtual_ip: &str) -> Result<RemoteInstanceInfo, String> {
    let endpoint = endpoint_for(host, port).map_err(|e| e.to_string())?;
    let ip = virtual_ip
        .split('/')
        .next()
        .unwrap_or(virtual_ip)
        .trim()
        .to_string();

    let result = call_with_endpoint(&endpoint, &format!("rpc-{host}-{port}"), "discover", |client| {
        let ip = ip.clone();
        Box::pin(async move {
            let peers = tokio::time::timeout(CALL_TIMEOUT, async {
                client
                    .scoped_client::<PeerManageRpcClientFactory<BaseController>>("".to_string())
                    .await
                    .map_err(|e| format!("connect failed: {e:#}"))?
                    .list_peer(rpc_controller(), ListPeerRequest { instance: None })
                    .await
                    .map_err(|e| explain_timeout("list_peer", &e))
            })
            .await
            .map_err(|_| "discover timed out".to_string())??;

            let routes = tokio::time::timeout(CALL_TIMEOUT, async {
                client
                    .scoped_client::<PeerManageRpcClientFactory<BaseController>>("".to_string())
                    .await
                    .map_err(|e| format!("connect failed: {e:#}"))?
                    .list_route(rpc_controller(), ListRouteRequest { instance: None })
                    .await
                    .map_err(|e| format!("list_route failed: {e:#}"))
            })
            .await
            .map_err(|_| "discover timed out".to_string())??;

            let peers = peers.peer_infos;
            let routes = routes.routes;
            let pairs = easytier::proto::api::instance::list_peer_route_pair(peers, routes);            for p in pairs {
                let Some(route) = p.route else { continue };
                let route_ip = route
                    .ipv4_addr
                    .as_ref()
                    .and_then(|i| i.address.as_ref())
                    .map(|a| std::net::Ipv4Addr::from(a.addr).to_string())
                    .unwrap_or_default();
                if route_ip == ip {
                    let hostname = route.hostname.clone();
                    let inst = route.inst_id.clone();
                    if inst.is_empty() {
                        return Err("该节点未上报实例 ID（可能版本过旧或未开启配置管理）".to_string());
                    }
                    return Ok(RemoteInstanceInfo { instance_id: inst, hostname, virtual_ip: ip });
                }
            }
            Err(format!("未在远端成员列表中找到虚拟 IP {ip} 的实例"))
        })
    })
    .await;

    match result {
        Ok(info) => Ok(info),
        Err(RpcError::Unavailable(m)) | Err(RpcError::Validate(m)) | Err(RpcError::Busy(m)) | Err(RpcError::Timeout(m)) | Err(RpcError::Denied(m)) => {
            set_connect_cooldown(&endpoint, m.clone());
            Err(m)
        }
    }
}

/// Fetch the remote instance's full config (`get_config`).
pub async fn load_remote_config(host: &str, port: u16, instance_id: &str) -> Result<Value, String> {
    let endpoint = endpoint_for(host, port).map_err(|e| e.to_string())?;
    let ident = instance_identifier(instance_id).map_err(|e| e.to_string())?;
    let result = call_with_endpoint(&endpoint, &format!("rpc-{host}-{port}"), "get_config", |client| {
        let ident = ident.clone();
        Box::pin(async move {
            let stub = client
                .scoped_client::<ConfigRpcClientFactory<BaseController>>("".to_string())
                .await
                .map_err(|e| format!("connect failed: {e:#}"))?;
            let response = stub
                .get_config(rpc_controller(), GetConfigRequest { instance: Some(ident) })
                .await
                .map_err(|e| format!("get_config failed: {e:#}"))?;
            serde_json::to_value(response).map_err(|e| format!("failed to encode config: {e}"))
        })
    })
    .await;
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            if matches!(e, RpcError::Unavailable(_)) {
                set_connect_cooldown(&endpoint, e.to_string());
            }
            Err(e.to_string())
        }
    }
}

/// Allowed top-level keys inside an `InstanceConfigPatch`.
const ALLOWED_PATCH_KEYS: &[&str] = &[
    "hostname",
    "ipv4",
    "port_forwards",
    "proxy_networks",
    "routes",
    "exit_nodes",
    "mapped_listeners",
    "connectors",
    "ipv6_public_addr_auto",
];

/// Validate the patch object from the frontend against the allowed key set.
fn validate_patch(patch: &Value) -> Result<(), RpcError> {
    let Some(obj) = patch.as_object() else {
        return Err(RpcError::Denied("patch must be a JSON object".into()));
    };
    for key in obj.keys() {
        if !ALLOWED_PATCH_KEYS.contains(&key.as_str()) {
            return Err(RpcError::Denied(format!("patch field is not remotely editable: {key}")));
        }
    }
    Ok(())
}

/// Apply a config patch to the remote instance (`patch_config`).
pub async fn patch_remote_config(host: &str, port: u16, instance_id: &str, patch: Value) -> Result<Value, String> {
    let endpoint = endpoint_for(host, port).map_err(|e| e.to_string())?;
    let patch_value = patch;
    validate_patch(&patch_value).map_err(|e| e.to_string())?;
    let ident = instance_identifier(instance_id).map_err(|e| e.to_string())?;
    // The frontend sends a JSON object keyed by InstanceConfigPatch field
    // names; convert it via the type's serde impl so unknown/typo'd keys fail
    // here instead of silently reaching the remote node.
    let patch_typed: easytier::proto::api::config::InstanceConfigPatch =
        serde_json::from_value(patch_value).map_err(|e| format!("invalid patch payload: {e}"))?;
    let result = call_with_endpoint(&endpoint, &format!("rpc-{host}-{port}"), "patch_config", |client| {
        let ident = ident.clone();
        let patch_typed = patch_typed.clone();
        Box::pin(async move {
            let stub = client
                .scoped_client::<ConfigRpcClientFactory<BaseController>>("".to_string())
                .await
                .map_err(|e| format!("connect failed: {e:#}"))?;
            let response = stub
                .patch_config(
                                        rpc_controller(),
                    PatchConfigRequest { patch: Some(patch_typed), instance: Some(ident) },
                )
                .await
                .map_err(|e| format!("patch_config failed: {e:#}"))?;
            serde_json::to_value(response).map_err(|e| format!("failed to encode response: {e}"))
        })
    })
    .await;
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            if matches!(e, RpcError::Unavailable(_)) {
                set_connect_cooldown(&endpoint, e.to_string());
            }
            Err(e.to_string())
        }
    }
}

pub fn is_allowed_service_method_test_hook(service: &str, method: &str) -> bool {
    is_allowed_service_method(service, method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_private_and_virtual_ips() {
        for ip in ["10.0.0.1", "172.16.0.1", "172.31.255.255", "192.168.1.1", "127.0.0.1", "169.254.1.1", "100.64.0.1", "100.127.255.255"] {
            assert!(is_allowed_rpc_ip(ip.parse().unwrap()), "{ip} should be allowed");
        }
        assert!(is_allowed_rpc_ip("fc00::1".parse().unwrap()));
        assert!(is_allowed_rpc_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn rejects_public_ips_and_looking_ones() {
        for ip in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "1.1.1.1"] {
            assert!(!is_allowed_rpc_ip(ip.parse().unwrap()), "{ip} should be rejected");
        }
        assert!(!is_allowed_rpc_ip("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn validate_rpc_url_accepts_ip_literal() {
        assert!(validate_rpc_url("10.144.144.10", 15888).is_ok());
        assert!(validate_rpc_url("127.0.0.1", 15888).is_ok());
    }

    #[test]
    fn validate_rpc_url_rejects_domain_and_bad_port() {
        let e = validate_rpc_url("example.com", 15888);
        assert!(matches!(e, Err(RpcError::Validate(_))));
        let e = validate_rpc_url("10.0.0.1", 0);
        assert!(matches!(e, Err(RpcError::Validate(_))));
    }

    #[test]
    fn service_whitelist_is_explicit() {
        assert!(is_allowed_service_method("api.config.ConfigRpcService", "get_config"));
        assert!(is_allowed_service_method("api.config.ConfigRpcService", "patch_config"));
        assert!(is_allowed_service_method("api.instance.PeerManageRpcService", "list_peer"));
        assert!(is_allowed_service_method("api.instance.PeerManageRpcService", "list_route"));
        assert!(!is_allowed_service_method("api.instance.PeerManageRpcService", "list_peer_route_pair"));
        assert!(!is_allowed_service_method("api.config.ConfigRpcService", "GetConfig"));
        assert!(!is_allowed_service_method("api.config.ConfigRpcService", "delete_everything"));
    }

    #[test]
    fn patch_key_whitelist_rejects_unknown_fields() {
        assert!(validate_patch(&serde_json::json!({"hostname": "a"})).is_ok());
        assert!(validate_patch(&serde_json::json!({})).is_ok());
        assert!(validate_patch(&serde_json::json!({"listeners": ["tcp://1.2.3.4:1"]})).is_err());
        assert!(validate_patch(&serde_json::json!("not-an-object")).is_err());
    }

    #[test]
    fn instance_identifier_requires_uuid() {
        assert!(instance_identifier("300497152").is_err());
        assert!(instance_identifier("").is_err());
        assert!(instance_identifier("f7c2f0e4-9c1d-4a8b-b3f0-6f5d9a1b2c3d").is_ok());
    }



}
