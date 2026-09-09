import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { defaultConfig, listenersForInstance, NetworkConfig, validateConfig } from './network-config';
import { decodeTOML, encodeTOML } from './toml-codec';
import { ConfigEditor } from './ConfigEditor';
import { NodeStatus, PeerColumn, PeerInfo, PEER_COLUMNS, RefreshInterval, RouteInfo, formatBytes, latencyTone, parseHumanBytes, parseNodeJSON, parsePeerJSON, parseRouteJSON, routeTone } from './status-data';
import { IconClipboard, IconCopy, IconDownload, IconGear, IconPlay, IconPlus, IconRefresh, IconSliders, IconStop, IconTerminal, IconTrash, IconUpload, IconUsers, IconGlobe, IconPencil } from './icons';
import easytierLogo from './assets/easytier-logo.png';
import { TrafficAreaChart, TrafficSample, formatSpeed } from './TrafficAreaChart';
import { RemoteConfigDialog } from './RemoteConfigDialog';

import { getServiceStatus, serviceRequest, ServiceInstanceState, ServiceStatus } from './service-client';
import { DialogHost, appAlert, appConfirm } from './dialogs';

type Status = 'running' | 'stopped' | 'starting' | 'stopping' | 'failed';
type Tab = 'status' | 'peers' | 'routes' | 'config' | 'logs' | 'settings';

