import { useEffect, useMemo, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { serviceRequest } from '../service-client';
import { defaultConfig, listenersForInstance, NetworkConfig } from '../network-config';
import { Instance, load, loadInstances, nextRpcPort } from '../types';

export function useInstances(
  addLog: (msg: string) => void,
  showToast: (msg: string) => void,
  clearInstanceTraffic: (id: string) => void
) {
  const [instances, setInstances] = useState<Instance[]>(loadInstances);
  const [activeId, setActiveId] = useState<string>(() => load('easytier.active.v2', ''));
  const [configSaved, setConfigSaved] = useState(false);

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
        config: { ...defaultConfig(), instance_id: id, listener_urls: listenersForInstance(xs.length) },
      },
    ]);
    setActiveId(id);
  }, []);

  // 删除实例：释放后端资源，优雅停止运行中进程，清空该实例会话流量，同步清除 Windows 服务中的残留实例
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
        invoke('stop_instance', { id }).catch(() => undefined);
        invoke('drop_status_endpoint', { port: target.rpcPort }).catch(() => undefined);
      }
      invoke('drop_instance_state', { id }).catch(() => undefined);
      serviceRequest('remove_instance', { instance_id: id }).catch(() => undefined);
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
      // 仅当实例名与原网络名保持一致，或者仍为默认实例名时，修改网络名称才同步更新侧边栏实例名；
      // 若用户显式自定义了实例名（如用于区分同一网络的不同节点/配置），不强制覆盖用户设置的实例名
      const trimmedNetworkName = patch.network_name?.trim();
      const shouldSyncName =
        Boolean(trimmedNetworkName) &&
        (current.name === current.config.network_name || current.name === '新网络' || current.name === '我的网络');
      const nextName = shouldSyncName && trimmedNetworkName ? trimmedNetworkName : current.name;
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, name: nextName, config: nextConfig } : i)));
      setConfigSaved(true);
      window.setTimeout(() => setConfigSaved(false), 1500);
    },
    [current]
  );

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
