// TOML codec for NetworkConfig — mirrors NetworkConfigTOMLCodec.swift from the
// macOS project. Encoding uses the EasyTierTOMLDocument layout (instance_name,
// network_identity, peer[], proxy_network[], vpn_portal_config, port_forward[],
// flags) so configs are interchangeable between the two clients.

import { defaultConfig, NetworkConfig, PortForwardConfig } from './network-config';

interface FlagsTOML {
  latency_first?: boolean;
  use_smoltcp?: boolean;
  enable_ipv6?: boolean;
  enable_kcp_proxy?: boolean;
  disable_kcp_input?: boolean;
  enable_quic_proxy?: boolean;
  disable_quic_input?: boolean;
  disable_p2p?: boolean;
  p2p_only?: boolean;
  lazy_p2p?: boolean;
  bind_device?: boolean;
  no_tun?: boolean;
  enable_exit_node?: boolean;
  relay_all_peer_rpc?: boolean;
  need_p2p?: boolean;
  multi_thread?: boolean;
  proxy_forward_by_system?: boolean;
  enable_encryption?: boolean;
  disable_tcp_hole_punching?: boolean;
  disable_udp_hole_punching?: boolean;
  disable_upnp?: boolean;
  enable_udp_broadcast_relay?: boolean;
  disable_sym_hole_punching?: boolean;
  accept_dns?: boolean;
  private_mode?: boolean;
  tld_dns_zone?: string;
  relay_network_whitelist?: string;
  dev_name?: string;
  instance_recv_bps_limit?: number;
}

interface DocumentTOML {
  instance_name?: string;
  instance_id?: string;
  dhcp?: boolean;
  ipv4?: string;
  ipv6_public_addr_auto?: boolean;
  hostname?: string;
  listeners?: string[];
  mapped_listeners?: string[];
  routes?: string[];
  exit_nodes?: string[];
  mtu?: number;
  credential_file?: string;
  socks5_proxy?: string;
  network_identity?: { network_name?: string; network_secret?: string };
  peer?: { uri: string }[];
  proxy_network?: { cidr: string; mapped_cidr?: string; allow?: string[] }[];
  vpn_portal_config?: { client_cidr?: string; wireguard_listen?: string; client_network_addr?: string; client_network_len?: number };
  port_forward?: { bind_addr: string; dst_addr: string; proto: string }[];
  flags?: FlagsTOML;
}

const T = (s: string) => '"' + s.replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';
const A = (items: string[]) => '[' + items.map(T).join(', ') + ']';

function flagsOf(c: NetworkConfig): FlagsTOML {
  const f: FlagsTOML = {};
  f.latency_first = c.latency_first;
  if (c.use_smoltcp != null) f.use_smoltcp = c.use_smoltcp;
  if (c.disable_ipv6 != null) f.enable_ipv6 = !c.disable_ipv6;
  if (c.enable_kcp_proxy != null) f.enable_kcp_proxy = c.enable_kcp_proxy;
  if (c.disable_kcp_input != null) f.disable_kcp_input = c.disable_kcp_input;
  if (c.enable_quic_proxy != null) f.enable_quic_proxy = c.enable_quic_proxy;
  if (c.disable_quic_input != null) f.disable_quic_input = c.disable_quic_input;
  if (c.disable_p2p != null) f.disable_p2p = c.disable_p2p;
  if (c.p2p_only != null) f.p2p_only = c.p2p_only;
  if (c.lazy_p2p != null) f.lazy_p2p = c.lazy_p2p;
  if (c.bind_device != null) f.bind_device = c.bind_device;
  if (c.no_tun != null) f.no_tun = c.no_tun;
  if (c.enable_exit_node != null) f.enable_exit_node = c.enable_exit_node;
  if (c.relay_all_peer_rpc != null) f.relay_all_peer_rpc = c.relay_all_peer_rpc;
  if (c.need_p2p != null) f.need_p2p = c.need_p2p;
  if (c.multi_thread != null) f.multi_thread = c.multi_thread;
  if (c.proxy_forward_by_system != null) f.proxy_forward_by_system = c.proxy_forward_by_system;
  if (c.disable_encryption != null) f.enable_encryption = !c.disable_encryption;
  if (c.disable_tcp_hole_punching != null) f.disable_tcp_hole_punching = c.disable_tcp_hole_punching;
  if (c.disable_udp_hole_punching != null) f.disable_udp_hole_punching = c.disable_udp_hole_punching;
  if (c.disable_upnp != null) f.disable_upnp = c.disable_upnp;
  if (c.enable_udp_broadcast_relay != null) f.enable_udp_broadcast_relay = c.enable_udp_broadcast_relay;
  if (c.disable_sym_hole_punching != null) f.disable_sym_hole_punching = c.disable_sym_hole_punching;
  if (c.enable_magic_dns != null) f.accept_dns = c.enable_magic_dns;
  if (c.enable_private_mode != null) f.private_mode = c.enable_private_mode;
  if (c.enable_relay_network_whitelist === true && c.relay_network_whitelist.length > 0) {
    f.relay_network_whitelist = c.relay_network_whitelist.join(' ');
  }
  if (c.dev_name.trim()) f.dev_name = c.dev_name;
  if (c.instance_recv_bps_limit != null) f.instance_recv_bps_limit = c.instance_recv_bps_limit;
  return f;
}