interface Instance {
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

const load = <T,>(k: string, d: T): T => {
  try { return JSON.parse(localStorage.getItem(k) || JSON.stringify(d)) as T; } catch { return d; }
};

function nextRpcPort(instances: Instance[]): number {
  const used = new Set(instances.map(i => i.rpcPort));
  for (let p = 15888; p < 16888; p++) if (!used.has(p)) return p;
  return 0; // random
}

function loadNodeRpcPort(ip: string): number | null {
  try {
    const raw = localStorage.getItem('easytier.node_rpc_ports.v1');
    if (raw) {
      const map = JSON.parse(raw);
      if (typeof map[ip] === 'number' && map[ip] > 0) return map[ip];
    }
  } catch { /* ignore */ }
  return null;
}

function saveNodeRpcPort(ip: string, port: number) {
  try {
    const raw = localStorage.getItem('easytier.node_rpc_ports.v1');
    const map = raw ? JSON.parse(raw) : {};
    map[ip] = port;
    localStorage.setItem('easytier.node_rpc_ports.v1', JSON.stringify(map));
  } catch { /* ignore */ }
}

function getCandidatePortsForIp(ip: string, instances: Instance[], currentRpcPort?: number): number[] {
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

interface runtimeInfo { available: boolean; version: string; core_path?: string }

interface KernelUpdateInfo { current_version: string; latest_version?: string | null; asset_name?: string | null; update_available: boolean; available_versions?: string[] | null; error?: string | null }
interface KernelUpdateProgress { task_id?: string | null; phase: string; downloaded_bytes: number; total_bytes?: number | null; percent?: number | null; current_file?: string | null; message: string; error?: string | null }
interface KernelUpdateTaskResponse { task_id?: string | null; status?: string; phase?: string; downloaded_bytes?: number; total_bytes?: number | null; percent?: number | null; message?: string; progress?: Partial<KernelUpdateProgress> | null; result?: Partial<KernelUpdateProgress> | null; error?: string | { message?: string } | null }
const KERNEL_TERMINAL_PHASES = ['completed', 'failed', 'cancelled'];
const KERNEL_TERMINAL_STATUSES = ['completed', 'complete', 'success', 'succeeded', 'failed', 'error', 'cancelled'];
const isKernelTerminal = (value?: string | null): boolean => !!value && KERNEL_TERMINAL_STATUSES.includes(value.toLowerCase());
const kernelErrorText = (error: KernelUpdateTaskResponse['error']): string | undefined => typeof error === 'string' ? error : error?.message;
const mergeKernelProgress = (base: KernelUpdateProgress | null, update: Partial<KernelUpdateProgress> | null | undefined, taskId?: string | null): KernelUpdateProgress => ({
  phase: update?.phase || base?.phase || 'checking', downloaded_bytes: update?.downloaded_bytes ?? base?.downloaded_bytes ?? 0,
  total_bytes: update?.total_bytes ?? base?.total_bytes ?? null, percent: update?.percent ?? base?.percent ?? 0,
  current_file: update?.current_file ?? base?.current_file ?? null, message: update?.message || base?.message || '正在更新内核',
  error: update?.error ?? base?.error, task_id: update?.task_id ?? taskId ?? base?.task_id ?? null,
});
const KERNEL_PROXIES = [
  { value: 'direct', label: '直连 (GitHub Official)' },
  { value: 'https://ghfast.top', label: 'ghfast.top' },
  { value: 'https://v6.gh-proxy.org', label: 'v6.gh-proxy.org' },
  { value: 'https://hk.gh-proxy.org', label: 'hk.gh-proxy.org' },
  { value: 'https://cdn.gh-proxy.org', label: 'cdn.gh-proxy.org' },
  { value: 'https://edgeone.gh-proxy.org', label: 'edgeone.gh-proxy.org' },
  { value: 'custom', label: '自定义镜像/代理前缀...' },
];

type ServiceRecoveryStep = 'starting' | 'waiting' | 'syncing';
const SERVICE_RECOVERY_TEXT: Record<ServiceRecoveryStep, string> = {
  starting: '正在重新拉起后台服务…',
  waiting: '服务已启动，等待就绪…',
  syncing: '服务已就绪，正在同步网络状态…',
};

const KERNEL_PHASE_TEXT: Record<string, string> = {
  checking: '检查版本', downloading: '下载内核', extracting: '校验并解压', stopping: '停止网络', installing: '替换内核', restarting: '恢复网络', completed: '更新完成', failed: '更新失败', cancelled: '已取消',
};

// Status persisted in localStorage may be stale after an abnormal exit (the
// backend child-process table is always empty on a fresh launch), so any
// leftover transitional or running marker is healed to 'stopped'.
const healStatus = (s: Status): Status => (s === 'starting' || s === 'stopping' || s === 'running' ? 'stopped' : s);

// Configs persisted by older builds may miss fields added later; merging each
// saved config over the defaults keeps the editor from reading undefined.
const loadInstances = (): Instance[] => {
  const saved = load<Instance[]>('easytier.instances.v2', []).map(i => ({
    ...i,
    status: healStatus(i.status),
    config: { ...defaultConfig(), ...i.config } as NetworkConfig,
  }));
  return saved.length
    ? saved
    : [{ id: crypto.randomUUID(), name: '我的网络', status: 'stopped', rpcPort: 15888, config: defaultConfig() }];
};

export default function App() {
  const [instances, setInstances] = useState<Instance[]>(loadInstances);
  const [activeId, setActiveId] = useState<string>(() => load('easytier.active.v2', ''));
  const [tab, setTab] = useState<Tab>('status');
  const [pollEpoch, setPollEpoch] = useState(0);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [runtime, setRuntime] = useState<runtimeInfo | null>(null);
  interface InstanceSnapshot { peers: PeerInfo[]; routes: RouteInfo[]; node: NodeStatus | null }
  const [statusByInstance, setStatusByInstance] = useState<Record<string, InstanceSnapshot>>({});
  // Cumulative per-peer traffic. The core's rx/tx counters are per-connection
  // and reset when a peer reconnects, which silently erased transferred data;
  // deltas accumulated here survive reconnects (and app restarts).
  const [trafficTotals, setTrafficTotals] = useState<Record<string, { rx: number; tx: number }>>(() => load('easytier.traffic.v1', {}));
  const lastPeerCounters = useRef<Record<string, { rx: number; tx: number }>>({});
  const [logsByInstance, setLogsByInstance] = useState<Record<string, string[]>>({});
  const [networkLogs, setNetworkLogs] = useState<string[]>([]);
  const [logView, setLogView] = useState<'runtime' | 'network'>('runtime');
  const [isElevated, setIsElevated] = useState<boolean | null>(null);
  const [elevationNoticeShown, setElevationNoticeShown] = useState(false);
  const [secretVisible, setSecretVisible] = useState(false);
  const [visibleCols, setVisibleCols] = useState<PeerColumn[]>(() =>
    load('easytier.peer-cols.v2', PEER_COLUMNS.filter(c => c.defaultOn).map(c => c.key)));
  const [refreshSecs, setRefreshSecs] = useState<RefreshInterval>(() => load('easytier.refresh.v1', 5));
  const [showDisplaySettings, setShowDisplaySettings] = useState(false);
  const [showPeerNodes, setShowPeerNodes] = useState<boolean>(() => load('easytier.show-peer-nodes.v1', false));
  const [tomlDraft, setTomlDraft] = useState<string | null>(null);
  const [tomlDirty, setTomlDirty] = useState(false);
  const [tomlError, setTomlError] = useState<string | null>(null);
  const [configSaved, setConfigSaved] = useState(false);
  const [kernelUpdate, setKernelUpdate] = useState<KernelUpdateProgress | null>(null);
  const [kernelInfo, setKernelInfo] = useState<KernelUpdateInfo | null>(null);
  const [availableKernelVersions, setAvailableKernelVersions] = useState<string[]>([]);
  const [selectedKernelVersion, setSelectedKernelVersion] = useState<string>('');
  const [kernelProxy, setKernelProxy] = useState<string>(() => load('easytier.kernel-update-proxy.v1', 'direct'));
  const [customKernelProxy, setCustomKernelProxy] = useState<string>(() => load('easytier.custom-kernel-proxy.v1', ''));
  useEffect(() => { localStorage.setItem('easytier.custom-kernel-proxy.v1', customKernelProxy); }, [customKernelProxy]);
  const effectiveKernelProxy = kernelProxy === 'custom' ? (customKernelProxy.trim() || 'direct') : kernelProxy;

  const [service, setService] = useState<ServiceStatus | null>(null);
  const [serviceChecking, setServiceChecking] = useState(true);
  const [serviceBusy, setServiceBusy] = useState(false);
  const [serviceResult, setServiceResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [remoteConfigTarget, setRemoteConfigTarget] = useState<{ host: string; port: number; candidatePorts?: number[] } | null>(null);
  const serviceMode = service?.running === true && service?.healthy !== false;
  const serviceInstalled = service?.installed === true;
  const [isWindowVisible, setIsWindowVisible] = useState<boolean>(() => !document.hidden);
  const logTimer = useRef<number | null>(null);
  const kernelTaskId = kernelUpdate?.task_id ?? null;

  const [toast, setToast] = useState<string | null>(null);
  const toastTimerRef = useRef<number | null>(null);

  const showToast = (message: string) => {
    if (toastTimerRef.current) {
      window.clearTimeout(toastTimerRef.current);
    }
    setToast(message);
    toastTimerRef.current = window.setTimeout(() => {
      setToast(null);
      toastTimerRef.current = null;
    }, 2200);
  };

  const copyText = async (text: string, label?: string) => {
    try {
      await navigator.clipboard.writeText(text);
      showToast(label ? `✓ ${label}` : `✓ 已复制: ${text}`);
    } catch (e) {
      showToast(`复制失败：${String(e)}`);
    }
  };

  const current = useMemo(() => instances.find(i => i.id === activeId) ?? instances[0], [instances, activeId]);
  const runningNow = current?.status === 'running';
  const curSnap = (runningNow ? statusByInstance[current?.id ?? ''] : undefined) ?? { peers: [] as PeerInfo[], routes: [] as RouteInfo[], node: null as NodeStatus | null };
  const peers = curSnap.peers;
  const routes = curSnap.routes;
  const node = curSnap.node;
  // Accumulated traffic (survives peer reconnects and app restarts).
  const trafficPrefix = `${current?.id ?? ''}:`;
  const instanceTraffic = useMemo(() => {
    let rx = 0, tx = 0;
    for (const [key, v] of Object.entries(trafficTotals)) {
      if (key.startsWith(trafficPrefix)) { rx += v.rx; tx += v.tx; }
    }
    return { rx, tx };
  }, [trafficTotals, trafficPrefix]);
  const clearTraffic = () => {
    setTrafficTotals(m => {
      const nextM: Record<string, { rx: number; tx: number }> = {};
      for (const [key, v] of Object.entries(m)) {
        if (!key.startsWith(trafficPrefix)) nextM[key] = v;
      }
      localStorage.setItem('easytier.traffic.v1', JSON.stringify(nextM));
      return nextM;
    });
    lastPeerCounters.current = {};
  };
  const visiblePeers = useMemo(() => {
    if (showPeerNodes || !current) return peers;
    return peers.filter(peer => !String(peer.hostname ?? '').toLowerCase().startsWith('publicserver'));
  }, [current, peers, showPeerNodes]);

  // 1-Second real-time traffic sampler for smooth area chart (restored from sessionStorage across refreshes)
  const [trafficHistory, setTrafficHistory] = useState<TrafficSample[]>(() => {
    try {
      const raw = sessionStorage.getItem('easytier.traffic_history.v1');
      if (raw) {
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed) && parsed.length === 30) {
          const latestTime = parsed[29]?.time ?? 0;
          if (Date.now() - latestTime < 60_000) {
            return parsed;
          }
        }
      }
    } catch { /* ignore */ }
    return Array.from({ length: 30 }, (_, i) => ({ time: Date.now() - (29 - i) * 1000, rxSpeed: 0, txSpeed: 0 }));
  });

  useEffect(() => {
    try {
      sessionStorage.setItem('easytier.traffic_history.v1', JSON.stringify(trafficHistory));
    } catch { /* ignore */ }
  }, [trafficHistory]);
  const [currentRxSpeed, setCurrentRxSpeed] = useState(0);
  const [currentTxSpeed, setCurrentTxSpeed] = useState(0);
  const lastTotalBytesRef = useRef<{ rx: number; tx: number; time: number } | null>(null);

  useEffect(() => {
    // Only perform 1-second real-time sampling when running, window is visible, and user is on the status overview tab
    if (!runningNow || !current?.rpcPort || !isWindowVisible || tab !== 'status') {
      setCurrentRxSpeed(0);
      setCurrentTxSpeed(0);
      lastTotalBytesRef.current = null;
      return;
    }

    let alive = true;
    const sample = async () => {
      try {
        const snap = await invoke<InstanceSnapshot>('status_query', { port: current.rpcPort });
        if (!alive) return;
        let totalRx = 0;
        let totalTx = 0;
        for (const p of snap.peers) {
          if (p.cost !== 'Local') {
            totalRx += parseHumanBytes(p.rx_bytes);
            totalTx += parseHumanBytes(p.tx_bytes);
          }
        }
        const now = Date.now();
        const last = lastTotalBytesRef.current;
        lastTotalBytesRef.current = { rx: totalRx, tx: totalTx, time: now };
        if (last) {
          const dt = Math.max(0.4, (now - last.time) / 1000);
          const rSpeed = totalRx >= last.rx ? (totalRx - last.rx) / dt : 0;
          const tSpeed = totalTx >= last.tx ? (totalTx - last.tx) / dt : 0;
          setCurrentRxSpeed(rSpeed);
          setCurrentTxSpeed(tSpeed);
          setTrafficHistory(prev => [...prev.slice(1), { time: now, rxSpeed: rSpeed, txSpeed: tSpeed }]);
        }
      } catch {
        // RPC blip
      }
    };

    void sample();
    const timer = window.setInterval(() => void sample(), 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [runningNow, current?.id, current?.rpcPort, isWindowVisible, tab]);

  const peakSpeed = useMemo(
    () => Math.max(0, ...trafficHistory.map(s => Math.max(s.rxSpeed, s.txSpeed))),
    [trafficHistory]
  );

  const activeVirtualIp = useMemo(() => {
    const ip = node?.ipv4_addr || current?.config?.virtual_ipv4;
    return ip ? ip.split('/')[0].trim() : null;
  }, [node?.ipv4_addr, current?.config?.virtual_ipv4]);

  useEffect(() => {
    if (!current) return;
    void invoke('update_tray_status', {
      payload: {
        running: current.status === 'running',
        instance_id: current.id,
        network_name: current.name,
        virtual_ip: activeVirtualIp,
        peer_count: peers.length,
        rx_speed: currentRxSpeed > 0 ? formatSpeed(currentRxSpeed) : '0 B/s',
        tx_speed: currentTxSpeed > 0 ? formatSpeed(currentTxSpeed) : '0 B/s',
      },
    }).catch(() => {});
  }, [current?.status, current?.id, current?.name, activeVirtualIp, peers.length]);

  // Inline double-click rename state
  const [inlineRename, setInlineRename] = useState<{
    peerId: number | string;
    originalName: string;
    currentName: string;
    host: string;
    saving: boolean;
  } | null>(null);

  const submitRename = async () => {
    if (!inlineRename || inlineRename.saving) return;
    const newName = inlineRename.currentName.trim();
    if (!newName) {
      showToast('主机名不能为空');
      setInlineRename(null);
      return;
    }
    if (newName === inlineRename.originalName) {
      setInlineRename(null);
      return;
    }
    setInlineRename(prev => (prev ? { ...prev, saving: true } : null));

    const candidatePorts = getCandidatePortsForIp(inlineRename.host, instances, current.rpcPort);

    let success = false;
    let matchedPort: number | null = null;
    let lastErr: unknown = null;

    const probeAndPatch = async (port: number) => {
      const disc = await invoke<{ instance_id: string; hostname: string; virtual_ip: string }>(
        'remote_config_discover',
        {
          host: inlineRename.host,
          port,
          virtualIp: inlineRename.host,
        }
      );
      await invoke('remote_config_patch', {
        host: inlineRename.host,
        port,
        instanceId: disc.instance_id,
        patch: { hostname: newName },
      });
      return port;
    };

    try {
      matchedPort = await Promise.any(candidatePorts.map(p => probeAndPatch(p)));
      success = true;
    } catch (e) {
      lastErr = e;
    }

    if (!success) {
      const input = window.prompt(
        `未能通过常见端口（已尝试 ${candidatePorts.slice(0, 5).join(', ')} 等）连接到 ${inlineRename.host} 的 RPC。\n\n若该节点使用了自定义 RPC 端口，请输入端口号（留空取消）：`,
        '15888'
      );
      if (input) {
        const customPort = parseInt(input.trim(), 10);
        if (customPort > 0 && customPort <= 65535) {
          try {
            matchedPort = await probeAndPatch(customPort);
            success = true;
          } catch (e) {
            lastErr = e;
          }
        }
      }
    }

    if (success && matchedPort) {
      saveNodeRpcPort(inlineRename.host, matchedPort);
      // Synchronize to local instance config if target is a local instance!
      setInstances(prev => {
        const next = prev.map(inst => {
          const cleanVirtual = inst.config.virtual_ipv4 ? inst.config.virtual_ipv4.split('/')[0].trim() : '';
          const isMatch = inst.rpcPort === matchedPort
            || cleanVirtual === inlineRename.host
            || (inst.id === current.id && (node?.hostname === inlineRename.originalName || !inlineRename.host));
          if (isMatch) {
            const updated = {
              ...inst,
              config: {
                ...inst.config,
                hostname: newName,
              },
            };
            if (serviceMode) {
              void serviceRequest('sync_instance', {
                instance_id: updated.id,
                name: updated.name,
                config_toml: encodeTOML(updated.config),
                rpc_port: updated.rpcPort,
                auto_start: updated.autoStart ?? false,
                desired_state: updated.status === 'running' ? 'running' : 'stopped',
                remote_manage_enabled: updated.remoteManageEnabled ?? false,
                rpc_whitelist_cidrs: updated.rpcWhitelistCidrs ?? [],
              }).catch(() => {});
            }
            return updated;
          }
          return inst;
        });
        localStorage.setItem('easytier.instances.v2', JSON.stringify(next));
        return next;
      });

      // Optimistically update current view
      setStatusByInstance(prev => {
        const cur = prev[current.id];
        if (!cur) return prev;
        return {
          ...prev,
          [current.id]: {
            ...cur,
            peers: cur.peers.map(p => (p.hostname === inlineRename.originalName ? { ...p, hostname: newName } : p)),
            routes: cur.routes.map(r => (r.hostname === inlineRename.originalName ? { ...r, hostname: newName } : r)),
          },
        };
      });
      showToast(`✓ 已将设备改名为「${newName}」，并同步至组网配置 (RPC :${matchedPort})`);
      setInlineRename(null);
    } else {
      showToast(`改名失败：无法连接 ${inlineRename.host} RPC。请确认对端已开启「允许远程管理」且端口配置正确。${lastErr ? ` (${String(lastErr)})` : ''}`);
      setInlineRename(null);
    }
  };

  useEffect(() => { localStorage.setItem('easytier.instances.v2', JSON.stringify(instances)); }, [instances]);
  useEffect(() => { if (current) localStorage.setItem('easytier.active.v2', current.id); }, [current]);
  useEffect(() => { invoke<runtimeInfo>('detect_runtime', {}).then(v => setRuntime({ ...v, core_path: v.core_path ?? 'core/easytier-core.exe' })).catch(() => setRuntime(null)); }, []);
  const wasServiceModeRef = useRef<boolean | null>(null);
  useEffect(() => {
    if (serviceChecking) return;
    if (serviceMode) {
      if (wasServiceModeRef.current === false) {
        showToast('✓ 已连接后台服务，当前为服务模式');
      }
      wasServiceModeRef.current = true;
    } else {
      if (wasServiceModeRef.current === true) {
        showToast('⚠ 后台服务已断开，当前为兼容模式');
      }
      wasServiceModeRef.current = false;
    }
  }, [serviceMode, serviceChecking]);

  useEffect(() => {
    // 3 seconds after launch, trigger process tree working set trim to drop physical RAM usage
    const timer = setTimeout(() => {
      void invoke('trim_memory').catch(() => {});
    }, 3200);
    const onBlur = () => {
      void invoke('trim_memory').catch(() => {});
    };
    window.addEventListener('blur', onBlur);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('blur', onBlur);
    };
  }, []);

  const refreshService = async (opts?: { skipAutoStart?: boolean }) => {
    try {
      const installed = await invoke<{ installed: boolean; running: boolean; message?: string }>('query_service_installation').catch(e => ({ installed: false, running: false, message: String(e) }));
      if (!installed.installed) {
        setService({ installed: false, running: false, message: installed.message });
        setServiceChecking(false);
        return;
      }
      if (!installed.running) {
        // Auto-start in the background so the UI never waits on UAC/SCM.
        if (!opts?.skipAutoStart && !sessionStorage.getItem('easytier.service-repaired.v1')) {
          setServiceChecking(true);
          setService({ installed: true, running: false, message: '正在自动启动后台服务…' });
          void (async () => {
            try {
              await invoke('start_service');
              await new Promise(r => setTimeout(r, 1500));
              let q = await invoke<{ installed: boolean; running: boolean }>('query_service_installation').catch(() => ({ installed: true, running: false }));
              if (q.installed && !q.running) {
                sessionStorage.setItem('easytier.service-repaired.v1', '1');
                setService({ installed: true, running: false, message: '服务启动异常，正在自动修复…' });
                await invoke('repair_service');
                await new Promise(r => setTimeout(r, 1200));
              }
              await refreshService({ skipAutoStart: true });
            } catch {
              setService({ installed: true, running: false, message: '服务已安装但启动失败（可在设置中重试或修复）。' });
              setServiceChecking(false);
            }
          })();
        } else {
          setService({ installed: true, running: false, message: '后台服务未运行' });
          setServiceChecking(false);
        }
        return;
      }
      const next = await withScmGuardedStatus();
      setService({ ...next, installed: true, running: true });
      setServiceChecking(false);
      if (next.running && next.healthy !== false) {
        const states = await serviceRequest<ServiceInstanceState[]>('list_instances');
        setInstances(xs => {
          const mapped = xs.map(i => {
            const s = states.find(x => x.id === i.id);
            return s ? { ...i, name: s.name || i.name, rpcPort: s.rpc_port ?? i.rpcPort, status: s.observed_state, autoStart: s.auto_start, desiredState: s.desired_state, lastError: s.last_error, remoteManageEnabled: s.remote_manage_enabled ?? i.remoteManageEnabled, rpcWhitelistCidrs: s.rpc_whitelist_cidrs ?? i.rpcWhitelistCidrs } : i;
          });
          const existingIds = new Set(xs.map(i => i.id));
          const additions: Instance[] = [];
          for (const s of states) {
            if (!existingIds.has(s.id)) {
              additions.push({
                id: s.id,
                name: s.name || '网络实例',
                status: s.observed_state,
                rpcPort: s.rpc_port ?? 15888,
                config: defaultConfig(),
                autoStart: s.auto_start,
                desiredState: s.desired_state,
                lastError: s.last_error,
                remoteManageEnabled: s.remote_manage_enabled,
                rpcWhitelistCidrs: s.rpc_whitelist_cidrs,
              });
            }
          }
          return additions.length > 0 ? [...mapped, ...additions] : mapped;
        });
      }
    } catch (e) {
      const sc = await invoke<{ installed: boolean; running: boolean }>('query_service_installation').catch(() => null);
      if (sc?.installed && sc?.running) {
        setService({ installed: true, running: true, healthy: false, message: '后台服务无响应，正在尝试自动修复…' });
        if (!sessionStorage.getItem('easytier.service-repaired-zombie.v1')) {
          sessionStorage.setItem('easytier.service-repaired-zombie.v1', '1');
          void (async () => {
            try {
              await invoke('repair_service');
              await new Promise(r => setTimeout(r, 1500));
              await refreshService({ skipAutoStart: true });
            } catch (err) {
              setService({ installed: true, running: false, healthy: false, message: `服务自动修复失败：${String(err)}（可在设置中重试）` });
            }
          })();
        }
        return;
      }
      setService({ installed: false, running: false, message: String(e) });
    } finally {
      setServiceChecking(false);
    }
  };
  // service_status via the named pipe, retried while SCM still reports the
  // service RUNNING — the pipe can transiently have no listening instance.
  const withScmGuardedStatus = async (): Promise<ServiceStatus> => {
    let lastError: unknown = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        return await getServiceStatus();
      } catch (e) {
        lastError = e;
        const sc = await invoke<{ installed: boolean; running: boolean }>('query_service_installation').catch(() => null);
        if (!sc?.installed || !sc?.running) throw e;
        await new Promise(r => setTimeout(r, 300));
      }
    }
    throw lastError;
  };
  useEffect(() => { void refreshService(); }, []);

