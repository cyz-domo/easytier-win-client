// Status panel data — parsed from `easytier-cli --output json peer` output.
// Field names match the actual CLI JSON: id, ipv4, cidr, hostname, cost,
// lat_ms, loss_rate, rx_bytes, tx_bytes, tunnel_proto, nat_type, version.

export interface PeerInfo {
  id?: string;
  ipv4?: string;
  cidr?: string;
  hostname?: string;
  cost?: string;
  lat_ms?: string;
  loss_rate?: string;
  rx_bytes?: string;
  tx_bytes?: string;
  tunnel_proto?: string;
  nat_type?: string;
  version?: string;
  [key: string]: unknown;
}

export interface RouteInfo {
  ipv4?: string;
  hostname?: string;
  proxy_cidrs?: string | string[];
  next_hop_ipv4?: string;
  next_hop_hostname?: string;
  next_hop_lat?: number;
  path_len?: number;
  path_latency?: number;
  version?: string;
  [key: string]: unknown;
}

export interface NodeStatus {
  peer_id?: number;
  ipv4_addr?: string;
  hostname?: string;
  version?: string;
  stun_info?: { udp_nat_type?: number; public_ip?: string[]; [k: string]: unknown };
  listeners?: string[];
  running?: boolean;
  [key: string]: unknown;
}

export type PeerColumn =
  | 'ipv4' | 'cidr' | 'hostname' | 'cost' | 'proto' | 'latency'
  | 'loss' | 'rx' | 'tx' | 'nat' | 'version' | 'relay' | 'routes';

export const PEER_COLUMNS: { key: PeerColumn; label: string; defaultOn: boolean }[] = [
  { key: 'ipv4', label: 'IPv4', defaultOn: true },
  { key: 'cidr', label: '网段', defaultOn: false },
  { key: 'hostname', label: '主机名', defaultOn: true },
  { key: 'cost', label: '穿透方式', defaultOn: true },
  { key: 'proto', label: '协议', defaultOn: true },
  { key: 'latency', label: '延迟', defaultOn: true },
  { key: 'loss', label: '丢包率', defaultOn: false },
  { key: 'rx', label: '下载', defaultOn: true },
  { key: 'tx', label: '上传', defaultOn: true },
  { key: 'nat', label: 'Nat类型', defaultOn: false },
  { key: 'version', label: '内核版本', defaultOn: false },
  { key: 'relay', label: '中继节点', defaultOn: false },
  { key: 'routes', label: '子网路由', defaultOn: false },
];

export type RefreshInterval = 1 | 3 | 5 | 10 | 30;

function unwrap(x: unknown): unknown {
  if (x == null) return null;
  if (Array.isArray(x) && x.length === 1) return x[0];
  return x;
}

export function parsePeerJSON(text: string): PeerInfo[] {
  const data = unwrap(JSON.parse(text));
  const list = Array.isArray(data) ? data : ((data as Record<string, unknown>)?.peers);
  return Array.isArray(list) ? (list as PeerInfo[]) : [];
}

export function parseRouteJSON(text: string): RouteInfo[] {
  const data = unwrap(JSON.parse(text));
  const list = Array.isArray(data) ? data : ((data as Record<string, unknown>)?.routes);
  return Array.isArray(list) ? (list as RouteInfo[]) : [];
}

export function parseNodeJSON(text: string): NodeStatus | null {
  const data = unwrap(JSON.parse(text));
  return data && typeof data === 'object' ? (data as NodeStatus) : null;
}

export function formatBytes(n: number | string | undefined): string {
  if (n == null || n === '-') return '—';
  if (typeof n === 'string' && /[KMG]/i.test(n)) return n; // already human-formatted
  const v = typeof n === 'string' ? parseFloat(n) : n;
  if (Number.isNaN(v)) return String(n);
  if (v < 1024) return `${v} B`;
  if (v < 1024 ** 2) return `${(v / 1024).toFixed(1)} KB`;
  if (v < 1024 ** 3) return `${(v / 1024 ** 2).toFixed(1)} MB`;
  return `${(v / 1024 ** 3).toFixed(2)} GB`;
}

