// Status panel data — parsed from `easytier-cli --output json` output.

export interface PeerInfo {
  peer_id?: string | number;
  ipv4?: string;
  hostname?: string;
  cost?: string;
  latency_ms?: number | string;
  loss_rate?: number | string;
  rx_bytes?: number | string;
  tx_bytes?: number | string;
  tunnel_proto?: string;
  nat_type?: string;
  version?: string;
  [key: string]: unknown;
}

export interface RouteInfo {
  peer_id?: string | number;
  ipv4_addr?: string;
  next_hop_peer_id?: string | number;
  next_hop_ipv4?: string;
  cost?: number | string;
  proxy_cidrs?: string[];
  [key: string]: unknown;
}

export interface NodeStatus {
  peer_id?: string | number;
  ipv4?: string;
  hostname?: string;
  version?: string;
  running?: boolean;
  [key: string]: unknown;
}

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
  const v = typeof n === 'string' ? parseFloat(n) : n;
  if (v == null || Number.isNaN(v)) return '—';
  if (v < 1024) return `${v} B`;
  if (v < 1024 ** 2) return `${(v / 1024).toFixed(1)} KB`;
  if (v < 1024 ** 3) return `${(v / 1024 ** 2).toFixed(1)} MB`;
  return `${(v / 1024 ** 3).toFixed(2)} GB`;
}
