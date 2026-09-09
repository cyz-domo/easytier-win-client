import { defaultConfig, NetworkConfig } from './network-config';
import { NodeStatus, PeerInfo, RouteInfo } from './status-data';

export type Status = 'running' | 'stopped' | 'starting' | 'stopping' | 'failed';
export type Tab = 'status' | 'peers' | 'routes' | 'config' | 'logs' | 'settings';

export interface Instance {
  id: string;
  name: string;
  status: Status;
  rpcPort: number;
  config: NetworkConfig;
  autoStart?: boolean;
  desiredState?: 'stopped' | 'running';
  lastError?: string | null;
  remoteManageEnabled?: boolean;
  rpcWhitelistCidrs?: string[];
}

export interface InstanceSnapshot {
  peers: PeerInfo[];
  routes: RouteInfo[];
  node: NodeStatus | null;
}

export interface AutoStartClientStatus {
  enabled: boolean;
  start_minimized: boolean;
  path_mismatch?: boolean;
  registered_path?: string | null;
}

export interface runtimeInfo {
  available: boolean;
  version: string;
  core_path?: string;
}

export interface KernelUpdateInfo {
  current_version: string;
  latest_version?: string | null;
  asset_name?: string | null;
  update_available: boolean;
  available_versions?: string[] | null;
  error?: string | null;
}

export interface KernelUpdateProgress {
  task_id?: string | null;
  phase: string;
  downloaded_bytes: number;
  total_bytes?: number | null;
  percent?: number | null;
  current_file?: string | null;
  message: string;
  error?: string | null;
}

export interface KernelUpdateTaskResponse {
  task_id?: string | null;
  status?: string;
  phase?: string;
  downloaded_bytes?: number;
  total_bytes?: number | null;
  percent?: number | null;
  message?: string;
  progress?: Partial<KernelUpdateProgress> | null;
  result?: Partial<KernelUpdateProgress> | null;
  error?: string | { message?: string } | null;
}

export const KERNEL_TERMINAL_PHASES = ['completed', 'failed', 'cancelled'];
export const KERNEL_TERMINAL_STATUSES = ['completed', 'complete', 'success', 'succeeded', 'failed', 'error', 'cancelled'];
export const isKernelTerminal = (value?: string | null): boolean =>
  !!value && KERNEL_TERMINAL_STATUSES.includes(value.toLowerCase());
export const kernelErrorText = (error: KernelUpdateTaskResponse['error']): string | undefined =>
  typeof error === 'string' ? error : error?.message;

export const mergeKernelProgress = (
  base: KernelUpdateProgress | null,
  update: Partial<KernelUpdateProgress> | null | undefined,
  taskId?: string | null
): KernelUpdateProgress => ({
  phase: update?.phase || base?.phase || 'checking',
  downloaded_bytes: update?.downloaded_bytes ?? base?.downloaded_bytes ?? 0,
  total_bytes: update?.total_bytes ?? base?.total_bytes ?? null,
  percent: update?.percent ?? base?.percent ?? 0,
  current_file: update?.current_file ?? base?.current_file ?? null,
  message: update?.message || base?.message || '正在更新内核',
  error: update?.error ?? base?.error,
  task_id: update?.task_id ?? taskId ?? base?.task_id ?? null,
});

export const KERNEL_PROXIES = [
  { value: 'direct', label: '直连 (GitHub Official)' },
  { value: 'https://ghfast.top', label: 'ghfast.top' },
  { value: 'https://v6.gh-proxy.org', label: 'v6.gh-proxy.org' },
  { value: 'https://hk.gh-proxy.org', label: 'hk.gh-proxy.org' },
  { value: 'https://cdn.gh-proxy.org', label: 'cdn.gh-proxy.org' },
  { value: 'https://edgeone.gh-proxy.org', label: 'edgeone.gh-proxy.org' },
  { value: 'custom', label: '自定义镜像/代理前缀...' },
];

export type ServiceRecoveryStep = 'starting' | 'waiting' | 'syncing';
export const SERVICE_RECOVERY_TEXT: Record<ServiceRecoveryStep, string> = {
  starting: '正在重新拉起后台服务…',
  waiting: '服务已启动，等待就绪…',
  syncing: '服务已就绪，正在同步网络状态…',
};

export const KERNEL_PHASE_TEXT: Record<string, string> = {
  checking: '检查版本',
  downloading: '下载内核',
  extracting: '校验并解压',
  stopping: '停止网络',
  installing: '替换内核',
  restarting: '恢复网络',
  completed: '更新完成',
  failed: '更新失败',
  cancelled: '已取消',
};

export const load = <T,>(k: string, d: T): T => {
  try {
    return JSON.parse(localStorage.getItem(k) || JSON.stringify(d)) as T;
  } catch {
    return d;
  }
};

export function nextRpcPort(instances: Instance[]): number {
  const used = new Set(instances.map(i => i.rpcPort));
  for (let p = 15888; p < 16888; p++) if (!used.has(p)) return p;
  return 0;
}

export function loadNodeRpcPort(ip: string): number | null {
  try {
    const raw = localStorage.getItem('easytier.node_rpc_ports.v1');
    if (raw) {
      const map = JSON.parse(raw);
      if (typeof map[ip] === 'number' && map[ip] > 0) return map[ip];
    }
  } catch { /* ignore */ }
  return null;
}

export function saveNodeRpcPort(ip: string, port: number) {
  try {
    const raw = localStorage.getItem('easytier.node_rpc_ports.v1');
    const map = raw ? JSON.parse(raw) : {};
    map[ip] = port;
    localStorage.setItem('easytier.node_rpc_ports.v1', JSON.stringify(map));
  } catch { /* ignore */ }
}

export function getCandidatePortsForIp(ip: string, instances: Instance[], currentRpcPort?: number): number[] {
  const cached = loadNodeRpcPort(ip);
  const matchedInstance = instances.find(i => {
    const instIp = i.config.virtual_ipv4 ? i.config.virtual_ipv4.split('/')[0].trim() : '';
    return instIp === ip;
  });
  const localInstancePort = matchedInstance?.rpcPort;
  const configuredPorts = instances.map(i => i.rpcPort).filter((p): p is number => typeof p === 'number' && p > 0);
  const standardRange = [15888, 15889, 15890, 15891, 15892, 15893, 15894, 15895];

  return Array.from(new Set([
    ...(localInstancePort ? [localInstancePort] : []),
    ...(cached ? [cached] : []),
    ...(currentRpcPort ? [currentRpcPort] : []),
    ...configuredPorts,
    ...standardRange,
  ]));
}

export const healStatus = (s: Status): Status =>
  s === 'starting' || s === 'stopping' || s === 'running' ? 'stopped' : s;

export const loadInstances = (): Instance[] => {
  const saved = load<Instance[]>('easytier.instances.v2', []).map(i => ({
    ...i,
    status: healStatus(i.status),
    config: { ...defaultConfig(), ...i.config } as NetworkConfig,
  }));
  return saved.length
    ? saved
    : [{ id: crypto.randomUUID(), name: '我的网络', status: 'stopped', rpcPort: 15888, config: defaultConfig() }];
};
