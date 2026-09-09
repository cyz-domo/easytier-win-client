import { useEffect, useRef, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { serviceRequest } from '../service-client';
import { encodeTOML } from '../toml-codec';
import { appConfirm } from '../dialogs';
import {
  Instance,
  KernelUpdateInfo,
  KernelUpdateProgress,
  KernelUpdateTaskResponse,
  KERNEL_TERMINAL_PHASES,
  isKernelTerminal,
  kernelErrorText,
  mergeKernelProgress,
  load,
} from '../types';

export function useKernelUpdate(
  instances: Instance[],
  serviceMode: boolean,
  refreshService: () => Promise<void>,
  showToast: (msg: string) => void,
  tab: string,
  runtimeVersion?: string
) {
  const [kernelUpdate, setKernelUpdate] = useState<KernelUpdateProgress | null>(null);
  const [kernelInfo, setKernelInfo] = useState<KernelUpdateInfo | null>(null);
  const [availableKernelVersions, setAvailableKernelVersions] = useState<string[]>([]);
  const [selectedKernelVersion, setSelectedKernelVersion] = useState<string>('');
  const [kernelProxy, setKernelProxy] = useState<string>(() => load('easytier.kernel-update-proxy.v1', 'direct'));
  const [customKernelProxy, setCustomKernelProxy] = useState<string>(() => load('easytier.custom-kernel-proxy.v1', ''));

  useEffect(() => {
    localStorage.setItem('easytier.custom-kernel-proxy.v1', customKernelProxy);
  }, [customKernelProxy]);

  useEffect(() => {
    localStorage.setItem('easytier.kernel-update-proxy.v1', JSON.stringify(kernelProxy));
  }, [kernelProxy]);

  const effectiveKernelProxy = kernelProxy === 'custom' ? customKernelProxy.trim() || 'direct' : kernelProxy;

  // 监听 Tauri 后端推送的内核更新进度事件
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void listen<KernelUpdateProgress>('kernel-update-progress', event => {
      const payload = event.payload;
      setKernelUpdate(currentProgress => {
        if (!currentProgress) return currentProgress;
        if (payload.task_id && currentProgress.task_id && payload.task_id !== currentProgress.task_id) {
          return currentProgress;
        }
        return mergeKernelProgress(currentProgress, payload, currentProgress.task_id);
      });
    }).then(fn => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const kernelTaskId = kernelUpdate?.task_id;

  // 在服务模式下轮询后台任务进度
  useEffect(() => {
    if (!kernelTaskId || !kernelUpdate || KERNEL_TERMINAL_PHASES.includes(kernelUpdate.phase)) return;
    let alive = true;
    let timer: number | null = null;
    const poll = async () => {
      try {
        const response = await serviceRequest<KernelUpdateTaskResponse>('get_task_status', { task_id: kernelTaskId });
        if (!alive || (response.task_id && response.task_id !== kernelTaskId)) return;
        const update: Partial<KernelUpdateProgress> =
          response.progress ||
          (response.phase
            ? {
                phase: response.phase,
                downloaded_bytes: response.downloaded_bytes ?? 0,
                total_bytes: response.total_bytes,
                percent: response.percent,
                message: response.message || '',
                error: kernelErrorText(response.error),
              }
            : response.result) ||
          {};
        const status = response.status?.toLowerCase();
        const terminal = isKernelTerminal(status) || isKernelTerminal(update?.phase);
        const phase = terminal
          ? status === 'failed' || status === 'error' || update?.phase === 'failed'
            ? 'failed'
            : status === 'cancelled' || update?.phase === 'cancelled'
            ? 'cancelled'
            : 'completed'
          : update?.phase;
        const next = mergeKernelProgress(
          kernelUpdate,
          {
            ...update,
            ...(phase ? { phase } : {}),
            ...(kernelErrorText(response.error) ? { error: kernelErrorText(response.error) } : {}),
          },
          kernelTaskId
        );
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
    return () => {
      alive = false;
      if (timer != null) window.clearTimeout(timer);
    };
  }, [serviceMode, kernelTaskId, kernelUpdate?.phase, effectiveKernelProxy, refreshService, showToast]);

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
      .catch(e =>
        setKernelInfo({ current_version: runtimeVersion ?? 'unknown', update_available: false, error: String(e) })
      );
  }, [tab, kernelProxy, runtimeVersion]);

  const cancelKernelUpdate = useCallback(async () => {
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
  }, [serviceMode, showToast]);

  const checkKernelUpdate = useCallback(async () => {
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
      setKernelInfo({ current_version: runtimeVersion ?? 'unknown', update_available: false, error: String(e) });
    }
  }, [effectiveKernelProxy, selectedKernelVersion, runtimeVersion]);

  const switchKernel = useCallback(
    async (targetVer?: string) => {
      if (kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)) return;
      const targetTag =
        targetVer || selectedKernelVersion || (kernelInfo?.latest_version ? `v${kernelInfo.latest_version}` : undefined);
      if (!targetTag) return;
      if (
        !(await appConfirm(
          `将切换/更新 EasyTier 内核至 ${targetTag}，操作期间会停止并自动重启当前运行中的网络。继续吗？`
        ))
      )
        return;

      const runningInstances = instances
        .filter(i => i.status === 'running')
        .map(i => ({
          id: i.id,
          config: encodeTOML(i.config),
          rpc_port: i.rpcPort,
          remote_manage_enabled: i.remoteManageEnabled ?? false,
          rpc_whitelist_cidrs: i.rpcWhitelistCidrs ?? [],
        }));

      setKernelUpdate({
        phase: 'checking',
        downloaded_bytes: 0,
        total_bytes: null,
        percent: 0,
        message: `正在准备切换至 ${targetTag}`,
      });

      try {
        let serviceTaskTerminal = false;
        if (serviceMode) {
          const response = await serviceRequest<KernelUpdateTaskResponse>('update_kernel', {
            proxy: effectiveKernelProxy,
            target_version: targetTag,
          });
          const taskId = response.task_id;
          const update: Partial<KernelUpdateProgress> =
            response.progress ||
            (response.phase
              ? {
                  phase: response.phase,
                  downloaded_bytes: response.downloaded_bytes ?? 0,
                  total_bytes: response.total_bytes,
                  percent: response.percent,
                  message: response.message || '',
                  error: kernelErrorText(response.error),
                }
              : response.result) ||
            {};
          const status = response.status?.toLowerCase();
          serviceTaskTerminal = isKernelTerminal(status) || isKernelTerminal(update?.phase);
          const phase = serviceTaskTerminal
            ? status === 'failed' || status === 'error' || update?.phase === 'failed'
              ? 'failed'
              : status === 'cancelled' || update?.phase === 'cancelled'
              ? 'cancelled'
              : 'completed'
            : update?.phase;
          setKernelUpdate(
            mergeKernelProgress(
              null,
              {
                task_id: taskId,
                ...update,
                ...(phase ? { phase } : {}),
                ...(kernelErrorText(response.error) ? { error: kernelErrorText(response.error) } : {}),
              },
              taskId
            )
          );
          if (serviceTaskTerminal && phase === 'completed') {
            void invoke<KernelUpdateInfo>('check_kernel_update', { proxy: effectiveKernelProxy })
              .then(fresh => {
                setKernelInfo(fresh);
                if (fresh.available_versions) setAvailableKernelVersions(fresh.available_versions);
              })
              .catch(() => {});
            void refreshService();
            showToast('✓ EasyTier 内核已成功更新/切换');
          }
        } else {
          await invoke('update_kernel', {
            proxy: effectiveKernelProxy,
            targetVersion: targetTag,
            instances: runningInstances,
          });
          const fresh = await invoke<KernelUpdateInfo>('check_kernel_update', { proxy: effectiveKernelProxy });
          setKernelInfo(fresh);
          if (fresh.available_versions) setAvailableKernelVersions(fresh.available_versions);
          showToast('✓ EasyTier 内核已成功更新/切换');
        }
      } catch (e) {
        setKernelUpdate(prev => ({
          phase: 'failed',
          downloaded_bytes: prev?.downloaded_bytes ?? 0,
          total_bytes: prev?.total_bytes ?? null,
          percent: prev?.percent ?? 0,
          message: '内核更新失败',
          error: String(e),
          task_id: prev?.task_id ?? null,
        }));
        showToast(`内核切换失败：${String(e)}`);
      }
    },
    [kernelUpdate, selectedKernelVersion, kernelInfo?.latest_version, instances, serviceMode, effectiveKernelProxy, refreshService, showToast]
  );

  return {
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
  };
}