export function encodeTOML(c: NetworkConfig, exportSecrets = true): string {
  const d: DocumentTOML = {};
  d.instance_name = c.network_name || c.instance_id;
  d.instance_id = c.instance_id;
  d.dhcp = c.dhcp;
  if (!c.dhcp && c.virtual_ipv4) d.ipv4 = `${c.virtual_ipv4}/${c.network_length}`;
  if (c.ipv6_public_addr_auto === true) d.ipv6_public_addr_auto = true;
  if (c.hostname) d.hostname = c.hostname;
  if (c.listener_urls?.length) d.listeners = c.listener_urls;
  if (c.mapped_listeners?.length) d.mapped_listeners = c.mapped_listeners;
  if (c.enable_manual_routes && c.routes?.length) d.routes = c.routes;
  if (c.exit_nodes?.length) d.exit_nodes = c.exit_nodes;
  if (c.mtu != null) d.mtu = c.mtu;
  if (c.credential_file) d.credential_file = c.credential_file;
  if (c.enable_socks5 === true) d.socks5_proxy = `socks5://127.0.0.1:${c.socks5_port}`;
  d.network_identity = { network_name: c.network_name, network_secret: exportSecrets ? (c.network_secret ?? '') : undefined };
  if (c.peer_urls.length) d.peer = c.peer_urls.map(uri => ({ uri }));
  if (c.proxy_cidrs.length) d.proxy_network = c.proxy_cidrs.map(cidr => ({ cidr, allow: ['tcp', 'udp', 'icmp'] }));
  if (c.enable_vpn_portal) {
    d.vpn_portal_config = {
      client_cidr: `${c.vpn_portal_client_network_addr}/${c.vpn_portal_client_network_len}`,
      wireguard_listen: `0.0.0.0:${c.vpn_portal_listen_port}`,
    };
  }
  if (c.port_forwards.length) {
    d.port_forward = c.port_forwards.map(f => ({
      bind_addr: `${f.bind_ip}:${f.bind_port}`,
      dst_addr: `${f.dst_ip}:${f.dst_port}`,
      proto: f.proto,
    }));
  }
  d.flags = flagsOf(c);

  const lines: string[] = [];
  const emit = (key: string, value: string | undefined | null) => {
    if (value == null || value === 'undefined') return;
    lines.push(`${key} = ${value}`);
  };
  emit('instance_name', d.instance_name ? T(d.instance_name) : null);
  emit('instance_id', d.instance_id ? T(d.instance_id) : null);
  if (d.dhcp != null) emit('dhcp', String(d.dhcp));
  emit('ipv4', d.ipv4 ? T(d.ipv4) : null);
  if (d.ipv6_public_addr_auto) emit('ipv6_public_addr_auto', 'true');
  emit('hostname', d.hostname ? T(d.hostname) : null);
  if (d.listeners) emit('listeners', A(d.listeners));
  if (d.mapped_listeners) emit('mapped_listeners', A(d.mapped_listeners));
  if (d.routes) emit('routes', A(d.routes));
  if (d.exit_nodes) emit('exit_nodes', A(d.exit_nodes));
  if (d.mtu != null) emit('mtu', String(d.mtu));
  emit('credential_file', d.credential_file ? T(d.credential_file) : null);
  emit('socks5_proxy', d.socks5_proxy ? T(d.socks5_proxy) : null);
  if (d.network_identity) {
    lines.push('');
    lines.push('[network_identity]');
    emit('network_name', d.network_identity.network_name ? T(d.network_identity.network_name) : null);
    if (d.network_identity.network_secret != null && d.network_identity.network_secret !== '') {
      emit('network_secret', T(d.network_identity.network_secret));
    }
  }
  if (d.peer) {
    for (const p of d.peer) {
      lines.push('');
      lines.push('[[peer]]');
      emit('uri', T(p.uri));
    }
  }
  if (d.proxy_network) {
    for (const p of d.proxy_network) {
      lines.push('');
      lines.push('[[proxy_network]]');
      emit('cidr', T(p.cidr));
      if (p.allow) emit('allow', A(p.allow));
    }
  }
  if (d.vpn_portal_config) {
    lines.push('');
    lines.push('[vpn_portal_config]');
    emit('client_cidr', d.vpn_portal_config.client_cidr ? T(d.vpn_portal_config.client_cidr) : null);
    emit('wireguard_listen', d.vpn_portal_config.wireguard_listen ? T(d.vpn_portal_config.wireguard_listen) : null);
  }
  if (d.port_forward) {
    for (const p of d.port_forward) {
      lines.push('');
      lines.push('[[port_forward]]');
      emit('bind_addr', T(p.bind_addr));
      emit('dst_addr', T(p.dst_addr));
      emit('proto', T(p.proto));
    }
  }
  if (d.flags) {
    lines.push('');
    lines.push('[flags]');
    const f = d.flags;
    const boolKeys: (keyof FlagsTOML)[] = [
      'latency_first', 'use_smoltcp', 'enable_ipv6', 'enable_kcp_proxy', 'disable_kcp_input',
      'enable_quic_proxy', 'disable_quic_input', 'disable_p2p', 'p2p_only', 'lazy_p2p',
      'bind_device', 'no_tun', 'enable_exit_node', 'relay_all_peer_rpc', 'need_p2p',
      'multi_thread', 'proxy_forward_by_system', 'enable_encryption', 'disable_tcp_hole_punching',
      'disable_udp_hole_punching', 'disable_upnp', 'enable_udp_broadcast_relay',
      'disable_sym_hole_punching', 'accept_dns', 'private_mode',
    ];
    for (const k of boolKeys) {
      const v = f[k];
      if (typeof v === 'boolean') lines.push(`${k} = ${v}`);
    }
    if (f.tld_dns_zone) lines.push(`tld_dns_zone = ${T(f.tld_dns_zone)}`);
    if (f.relay_network_whitelist) lines.push(`relay_network_whitelist = ${T(f.relay_network_whitelist)}`);
    if (f.dev_name) lines.push(`dev_name = ${T(f.dev_name)}`);
    if (f.instance_recv_bps_limit != null) lines.push(`instance_recv_bps_limit = ${f.instance_recv_bps_limit}`);
  }
  return lines.join('\n') + '\n';
}

