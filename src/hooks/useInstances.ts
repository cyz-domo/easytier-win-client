import { useEffect, useMemo, useRef, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { defaultConfig, listenersForInstance, NetworkConfig } from '../network-config';
import { encodeTOML } from '../toml-codec';
import { Instance, load, loadInstances, nextRpcPort, Status } from '../types';

export function useInstances(
  serviceMode: boolean,
  serviceChecking: boolean,
  addLog: (msg: string) => void,
  showToast: (msg: string) => void,
  clearInstanceTraffic: (id: string) => void
) {
  const [instances, setInstances] = useState<Instance[]>(loadInstances);
  const [activeId, setActiveId] = useState<string>(() => load('easytier.active.v2', ''));
  const [configSaved, setConfigSaved] = useState(false);

  // 跟踪已尝试自动连接的实例 ID 集合，解决实例分批异步加载引发的连接跳过竞态
  const attemptedAutoConnectIds = useRef<Set<string>>(new Set());

  // 持久化实例列表与当前选中项
  useEffect(() => {
    localStorage.setItem('easytier.instances.v2', JSON.stringify(instances));
  }, [instances]);

  const current = useMemo(
    () => instances.find(i => i.id === activeId) ?? instances[0],
    [instances, activeId]
  );

  useEffect(() => {
    if (current) localStorage.setItem('easytier.active.v2', current.id);
  }, [current]);

  // 新增实例
  const addInstance = useCallback(() => {
    const id = crypto.randomUUID();
    setInstances(xs => [
      ...xs,
      {
        id,
        name: '新网络',
        status: 'stopped',
        rpcPort: nextRpcPort(xs),
        config: { ...defaultConfig(), listener_urls: listenersForInstance(xs.length) },
      },
    ]);
    setActiveId(id);
  }, []);

  // 删除实例：释放后端资源，清空该实例会话流量，且绝不再向 localStorage 写入旧版 traffic 数据
  const removeInstance = useCallback(
    (id: string) => {
      if (instances.length <= 1) return;
      const target = instances.find(i => i.id === id);
      setInstances(xs => xs.filter(i => i.id !== id));
      if (activeId === id) {
        const nextActive = instances.find(i => i.id !== id);
        if (nextActive) setActiveId(nextActive.id);
      }
      if (target) {
        invoke('drop_status_endpoint', { port: target.rpcPort }).catch(() => undefined);
      }
      invoke('drop_instance_state', { id }).catch(() => undefined);
      clearInstanceTraffic(id);
    },
    [instances, activeId, clearInstanceTraffic]
  );

  // 重命名当前实例
  const renameInstance = useCallback(
    (name: string) => {
      if (!current) return;
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, name } : i)));
    },
    [current]
  );

  // 修改配置
  const patchConfig = useCallback(
    (patch: Partial<NetworkConfig>) => {
      if (!current) return;
      const nextConfig = { ...current.config, ...patch };
      const nextName = patch.network_name !== undefined ? patch.network_name || current.name : current.name;
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, name: nextName, config: nextConfig } : i)));
      setConfigSaved(true);
      window.setTimeout(() => setConfigSaved(false), 1500);
    },
    [current]
  );

  // 启动全链路自启动：监听 instances 列表变化，凡发现 autoStart 为 true 且未连接也未尝试过的，立即连接
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
  }, [instances, serviceChecking, serviceMode, addLog, showToast, clearInstanceTraffic]);

  return {
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
  };
}
