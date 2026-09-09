import React, { useEffect, useMemo, useRef, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { defaultConfig, NetworkConfig, validateConfig } from './network-config';
import { decodeTOML, encodeTOML } from './toml-codec';
import {
  AutoStartClientStatus,
  Instance,
  InstanceSnapshot,
  load,
  runtimeInfo,
  Status,
  Tab,
} from './types';
import { PeerColumn, PEER_COLUMNS, RefreshInterval } from './status-data';
import { formatSpeed } from './TrafficAreaChart';
import { RemoteConfigDialog } from './RemoteConfigDialog';
import { serviceRequest } from './service-client';
import { DialogHost, appAlert, appConfirm } from './dialogs';

// Custom Hooks
import { useInstances } from './hooks/useInstances';
import { useService } from './hooks/useService';
import { useTraffic } from './hooks/useTraffic';
import { useKernelUpdate } from './hooks/useKernelUpdate';

// Subcomponents
import { Sidebar } from './components/common/Sidebar';
import { HeaderActions } from './components/common/HeaderActions';
import { StatusTab } from './components/tabs/StatusTab';
import { PeersTab } from './components/tabs/PeersTab';
import { RoutesTab } from './components/tabs/RoutesTab';
import { ConfigTab } from './components/tabs/ConfigTab';
import { LogsTab } from './components/tabs/LogsTab';
import { SettingsTab } from './components/tabs/SettingsTab';

export default function App() {
  const [tab, setTab] = useState<Tab>('status');
  const [pollEpoch, setPollEpoch] = useState(0);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [runtime, setRuntime] = useState<runtimeInfo | null>(null);
  const [statusByInstance, setStatusByInstance] = useState<Record<string, InstanceSnapshot>>({});
  const [logsByInstance, setLogsByInstance] = useState<Record<string, string[]>>({});
  const [isElevated, setIsElevated] = useState<boolean | null>(null);
  const [secretVisible, setSecretVisible] = useState(false);
  const [visibleCols, setVisibleCols] = useState<PeerColumn[]>(() =>
    load('easytier.peer-cols.v2', PEER_COLUMNS.filter(c => c.defaultOn).map(c => c.key))
  );
  const [refreshSecs, setRefreshSecs] = useState<RefreshInterval>(() => load('easytier.refresh.v1', 5));
  const [showDisplaySettings, setShowDisplaySettings] = useState(false);
  const [showPeerNodes, setShowPeerNodes] = useState<boolean>(() => load('easytier.show-peer-nodes.v1', false));
  const [remoteConfigTarget, setRemoteConfigTarget] = useState<{
    host: string;
    port: number;
    candidatePorts?: number[];
  } | null>(null);
  const [isWindowVisible, setIsWindowVisible] = useState<boolean>(() => !document.hidden);

  const [toast, setToast] = useState<string | null>(null);
  const toastTimerRef = useRef<number | null>(null);

  const showToast = useCallback((message: string) => {
    if (toastTimerRef.current) {
      window.clearTimeout(toastTimerRef.current);
    }
    setToast(message);
    toastTimerRef.current = window.setTimeout(() => {
      setToast(null);
      toastTimerRef.current = null;
    }, 2200);
  }, []);

  const copyText = useCallback(async (text: string, label?: string) => {
    try {
      await navigator.clipboard.writeText(text);
      showToast(label ? `✓ ${label}` : `✓ 已复制: ${text}`);
    } catch (e) {
      showToast(`复制失败：${String(e)}`);
    }
  }, [showToast]);

  const addLog = useCallback((line: string) => {
    const timestamp = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    setLogsByInstance(m => {
      const id = currentRef.current?.id;
      if (!id) return m;
      return { ...m, [id]: [...(m[id] ?? []).slice(-300), `[${timestamp}] ${line}`] };
    });
  }, []);

  // 1. Instances Hook
  const {
    instances,
    setInstances,
    activeId,
    setActiveId,
    current,
    configSaved,
    addInstance,
    removeInstance,
    renameInstance,
    patchConfig,
  } = useInstances(
    addLog,
    showToast,
    (id: string) => clearInstanceTraffic(id)
  );

  // 2. Service Hook (直接操作真实的 setInstances)
  const {
    service,
    serviceChecking,
    serviceBusy,
    setServiceBusy,
    serviceResult,
    setServiceResult,
    serviceRecovery,
    serviceMode,
    wasServiceModeRef,
    refreshService,
  } = useService(setInstances, isWindowVisible);

  // 3. Traffic Hook
  const {
    instanceTraffic,
    trafficHistory,
    currentRxSpeed,
    currentTxSpeed,
    peakSpeed,
    clearInstanceTraffic,
    recordPollResults,
  } = useTraffic(instances, current, isWindowVisible, tab);

  // 启动全链路自启动：在非服务模式下，监听 instances 列表变化自动拉起
  const attemptedAutoConnectIds = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (serviceChecking || serviceMode || instances.length === 0) return;
    const toStart = instances.filter(
      i => i.autoStart && i.status !== 'running' && !attemptedAutoConnectIds.current.has(i.id)
    );
    if (toStart.length === 0) return;
    for (const inst of toStart) {
      attemptedAutoConnectIds.current.add(inst.id);
      void (async () => {
        try {
          addLog(`[${inst.name}] 检测到已开启自启动，正在自动连接网络…`);
          clearInstanceTraffic(inst.id);
          const toml = encodeTOML(inst.config);
          await invoke('start_instance', {
            id: inst.id,
            config: toml,
            rpcPortal: inst.remoteManageEnabled ? undefined : `127.0.0.1:${inst.rpcPort}`,
            remoteManageEnabled: inst.remoteManageEnabled ?? false,
            rpcWhitelistCidrs: inst.rpcWhitelistCidrs ?? [],
          });
          await new Promise(r => setTimeout(r, 1200));
          const s = await invoke<{ status: Status; error?: string }>('wait_for_exit', { id: inst.id });
          if (s.status === 'failed') {
            addLog(`[${inst.name}] 自动启动失败: ${s.error || 'core 启动异常'}`);
          } else {
            setInstances(xs => xs.map(i => (i.id === inst.id ? { ...i, status: 'running' } : i)));
            addLog(`[${inst.name}] 网络已自动启动运行`);
            showToast(`✓ 已自动连接网络: ${inst.name}`);
          }
        } catch (err) {
          addLog(`[${inst.name}] 自动连接异常: ${String(err)}`);
        }
      })();
    }
  }, [instances, serviceChecking, serviceMode, addLog, showToast, clearInstanceTraffic, setInstances]);

  const currentRef = useRef(current);
  currentRef.current = current;

  // 4. Kernel Update Hook
  const {
    kernelUpdate,
    kernelInfo,
    availableKernelVersions,
    selectedKernelVersion,
    setSelectedKernelVersion,
    kernelProxy,
    setKernelProxy,
    customKernelProxy,
    setCustomKernelProxy,
    effectiveKernelProxy,
    checkKernelUpdate,
    switchKernel,
    cancelKernelUpdate,
  } = useKernelUpdate(
    instances,
    serviceMode,
    refreshService,
    showToast,
    tab,
    runtime?.version
  );

  // 5. Windows Autostart Client Status
  const [clientAutoStart, setClientAutoStart] = useState<AutoStartClientStatus>({
    enabled: false,
    start_minimized: false,
  });

  useEffect(() => {
    try {
      localStorage.removeItem('easytier.traffic.v1');
    } catch {}
    void invoke<AutoStartClientStatus>('get_client_autostart')
      .then(s => setClientAutoStart(s))
      .catch(() => {});
  }, []);

  const updateClientAutoStart = useCallback(async (enabled: boolean, startMinimized: boolean) => {
    try {
      await invoke('set_client_autostart', { enabled, startMinimized });
      setClientAutoStart({ enabled, start_minimized: startMinimized, path_mismatch: false });
      showToast(enabled ? '✓ 已开启开机自启动客户端' : '✓ 已关闭开机自启动客户端');
    } catch (e) {
      await appAlert(`设置开机自启失败：${String(e)}`);
    }
  }, [showToast]);

  // Window hidden optimization attribute
  useEffect(() => {
    if (isWindowVisible) {
      document.documentElement.removeAttribute('data-window-hidden');
    } else {
      document.documentElement.setAttribute('data-window-hidden', 'true');
    }
  }, [isWindowVisible]);

  // Window blur memory trim & launch memory trim
  useEffect(() => {
    const t = setTimeout(() => {
      void invoke('trim_memory').catch(() => {});
    }, 3200);
    const onBlur = () => {
      void invoke('trim_memory').catch(() => {});
    };
    window.addEventListener('blur', onBlur);
    return () => {
      clearTimeout(t);
      window.removeEventListener('blur', onBlur);
    };
  }, []);

  // Runtime environment detection
  useEffect(() => {
    invoke<runtimeInfo>('detect_runtime', {})
      .then(v => setRuntime({ ...v, core_path: v.core_path ?? 'core/easytier-core.exe' }))
      .catch(() => setRuntime(null));
  }, []);

  // UAC elevation check
  useEffect(() => {
    void invoke<boolean>('is_elevated')
      .then(v => setIsElevated(Boolean(v)))
      .catch(() => setIsElevated(null));
  }, []);

  // Service mode transition toast
  useEffect(() => {
    if (serviceChecking) return;
    if (serviceMode) {
      if (wasServiceModeRef.current === false) {
        showToast('🟢 后台服务已连接，当前运行于服务模式');
      }
      wasServiceModeRef.current = true;
    } else {
      if (wasServiceModeRef.current === true) {
        showToast('🟠 后台服务不可用，已切换为兼容模式');
      }
      wasServiceModeRef.current = false;
    }
  }, [serviceMode, serviceChecking, wasServiceModeRef, showToast]);

  const runningNow = current?.status === 'running';
  const curSnap = (runningNow ? statusByInstance[current?.id ?? ''] : undefined) ?? {
    peers: [],
    routes: [],
    node: null,
  };
  const peers = curSnap.peers;
  const routes = curSnap.routes;
  const node = curSnap.node;

  const visiblePeers = useMemo(() => {
    if (showPeerNodes || !current) return peers;
    return peers.filter(peer => !String(peer.hostname ?? '').toLowerCase().startsWith('publicserver'));
  }, [current, peers, showPeerNodes]);

  const activeVirtualIp = useMemo(() => {
    const ip = node?.ipv4_addr || current?.config?.virtual_ipv4;
    return ip ? ip.split('/')[0].trim() : null;
  }, [node?.ipv4_addr, current?.config?.virtual_ipv4]);

  // Sync state to system tray
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
  }, [current, activeVirtualIp, peers.length, currentRxSpeed, currentTxSpeed]);

  // Main status poller: unified single loop avoiding duplicate RPC process spawning
  useEffect(() => {
    const running = instances.filter(i => i.status === 'running' && i.rpcPort);
    if (running.length === 0) {
      setStatusByInstance({});
      return;
    }
    const pollWorthy = tab === 'status' || tab === 'peers' || tab === 'routes';
    if (!pollWorthy || !isWindowVisible) return;

    let alive = true;
    let timer: number | null = null;
    const refresh = async () => {
      const results = await Promise.all(
        running.map(async ({ id, rpcPort }) => {
          try {
            const snapshot = await invoke<InstanceSnapshot>('status_query', { port: rpcPort });
            return [id, snapshot] as const;
          } catch {
            return [id, null] as const;
          }
        })
      );
      if (!alive) return;
      setStatusByInstance(prev => {
        const next = { ...prev };
        for (const [id, snapshot] of results) {
          if (snapshot) next[id] = snapshot;
        }
        return next;
      });

      recordPollResults(results);

      // Status tab uses adaptive 1-2s sampling for smooth rate area chart; other tabs use refreshSecs
      const delay = tab === 'status' ? Math.min(refreshSecs, 2) * 1000 : refreshSecs * 1000;
      if (alive) {
        timer = window.setTimeout(() => void refresh(), delay);
      }
    };

    void refresh();
    return () => {
      alive = false;
      if (timer != null) window.clearTimeout(timer);
    };
  }, [instances, tab, refreshSecs, pollEpoch, serviceMode, isWindowVisible, recordPollResults]);

  // Window visibility listener
  useEffect(() => {
    const onVisibilityChange = () => {
      const visible = !document.hidden;
      setIsWindowVisible(visible);
      if (visible) setPollEpoch(n => n + 1);
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => document.removeEventListener('visibilitychange', onVisibilityChange);
  }, []);

  // Network Start/Stop Toggle
  const toggle = useCallback(async () => {
    if (!current) return;
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
      // 1. Auto-resolve RPC port conflict
      let assignedRpc = current.rpcPort;
      const otherRunning = instances.filter(i => i.id !== current.id && i.status === 'running');
      const occupiedRpcPorts = new Set(otherRunning.map(i => i.rpcPort));
      while (
        occupiedRpcPorts.has(assignedRpc) ||
        (await invoke<boolean>('is_port_in_use', { port: assignedRpc }).catch(() => false))
      ) {
        assignedRpc++;
      }
      if (assignedRpc !== current.rpcPort) {
        addLog(`[${current.name}] 探测到 RPC 端口 ${current.rpcPort} 已被占用，已自动调整为空闲端口 ${assignedRpc}`);
        current.rpcPort = assignedRpc;
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, rpcPort: assignedRpc } : i)));
      }

      // 2. Pre-flight listener conflicts
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
        } catch {}
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
          setInstances(xs =>
            xs.map(i => (i.id === current.id ? { ...i, config: { ...i.config, listener_urls: updatedUrls } } : i))
          );
          addLog(`[${current.name}] 已自动将冲突监听端口调整为动态端口 0`);
        } else {
          return;
        }
      }

      // 3. TUN adapter unique dev_name
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

    if (!running) {
      clearInstanceTraffic(current.id);
    }

    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: running ? 'stopping' : 'starting' } : i)));

    try {
      if (serviceMode) {
        await serviceRequest('sync_instance', {
          instance_id: current.id,
          name: current.name,
          config_toml: encodeTOML(current.config),
          rpc_port: current.rpcPort,
          auto_start: current.autoStart ?? false,
          desired_state: running ? 'stopped' : 'running',
          remote_manage_enabled: current.remoteManageEnabled ?? false,
          rpc_whitelist_cidrs: current.rpcWhitelistCidrs ?? [],
        });
        await serviceRequest(running ? 'stop_instance' : 'start_instance', { instance_id: current.id });
        if (running) invoke('drop_status_endpoint', { port: current.rpcPort }).catch(() => undefined);
        await refreshService();
        if (!running) {
          setTimeout(() => void refreshService(), 800);
        }
        addLog(`[${current.name}] 服务已${running ? '停止' : '启动'}网络`);
        return;
      }

      if (running) {
        const timeout = new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error('停止操作超时（5 秒）——可再次点击「停止网络」重试')), 5000)
        );
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
      invoke('get_instance_state', { id: current.id }).catch(() => undefined);
    }
  }, [current, instances, kernelUpdate, serviceMode, isElevated, addLog, clearInstanceTraffic, refreshService, setInstances]);

  // System Tray & Power Events
  useEffect(() => {
    let unlistenCopy: UnlistenFn | undefined;
    let unlistenToggle: UnlistenFn | undefined;
    let unlistenResume: UnlistenFn | undefined;
    let unlistenVisibility: UnlistenFn | undefined;

    void listen<boolean>('app-window-visibility', event => {
      const visible = Boolean(event.payload);
      setIsWindowVisible(visible);
      if (visible) setPollEpoch(n => n + 1);
    }).then(fn => {
      unlistenVisibility = fn;
    });

    void listen('tray-copy-ip', () => {
      if (activeVirtualIp) {
        void navigator.clipboard.writeText(activeVirtualIp);
        showToast(`✓ 已从托盘复制虚拟 IP: ${activeVirtualIp}`);
      } else {
        showToast('虚拟 IP 尚未分配或网络未运行');
      }
    }).then(fn => {
      unlistenCopy = fn;
    });

    void listen('tray-toggle-network', () => {
      void toggle();
    }).then(fn => {
      unlistenToggle = fn;
    });

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
    }).then(fn => {
      unlistenResume = fn;
    });

    return () => {
      if (unlistenCopy) unlistenCopy();
      if (unlistenToggle) unlistenToggle();
      if (unlistenResume) unlistenResume();
      if (unlistenVisibility) unlistenVisibility();
    };
  }, [activeVirtualIp, toggle, serviceMode, instances, refreshService, showToast]);

  const navItems: [Tab, string][] = [
    ['status', '状态概览'],
    ['peers', '组网成员'],
    ['routes', '路由信息'],
    ['config', '组网配置'],
    ['logs', '运行日志'],
    ['settings', '客户端设置'],
  ];

  const statusText =
    current?.status === 'running'
      ? '运行中'
      : current?.status === 'starting'
      ? '正在启动…'
      : current?.status === 'stopping'
      ? '正在停止…'
      : current?.status === 'failed'
      ? '启动失败'
      : '已停止';

  return (
    <>
      <DialogHost />
      {toast && <div className="app-toast">{toast}</div>}

      <main className="app-shell">
        <Sidebar
          instances={instances}
          current={current}
          tab={tab}
          setActiveId={setActiveId}
          setTab={setTab}
          addInstance={addInstance}
          runtime={runtime}
        />

        <section className="content">
          <HeaderActions
            current={current}
            tab={tab}
            setTab={setTab}
            running={runningNow}
            serviceMode={serviceMode}
            serviceChecking={serviceChecking}
            serviceRecovery={serviceRecovery}
            kernelUpdate={kernelUpdate}
            cancelKernelUpdate={cancelKernelUpdate}
            toggle={toggle}
            navItems={navItems}
          />

          {tab === 'status' && (
            <StatusTab
              current={current}
              running={runningNow}
              statusText={statusText}
              node={node}
              peers={peers}
              routes={routes}
              instanceTraffic={instanceTraffic}
              clearTraffic={() => clearInstanceTraffic(current.id)}
              trafficHistory={trafficHistory}
              currentRxSpeed={currentRxSpeed}
              currentTxSpeed={currentTxSpeed}
              peakSpeed={peakSpeed}
              copyText={copyText}
              setTab={setTab}
            />
          )}

          {tab === 'peers' && (
            <PeersTab
              current={current}
              instances={instances}
              peers={peers}
              routes={routes}
              visiblePeers={visiblePeers}
              running={runningNow}
              visibleCols={visibleCols}
              setVisibleCols={setVisibleCols}
              showDisplaySettings={showDisplaySettings}
              setShowDisplaySettings={setShowDisplaySettings}
              showPeerNodes={showPeerNodes}
              setShowPeerNodes={setShowPeerNodes}
              refreshSecs={refreshSecs}
              setRefreshSecs={setRefreshSecs}
              pollEpoch={pollEpoch}
              setPollEpoch={setPollEpoch}
              setRemoteConfigTarget={setRemoteConfigTarget}
              showToast={showToast}
              copyText={copyText}
            />
          )}

          {tab === 'routes' && <RoutesTab running={runningNow} routes={routes} />}

          {tab === 'config' && (
            <ConfigTab
              current={current}
              instances={instances}
              configSaved={configSaved}
              renameInstance={renameInstance}
              removeInstance={removeInstance}
              patchConfig={patchConfig}
              showAdvanced={showAdvanced}
              setShowAdvanced={setShowAdvanced}
              secretVisible={secretVisible}
              addLog={addLog}
              showToast={showToast}
              copyText={copyText}
              setInstances={setInstances}
            />
          )}

          {tab === 'logs' && (
            <LogsTab
              current={current}
              logsByInstance={logsByInstance}
              setLogsByInstance={setLogsByInstance}
              serviceMode={serviceMode}
              refreshSecs={refreshSecs}
              isWindowVisible={isWindowVisible}
            />
          )}

          {tab === 'settings' && (
            <SettingsTab
              current={current}
              instances={instances}
              setInstances={setInstances}
              service={service}
              serviceMode={serviceMode}
              serviceBusy={serviceBusy}
              setServiceBusy={setServiceBusy}
              serviceResult={serviceResult}
              setServiceResult={setServiceResult}
              refreshService={refreshService}
              runtime={runtime}
              kernelInfo={kernelInfo}
              kernelUpdate={kernelUpdate}
              kernelProxy={kernelProxy}
              setKernelProxy={setKernelProxy}
              customKernelProxy={customKernelProxy}
              setCustomKernelProxy={setCustomKernelProxy}
              availableKernelVersions={availableKernelVersions}
              selectedKernelVersion={selectedKernelVersion}
              setSelectedKernelVersion={setSelectedKernelVersion}
              checkKernelUpdate={checkKernelUpdate}
              switchKernel={switchKernel}
              cancelKernelUpdate={cancelKernelUpdate}
              clientAutoStart={clientAutoStart}
              updateClientAutoStart={updateClientAutoStart}
              secretVisible={secretVisible}
              setSecretVisible={setSecretVisible}
            />
          )}
        </section>
      </main>

      {remoteConfigTarget && (
        <RemoteConfigDialog
          host={remoteConfigTarget.host}
          port={remoteConfigTarget.port}
          candidatePorts={remoteConfigTarget.candidatePorts}
          onClose={() => setRemoteConfigTarget(null)}
          onSaved={() => {
            showToast('✓ 远程节点配置修改成功，节点已重启生效');
            setRemoteConfigTarget(null);
            setPollEpoch(n => n + 1);
          }}
        />
      )}
    </>
  );
}