  // Self-heal: the resident service can die while the GUI stays open (crash,
  // manual sc stop). Poll its health every 20s; on failure, surface a
  // "重新拉起服务" progress banner and run a staged recovery: start the
  // service (one attempt per minute), wait for RUNNING, then resync instance
  // state. The service adopts still-running networks as-is — it never
  // restarts healthy ones.
  const [serviceRecovery, setServiceRecovery] = useState<ServiceRecoveryStep | null>(null);
  const recoveryRef = useRef(false);
  const lastServiceHealAt = useRef(0);
  const runServiceRecovery = async () => {
    if (recoveryRef.current) return;
    recoveryRef.current = true;
    setServiceRecovery('starting');
    setService(s => (s ? { ...s, running: false, message: '检测到后台服务异常退出，正在自动重新拉起…' } : s));
    try {
      await invoke('repair_service');
      setServiceRecovery('waiting');
      let ready = false;
      for (let i = 0; i < 30; i++) {
        await new Promise(r => setTimeout(r, 1000));
        try {
          const q = await invoke<{ installed: boolean; running: boolean }>('query_service_installation');
          if (q.installed && q.running) { ready = true; break; }
        } catch { /* SCM hiccup — keep waiting */ }
      }
      if (!ready) {
        setService({ installed: true, running: false, message: '后台服务自动恢复超时，可在设置中重试或修复。' });
        return;
      }
      setServiceRecovery('syncing');
      await refreshService({ skipAutoStart: true });
    } catch (e) {
      setService({ installed: true, running: false, message: `后台服务自动恢复失败：${String(e)}（可在设置中重试或修复）` });
    } finally {
      recoveryRef.current = false;
      setServiceRecovery(null);
    }
  };
  useEffect(() => {
    if (!service?.installed) return;
    const timer = window.setInterval(async () => {
      if (document.hidden || recoveryRef.current) return;
      try {
        const q = await invoke<{ installed: boolean; running: boolean }>('query_service_installation');
        if (q.installed && !q.running) {
          setService(s => (s ? { ...s, running: false, message: '后台服务掉线，正在准备自动恢复…' } : s));
          if (Date.now() - lastServiceHealAt.current > 60_000) {
            lastServiceHealAt.current = Date.now();
            void runServiceRecovery();
          }
        } else if (q.installed && q.running) {
          try {
            await getServiceStatus();
          } catch {
            setService(s => (s ? { ...s, healthy: false, message: '后台服务无响应，正在准备自动恢复…' } : s));
            if (Date.now() - lastServiceHealAt.current > 60_000) {
              lastServiceHealAt.current = Date.now();
              void runServiceRecovery();
            }
          }
        }
      } catch { /* sc query hiccup — the next tick retries */ }
    }, 20_000);
    return () => { clearInterval(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [service?.installed]);

  const normalizeNetworkLogs = (value: string[] | string | { text?: string } | null | undefined): string[] => {
    if (Array.isArray(value)) return value;
    const text = typeof value === 'string' ? value : value?.text || '';
    return text.split(/\r?\n/).filter(Boolean);
  };

  useEffect(() => { invoke<boolean>('is_elevated').then(setIsElevated).catch(() => setIsElevated(null)); }, []);
  useEffect(() => {
    if (serviceMode || isElevated !== false) return;
    const seen = sessionStorage.getItem('easytier.elevation-notice.v1') === '1';
    setElevationNoticeShown(seen);
  }, [serviceMode, isElevated]);
  const clearNetworkLogs = () => setNetworkLogs([]);
  useEffect(() => {
    if (current?.status !== 'running' || !isWindowVisible || tab !== 'logs') {
      if (current?.status !== 'running') clearNetworkLogs();
      return;
    }
    let alive = true;
    let timer: number | null = null;
    const refresh = async () => {
      try {
        const value = serviceMode
          ? await serviceRequest<string[] | string>('get_network_logs', { instance_id: current.id })
          : await invoke<string[] | string>('get_network_logs', { id: current.id });
        if (!alive) return;
        setNetworkLogs(normalizeNetworkLogs(value));
      } catch { /* log bridge may be unavailable while the core starts */ }
      finally { if (alive) timer = window.setTimeout(() => void refresh(), refreshSecs * 1000); }
    };
    void refresh();
    return () => { alive = false; if (timer != null) window.clearTimeout(timer); };
  }, [current?.id, current?.status, serviceMode, refreshSecs, isWindowVisible, tab]);

  // A core restart resets the per-connection traffic counters — clear the
  // accumulated totals for that instance so the ledger restarts with it.
  const prevStatuses = useRef<Record<string, string>>({});
  useEffect(() => {
    for (const i of instances) {
      const prev = prevStatuses.current[i.id];
      if (prev === 'running' && i.status !== 'running') {
        setTrafficTotals(m => {
          const next: Record<string, { rx: number; tx: number }> = {};
          for (const [k, v] of Object.entries(m)) {
            if (!k.startsWith(`${i.id}:`)) next[k] = v;
          }
          localStorage.setItem('easytier.traffic.v1', JSON.stringify(next));
          return next;
        });
        for (const k of Object.keys(lastPeerCounters.current)) {
          if (k.startsWith(`${i.id}:`)) delete lastPeerCounters.current[k];
        }
      }
      prevStatuses.current[i.id] = i.status;
    }
  }, [instances]);
  useEffect(() => { localStorage.setItem('easytier.refresh.v1', JSON.stringify(refreshSecs)); }, [refreshSecs]);
  useEffect(() => { localStorage.setItem('easytier.show-peer-nodes.v1', JSON.stringify(showPeerNodes)); }, [showPeerNodes]);
  useEffect(() => { localStorage.setItem('easytier.kernel-update-proxy.v1', JSON.stringify(kernelProxy)); }, [kernelProxy]);
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void listen<KernelUpdateProgress>('kernel-update-progress', event => {
      const payload = event.payload;
      setKernelUpdate(currentProgress => {
        if (!currentProgress) return currentProgress;
        if (payload.task_id && currentProgress.task_id && payload.task_id !== currentProgress.task_id) return currentProgress;
        const next = mergeKernelProgress(currentProgress, payload, currentProgress.task_id);
        return next;
      });
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);
  useEffect(() => {
    if (!kernelTaskId || !kernelUpdate || KERNEL_TERMINAL_PHASES.includes(kernelUpdate.phase)) return;
    let alive = true;
    let timer: number | null = null;
    const poll = async () => {
      try {
        const response = await serviceRequest<KernelUpdateTaskResponse>('get_task_status', { task_id: kernelTaskId });
        if (!alive || response.task_id && response.task_id !== kernelTaskId) return;
        const update: Partial<KernelUpdateProgress> = response.progress || (response.phase ? {
          phase: response.phase,
          downloaded_bytes: response.downloaded_bytes ?? 0,
          total_bytes: response.total_bytes,
          percent: response.percent,
          message: response.message || '',
          error: kernelErrorText(response.error),
        } : response.result) || {};
        const status = response.status?.toLowerCase();
        const terminal = isKernelTerminal(status) || isKernelTerminal(update?.phase);
        const phase = terminal ? (status === 'failed' || status === 'error' || update?.phase === 'failed' ? 'failed' : status === 'cancelled' || update?.phase === 'cancelled' ? 'cancelled' : 'completed') : update?.phase;
        const next = mergeKernelProgress(kernelUpdate, { ...update, ...(phase ? { phase } : {}), ...(kernelErrorText(response.error) ? { error: kernelErrorText(response.error) } : {}) }, kernelTaskId);
        setKernelUpdate(next);
        if (terminal) {
          if (phase === 'completed') {
            void invoke<KernelUpdateInfo>('check_kernel_update', { proxy: effectiveKernelProxy })
              .then(fresh => {
                setKernelInfo(fresh);
                if (fresh.available_versions) setAvailableKernelVersions(fresh.available_versions);
              })
              .catch(() => {});
            void refreshService();
            showToast('✓ EasyTier 内核已成功更新/切换');
          }
        } else if (alive) {
          timer = window.setTimeout(() => void poll(), 1000);
        }
      } catch {
        if (alive) timer = window.setTimeout(() => void poll(), 1500);
      }
    };
    void poll();
    return () => { alive = false; if (timer != null) window.clearTimeout(timer); };
  }, [serviceMode, kernelTaskId, kernelUpdate?.phase]);
  const kernelCheckedRef = useRef(false);
  useEffect(() => {
    if (tab !== 'settings' || kernelCheckedRef.current) return;
    kernelCheckedRef.current = true;
    void invoke<KernelUpdateInfo>('check_kernel_update', { proxy: kernelProxy })
      .then(info => {
        setKernelInfo(info);
        if (info.available_versions && info.available_versions.length > 0) {
          setAvailableKernelVersions(info.available_versions);
          if (info.latest_version) setSelectedKernelVersion(`v${info.latest_version}`);
        }
      })
      .catch(e => setKernelInfo({ current_version: runtime?.version ?? 'unknown', update_available: false, error: String(e) }));
  }, [tab, kernelProxy, runtime?.version]);

  // Poll peer/route status while an instance is running. Self-scheduling
  // timeout: the next round starts only after the previous one settles, so a
  // hung CLI round can never pile up overlapping rounds. Skipped when the
  // status-affecting tabs are not visible or the window is hidden — each
  // round spawns three CLI processes, so idle polling is real load.
  // Poll ALL running instances in parallel so switching between them shows
  // last-known data instantly instead of waiting for a fresh query.
  useEffect(() => {
    const running = instances.filter(i => i.status === 'running' && i.rpcPort);
    if (running.length === 0) { setStatusByInstance({}); return; }
    const pollWorthy = tab === 'status' || tab === 'peers' || tab === 'routes';
    if (!pollWorthy || !isWindowVisible) return;
    let alive = true;
    let timer: number | null = null;
    const refresh = async () => {
      const results = await Promise.all(running.map(async ({ id, rpcPort }) => {
        try {
          const snapshot = await invoke<InstanceSnapshot>('status_query', { port: rpcPort });
          return [id, snapshot] as const;
        } catch {
          return [id, null] as const;
        }
      }));
      if (!alive) return;
      setStatusByInstance(prev => {
        const next = { ...prev };
        for (const [id, snapshot] of results) {
          if (snapshot) next[id] = snapshot;
        }
        return next;
      });

      // Accumulate per-peer traffic deltas into a single batched state update
      const updates: Record<string, { rx: number; tx: number }> = {};
      for (const [id, snapshot] of results) {
        if (!snapshot) continue;
        for (const p of snapshot.peers) {
          if (p.cost === 'Local') continue;
          const normalizedIp = (p.ipv4 ?? '').split('/')[0].trim();
          const key = `${id}:${p.hostname || p.id || 'peer'}:${normalizedIp}`;
          const rx = parseHumanBytes(p.rx_bytes);
          const tx = parseHumanBytes(p.tx_bytes);
          updates[key] = { rx, tx };
        }
      }

      if (Object.keys(updates).length > 0) {
        setTrafficTotals(m => {
          let changed = false;
          const nextM = { ...m };
          for (const [key, { rx, tx }] of Object.entries(updates)) {
            const cur = nextM[key];
            const last = lastPeerCounters.current[key];
            lastPeerCounters.current[key] = { rx, tx };

            if (!cur) {
              nextM[key] = { rx, tx };
              changed = true;
            } else if (last) {
              const drx = rx >= last.rx ? rx - last.rx : rx;
              const dtx = tx >= last.tx ? tx - last.tx : tx;
              if (drx > 0 || dtx > 0) {
                nextM[key] = { rx: cur.rx + drx, tx: cur.tx + dtx };
                changed = true;
              }
            } else {
              const drx = rx > cur.rx ? rx - cur.rx : 0;
              const dtx = tx > cur.tx ? tx - cur.tx : 0;
              if (drx > 0 || dtx > 0) {
                nextM[key] = { rx: cur.rx + drx, tx: cur.tx + dtx };
                changed = true;
              }
            }
          }
          if (changed) {
            try { localStorage.setItem('easytier.traffic.v1', JSON.stringify(nextM)); } catch {}
            return nextM;
          }
          return m;
        });
      }

      timer = window.setTimeout(() => void refresh(), refreshSecs * 1000);
    };
    void refresh();
    return () => { alive = false; if (timer != null) window.clearTimeout(timer); };
  }, [instances, tab, refreshSecs, pollEpoch, serviceMode, isWindowVisible]);



  // Re-arm status polling when the window becomes visible again; the polling
  // effect above deliberately skips rounds while document.hidden is true.
  useEffect(() => {
    const onVisibilityChange = () => {
      const visible = !document.hidden;
      setIsWindowVisible(visible);
      if (visible) setPollEpoch(n => n + 1);
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => document.removeEventListener('visibilitychange', onVisibilityChange);
  }, []);

  // Append runtime messages to the log pane.
  const addLog = (line: string) => setLogsByInstance(m => {
    const key = current?.id ?? 'app';
    return { ...m, [key]: [...(m[key] ?? []).slice(-300), `${new Date().toLocaleTimeString()}  ${line}`] };
  });

  const patchConfig = (patch: Partial<NetworkConfig>) => {
    setInstances(xs => xs.map(i => {
      if (i.id !== current.id) return i;
      const config = { ...i.config, ...patch };
      // Keep the sidebar instance name in sync with the network name.
      const name = patch.network_name !== undefined && patch.network_name.trim() ? patch.network_name : i.name;
      return { ...i, name, config };
    }));
    setConfigSaved(true);
    window.setTimeout(() => setConfigSaved(false), 1500);
  };

  useEffect(() => {
    if (tomlDraft !== null && !tomlDirty) setTomlDraft(encodeTOML(current.config));
  }, [current.config, tomlDraft, tomlDirty]);

  const addInstance = () => {
    const id = crypto.randomUUID();
    setInstances(xs => [...xs, {
      id,
      name: '新网络',
      status: 'stopped',
      rpcPort: nextRpcPort(xs),
      config: { ...defaultConfig(), listener_urls: listenersForInstance(xs.length) },
    }]);
    setActiveId(id);
    setTab('config');
  };

  const removeInstance = (id: string) => {
    if (instances.length === 1) return;
    const target = instances.find(i => i.id === id);
    setInstances(xs => xs.filter(i => i.id !== id));
    if (activeId === id) setActiveId(instances.find(i => i.id !== id)!.id);
    // Release backend state: cached RPC endpoint for the instance's status
    // port, retained log buffer, staged temp config, and this instance's
    // traffic-ledger entries.
    if (target) invoke('drop_status_endpoint', { port: target.rpcPort }).catch(() => undefined);
    invoke('drop_instance_state', { id }).catch(() => undefined);
    setTrafficTotals(m => {
      const prefix = `${id}:`;
      const nextM: Record<string, { rx: number; tx: number }> = {};
      for (const [key, v] of Object.entries(m)) {
        if (!key.startsWith(prefix)) nextM[key] = v;
      }
      localStorage.setItem('easytier.traffic.v1', JSON.stringify(nextM));
      return nextM;
    });
  };

  const renameInstance = (name: string) =>
    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, name } : i)));

  const toggle = async () => {
    if (kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase)) return;
    const running = current.status === 'running';
    if (!running && !serviceMode && isElevated === false) {
      const ask = await appConfirm(
        '【权限提示】兼容模式需要在 Windows 中创建虚拟网卡 (Wintun) 与加载驱动，必须以管理员身份运行。\n\n检测到当前客户端运行在普通用户权限下，可能导致虚拟网卡创建失败（exit code 1）。\n\n是否立即以管理员身份重新启动客户端？'
      );
      if (ask) {
        try {
          await invoke('restart_as_admin');
          return;
        } catch (e) {
          addLog(`请求管理员权限启动失败：${String(e)}，请手动右键客户端选择“以管理员身份运行”`);
        }
      }
    }
    if (!running) {
      // 1. Auto-resolve RPC port conflict with other running instances or host system
      let assignedRpc = current.rpcPort;
      const otherRunning = instances.filter(i => i.id !== current.id && i.status === 'running');
      const occupiedRpcPorts = new Set(otherRunning.map(i => i.rpcPort));
      while (occupiedRpcPorts.has(assignedRpc) || (await invoke<boolean>('is_port_in_use', { port: assignedRpc }).catch(() => false))) {
        assignedRpc++;
      }
      if (assignedRpc !== current.rpcPort) {
        addLog(`[${current.name}] 探测到 RPC 端口 ${current.rpcPort} 已被占用，已自动调整为空闲端口 ${assignedRpc}`);
        current.rpcPort = assignedRpc;
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, rpcPort: assignedRpc } : i)));
      }

      // 2. Pre-flight: detect listener port conflicts and offer 1-click auto-fix to dynamic port 0
      const portOf = (url: string): number | null => {
        const m = url.match(/:(\d+)(?:\/.*)?$/);
        return m ? parseInt(m[1], 10) : null;
      };
      const myPorts = current.config.listener_urls.map(portOf).filter((p): p is number => p != null && p !== 0);
      const conflicts: string[] = [];
      const conflictPortSet = new Set<number>();
      for (const other of instances) {
        if (other.id === current.id || other.status !== 'running') continue;
        const otherPorts = new Set(other.config.listener_urls.map(portOf).filter((p): p is number => p != null));
        for (const p of myPorts) {
          if (otherPorts.has(p)) {
            conflicts.push(`端口 ${p} 已被运行中的实例「${other.name}」占用`);
            conflictPortSet.add(p);
          }
        }
      }
      for (const p of myPorts) {
        if (conflictPortSet.has(p)) continue;
        try {
          const busy = await invoke<boolean>('is_port_in_use', { port: p });
          if (busy) {
            conflicts.push(`端口 ${p} 已被系统其它程序或后台服务占用`);
            conflictPortSet.add(p);
          }
        } catch { /* check unavailable — skip */ }
      }
      if (conflictPortSet.size > 0) {
        const autoFix = await appConfirm(
          `监听器端口冲突：\n\n${conflicts.join('\n')}\n\n是否自动将冲突端口转换为自动分配端口（端口 0）并立即启动？\n（点击「确定」将自动切换到动态空闲端口避免启动冲突，点击「取消」返回手动修改）`
        );
        if (autoFix) {
          const updatedUrls = current.config.listener_urls.map(url => {
            const p = portOf(url);
            if (p != null && conflictPortSet.has(p)) {
              return url.replace(/:(\d+)(?:\/.*)?$/, ':0');
            }
            return url;
          });
          current.config.listener_urls = updatedUrls;
          setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, config: { ...i.config, listener_urls: updatedUrls } } : i)));
          addLog(`[${current.name}] 已自动将冲突监听端口调整为动态端口 0`);
        } else {
          return;
        }
      }

      // 3. TUN adapter names must be unique machine-wide: two cores claiming the
      // same dev_name fight over one Wintun adapter (Failed to create adapter error).
      const otherDevNames = new Set(
        instances
          .filter(i => i.id !== current.id)
          .map(i => i.config.dev_name?.trim())
          .filter(Boolean)
      );
      if (!current.config.dev_name?.trim() || otherDevNames.has(current.config.dev_name.trim())) {
        const unique = `et_${current.id.replace(/[^a-zA-Z0-9]/g, '').slice(0, 6)}`;
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, config: { ...i.config, dev_name: unique } } : i)));
        current.config.dev_name = unique;
        addLog(`[${current.name}] TUN 虚拟网卡名已自动设为独立设备名 ${unique}`);
      }
    }
    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: running ? 'stopping' : 'starting' } : i)));
    try {
      if (serviceMode) {
        await serviceRequest('sync_instance', {
          instance_id: current.id, name: current.name, config_toml: encodeTOML(current.config),
          rpc_port: current.rpcPort, auto_start: current.autoStart ?? false,
          desired_state: running ? 'stopped' : 'running',
          remote_manage_enabled: current.remoteManageEnabled ?? false,
          rpc_whitelist_cidrs: current.rpcWhitelistCidrs ?? [],
        });
        await serviceRequest(running ? 'stop_instance' : 'start_instance', { instance_id: current.id });
        if (running) invoke('drop_status_endpoint', { port: current.rpcPort }).catch(() => undefined);
        await refreshService();
        addLog(`[${current.name}] 服务已${running ? '停止' : '启动'}网络`);
        return;
      }
      if (running) {
        // Race the kill against a timeout: if the backend never resolves (or
        // the CLI bridge wedges), fail visibly instead of hanging in
        // "stopping" forever. stop_instance is idempotent, so a late
        // completion after a timeout needs no rollback.
        const timeout = new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error('停止操作超时（5 秒）——可再次点击「停止网络」重试')), 5000));
        await Promise.race([invoke('stop_instance', { id: current.id }), timeout]);
        invoke('drop_status_endpoint', { port: current.rpcPort }).catch(() => undefined);
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: 'stopped' } : i)));
        addLog(`[${current.name}] 网络已停止`);
      } else {
        const errors = validateConfig(current.config);
        if (errors.length) throw new Error(errors[0].message);
        if (current.remoteManageEnabled && !(current.rpcWhitelistCidrs ?? []).length) {
          throw new Error('开启远程管理必须填写 RPC 白名单（填 EasyTier 虚拟网段，如 10.126.126.0/24）');
        }
        const toml = encodeTOML(current.config);
        await invoke('start_instance', {
          id: current.id,
          config: toml,
          rpcPortal: current.remoteManageEnabled ? undefined : `127.0.0.1:${current.rpcPort}`,
          remoteManageEnabled: current.remoteManageEnabled ?? false,
          rpcWhitelistCidrs: current.rpcWhitelistCidrs ?? [],
        });
        // Give the core a moment, then check whether it exited instantly
        // (port conflict / config error) so we fail instead of hanging.
        await new Promise(r => setTimeout(r, 1200));
        const s = await invoke<{ status: Status; error?: string }>('wait_for_exit', { id: current.id });
        if (s.status === 'failed') throw new Error(s.error || 'core 进程启动失败');
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: 'running' } : i)));
        addLog(`[${current.name}] 网络已启动，RPC ${current.rpcPort}`);
      }
    } catch (e) {
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: 'failed' } : i)));
      addLog(`[${current.name}] 操作失败：${String(e)}`);
      await appAlert(`网络操作失败：${String(e)}`);
      // Refresh state so a self-exited core transitions out of "failed" cleanly.
      invoke('get_instance_state', { id: current.id }).catch(() => undefined);
    }
  };

  useEffect(() => {
    let unlistenCopy: UnlistenFn | undefined;
    let unlistenToggle: UnlistenFn | undefined;
    let unlistenResume: UnlistenFn | undefined;
    let unlistenVisibility: UnlistenFn | undefined;

    void listen<boolean>('app-window-visibility', (event) => {
      const visible = Boolean(event.payload);
      setIsWindowVisible(visible);
      if (visible) setPollEpoch(n => n + 1);
    }).then(fn => { unlistenVisibility = fn; });

    void listen('tray-copy-ip', () => {
      if (activeVirtualIp) {
        void navigator.clipboard.writeText(activeVirtualIp);
        showToast(`✓ 已从托盘复制虚拟 IP: ${activeVirtualIp}`);
      } else {
        showToast('虚拟 IP 尚未分配或网络未运行');
      }
    }).then(fn => { unlistenCopy = fn; });

    void listen('tray-toggle-network', () => {
      void toggle();
    }).then(fn => { unlistenToggle = fn; });

    void listen('system-power-resumed', () => {
      console.log('[Power] 系统自休眠唤醒，触发网络自愈感知...');
      showToast('⚡ 检测到系统休眠唤醒，正在自愈重连 EasyTier 网络...');
      if (serviceMode) {
        void refreshService();
      } else {
        for (const inst of instances) {
          if (inst.status === 'running') {
            void invoke<{ status: Status; error?: string }>('get_instance_state', { id: inst.id }).then(st => {
              if (st.status !== 'running') {
                showToast(`正在自动恢复网络「${inst.name}」...`);
                const toml = encodeTOML(inst.config);
                void invoke('start_instance', {
                  id: inst.id,
                  config: toml,
                  rpcPortal: inst.remoteManageEnabled ? undefined : `127.0.0.1:${inst.rpcPort}`,
                  remoteManageEnabled: inst.remoteManageEnabled ?? false,
                  rpcWhitelistCidrs: inst.rpcWhitelistCidrs ?? [],
                }).catch(() => {});
              }
            }).catch(() => {});
          }
        }
      }
    }).then(fn => { unlistenResume = fn; });

    return () => {
      if (unlistenCopy) unlistenCopy();
      if (unlistenToggle) unlistenToggle();
      if (unlistenResume) unlistenResume();
      if (unlistenVisibility) unlistenVisibility();
    };
  }, [activeVirtualIp, toggle, serviceMode, instances]);

  const importToml = async (text: string) => {
    try {
      const config = decodeTOML(text);
      // A different network must not reuse the current listener ports or the
      // core will fail with a port conflict; keep the imported listeners only
      // when they differ from the current ones.
      const sameListeners =
        config.listener_urls.length === current.config.listener_urls.length &&
        config.listener_urls.every((u, i) => u === current.config.listener_urls[i]);
      if (sameListeners) config.listener_urls = listenersForInstance(instances.indexOf(current) === 0 ? 0 : instances.indexOf(current));
      patchConfig(config);
      addLog(`TOML 配置导入成功（监听器 ${config.listener_urls.join(', ')}）`);
      await appAlert(`配置导入成功\n\n监听器：${config.listener_urls.join('\n')}\n\n若与其它实例端口冲突，请在监听器列表中修改端口后重新启动网络。`);
    } catch (e) {
      addLog(`TOML 导入失败：${String(e)}`);
      await appAlert(`导入失败：${String(e)}`);
    }
  };

  const exportToml = async (copy = false) => {
    const hasSecret = !!current.config.network_secret;
    if (copy && hasSecret && !secretVisible) {
      if (!(await appConfirm('配置中包含网络密钥，确定复制到剪贴板吗？'))) return;
    }
    const text = encodeTOML(current.config, true);
    try {
      if (copy) {
        await copyText(text, 'TOML 已复制到剪贴板');
      } else {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(new Blob([text], { type: 'application/toml' }));
        a.download = `${current.name || 'easytier'}.toml`;
        a.click();
        URL.revokeObjectURL(a.href);
        showToast('✓ TOML 已导出为文件');
      }
      addLog(copy ? 'TOML 已复制到剪贴板' : 'TOML 已导出为文件');
    } catch (e) { await appAlert(`导出失败：${String(e)}`); }
  };

  const openTomlFile = async () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.toml,text/plain';
    input.onchange = async () => {
      const f = input.files?.[0];
      if (f) await importToml(await f.text());
    };
    input.click();
  };

  const startTomlEdit = () => {
    if (tomlDraft === null || !tomlDirty) setTomlDraft(encodeTOML(current.config));
    setTomlDirty(false);
    setTomlError(null);
  };

  const applyTomlDraft = () => {
    if (tomlDraft === null) return;
    try {
      const config = decodeTOML(tomlDraft);
      const errors = validateConfig(config);
      if (errors.length) throw new Error(errors[0].message);
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, config } : i)));
      setTomlDraft(null);
      setTomlError(null);
      addLog(`[${current.name}] TOML 已应用到表单`);
    } catch (e) {
      setTomlError(String(e instanceof Error ? e.message : e));
    }
  };

  const revertTomlDraft = () => {
    setTomlDraft(encodeTOML(current.config));
    setTomlError(null);
  };

  const cancelKernelUpdate = async () => {
    try {
      if (serviceMode) {
        await serviceRequest('cancel_kernel_update').catch(() => {});
      }
      await invoke('cancel_kernel_update').catch(() => {});
      setKernelUpdate({
        phase: 'cancelled',
        downloaded_bytes: 0,
        total_bytes: null,
        percent: 0,
        message: '已取消内核更新',
      });
      showToast('✓ 已取消内核更新');
    } catch (e) {
      showToast(`取消失败：${String(e)}`);
    }
  };

  const checkKernelUpdate = async () => {
    setKernelInfo(null);
    try {
      const res = await invoke<KernelUpdateInfo>('check_kernel_update', { proxy: effectiveKernelProxy });
      setKernelInfo(res);
      if (res.available_versions && res.available_versions.length > 0) {
        setAvailableKernelVersions(res.available_versions);
        if (!selectedKernelVersion) setSelectedKernelVersion(res.available_versions[0]);
      } else {
        const list = await invoke<string[]>('list_kernel_versions', { proxy: effectiveKernelProxy });
        if (list.length > 0) {
          setAvailableKernelVersions(list);
          if (!selectedKernelVersion) setSelectedKernelVersion(list[0]);
        }
      }
    } catch (e) {
      setKernelInfo({ current_version: runtime?.version ?? 'unknown', update_available: false, error: String(e) });
    }
  };

  const switchKernel = async (targetVer?: string) => {
    if (kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)) return;
    const targetTag = targetVer || selectedKernelVersion || (kernelInfo?.latest_version ? `v${kernelInfo.latest_version}` : undefined);
    if (!targetTag) return;
    if (!(await appConfirm(`将切换/更新 EasyTier 内核至 ${targetTag}，操作期间会停止并自动重启当前运行中的网络。继续吗？`))) return;
    const runningInstances = instances.filter(i => i.status === 'running').map(i => ({ id: i.id, config: encodeTOML(i.config), rpc_port: i.rpcPort, remote_manage_enabled: i.remoteManageEnabled ?? false, rpc_whitelist_cidrs: i.rpcWhitelistCidrs ?? [] }));
    setKernelUpdate({ phase: 'checking', downloaded_bytes: 0, total_bytes: null, percent: 0, message: `正在准备切换至 ${targetTag}` });
    try {
      let serviceTaskTerminal = false;
      if (serviceMode) {
        const response = await serviceRequest<KernelUpdateTaskResponse>('update_kernel', { proxy: effectiveKernelProxy, target_version: targetTag });
        const taskId = response.task_id;
        const update: Partial<KernelUpdateProgress> = response.progress || (response.phase ? {
          phase: response.phase,
          downloaded_bytes: response.downloaded_bytes ?? 0,
          total_bytes: response.total_bytes,
          percent: response.percent,
          message: response.message || '',
          error: kernelErrorText(response.error),
        } : response.result) || {};
        const status = response.status?.toLowerCase();
        serviceTaskTerminal = isKernelTerminal(status) || isKernelTerminal(update?.phase);
        const phase = serviceTaskTerminal ? (status === 'failed' || status === 'error' || update?.phase === 'failed' ? 'failed' : status === 'cancelled' || update?.phase === 'cancelled' ? 'cancelled' : 'completed') : update?.phase;
        setKernelUpdate(currentProgress => mergeKernelProgress(currentProgress, {
          ...update,
          ...(phase ? { phase } : {}),
          ...(kernelErrorText(response.error) ? { error: kernelErrorText(response.error) } : {}),
        }, taskId));
      } else {
        await invoke<KernelUpdateInfo>('update_kernel', { proxy: effectiveKernelProxy, targetVersion: targetTag, instances: runningInstances });
      }
      if (!serviceMode || serviceTaskTerminal) {
        const fresh = await invoke<KernelUpdateInfo>('check_kernel_update', { proxy: effectiveKernelProxy });
        setKernelInfo(fresh);
        if (fresh.available_versions) setAvailableKernelVersions(fresh.available_versions);
        if (serviceMode) await refreshService();
        else setInstances(xs => xs.map(i => runningInstances.some(r => r.id === i.id) ? { ...i, status: 'running' } : i));
        showToast(`✓ 内核已成功切换至 ${targetTag}`);
      }
    } catch (e) {
      const errStr = String(e);
      const isCancel = errStr.includes('取消');
      setKernelUpdate({
        phase: isCancel ? 'cancelled' : 'failed',
        downloaded_bytes: 0,
        total_bytes: null,
        percent: 0,
        message: isCancel ? '内核切换已取消' : '内核切换失败',
        error: isCancel ? undefined : errStr,
      });
    }
  };

  const updateKernel = () => switchKernel();

  if (!current) return null;
  const running = current.status === 'running';
  const statusText = { running: '网络运行中', stopped: '网络已停止', starting: '正在启动…', stopping: '正在停止…', failed: '启动失败' }[current.status];

  const navItems: [Tab, string][] = [['status', '状态总览'], ['peers', '组网成员'], ['routes', '路由信息'], ['config', '组网配置'], ['logs', '运行日志'], ['settings', '设置']];

  return (
    <>
      <DialogHost />
      {toast && <div className="app-toast">{toast}</div>}
      <main className="app-shell">
      <aside>
        <div className="brand"><span className="brand-mark"><img src={easytierLogo} alt="EasyTier" draggable={false} /></span><div><strong>EasyTier</strong><small>Windows Client</small></div></div>
        <div className="section-label">网络实例</div>
        <nav>
          {instances.map(i => (
            <button className={i.id === current.id ? 'nav-item active' : 'nav-item'} onClick={() => { setActiveId(i.id); if (tab === 'settings') setTab('status'); }} key={i.id}>
              <span className={i.status === 'running' ? 'dot on' : i.status === 'failed' ? 'dot err' : i.status === 'starting' || i.status === 'stopping' ? 'dot connecting' : 'dot'} /><span className="nav-name">{i.name}</span>
              <span className="chevron">›</span>
            </button>
          ))}
        </nav>
        <button className="add-button" onClick={addInstance}><IconPlus size={14} /> 新建实例</button>
        <div className="sidebar-bottom">
          <button className="quiet" onClick={() => setTab('settings')}><IconGear size={14} /> 设置</button>
          <div className="version-block">
            <span className="version-row">核心版本：{runtime?.version ? runtime.version.split(' ').pop() : '未检测'}</span>
            <span className="version-row">客户端版本：v0.1.0</span>
          </div>
        </div>
      </aside>

      <section className="content">
        {serviceRecovery && (
          <div className="kernel-progress-banner">
            <div className="kernel-progress-head"><strong>后台服务恢复</strong><span>{SERVICE_RECOVERY_TEXT[serviceRecovery]}</span></div>
            <div className="kernel-progress-track"><div className="kernel-progress-bar indeterminate" /></div>
            <small className="kernel-progress-msg">恢复期间正常运行中的网络不会被重启；服务就绪后将直接接管并同步状态。</small>
          </div>
        )}
        {kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase) && tab !== 'settings' && (
          <div className="kernel-progress-banner">
            <div className="kernel-progress-head">
              <strong>EasyTier 内核更新中</strong>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ cursor: 'pointer' }} onClick={() => setTab('settings')}>{KERNEL_PHASE_TEXT[kernelUpdate.phase] || kernelUpdate.phase} · 前往设置 ›</span>
                <button
                  type="button"
                  className="mini-button danger"
                  style={{ padding: '2px 8px', fontSize: 11 }}
                  onClick={(e) => { e.stopPropagation(); void cancelKernelUpdate(); }}
                >
                  取消
                </button>
              </div>
            </div>
            <div className="kernel-progress-track">
              <div className="kernel-progress-bar" style={{ width: `${kernelUpdate.percent ?? 0}%` }} />
            </div>
            <small className="kernel-progress-msg">{kernelUpdate.message}{kernelUpdate.percent != null ? ` · ${kernelUpdate.percent}%` : ''}</small>
          </div>
        )}
        <div className="mode-segmented-card">
          <div className={`mode-module ${serviceMode && !serviceChecking ? 'active' : ''}`}>
            <div className="mode-module-header">
              <span className={`mode-badge ${serviceMode && !serviceChecking ? 'on' : 'off'}`}>
                {serviceChecking ? '⏳ 正在检测' : serviceMode ? '🟢 服务模式 (推荐)' : '⚪ 服务模式'}
              </span>
              {serviceMode && !serviceChecking && <span className="mode-tag active-tag">当前运行中</span>}
            </div>
            <p className="mode-desc">
              {serviceChecking
                ? '正在探测 EasyTier Service 守护服务连接…'
                : serviceMode
                ? 'Windows 后台服务守护运行，多用户共享、系统自启且低权限静默管理。'
                : 'Windows 服务未运行或未安装，当前由兼容模式接管。'}
            </p>
          </div>

          <div className={`mode-module ${!serviceMode && !serviceChecking ? 'active' : ''}`}>
            <div className="mode-module-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span className={`mode-badge ${!serviceMode && !serviceChecking ? 'on-compat' : 'off'}`}>
                  {!serviceMode && !serviceChecking ? '🟠 兼容模式' : '⚪ 兼容模式'}
                </span>
                {!serviceMode && !serviceChecking && <span className="mode-tag active-tag compat-tag">当前运行中</span>}
              </div>
              {!serviceMode && !serviceChecking && (
                <button type="button" className="mini-button ghost" onClick={() => setTab('settings')}>
                  服务设置 ›
                </button>
              )}
            </div>
            <p className="mode-desc">
              {!serviceMode && !serviceChecking
                ? '后台服务不可用，当前由客户端直接管理内核进程；启停需管理员权限。'
                : '应急备用模式，在后台服务异常时可直接拉起内核子进程。'}
            </p>
          </div>
        </div>
        <header>
          <div>
            <p className="eyebrow">{navItems.find(([t]) => t === tab)?.[1]}</p>
            <h1>{current.name}</h1>
          </div>
          <div className="header-actions">
            {running && <span className="rpc-badge">RPC :{current.rpcPort}</span>}
              <button className={running ? 'stop' : 'primary'} onClick={() => void toggle()} disabled={current.status === 'starting' || current.status === 'stopping' || (!!kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase))}>
              {running ? <IconStop size={14} /> : <IconPlay size={14} />}
              {running ? '停止网络' : '启动网络'}
            </button>
          </div>
        </header>

        <nav className="tabbar">
          {navItems.map(([t, label]) => (
            <button key={t} className={tab === t ? 'tab active' : 'tab'} onClick={() => setTab(t)}>{label}</button>
          ))}
        </nav>

        {tab === 'status' && (
          <>
            <div className={`status-card status-${running ? 'connected' : current.status === 'failed' ? 'error' : current.status === 'starting' || current.status === 'stopping' ? 'connecting' : 'muted'}`}>
              <div className="status-icon">{running ? '✓' : current.status === 'failed' ? '!' : current.status === 'starting' || current.status === 'stopping' ? '…' : '−'}</div>
              <div>
                <span className="card-label">当前状态</span>
                <h2>{statusText}</h2>
                <p>{running ? `RPC 管理端口 ${current.remoteManageEnabled ? `0.0.0.0（已允许远程管理）:${current.rpcPort}` : `127.0.0.1:${current.rpcPort}`}${node?.version ? ` · 核心 ${node.version}` : ''}` : '启动网络后，设备将加入 EasyTier 虚拟网络。'}</p>
                {running && !node?.ipv4_addr && (
                  <p className="warn-inline">⚠ 本机尚未获得虚拟 IPv4 —— 请在「组网配置 → 基础配置」勾选 DHCP 或手动填写虚拟 IPv4，然后重启网络（TUN 需要管理员权限运行客户端）。</p>
                )}
              </div>
              <div className="status-meta">
                <span>本机虚拟地址</span>
                <b
                  style={{ cursor: (node?.ipv4_addr || current.config.virtual_ipv4) ? 'pointer' : 'default', display: 'inline-flex', alignItems: 'center', gap: 6 }}
                  title="点击复制虚拟 IP"
                  onClick={() => {
                    const ip = node?.ipv4_addr || current.config.virtual_ipv4;
                    if (ip) {
                      const clean = ip.split('/')[0].trim();
                      void copyText(clean, `已复制本机虚拟 IP: ${clean}`);
                    }
                  }}
                >
                  {node?.ipv4_addr || (current.config.dhcp ? '等待分配' : current.config.virtual_ipv4 || '—')}
                  {(node?.ipv4_addr || current.config.virtual_ipv4) && <IconCopy size={13} style={{ opacity: 0.55 }} />}
                </b>
              </div>
            </div>
            <div className="metrics">
              <article><span>组网成员</span><strong>{running ? peers.length : '—'}</strong><small>{running ? '在线节点' : '未运行'}</small></article>
              <article><span>路由条目</span><strong>{running ? routes.length : '—'}</strong><small>{running ? '已知网段' : '未运行'}</small></article>
              <article><span>累计收发<button type="button" className="mini-button" style={{ marginLeft: 8, padding: '2px 8px', fontSize: 10 }} onClick={clearTraffic} title="清零累计统计">清零</button></span><strong>{running ? formatBytes(instanceTraffic.rx + instanceTraffic.tx) : '—'}</strong><small>累计值（重连不丢失）</small></article>
            </div>

            {running && (
              <div className="card traffic-card">
                <div className="card-title-row">
                  <div className="traffic-title-left">
                    <h3 className="card-title">实时网络流量</h3>
                    <span className="hint-inline">1 秒级采样 · 近 30 秒动态面积图</span>
                  </div>
                  <div className="traffic-speed-badges">
                    <span className="speed-badge rx" title="实时下行速率">
                      <span className="speed-dot rx" /> ↓ {formatSpeed(currentRxSpeed)}
                    </span>
                    <span className="speed-badge tx" title="实时上行速率">
                      <span className="speed-dot tx" /> ↑ {formatSpeed(currentTxSpeed)}
                    </span>
                    <span className="speed-badge peak" title="近 30 秒峰值速率">
                      ★ 峰值 {formatSpeed(peakSpeed)}
                    </span>
                  </div>
                </div>
                <TrafficAreaChart
                  history={trafficHistory}
                  currentRxSpeed={currentRxSpeed}
                  currentTxSpeed={currentTxSpeed}
                  peakSpeed={peakSpeed}
                />
              </div>
            )}

            <div className="section-heading"><div><h2>快速操作</h2><p>常用配置与信息入口</p></div></div>
            <div className="config-grid">
              <button className="config-card" onClick={() => setTab('config')}><span className="config-icon"><IconGlobe size={15} /></span><div><b>组网配置</b><small>网络名称、密钥、地址与高级参数</small></div><span>›</span></button>
              <button className="config-card" onClick={() => setTab('peers')}><span className="config-icon"><IconUsers size={15} /></span><div><b>组网成员</b><small>查看在线节点与连接质量</small></div><span>›</span></button>
              <button className="config-card" onClick={() => setTab('logs')}><span className="config-icon"><IconTerminal size={15} /></span><div><b>运行日志</b><small>启动、停止与错误记录</small></div><span>›</span></button>
            </div>
          </>
        )}

        {tab === 'peers' && (
          <div className="card">
            <div className="card-title-row">
              <h3 className="card-title">组网成员{running ? `（${visiblePeers.length}${showPeerNodes ? '' : `，已隐藏 ${peers.length - visiblePeers.length} 个 Peer`}）` : ''}</h3>
              <div className="title-actions">
                <span className="hint-inline">刷新 {refreshSecs}s</span>
                <button className="ghost" onClick={() => setShowDisplaySettings(x => !x)}><IconSliders size={13} /> 显示设置</button>
              </div>
            </div>
            {showDisplaySettings && (
              <div className="display-settings">
                <div className="display-cols">
                  <span className="field-label">数据项</span>
                  <div className="display-col-list">
                    <label className="check-item">
                      <input type="checkbox" checked={showPeerNodes} onChange={e => setShowPeerNodes(e.target.checked)} />
                      显示 Peer 节点
                    </label>
                    {PEER_COLUMNS.map(c => (
                      <label key={c.key} className="check-item">
                        <input type="checkbox" checked={visibleCols.includes(c.key)}
                          onChange={e => setVisibleCols(xs => {
                            const next = e.target.checked ? [...xs, c.key] : xs.filter(x => x !== c.key);
                            localStorage.setItem('easytier.peer-cols.v2', JSON.stringify(next));
                            return next;
                          })} />
                        {c.label}
                      </label>
                    ))}
                  </div>
                </div>
                <div className="display-refresh">
                  <span className="field-label">刷新</span>
                  <select className="field-input narrow" value={refreshSecs}
                    onChange={e => setRefreshSecs(parseInt(e.target.value, 10) as RefreshInterval)}>
                    <option value={1}>1 秒</option>
                    <option value={3}>3 秒</option>
                    <option value={5}>5 秒</option>
                    <option value={10}>10 秒</option>
                    <option value={30}>30 秒</option>
                  </select>
                </div>
              </div>
            )}
            {!running && <p className="list-empty">网络未运行，启动后此处显示在线成员。</p>}
            {running && !curSnap.peers.length && !curSnap.node && <p className="list-empty">正在获取实例状态…（实例启动中或短暂无响应时会自动恢复）</p>}
            {running && (
              <div className="table-scroll">
              <table className="data-table">
                <thead><tr>
                  {visibleCols.includes('nodeid') && <th>节点 ID</th>}
                  {visibleCols.includes('ipv4') && <th>IPv4</th>}
                  {visibleCols.includes('cidr') && <th>网段</th>}
                  {visibleCols.includes('hostname') && <th>主机名</th>}
                  {visibleCols.includes('cost') && <th>穿透方式</th>}
                  {visibleCols.includes('proto') && <th>协议</th>}
                  {visibleCols.includes('latency') && <th>延迟</th>}
                  {visibleCols.includes('loss') && <th>丢包率</th>}
                  {visibleCols.includes('rx') && <th>下载</th>}
                  {visibleCols.includes('tx') && <th>上传</th>}
                  {visibleCols.includes('nat') && <th>Nat类型</th>}
                  {visibleCols.includes('version') && <th>内核版本</th>}
                  {visibleCols.includes('relay') && <th>中继节点</th>}
                  {visibleCols.includes('routes') && <th>子网路由</th>}
                  <th>操作</th>
                </tr></thead>
                <tbody>
                  {visiblePeers.map((p, i) => {
                    const route = routes.find(r => r.hostname === p.hostname);
                    const isLocal = p.cost === 'Local';
                    const nodeType = isLocal ? '本机' : (route && (route.path_len ?? 0) > 1 ? '服务节点' : '普通节点');
                    const remoteIp = p.ipv4 || route?.ipv4?.split('/')[0];
                    return (
                      <tr key={String(p.id ?? i)}>
                        {visibleCols.includes('nodeid') && <td>{String(p.id ?? '—')}<div className="cell-sub">{nodeType}</div></td>}
                        {visibleCols.includes('ipv4') && (
                          <td>
                            <span
                              style={{ cursor: p.ipv4 ? 'pointer' : 'default', display: 'inline-flex', alignItems: 'center', gap: 4 }}
                              title={p.ipv4 ? '点击复制 IP' : undefined}
                              onClick={() => {
                                if (p.ipv4) {
                                  const clean = p.ipv4.split('/')[0].trim();
                                  void copyText(clean, `已复制对端 IP: ${clean}`);
                                }
                              }}
                            >
                              {p.ipv4 || '—'}
                              {p.ipv4 && <IconCopy size={11} style={{ opacity: 0.5 }} />}
                            </span>
                            {!isLocal && remoteIp && <div className="cell-sub">{nodeType}</div>}
                          </td>
                        )}
                        {visibleCols.includes('cidr') && <td>{p.cidr || '—'}</td>}
                        {visibleCols.includes('hostname') && (
                          <td>
                            {inlineRename?.peerId === (p.id ?? i) ? (
                              <div className="inline-rename-cell">
                                <input
                                  className="field-input inline-edit-input"
                                  autoFocus
                                  disabled={inlineRename.saving}
                                  value={inlineRename.currentName}
                                  onChange={e => setInlineRename({ ...inlineRename, currentName: e.target.value })}
                                  onKeyDown={e => {
                                    if (e.key === 'Enter') {
                                      e.preventDefault();
                                      void submitRename();
                                    } else if (e.key === 'Escape') {
                                      setInlineRename(null);
                                    }
                                  }}
                                  onBlur={() => {
                                    if (!inlineRename.saving) setInlineRename(null);
                                  }}
                                />
                                {inlineRename.saving && <span className="inline-spin">…</span>}
                              </div>
                            ) : (
                              <span
                                className={!isLocal && remoteIp ? 'peer-hostname-cell' : undefined}
                                title={!isLocal && remoteIp ? '双击远程改名 (Enter 确认, Esc 取消)' : undefined}
                                onDoubleClick={() => {
                                  if (!isLocal && remoteIp) {
                                    const cleanIp = remoteIp.split('/')[0].trim();
                                    setInlineRename({
                                      peerId: p.id ?? i,
                                      originalName: p.hostname || '',
                                      currentName: p.hostname || '',
                                      host: cleanIp,
                                      saving: false,
                                    });
                                  }
                                }}
                              >
                                {p.hostname || '—'}
                                {!isLocal && remoteIp && <IconPencil size={11} className="rename-hint-icon" />}
                              </span>
                            )}
                          </td>
                        )}
                        {visibleCols.includes('cost') && <td><span className={`route-badge tone-${routeTone(p.cost)}`}>{p.cost || '—'}</span></td>}
                        {visibleCols.includes('proto') && <td>{p.tunnel_proto || '—'}</td>}
                        {visibleCols.includes('latency') && <td className={`tone-${latencyTone(p.lat_ms)}`}>{p.lat_ms || '—'}</td>}
                        {visibleCols.includes('loss') && <td>{p.loss_rate || '—'}</td>}
                        {visibleCols.includes('rx') && <td>{formatBytes(p.rx_bytes)}</td>}
                        {visibleCols.includes('tx') && <td>{formatBytes(p.tx_bytes)}</td>}
                        {visibleCols.includes('nat') && <td>{p.nat_type || '—'}</td>}
                        {visibleCols.includes('version') && <td>{p.version || '—'}</td>}
                        {visibleCols.includes('relay') && <td><span className={`route-badge tone-${routeTone(route && (route.path_len ?? 0) > 1 ? `Relay (${route.path_len})` : 'Local')}`}>{route && (route.path_len ?? 0) > 1 ? route.next_hop_hostname : '—'}</span></td>}
                        {visibleCols.includes('routes') && <td>{route?.proxy_cidrs && route.proxy_cidrs !== '' ? String(route.proxy_cidrs) : '—'}</td>}
                        <td>
                          {!isLocal && remoteIp ? (
                            <button
                              type="button"
                              className="mini-button ghost"
                              style={{ padding: '2px 8px', fontSize: 11 }}
                              onClick={() => {
                                const cleanIp = remoteIp.split('/')[0].trim();
                                const candidatePorts = getCandidatePortsForIp(cleanIp, instances, current.rpcPort);
                                const initialPort = candidatePorts[0] || 15888;
                                setRemoteConfigTarget({ host: cleanIp, port: initialPort, candidatePorts });
                              }}
                              title="远程配置该设备的主机名/虚拟IP/代理网段/出口"
                            >
                              远程配置
                            </button>
                          ) : (
                            <span style={{ opacity: 0.35, fontSize: 11 }}>—</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                  {visiblePeers.length === 0 && <tr><td colSpan={15} className="list-empty">暂无可显示成员</td></tr>}
                </tbody>
              </table>
              </div>
            )}
          </div>
        )}

        {tab === 'routes' && (
          <div className="card">
            <h3 className="card-title">路由信息{running ? `（${routes.length}）` : ''}</h3>
            {!running && <p className="list-empty">网络未运行，启动后此处显示路由表。</p>}
            {running && (
              <div className="table-scroll">
              <table className="data-table">
                <thead><tr><th>虚拟地址</th><th>主机名</th><th>下一跳</th><th>跳数</th><th>路径延迟</th><th>子网代理</th><th>版本</th></tr></thead>
                <tbody>
                  {routes.map((r, i) => (
                    <tr key={i}>
                      <td>{r.ipv4 || '—'}</td>
                      <td>{r.hostname || '—'}</td>
                      <td>{r.next_hop_hostname || r.next_hop_ipv4 || '—'}</td>
                      <td>{r.path_len ?? '—'}</td>
                      <td className={`tone-${latencyTone(r.path_latency)}`}>{r.path_latency ? `${r.path_latency} ms` : '—'}</td>
                      <td>{r.proxy_cidrs && String(r.proxy_cidrs) !== '' ? String(r.proxy_cidrs) : '—'}</td>
                      <td>{r.version || '—'}</td>
                    </tr>
                  ))}
                  {routes.length === 0 && <tr><td colSpan={7} className="list-empty">暂无路由</td></tr>}
                </tbody>
              </table>
              </div>
            )}
          </div>
        )}

        {tab === 'config' && (
          <div className="card config-card-wide">
              <div className="card-title-row">
                <input className="title-input" value={current.name} onChange={e => renameInstance(e.target.value)} />
                {configSaved && <span className="save-status" role="status">✓ 已自动保存</span>}
              <div className="title-actions">
                <button className="ghost" onClick={() => void openTomlFile()}><IconUpload size={13} /> 导入 TOML</button>
                <button className="ghost" onClick={async () => { try { await importToml(await navigator.clipboard.readText()); } catch (e) { await appAlert(`读取剪贴板失败：${String(e)}`); } }}><IconClipboard size={13} /> 剪贴板导入</button>
                <button className="ghost" onClick={() => void exportToml()}><IconDownload size={13} /> 导出 TOML</button>
                <button className="ghost" onClick={() => void exportToml(true)}><IconCopy size={13} /> 复制</button>
                <button className="mini-button danger" onClick={() => removeInstance(current.id)}><IconTrash size={12} /> 删除实例</button>
              </div>
            </div>
            <ConfigEditor config={current.config} onChange={patchConfig} showAdvanced={showAdvanced} onToggleAdvanced={() => setShowAdvanced(x => !x)} />
            <details className="toml-preview" onToggle={e => { if ((e.target as HTMLDetailsElement).open) startTomlEdit(); }}>
              <summary>查看当前 TOML（可编辑）</summary>
              <textarea
                className="toml-editor"
                value={tomlDraft ?? encodeTOML(current.config)}
                onChange={e => { setTomlDraft(e.target.value); setTomlDirty(true); }}
                spellCheck={false}
                rows={18}
              />
              {tomlError && <p className="toml-error">✗ {tomlError}</p>}
              <div className="toml-actions">
                <button className="primary" onClick={applyTomlDraft}>应用更改</button>
                <button className="ghost" onClick={revertTomlDraft}>还原</button>
              </div>
            </details>
          </div>
        )}

        

        {tab === 'logs' && (
          <div className="card">
            <div className="card-title-row">
              <div className="segmented" role="tablist" aria-label="日志类型">
                <button className={logView === 'runtime' ? 'seg active' : 'seg'} onClick={() => setLogView('runtime')}>运行日志</button>
                <button className={logView === 'network' ? 'seg active' : 'seg'} onClick={() => setLogView('network')}>组网日志</button>
              </div>
              <button className="ghost" onClick={() => logView === 'runtime' ? setLogsByInstance(m => ({ ...m, [current.id]: [] })) : clearNetworkLogs()}>清空</button>
            </div>
            <div className="log-pane" ref={el => { if (el && logTimer.current == null) el.scrollTop = el.scrollHeight; }}>
              {(logView === 'runtime' ? logsByInstance[current.id] ?? [] : networkLogs).length === 0 ? <p className="list-empty">暂无{logView === 'runtime' ? '运行' : '组网'}日志</p> : (logView === 'runtime' ? logsByInstance[current.id] ?? [] : networkLogs).map((l, i) => <div className="log-line" key={i}>{l}</div>)}
            </div>
          </div>
        )}

        {tab === 'settings' && (
          <>
            <div className="card service-card">
              <div className="card-title-row"><h3 className="card-title">后台服务</h3><span className={serviceMode ? 'service-state online' : 'service-state'}>{serviceMode ? '服务模式' : '兼容模式'}</span></div>
              <p className="hint">{service?.message || (service?.installed ? (service?.running ? '服务正在运行。' : '服务已安装但尚未运行。') : '未检测到 EasyTierService。')}</p>
              {serviceResult && <p className={serviceResult.ok ? 'hint' : 'list-empty err'} style={{ marginTop: 6 }}>{serviceResult.ok ? '✓ ' : '✗ '}{serviceResult.text}</p>}
              <div className="service-actions">
                {!service?.installed && <button className="primary" disabled={serviceBusy} onClick={async () => { setServiceBusy(true); setServiceResult(null); try { await invoke('install_service'); setServiceResult({ ok: true, text: '后台服务已安装并启动。' }); await refreshService({ skipAutoStart: true }); } catch (e) { setServiceResult({ ok: false, text: `安装服务失败：${String(e)}` }); } finally { setServiceBusy(false); } }}>{serviceBusy ? '正在安装…' : '安装后台服务'}</button>}
                {service?.installed && !service?.running && <button className="primary" disabled={serviceBusy} onClick={async () => { setServiceBusy(true); try { await invoke('start_service'); await refreshService(); } catch (e) { await appAlert(`启动服务失败：${String(e)}`); } finally { setServiceBusy(false); } }}>启动服务</button>}
                {service?.installed && <button className="ghost" disabled={serviceBusy} onClick={async () => { setServiceBusy(true); setServiceResult(null); try { await invoke('repair_service'); setServiceResult({ ok: true, text: '后台服务已修复并启动。' }); await refreshService(); } catch (e) { setServiceResult({ ok: false, text: `修复服务失败：${String(e)}` }); } finally { setServiceBusy(false); } }}>修复服务</button>}
              </div>
            </div>
            <div className="card">
              <h3 className="card-title">运行时</h3>
              <table className="kv-table">
                <tbody>
                  <tr><th>核心路径</th><td>{runtime?.core_path ?? 'core/easytier-core.exe'}</td></tr>
                  <tr><th>核心版本</th><td>{runtime?.version ?? '未检测'}</td></tr>
                  <tr><th>运行时可用</th><td>{runtime?.available ? '是' : '否（请检查 core 目录）'}</td></tr>
                </tbody>
              </table>
            </div>
            <div className="card">
              <div className="card-title-row">
                <h3 className="card-title">EasyTier 内核版本管理与切换</h3>
                <span className="hint-inline">当前版本: {kernelInfo?.current_version || runtime?.version || '未检测'}</span>
              </div>
              <div className="kernel-update-controls">
                <label className="field"><span className="field-label">GitHub 下载线路</span>
                  <select className="field-input" value={kernelProxy} onChange={e => setKernelProxy(e.target.value)} disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}>
                    {KERNEL_PROXIES.map(p => <option value={p.value} key={p.value}>{p.label}</option>)}
                  </select>
                </label>
                {kernelProxy === 'custom' && (
                  <label className="field" style={{ minWidth: 260 }}>
                    <span className="field-label">自定义镜像前缀 (如 https://ghproxy.net)</span>
                    <input
                      className="field-input"
                      placeholder="https://your-mirror.example.com"
                      value={customKernelProxy}
                      onChange={e => setCustomKernelProxy(e.target.value)}
                      disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}
                    />
                  </label>
                )}
                <label className="field"><span className="field-label">目标核心版本</span>
                  <select
                    className="field-input"
                    value={selectedKernelVersion}
                    onChange={e => setSelectedKernelVersion(e.target.value)}
                    disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}
                  >
                    {availableKernelVersions.length === 0 ? (
                      <option value="">{kernelInfo?.latest_version ? `v${kernelInfo.latest_version} (最新稳定版)` : '点击「获取版本列表」'}</option>
                    ) : (
                      availableKernelVersions.map(ver => (
                        <option value={ver} key={ver}>
                          {ver}{ver === `v${kernelInfo?.latest_version}` || ver === kernelInfo?.latest_version ? ' (最新稳定版)' : ''}
                        </option>
                      ))
                    )}
                  </select>
                </label>
                <div className="kernel-update-actions">
                  <button className="ghost" onClick={() => void checkKernelUpdate()} disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}>
                    {availableKernelVersions.length > 0 ? '刷新版本列表' : '获取版本列表'}
                  </button>
                  <button
                    className="primary"
                    onClick={() => void switchKernel(selectedKernelVersion)}
                    disabled={
                      (!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)) ||
                      (!selectedKernelVersion && !kernelInfo?.update_available)
                    }
                  >
                    {selectedKernelVersion ? `切换/安装 ${selectedKernelVersion}` : kernelInfo?.update_available ? `更新到 v${kernelInfo.latest_version}` : '重新安装当前版本'}
                  </button>
                </div>
              </div>
              {kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase) && (
                <div className="kernel-card-progress">
                  <div className="kernel-progress-head">
                    <strong>更新进度</strong>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                      <span>{KERNEL_PHASE_TEXT[kernelUpdate.phase] || kernelUpdate.phase}</span>
                      <button
                        type="button"
                        className="mini-button danger"
                        style={{ padding: '2px 8px', fontSize: 11 }}
                        onClick={() => void cancelKernelUpdate()}
                      >
                        取消更新
                      </button>
                    </div>
                  </div>
                  <div className="kernel-progress-track">
                    <div className="kernel-progress-bar" style={{ width: `${kernelUpdate.percent ?? 0}%` }} />
                  </div>
                  <small className="kernel-progress-msg">{kernelUpdate.message}{kernelUpdate.percent != null ? ` · ${kernelUpdate.percent}%` : ''}</small>
                </div>
              )}
              {kernelInfo?.error && <p className="list-empty err">检查失败：{kernelInfo.error}</p>}
              {kernelInfo && !kernelInfo.error && !kernelInfo.update_available && !selectedKernelVersion && <p className="hint">当前已是最新正式版本。可通过上方下拉选择历史版本进行自由切换。</p>}
              {kernelUpdate?.phase === 'failed' && <p className="list-empty err">操作失败：{kernelUpdate.error || kernelUpdate.message}</p>}
              {kernelUpdate?.phase === 'cancelled' && <p className="hint">已取消内核更新操作。</p>}
              {kernelUpdate?.phase === 'completed' && <p className="hint">内核切换/更新完成，原来运行中的网络已尝试自动恢复。</p>}
            </div>
            <div className="card">
              <p className="hint">每个实例使用独立 RPC 端口，避免多实例冲突。启动网络后可通过 easytier-cli 连接该端口查询状态。</p>
              <table className="kv-table">
                <tbody>
                  {instances.map(i => (
                    <tr key={i.id}>
                      <th>{i.name}</th>
                      <td className="inline">
                        <input className="field-input narrow" type="number" value={i.rpcPort} min={1024} max={65535}
                          onChange={e => setInstances(xs => xs.map(x => (x.id === i.id ? { ...x, rpcPort: parseInt(e.target.value, 10) || x.rpcPort } : x)))} />
                        <span className="hint-inline">{i.status === 'running' ? '运行中' : '已停止'}</span>
                        <button className={i.autoStart ? 'switch on' : 'switch'} role="switch" aria-checked={i.autoStart ?? false} title="开机自动启动" onClick={async () => {
                          const next = !(i.autoStart ?? false);
                          if (serviceMode) { try { await serviceRequest('set_auto_start', { instance_id: i.id, auto_start: next }); } catch (e) { await appAlert(`更新自动启动失败：${String(e)}`); return; } }
                          setInstances(xs => xs.map(x => x.id === i.id ? { ...x, autoStart: next } : x));
                        }}><span className="knob" /></button><span className="hint-inline">自动启动</span>
                        <button className={i.remoteManageEnabled ? 'switch on' : 'switch'} role="switch" aria-checked={i.remoteManageEnabled ?? false}
                          title="允许虚拟网内其他设备修改本实例配置（RPC 监听 0.0.0.0）"
                          onClick={() => setInstances(xs => xs.map(x => (x.id === i.id ? { ...x, remoteManageEnabled: !(x.remoteManageEnabled ?? false) } : x)))}
                        ><span className="knob" /></button><span className="hint-inline">允许远程管理</span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {current?.remoteManageEnabled && (
                <label className="field">
                  <span className="field-label">RPC 白名单（当前实例，逗号分隔 CIDR，环回地址自动附加）</span>
                  <input className="field-input" value={(current.rpcWhitelistCidrs ?? []).join(',')}
                    placeholder="10.126.126.0/24"
                    onChange={e => setInstances(xs => xs.map(x => (x.id === current.id
                      ? { ...x, rpcWhitelistCidrs: e.target.value.split(',').map(s => s.trim()).filter(Boolean) }
                      : x)))} />
                  <span className="hint-inline">开启后 RPC 监听 0.0.0.0，会暴露到物理局域网；白名单务必只填 EasyTier 虚拟网段。</span>
                </label>
              )}
            </div>
            <div className="card">
              <h3 className="card-title">关于</h3>
              <p className="hint">EasyTier Windows Client 0.1.0 · Tauri 2 + React · 设计文档见仓库 docs/ 目录。</p>
              <label className="toggle-row"><span className="field-label">显示密钥（导出时提醒）</span>
                <button className={secretVisible ? 'switch on' : 'switch'} role="switch" aria-checked={secretVisible} onClick={() => setSecretVisible(v => !v)}><span className="knob" /></button>
              </label>
            </div>
          </>
        )}
      </section>
    </main>
      {remoteConfigTarget && (
        <RemoteConfigDialog
          host={remoteConfigTarget.host}
          port={remoteConfigTarget.port}
          candidatePorts={remoteConfigTarget.candidatePorts}
          onSaved={(patched, effectivePort) => {
            saveNodeRpcPort(remoteConfigTarget.host, effectivePort);
            // Synchronize back to local instance config if target is a local instance!
            setInstances(prev => {
              const next = prev.map(inst => {
                const cleanVirtual = inst.config.virtual_ipv4 ? inst.config.virtual_ipv4.split('/')[0].trim() : '';
                const isMatch = inst.rpcPort === effectivePort
                  || cleanVirtual === remoteConfigTarget.host
                  || (inst.id === current.id && !remoteConfigTarget.host);
                if (isMatch) {
                  const updated = {
                    ...inst,
                    config: {
                      ...inst.config,
                      hostname: patched.hostname || inst.config.hostname,
                      virtual_ipv4: patched.ipv4Addr ? `${patched.ipv4Addr}/${patched.ipv4Len}` : inst.config.virtual_ipv4,
                      proxy_cidrs: patched.proxyCidrs.filter(Boolean),
                    },
                  };
                  if (serviceMode) {
                    void serviceRequest('sync_instance', {
                      instance_id: updated.id,
                      name: updated.name,
                      config_toml: encodeTOML(updated.config),
                      rpc_port: updated.rpcPort,
                      auto_start: updated.autoStart ?? false,
                      desired_state: updated.status === 'running' ? 'running' : 'stopped',
                      remote_manage_enabled: updated.remoteManageEnabled ?? false,
                      rpc_whitelist_cidrs: updated.rpcWhitelistCidrs ?? [],
                    }).catch(() => {});
                  }
                  return updated;
                }
                return inst;
              });
              localStorage.setItem('easytier.instances.v2', JSON.stringify(next));
              return next;
            });
            // Optimistically update current peers/routes in memory
            setStatusByInstance(prev => {
              const cur = prev[current.id];
              if (!cur) return prev;
              return {
                ...prev,
                [current.id]: {
                  ...cur,
                  peers: cur.peers.map(p => {
                    const pIp = p.ipv4 ? p.ipv4.split('/')[0].trim() : '';
                    return pIp === remoteConfigTarget.host ? { ...p, hostname: patched.hostname, ipv4: patched.ipv4Addr } : p;
                  }),
                  routes: cur.routes.map(r => {
                    const rIp = r.ipv4 ? r.ipv4.split('/')[0].trim() : '';
                    return rIp === remoteConfigTarget.host ? { ...r, hostname: patched.hostname, ipv4: patched.ipv4Addr, proxy_cidrs: patched.proxyCidrs.join(', ') } : r;
                  }),
                },
              };
            });
            showToast(`✓ 配置已在远端生效，并已同步到本地记录 (RPC :${effectivePort})`);
          }}
          onClose={() => setRemoteConfigTarget(null)}
        />
      )}
    </>
  );
}
