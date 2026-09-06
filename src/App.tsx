import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { defaultConfig, listenersForInstance, NetworkConfig, validateConfig } from './network-config';
import { decodeTOML, encodeTOML } from './toml-codec';
import { ConfigEditor } from './ConfigEditor';
import { NodeStatus, PeerColumn, PeerInfo, PEER_COLUMNS, RefreshInterval, RouteInfo, formatBytes, latencyTone, parseHumanBytes, parseNodeJSON, parsePeerJSON, parseRouteJSON, routeTone } from './status-data';
import { RemoteConfigDialog, RemoteConfigEditor } from './RemoteConfigDialog';
import { IconClipboard, IconCopy, IconDownload, IconGear, IconPlay, IconPlus, IconRefresh, IconRemote, IconSliders, IconStop, IconTerminal, IconTrash, IconUpload, IconUsers, IconGlobe } from './icons';

function RemoteConfigPanel({ host, running, peers, onPickPeer, localIp }: {
  host: string | null;
  running: boolean;
  peers: PeerInfo[];
  onPickPeer: (ip: string) => void;
  localIp?: string;
}) {
  if (!running) return <div className="card"><p className="list-empty">网络未运行。启动网络后才能访问远端节点的 RPC。</p></div>;
  return (
    <>
      <div className="card">
        <h3 className="card-title">选择远端节点</h3>
        {peers.length === 0 && <p className="list-empty">暂无在线远端节点。</p>}
        <div className="remote-peer-list">
          {peers.map(p => {
            const ip = p.ipv4 || '—';
            return (
              <button key={String(p.id ?? ip)} type="button" className={host === ip ? 'remote-peer active' : 'remote-peer'}
                onClick={() => onPickPeer(ip)}>
                <strong>{p.hostname || '未命名节点'}</strong>
                <span className="hint-inline">{ip} · {p.version || '—'}</span>
              </button>
            );
          })}
        </div>
      </div>
      {host && <div className="card"><h3 className="card-title">远程配置 — {host}</h3><RemoteConfigEditor key={host} host={host} /></div>}
    </>
  );
}
import { getServiceStatus, serviceRequest, ServiceInstanceState, ServiceStatus } from './service-client';

type Status = 'running' | 'stopped' | 'starting' | 'stopping' | 'failed';
type Tab = 'status' | 'peers' | 'routes' | 'config' | 'remote' | 'logs' | 'settings';

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

interface runtimeInfo { available: boolean; version: string; core_path?: string }

interface KernelUpdateInfo { current_version: string; latest_version?: string | null; asset_name?: string | null; update_available: boolean; error?: string | null }
interface KernelUpdateProgress { task_id?: string | null; phase: string; downloaded_bytes: number; total_bytes?: number | null; percent?: number | null; current_file?: string | null; message: string; error?: string | null }
interface KernelUpdateTaskResponse { task_id?: string | null; status?: string; progress?: Partial<KernelUpdateProgress> | null; result?: Partial<KernelUpdateProgress> | null; error?: string | { message?: string } | null }
const KERNEL_TERMINAL_PHASES = ['completed', 'failed'];
const KERNEL_TERMINAL_STATUSES = ['completed', 'complete', 'success', 'succeeded', 'failed', 'error'];
const isKernelTerminal = (value?: string | null): boolean => !!value && KERNEL_TERMINAL_STATUSES.includes(value.toLowerCase());
const kernelErrorText = (error: KernelUpdateTaskResponse['error']): string | undefined => typeof error === 'string' ? error : error?.message;
const mergeKernelProgress = (base: KernelUpdateProgress | null, update: Partial<KernelUpdateProgress> | null | undefined, taskId?: string | null): KernelUpdateProgress => ({
  phase: update?.phase || base?.phase || 'checking', downloaded_bytes: update?.downloaded_bytes ?? base?.downloaded_bytes ?? 0,
  total_bytes: update?.total_bytes ?? base?.total_bytes ?? null, percent: update?.percent ?? base?.percent ?? 0,
  current_file: update?.current_file ?? base?.current_file ?? null, message: update?.message || base?.message || '正在更新内核',
  error: update?.error ?? base?.error, task_id: update?.task_id ?? taskId ?? base?.task_id ?? null,
});
const KERNEL_PROXIES = [
  { value: 'direct', label: '直连' },
  { value: 'https://ghfast.top', label: 'https://ghfast.top/' },
  { value: 'https://v6.gh-proxy.org', label: 'https://v6.gh-proxy.org/' },
  { value: 'https://hk.gh-proxy.org', label: 'https://hk.gh-proxy.org/' },
  { value: 'https://cdn.gh-proxy.org', label: 'https://cdn.gh-proxy.org/' },
  { value: 'https://edgeone.gh-proxy.org', label: 'https://edgeone.gh-proxy.org/' },
];