function parseSectionBlock(lines: string[], start: number): { body: string[]; end: number } {
  const body: string[] = [];
  let i = start + 1;
  while (i < lines.length && !/^\s*\[\[?[^[\]]+\]\]?\s*$/.test(lines[i])) {
    if (lines[i].trim()) body.push(lines[i].trim());
    i += 1;
  }
  return { body, end: i - 1 };
}

function scalar(raw: string): string {
  const m = raw.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/);
  return m ? m[2].trim() : '';
}

function str(raw: string): string | null {
  const v = scalar(raw);
  const m = v.match(/^"(.*)"$/);
  return m ? m[1] : v || null;
}

function bool(raw: string): boolean | null {
  const v = scalar(raw);
  if (v === 'true') return true;
  if (v === 'false') return false;
  return null;
}

function num(raw: string): number | null {
  const v = scalar(raw);
  const n = parseInt(v, 10);
  return Number.isNaN(n) ? null : n;
}

function strArray(raw: string): string[] {
  const v = scalar(raw);
  const inner = v.replace(/^\[|\]$/g, '');
  if (!inner.trim()) return [];
  const out: string[] = [];
  const re = /"((?:[^"\\]|\\.)*)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(inner))) out.push(m[1].replace(/\\"/g, '"').replace(/\\\\/g, '\\'));
  return out;
}

export function decodeTOML(text: string): NetworkConfig {
  const lines = text.split(/\r?\n/);
  const root: string[] = [];
  const sections: Record<string, string[]> = {};
  const arraySections: Record<string, string[][]> = {};
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const arraySection = line.match(/^\s*\[\[([^\]]+)\]\]\s*$/);
    if (arraySection) {
      const { body, end } = parseSectionBlock(lines, i);
      (arraySections[arraySection[1].trim()] ||= []).push(body);
      i = end + 1;
      continue;
    }
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      const { body, end } = parseSectionBlock(lines, i);
      sections[section[1].trim()] = (sections[section[1].trim()] || []).concat(body);
      i = end + 1;
      continue;
    }
    if (line.trim()) root.push(line.trim());
    i += 1;
  }

  const errors: string[] = [];
  const cfg: Partial<NetworkConfig> & Record<string, unknown> = {};
  const find = (arr: string[], key: string) => arr.find(l => l.startsWith(key + ' ') || l.startsWith(key + '='));

  cfg.instance_id = str(find(root, 'instance_id') || '') || crypto.randomUUID();
  const dhcpRaw = bool(find(root, 'dhcp') || '');
  cfg.dhcp = dhcpRaw ?? true;
  const hasIpv4Key = root.some(l => /^ipv4(\s|=)/.test(l));
  const ipv4 = hasIpv4Key ? (str(find(root, 'ipv4') || '') || '') : '';
  if (ipv4) {
    const parts = ipv4.split('/');
    cfg.virtual_ipv4 = parts[0];
    if (parts[1]) {
      const len = parseInt(parts[1], 10);
      if (Number.isNaN(len)) errors.push('ipv4 前缀必须是数字');
      else cfg.network_length = len;
    }
    cfg.dhcp = false;
  } else if (hasIpv4Key && !ipv4) {
    // `ipv4 = ""` present but empty: fall back to DHCP like the macOS codec.
    cfg.dhcp = true;
  }
  cfg.hostname = str(find(root, 'hostname') || '');
  const listeners = strArray(find(root, 'listeners') || '');
  if (listeners.length) cfg.listener_urls = listeners;
  const mapped = strArray(find(root, 'mapped_listeners') || '');
  if (mapped.length) cfg.mapped_listeners = mapped;
  const routes = strArray(find(root, 'routes') || '');
  if (routes.length) {
    cfg.routes = routes;
    cfg.enable_manual_routes = true;
  }
  const exitNodes = strArray(find(root, 'exit_nodes') || '');
  if (exitNodes.length) cfg.exit_nodes = exitNodes;
  const mtu = num(find(root, 'mtu') || '');
  if (mtu != null) cfg.mtu = mtu;
  const credFile = str(find(root, 'credential_file') || '');
  if (credFile) cfg.credential_file = credFile;
  const socks5 = str(find(root, 'socks5_proxy') || '');
  if (socks5) {
    const port = parseInt(socks5.split(':').pop() || '', 10);
    if (Number.isNaN(port)) errors.push('socks5_proxy 必须包含有效端口');
    else {
      cfg.enable_socks5 = true;
      cfg.socks5_port = port;
    }
  }
  const ipv6Auto = bool(find(root, 'ipv6_public_addr_auto') || '');
  if (ipv6Auto != null) cfg.ipv6_public_addr_auto = ipv6Auto;

  const identity = sections['network_identity'];
  if (identity) {
    const name = str(find(identity, 'network_name') || '');
    if (name) cfg.network_name = name;
    const secret = str(find(identity, 'network_secret') || '');
    if (secret != null) cfg.network_secret = secret;
  }
  const peers = arraySections['peer'] || [];
  if (peers.length) {
    cfg.peer_urls = peers.map(b => str(find(b, 'uri') || '') || '').filter(Boolean);
  }
  const proxies = arraySections['proxy_network'] || [];
  if (proxies.length) {
    cfg.proxy_cidrs = proxies.map(b => str(find(b, 'cidr') || '') || '').filter(Boolean);
  }
  const vpn = sections['vpn_portal_config'];
  if (vpn) {
    cfg.enable_vpn_portal = true;
    const cidr = str(find(vpn, 'client_cidr') || '');
    if (cidr) {
      const parts = cidr.split('/');
      cfg.vpn_portal_client_network_addr = parts[0];
      if (parts[1]) {
        const len = parseInt(parts[1], 10);
        if (!Number.isNaN(len)) cfg.vpn_portal_client_network_len = len;
      }
    }
    const addr = str(find(vpn, 'client_network_addr') || '');
    if (addr) cfg.vpn_portal_client_network_addr = addr;
    const len = num(find(vpn, 'client_network_len') || '');
    if (len != null) cfg.vpn_portal_client_network_len = len;
    const listen = str(find(vpn, 'wireguard_listen') || '');
    if (listen) {
      const port = parseInt(listen.split(':').pop() || '', 10);
      if (!Number.isNaN(port)) cfg.vpn_portal_listen_port = port;
    }
  }
  const forwards = arraySections['port_forward'] || [];
  cfg.port_forwards = forwards.map(b => {
    const bind = str(find(b, 'bind_addr') || '') || '';
    const dst = str(find(b, 'dst_addr') || '') || '';
    const parse = (v: string) => {
      const idx = v.lastIndexOf(':');
      return { ip: idx > 0 ? v.slice(0, idx) : v, port: parseInt(v.slice(idx + 1), 10) || 0 };
    };
    const bi = parse(bind);
    const di = parse(dst);
    const fw: PortForwardConfig = { id: crypto.randomUUID(), bind_ip: bi.ip, bind_port: bi.port, dst_ip: di.ip, dst_port: di.port, proto: str(find(b, 'proto') || '') || 'tcp' };
    return fw;
  });

  const flags = sections['flags'];
  if (flags) {
    const b = (k: string) => bool(find(flags, k) || '');
    if (b('latency_first') != null) cfg.latency_first = b('latency_first')!;
    if (b('use_smoltcp') != null) cfg.use_smoltcp = b('use_smoltcp');
    if (b('enable_ipv6') != null) cfg.disable_ipv6 = !b('enable_ipv6');
    if (b('enable_kcp_proxy') != null) cfg.enable_kcp_proxy = b('enable_kcp_proxy');
    if (b('disable_kcp_input') != null) cfg.disable_kcp_input = b('disable_kcp_input');
    if (b('enable_quic_proxy') != null) cfg.enable_quic_proxy = b('enable_quic_proxy');
    if (b('disable_quic_input') != null) cfg.disable_quic_input = b('disable_quic_input');
    if (b('disable_p2p') != null) cfg.disable_p2p = b('disable_p2p');
    if (b('p2p_only') != null) cfg.p2p_only = b('p2p_only');
    if (b('lazy_p2p') != null) cfg.lazy_p2p = b('lazy_p2p');
    if (b('bind_device') != null) cfg.bind_device = b('bind_device');
    if (b('no_tun') != null) cfg.no_tun = b('no_tun');
    if (b('enable_exit_node') != null) cfg.enable_exit_node = b('enable_exit_node');
    if (b('relay_all_peer_rpc') != null) cfg.relay_all_peer_rpc = b('relay_all_peer_rpc');
    if (b('need_p2p') != null) cfg.need_p2p = b('need_p2p');
    if (b('multi_thread') != null) cfg.multi_thread = b('multi_thread');
    if (b('proxy_forward_by_system') != null) cfg.proxy_forward_by_system = b('proxy_forward_by_system');
    if (b('enable_encryption') != null) cfg.disable_encryption = !b('enable_encryption');
    if (b('disable_tcp_hole_punching') != null) cfg.disable_tcp_hole_punching = b('disable_tcp_hole_punching');
    if (b('disable_udp_hole_punching') != null) cfg.disable_udp_hole_punching = b('disable_udp_hole_punching');
    if (b('disable_upnp') != null) cfg.disable_upnp = b('disable_upnp');
    if (b('enable_udp_broadcast_relay') != null) cfg.enable_udp_broadcast_relay = b('enable_udp_broadcast_relay');
    if (b('disable_sym_hole_punching') != null) cfg.disable_sym_hole_punching = b('disable_sym_hole_punching');
    if (b('accept_dns') != null) cfg.enable_magic_dns = b('accept_dns');
    if (b('private_mode') != null) cfg.enable_private_mode = b('private_mode');
    const zone = str(find(flags, 'tld_dns_zone') || '');
    if (zone) cfg.magic_dns_zone = zone;
    const whitelist = str(find(flags, 'relay_network_whitelist') || '');
    if (whitelist != null) {
      cfg.enable_relay_network_whitelist = whitelist !== '*';
      cfg.relay_network_whitelist = whitelist === '*' ? [] : whitelist.split(' ').filter(Boolean);
    }
    const dev = str(find(flags, 'dev_name') || '');
    if (dev) cfg.dev_name = dev;
    const limit = num(find(flags, 'instance_recv_bps_limit') || '');
    if (limit != null) cfg.instance_recv_bps_limit = limit;
  }

  if (errors.length) throw new Error(errors.join('；'));
  // Merge over a full default config: fields absent from the TOML document
  // must not be left undefined, or the advanced editor crashes on them.
  return { ...defaultConfig(), ...cfg } as NetworkConfig;
}
