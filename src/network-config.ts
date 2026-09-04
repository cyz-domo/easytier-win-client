// NetworkConfig model — mirrors Sources/EasyTierShared/Models/NetworkModels.swift
// in the macOS project, and the TOML document layout of NetworkConfigTOMLCodec.swift.

export type NetworkingMethod = 0 | 1 | 2; // 0 publicServer, 1 manual, 2 standalone

export interface PortForwardConfig {
  id: string;
  bind_ip: string;
  bind_port: number;
  dst_ip: string;
  dst_port: number;
  proto: string;
}

export interface NetworkConfig {
  instance_id: string;
  dhcp: boolean;
  virtual_ipv4: string;
  network_length: number;
  hostname: string | null;
  network_name: string;
  network_secret: string | null;
  credential_file: string | null;
  networking_method: NetworkingMethod;
  public_server_url: string;
  peer_urls: string[];
  proxy_cidrs: string[];
  enable_vpn_portal: boolean;
  vpn_portal_listen_port: number;
  vpn_portal_client_network_addr: string;
  vpn_portal_client_network_len: number;
  advanced_settings: boolean;
  listener_urls: string[];
  latency_first: boolean;
  dev_name: string;
  use_smoltcp: boolean | null;
  disable_ipv6: boolean | null;
  ipv6_public_addr_auto: boolean | null;
  enable_kcp_proxy: boolean | null;
  disable_kcp_input: boolean | null;
  enable_quic_proxy: boolean | null;
  disable_quic_input: boolean | null;
  disable_p2p: boolean | null;
  p2p_only: boolean | null;
  lazy_p2p: boolean | null;
  bind_device: boolean | null;
  no_tun: boolean | null;
  enable_exit_node: boolean | null;
  relay_all_peer_rpc: boolean | null;
  need_p2p: boolean | null;
  multi_thread: boolean | null;
  proxy_forward_by_system: boolean | null;
  disable_encryption: boolean | null;
  disable_tcp_hole_punching: boolean | null;
  disable_udp_hole_punching: boolean | null;
  disable_upnp: boolean | null;
  enable_udp_broadcast_relay: boolean | null;
  disable_sym_hole_punching: boolean | null;
  enable_relay_network_whitelist: boolean | null;
  relay_network_whitelist: string[];
  enable_manual_routes: boolean;
  routes: string[];
  exit_nodes: string[];
  enable_socks5: boolean | null;
  socks5_port: number;
  mtu: number | null;
  instance_recv_bps_limit: number | null;
  mapped_listeners: string[];
  enable_magic_dns: boolean | null;
  enable_private_mode: boolean | null;
  port_forwards: PortForwardConfig[];
}

export const LISTENER_SUGGESTIONS = [
  'tcp://0.0.0.0:11010',
  'udp://0.0.0.0:11010',
  'wg://0.0.0.0:11011',
  'ws://0.0.0.0:11011',
  'wss://0.0.0.0:11012',
  'quic://0.0.0.0:11012',
  'faketcp://0.0.0.0:11013',
];

export function defaultConfig(): NetworkConfig {
  return {
    instance_id: crypto.randomUUID(),
    dhcp: true,
    virtual_ipv4: '',
    network_length: 24,
    hostname: 'localhost',
    network_name: 'easytier',
    network_secret: '',
    credential_file: '',
    networking_method: 1,
    public_server_url: '',
    peer_urls: [],
    proxy_cidrs: [],
    enable_vpn_portal: false,
    vpn_portal_listen_port: 22022,
    vpn_portal_client_network_addr: '',
    vpn_portal_client_network_len: 24,
    advanced_settings: false,
    listener_urls: ['tcp://0.0.0.0:11010', 'udp://0.0.0.0:11010', 'wg://0.0.0.0:11011'],
    latency_first: false,
    dev_name: '',
    use_smoltcp: null,
    disable_ipv6: null,
    ipv6_public_addr_auto: null,
    enable_kcp_proxy: null,
    disable_kcp_input: null,
    enable_quic_proxy: null,
    disable_quic_input: null,
    disable_p2p: null,
    p2p_only: null,
    lazy_p2p: null,
    bind_device: null,
    no_tun: null,
    enable_exit_node: null,
    relay_all_peer_rpc: null,
    need_p2p: null,
    multi_thread: null,
    proxy_forward_by_system: null,
    disable_encryption: null,
    disable_tcp_hole_punching: null,
    disable_udp_hole_punching: null,
    disable_upnp: null,
    enable_udp_broadcast_relay: null,
    disable_sym_hole_punching: null,
    enable_relay_network_whitelist: null,
    relay_network_whitelist: [],
    enable_manual_routes: false,
    routes: [],
    exit_nodes: [],
    enable_socks5: null,
    socks5_port: 1080,
    mtu: null,
    instance_recv_bps_limit: null,
    mapped_listeners: [],
    enable_magic_dns: null,
    enable_private_mode: null,
    port_forwards: [],
  };
}

export interface ValidationError {
  field: string;
  message: string;
}

export function validateConfig(config: NetworkConfig): ValidationError[] {
  const errors: ValidationError[] = [];
  if (!config.network_name.trim()) errors.push({ field: 'network_name', message: '网络名称不能为空' });
  if (!config.dhcp) {
    if (!config.virtual_ipv4.trim()) {
      errors.push({ field: 'virtual_ipv4', message: '手动模式下必须填写虚拟 IPv4 地址' });
    } else if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(config.virtual_ipv4)) {
      errors.push({ field: 'virtual_ipv4', message: '虚拟 IPv4 地址格式不正确' });
    }
    if (config.network_length < 1 || config.network_length > 32) {
      errors.push({ field: 'network_length', message: '子网前缀长度必须在 1-32 之间' });
    }
  }
  if (config.networking_method === 0 && !config.public_server_url.trim()) {
    errors.push({ field: 'public_server_url', message: '公共服务器模式下必须填写服务器地址' });
  }
  if (config.enable_vpn_portal) {
    if (!config.vpn_portal_client_network_addr.trim()) {
      errors.push({ field: 'vpn_portal_client_network_addr', message: 'VPN 门户客户端网段地址不能为空' });
    }
  }
  if (config.enable_socks5 === true) {
    if (config.socks5_port < 1 || config.socks5_port > 65535) {
      errors.push({ field: 'socks5_port', message: 'SOCKS5 端口必须在 1-65535 之间' });
    }
  }
  if (config.mtu != null && (config.mtu < 576 || config.mtu > 9000)) {
    errors.push({ field: 'mtu', message: 'MTU 必须在 576-9000 之间' });
  }
  for (const f of config.port_forwards) {
    if (!f.bind_ip || !f.dst_ip) {
      errors.push({ field: 'port_forwards', message: '端口转发必须填写完整的绑定与目标地址' });
    }
  }
  return errors;
}