const KERNEL_PHASE_TEXT: Record<string, string> = {
  checking: '检查版本', downloading: '下载内核', extracting: '校验并解压', stopping: '停止网络', installing: '替换内核', restarting: '恢复网络', completed: '更新完成', failed: '更新失败',
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
  const [remoteTabHost, setRemoteTabHost] = useState<string | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [runtime, setRuntime] = useState<runtimeInfo | null>(null);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [routes, setRoutes] = useState<RouteInfo[]>([]);
  const [node, setNode] = useState<NodeStatus | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [networkLogs, setNetworkLogs] = useState<string[]>([]);
  const [logView, setLogView] = useState<'runtime' | 'network'>('runtime');
  const [isElevated, setIsElevated] = useState<boolean | null>(null);
  const [elevationNoticeShown, setElevationNoticeShown] = useState(false);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [secretVisible, setSecretVisible] = useState(false);
  const [visibleCols, setVisibleCols] = useState<PeerColumn[]>(() =>
    load('easytier.peer-cols.v2', PEER_COLUMNS.filter(c => c.defaultOn).map(c => c.key)));
  const [refreshSecs, setRefreshSecs] = useState<RefreshInterval>(() => load('easytier.refresh.v1', 5));
  const [showDisplaySettings, setShowDisplaySettings] = useState(false);
  const [showPeerNodes, setShowPeerNodes] = useState<boolean>(() => load('easytier.show-peer-nodes.v1', false));
  const [remoteEditHost, setRemoteEditHost] = useState<string | null>(null);
  const [tomlDraft, setTomlDraft] = useState<string | null>(null);
  const [tomlDirty, setTomlDirty] = useState(false);
  const [tomlError, setTomlError] = useState<string | null>(null);
  const [configSaved, setConfigSaved] = useState(false);
  const [kernelUpdate, setKernelUpdate] = useState<KernelUpdateProgress | null>(null);
  const [kernelInfo, setKernelInfo] = useState<KernelUpdateInfo | null>(null);
  const [kernelProxy, setKernelProxy] = useState<string>(() => load('easytier.kernel-update-proxy.v1', 'direct'));
  const [service, setService] = useState<ServiceStatus | null>(null);
  const [serviceBusy, setServiceBusy] = useState(false);
  const serviceMode = service?.running === true && service?.healthy !== false;
  const serviceInstalled = service?.installed === true;
  const logTimer = useRef<number | null>(null);
  const kernelTaskId = kernelUpdate?.task_id ?? null;

  const current = useMemo(() => instances.find(i => i.id === activeId) ?? instances[0], [instances, activeId]);
  const visiblePeers = useMemo(() => {
    if (showPeerNodes || !current) return peers;
    return peers.filter(peer => !String(peer.hostname ?? '').toLowerCase().startsWith('publicserver'));
  }, [current, peers, showPeerNodes]);

  useEffect(() => { localStorage.setItem('easytier.instances.v2', JSON.stringify(instances)); }, [instances]);
  useEffect(() => { if (current) localStorage.setItem('easytier.active.v2', current.id); }, [current]);
  useEffect(() => { invoke<runtimeInfo>('detect_runtime', {}).then(v => setRuntime({ ...v, core_path: v.core_path ?? 'core/easytier-core.exe' })).catch(() => setRuntime(null)); }, []);
  const refreshService = async () => {
    try {
      const installed = await invoke<{ installed: boolean; running: boolean; message?: string }>('query_service_installation');
      if (!installed.installed) {
        setService({ installed: false, running: false, message: installed.message });
        return;
      }
      if (!installed.running) {
        // Auto-start an installed-but-stopped service so the user does not
        // have to visit settings after a reboot or manual service stop.
        try {
          await invoke('start_service');
          await new Promise(r => setTimeout(r, 800));
        } catch {
          setService({ installed: true, running: false, message: '服务已安装但尚未运行。' });
          return;
        }
      }
      const next = await getServiceStatus();
      setService({ ...next, installed: true, running: true });
      if (next.running && next.healthy !== false) {
        const states = await serviceRequest<ServiceInstanceState[]>('list_instances');
        setInstances(xs => xs.map(i => {
          const s = states.find(x => x.id === i.id);
          return s ? { ...i, name: s.name || i.name, rpcPort: s.rpc_port ?? i.rpcPort, status: s.observed_state, autoStart: s.auto_start, desiredState: s.desired_state, lastError: s.last_error, remoteManageEnabled: s.remote_manage_enabled ?? i.remoteManageEnabled, rpcWhitelistCidrs: s.rpc_whitelist_cidrs ?? i.rpcWhitelistCidrs } : i;
        }));
      }
    } catch (e) {
      setService({ installed: false, running: false, message: String(e) });
    }
  };
  useEffect(() => { void refreshService(); }, []);

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
    if (current?.status !== 'running') { clearNetworkLogs(); return; }
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
  }, [current?.id, current?.status, serviceMode, refreshSecs]);

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
        const update = response.progress || response.result;
        const status = response.status?.toLowerCase();
        const terminal = isKernelTerminal(status) || isKernelTerminal(update?.phase);
        const phase = terminal ? (status === 'failed' || status === 'error' || update?.phase === 'failed' ? 'failed' : 'completed') : update?.phase;
        const next = mergeKernelProgress(kernelUpdate, { ...update, ...(phase ? { phase } : {}), ...(kernelErrorText(response.error) ? { error: kernelErrorText(response.error) } : {}) }, kernelTaskId);
        setKernelUpdate(next);
        if (!terminal && alive) timer = window.setTimeout(() => void poll(), 1000);
      } catch {
        if (alive) timer = window.setTimeout(() => void poll(), 1500);
      }
    };
    void poll();
    return () => { alive = false; if (timer != null) window.clearTimeout(timer); };
  }, [serviceMode, kernelTaskId, kernelUpdate?.phase]);
  useEffect(() => {
    void invoke<KernelUpdateInfo>('check_kernel_update', { proxy: kernelProxy }).then(setKernelInfo).catch(e => setKernelInfo({ current_version: runtime?.version ?? 'unknown', update_available: false, error: String(e) }));
  // Runtime detection and the initial update check intentionally run once per app launch.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Poll peer/route status while an instance is running. Self-scheduling
  // timeout: the next round starts only after the previous one settles, so a
  // hung CLI round can never pile up overlapping rounds. Skipped when the
  // status-affecting tabs are not visible or the window is hidden — each
  // round spawns three CLI processes, so idle polling is real load.
  useEffect(() => {
    if (current?.status !== 'running') { setPeers([]); setRoutes([]); setNode(null); setStatusError(null); return; }
    const pollWorthy = tab === 'status' || tab === 'peers' || tab === 'routes';
    if (!pollWorthy || document.hidden) return;
    let alive = true;
    let timer: number | null = null;
    const refresh = async () => {
      try {
        // One persistent-connection RPC query replaces three CLI subprocess
        // spawns — ~5ms instead of ~0.5s of process overhead per poll.
        const snapshot = await invoke<{ peers: PeerInfo[]; routes: RouteInfo[]; node: NodeStatus }>('status_query', { port: current.rpcPort });
        if (!alive) return;
        setPeers(parsePeerJSON(JSON.stringify(snapshot.peers)));
        setRoutes(parseRouteJSON(JSON.stringify(snapshot.routes)));
        setNode(snapshot.node ?? null);
        setStatusError(null);
      } catch (e) {
        if (alive) setStatusError(String(e));
      } finally {
        if (alive) timer = window.setTimeout(() => void refresh(), refreshSecs * 1000);
      }
    };
    void refresh();
    return () => { alive = false; if (timer != null) window.clearTimeout(timer); };
  }, [current?.id, current?.status, current?.rpcPort, refreshSecs, tab, serviceMode, pollEpoch]);

  // Re-arm status polling when the window becomes visible again; the polling
  // effect above deliberately skips rounds while document.hidden is true.
  useEffect(() => {
    const onVisible = () => setPollEpoch(n => n + 1);
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, []);

  // Append runtime messages to the log pane.
  const addLog = (line: string) => setLogs(xs => [...xs.slice(-300), `${new Date().toLocaleTimeString()}  ${line}`]);
  useEffect(() => { if (statusError) addLog(`状态查询失败：${statusError}`); }, [statusError]);

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
    setInstances(xs => xs.filter(i => i.id !== id));
    if (activeId === id) setActiveId(instances.find(i => i.id !== id)!.id);
  };

  const renameInstance = (name: string) =>
    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, name } : i)));

  const toggle = async () => {
    if (kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase)) return;
    const running = current.status === 'running';
    if (!running && !serviceMode && isElevated === false && !elevationNoticeShown) {
      const message = '当前为普通权限运行，兼容模式启动网络可能失败。请右键客户端并选择“以管理员身份运行”。';
      addLog(`权限提示：${message}`);
      alert(message);
      sessionStorage.setItem('easytier.elevation-notice.v1', '1');
      setElevationNoticeShown(true);
    }
    if (!running) {
      // Pre-flight: detect listener port conflicts between instances so the
      // user can change the port instead of hitting an opaque core failure.
      const portOf = (url: string): number | null => {
        const m = url.match(/:(\d+)(?:\/.*)?$/);
        return m ? parseInt(m[1], 10) : null;
      };
      const myPorts = current.config.listener_urls.map(portOf).filter((p): p is number => p != null && p !== 0);
      const conflicts: string[] = [];
      for (const other of instances) {
        if (other.id === current.id || other.status !== 'running') continue;
        const otherPorts = new Set(other.config.listener_urls.map(portOf).filter((p): p is number => p != null));
        for (const p of myPorts) {
          if (otherPorts.has(p)) conflicts.push(`端口 ${p} 已被实例「${other.name}」占用`);
        }
      }
      const systemConflicts: string[] = [];
      for (const p of myPorts) {
        try {
          const busy = await invoke<boolean>('is_port_in_use', { port: p });
          if (busy) systemConflicts.push(`端口 ${p} 已被系统其它程序占用`);
        } catch { /* check unavailable — skip */ }
      }
      const all = [...new Set([...conflicts, ...systemConflicts])];
      if (all.length) {
        if (!confirm(`监听器端口冲突：\n\n${all.join('\n')}\n\n建议修改本实例监听器端口（或改用端口 0 自动分配）后再启动。仍要继续吗？`)) return;
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
      alert(`网络操作失败：${String(e)}`);
      // Refresh state so a self-exited core transitions out of "failed" cleanly.
      invoke('get_instance_state', { id: current.id }).catch(() => undefined);
    }
  };

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
      alert(`配置导入成功\n\n监听器：${config.listener_urls.join('\n')}\n\n若与其它实例端口冲突，请在监听器列表中修改端口后重新启动网络。`);
    } catch (e) {
      addLog(`TOML 导入失败：${String(e)}`);
      alert(`导入失败：${String(e)}`);
    }
  };

  const exportToml = async (copy = false) => {
    const hasSecret = !!current.config.network_secret;
    if (copy && hasSecret && !secretVisible) {
      if (!confirm('配置中包含网络密钥，确定复制到剪贴板吗？')) return;
    }
    const text = encodeTOML(current.config, true);
    try {
      if (copy) await navigator.clipboard.writeText(text);
      else {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(new Blob([text], { type: 'application/toml' }));
        a.download = `${current.name || 'easytier'}.toml`;
        a.click();
        URL.revokeObjectURL(a.href);
      }
      addLog(copy ? 'TOML 已复制到剪贴板' : 'TOML 已导出为文件');
    } catch (e) { alert(`导出失败：${String(e)}`); }
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

  const checkKernelUpdate = async () => {
    setKernelInfo(null);
    try { setKernelInfo(await invoke<KernelUpdateInfo>('check_kernel_update', { proxy: kernelProxy })); }
    catch (e) { setKernelInfo({ current_version: runtime?.version ?? 'unknown', update_available: false, error: String(e) }); }
  };

  const updateKernel = async () => {
    if (kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase)) return;
    if (!kernelInfo?.update_available) return;
    if (!confirm(`将更新 EasyTier 内核至 v${kernelInfo.latest_version}，更新期间会停止并自动重启当前运行中的网络。继续吗？`)) return;
    const runningInstances = instances.filter(i => i.status === 'running').map(i => ({ id: i.id, config: encodeTOML(i.config), rpc_port: i.rpcPort, remote_manage_enabled: i.remoteManageEnabled ?? false, rpc_whitelist_cidrs: i.rpcWhitelistCidrs ?? [] }));
    setKernelUpdate({ phase: 'checking', downloaded_bytes: 0, total_bytes: null, percent: 0, message: '正在准备更新' });
    try {
      let serviceTaskTerminal = false;
      if (serviceMode) {
        const response = await serviceRequest<KernelUpdateTaskResponse>('update_kernel', { proxy: kernelProxy });
        const taskId = response.task_id;
        const update = response.progress || response.result;
        const status = response.status?.toLowerCase();
        serviceTaskTerminal = isKernelTerminal(status) || isKernelTerminal(update?.phase);
        const phase = serviceTaskTerminal ? (status === 'failed' || status === 'error' || update?.phase === 'failed' ? 'failed' : 'completed') : update?.phase;
        setKernelUpdate(currentProgress => mergeKernelProgress(currentProgress, {
          ...update,
          ...(phase ? { phase } : {}),
          ...(kernelErrorText(response.error) ? { error: kernelErrorText(response.error) } : {}),
        }, taskId));
      } else {
        await invoke<KernelUpdateInfo>('update_kernel', { proxy: kernelProxy, instances: runningInstances });
      }
      if (!serviceMode || serviceTaskTerminal) {
        setKernelInfo(await invoke<KernelUpdateInfo>('check_kernel_update', { proxy: kernelProxy }));
        if (serviceMode) await refreshService();
        else setInstances(xs => xs.map(i => runningInstances.some(r => r.id === i.id) ? { ...i, status: 'running' } : i));
      }
    } catch (e) {
      setKernelUpdate({ phase: 'failed', downloaded_bytes: 0, total_bytes: null, percent: 0, message: '内核更新失败', error: String(e) });
    }
  };

  if (!current) return null;
  const running = current.status === 'running';
  const statusText = { running: '网络运行中', stopped: '网络已停止', starting: '正在启动…', stopping: '正在停止…', failed: '启动失败' }[current.status];

  const navItems: [Tab, string][] = [['status', '状态总览'], ['peers', '组网成员'], ['routes', '路由信息'], ['config', '组网配置'], ['remote', '远程管理'], ['logs', '运行日志'], ['settings', '设置']];

  return (
    <main className="app-shell">
      {kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase) && (
        <div className="kernel-progress-global">
          <div className="kernel-progress-head"><strong>EasyTier 内核更新</strong><span>{KERNEL_PHASE_TEXT[kernelUpdate.phase] || kernelUpdate.phase}</span></div>
          <div className="kernel-progress-track"><div className="kernel-progress-bar" style={{ width: `${kernelUpdate.percent ?? 0}%` }} /></div>
          <small>{kernelUpdate.message}{kernelUpdate.percent != null ? ` · ${kernelUpdate.percent}%` : ''}</small>
        </div>
      )}
      <aside>
        <div className="brand"><span className="brand-mark">E</span><div><strong>EasyTier</strong><small>Windows Client</small></div></div>
        <div className="section-label">网络实例</div>
        <nav>
          {instances.map(i => (
            <button className={i.id === current.id ? 'nav-item active' : 'nav-item'} onClick={() => { setActiveId(i.id); setTab('status'); }} key={i.id}>
              <span className={i.status === 'running' ? 'dot on' : i.status === 'failed' ? 'dot err' : i.status === 'starting' || i.status === 'stopping' ? 'dot connecting' : 'dot'} />{i.name}
              <span className="chevron">›</span>
            </button>
          ))}
        </nav>
        <button className="add-button" onClick={addInstance}><IconPlus size={14} /> 新建实例</button>
        <div className="sidebar-bottom">
          <button className="quiet" onClick={() => setTab('settings')}><IconGear size={14} /> 设置</button>
          <span className="version">{runtime?.version ? `核心 ${runtime.version.split(' ').pop()}` : '核心未检测'} · 客户端 0.1.0</span>
        </div>
      </aside>

      <section className="content">
        {!serviceMode && <div className="service-banner"><strong>兼容模式</strong><span>后台服务不可用，当前使用 GUI 直接管理内核；启停网络可能需要管理员权限。</span><button className="ghost" onClick={() => setTab('settings')}>查看服务设置</button></div>}
        {serviceMode && <div className="service-banner service-online"><strong>服务模式</strong><span>EasyTier Service 正在管理网络实例。</span></div>}
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
              <div className="status-meta"><span>本机虚拟地址</span><b>{node?.ipv4_addr || (current.config.dhcp ? '等待分配' : current.config.virtual_ipv4 || '—')}</b></div>
            </div>
            <div className="metrics">
              <article><span>组网成员</span><strong>{running ? peers.length : '—'}</strong><small>{running ? '在线节点' : '未运行'}</small></article>
              <article><span>路由条目</span><strong>{running ? routes.length : '—'}</strong><small>{running ? '已知网段' : '未运行'}</small></article>
              <article><span>收发总量</span><strong>{running ? formatBytes(peers.reduce((s, p) => s + parseHumanBytes(p.rx_bytes) + parseHumanBytes(p.tx_bytes), 0)) : '—'}</strong><small>所有成员合计</small></article>
            </div>
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
            {running && statusError && <p className="list-empty err">状态查询失败：{statusError}</p>}
            {running && !statusError && (
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
                </tr></thead>
                <tbody>
                  {visiblePeers.map((p, i) => {
                    const route = routes.find(r => r.hostname === p.hostname);
                    const isLocal = p.cost === 'Local';
                    const nodeType = isLocal ? '本机' : (route && (route.path_len ?? 0) > 1 ? '服务节点' : '普通节点');
                    const remoteIp = p.ipv4 || route?.ipv4?.split('/')[0];
                    const openRemote = () => {
                      if (isLocal || !remoteIp) return;
                      setRemoteTabHost(remoteIp);
                      setTab('remote');
                    };
                    return (
                      <tr key={String(p.id ?? i)}>
                        {visibleCols.includes('nodeid') && <td>{String(p.id ?? '—')}<div className="cell-sub">{nodeType}</div></td>}
                        {visibleCols.includes('ipv4') && <td>{p.ipv4 || '—'}{!isLocal && remoteIp && <div className="cell-sub">{nodeType}</div>}</td>}
                        {visibleCols.includes('cidr') && <td>{p.cidr || '—'}</td>}
                        {visibleCols.includes('hostname') && <td>{!isLocal && remoteIp
                          ? <button type="button" className="linklike" title="打开远程管理" onClick={openRemote}>{p.hostname || '—'}</button>
                          : (p.hostname || '—')}</td>}
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
                      </tr>
                    );
                  })}
                  {visiblePeers.length === 0 && <tr><td colSpan={14} className="list-empty">暂无可显示成员</td></tr>}
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
                <button className="ghost" onClick={async () => { try { await importToml(await navigator.clipboard.readText()); } catch (e) { alert(`读取剪贴板失败：${String(e)}`); } }}><IconClipboard size={13} /> 剪贴板导入</button>
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

        {tab === 'remote' && (
          <RemoteConfigPanel host={remoteTabHost} running={running} localIp={node?.ipv4_addr?.split('/')[0]} onPickPeer={(ip) => setRemoteTabHost(ip)} peers={visiblePeers.filter(p => {
            const ip = p.ipv4 || routes.find(r => r.hostname === p.hostname)?.ipv4?.split('/')[0];
            if (!ip) return false;
            if (p.cost === 'Local') return false;
            // DHCP may reassign our own virtual IP; the local node can still
            // linger in the peer list as a remote-cost entry — never offer it.
            const localIp = node?.ipv4_addr?.split('/')[0];
            if (localIp && ip === localIp) return false;
            return true;
          })} />
        )}

        {tab === 'logs' && (
          <div className="card">
            <div className="card-title-row">
              <div className="segmented" role="tablist" aria-label="日志类型">
                <button className={logView === 'runtime' ? 'seg active' : 'seg'} onClick={() => setLogView('runtime')}>运行日志</button>
                <button className={logView === 'network' ? 'seg active' : 'seg'} onClick={() => setLogView('network')}>组网日志</button>
              </div>
              <button className="ghost" onClick={() => logView === 'runtime' ? setLogs([]) : clearNetworkLogs()}>清空</button>
            </div>
            <div className="log-pane" ref={el => { if (el && logTimer.current == null) el.scrollTop = el.scrollHeight; }}>
              {(logView === 'runtime' ? logs : networkLogs).length === 0 ? <p className="list-empty">暂无{logView === 'runtime' ? '运行' : '组网'}日志</p> : (logView === 'runtime' ? logs : networkLogs).map((l, i) => <div className="log-line" key={i}>{l}</div>)}
            </div>
          </div>
        )}

        {tab === 'settings' && (
          <>
            <div className="card service-card">
              <div className="card-title-row"><h3 className="card-title">后台服务</h3><span className={serviceMode ? 'service-state online' : 'service-state'}>{serviceMode ? '服务模式' : '兼容模式'}</span></div>
              <p className="hint">{service?.message || (service?.installed ? (service?.running ? '服务正在运行。' : '服务已安装但尚未运行。') : '未检测到 EasyTierService。')}</p>
              <div className="service-actions">
                {!service?.installed && <button className="primary" disabled={serviceBusy} onClick={async () => { setServiceBusy(true); try { await invoke('install_service'); await refreshService(); } catch (e) { alert(`安装服务失败：${String(e)}`); } finally { setServiceBusy(false); } }}>安装后台服务</button>}
                {service?.installed && !service?.running && <button className="primary" disabled={serviceBusy} onClick={async () => { setServiceBusy(true); try { await invoke('start_service'); await refreshService(); } catch (e) { alert(`启动服务失败：${String(e)}`); } finally { setServiceBusy(false); } }}>启动服务</button>}
                {service?.installed && <button className="ghost" disabled={serviceBusy} onClick={async () => { setServiceBusy(true); try { await invoke('repair_service'); await refreshService(); } catch (e) { alert(`修复服务失败：${String(e)}`); } finally { setServiceBusy(false); } }}>修复服务</button>}
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
                <h3 className="card-title">EasyTier 内核更新</h3>
                <span className="hint-inline">当前 v{kernelInfo?.current_version || runtime?.version || '未检测'}</span>
              </div>
              <div className="kernel-update-controls">
                <label className="field"><span className="field-label">GitHub 下载线路</span>
                  <select className="field-input" value={kernelProxy} onChange={e => setKernelProxy(e.target.value)} disabled={!!kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase)}>
                    {KERNEL_PROXIES.map(p => <option value={p.value} key={p.value}>{p.label}</option>)}
                  </select>
                </label>
                <div className="kernel-update-actions">
                  <button className="ghost" onClick={() => void checkKernelUpdate()} disabled={!!kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase)}>检查更新</button>
                  {kernelInfo?.update_available && <button className="primary" onClick={() => void updateKernel()} disabled={!!kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase)}>更新到 v{kernelInfo.latest_version}</button>}
                </div>
              </div>
              {kernelInfo?.error && <p className="list-empty err">检查失败：{kernelInfo.error}</p>}
              {kernelInfo && !kernelInfo.error && !kernelInfo.update_available && <p className="hint">当前已是最新正式版本。</p>}
              {kernelUpdate?.phase === 'failed' && <p className="list-empty err">更新失败：{kernelUpdate.error || kernelUpdate.message}</p>}
              {kernelUpdate?.phase === 'completed' && <p className="hint">内核更新完成，原来运行中的网络已尝试自动恢复。</p>}
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
                          if (serviceMode) { try { await serviceRequest('set_auto_start', { instance_id: i.id, auto_start: next }); } catch (e) { alert(`更新自动启动失败：${String(e)}`); return; } }
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
      {remoteEditHost && <RemoteConfigDialog host={remoteEditHost} onClose={() => setRemoteEditHost(null)} />}
    </main>
  );
}
