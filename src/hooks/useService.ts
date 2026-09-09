import { useEffect, useRef, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getServiceStatus, serviceRequest, ServiceInstanceState, ServiceStatus } from '../service-client';
import { defaultConfig } from '../network-config';
import { Instance, ServiceRecoveryStep } from '../types';

export function useService(
  setInstances: React.Dispatch<React.SetStateAction<Instance[]>>,
  isWindowVisible: boolean
) {
  const [service, setService] = useState<ServiceStatus | null>(null);
  const [serviceChecking, setServiceChecking] = useState(true);
  const [serviceBusy, setServiceBusy] = useState(false);
  const [serviceResult, setServiceResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [serviceRecovery, setServiceRecovery] = useState<ServiceRecoveryStep | null>(null);

  const recoveryRef = useRef(false);
  const lastServiceHealAt = useRef(0);
  const wasServiceModeRef = useRef<boolean | null>(null);

  const serviceMode = service?.running === true && service?.healthy !== false;
  const serviceInstalled = service?.installed === true;

  // service_status via named pipe with retries while SCM reports RUNNING
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

  const refreshService = useCallback(async (opts?: { skipAutoStart?: boolean }) => {
    try {
      const installed = await invoke<{ installed: boolean; running: boolean; message?: string }>('query_service_installation').catch(e => ({
        installed: false,
        running: false,
        message: String(e),
      }));

      if (!installed.installed) {
        setService({ installed: false, running: false, message: installed.message });
        setServiceChecking(false);
        return;
      }

      if (!installed.running) {
        if (!opts?.skipAutoStart && !sessionStorage.getItem('easytier.service-repaired.v1')) {
          setServiceChecking(true);
          setService({ installed: true, running: false, message: '正在自动启动后台服务…' });
          void (async () => {
            try {
              await invoke('start_service');
              await new Promise(r => setTimeout(r, 1500));
              const q = await invoke<{ installed: boolean; running: boolean }>('query_service_installation').catch(() => ({
                installed: true,
                running: false,
              }));
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
            return s
              ? {
                  ...i,
                  name: i.name || s.name || '网络实例',
                  rpcPort: s.rpc_port ?? i.rpcPort,
                  status: s.observed_state,
                  autoStart: s.auto_start,
                  desiredState: s.desired_state,
                  lastError: s.last_error,
                  remoteManageEnabled: s.remote_manage_enabled ?? i.remoteManageEnabled,
                  rpcWhitelistCidrs: s.rpc_whitelist_cidrs ?? i.rpcWhitelistCidrs,
                }
              : i;
          });
          const existingIds = new Set(mapped.map(i => i.id));
          // 如果服务中残留了前端已经不存在的遗留/孤儿实例，自动向服务发送清理指令，绝不再在前端复活空实例
          for (const s of states) {
            if (!existingIds.has(s.id)) {
              serviceRequest('remove_instance', { instance_id: s.id }).catch(() => undefined);
            }
          }
          return mapped;
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
              setService({
                installed: true,
                running: false,
                healthy: false,
                message: `服务自动修复失败：${String(err)}（可在设置中重试）`,
              });
            }
          })();
        }
        return;
      }
      setService({ installed: false, running: false, message: String(e) });
    } finally {
      setServiceChecking(false);
    }
  }, [setInstances]);

  // 后台服务自愈流程
  const runServiceRecovery = useCallback(async () => {
    if (recoveryRef.current) return;
    recoveryRef.current = true;
    setServiceRecovery('starting');
    setService(s => (s ? { ...s, running: false, message: '检测到后台服务异常退出，正在自动重新拉起…' } : s));
    try {
      await invoke('repair_service');
      await new Promise(r => setTimeout(r, 1500));
      setServiceRecovery('waiting');
      const maxWait = 10;
      let ok = false;
      for (let i = 0; i < maxWait; i++) {
        const q = await invoke<{ installed: boolean; running: boolean }>('query_service_installation').catch(() => null);
        if (q?.installed && q?.running) {
          ok = true;
          break;
        }
        await new Promise(r => setTimeout(r, 1000));
      }
      if (!ok) throw new Error('服务拉起超时（10秒）');
      setServiceRecovery('syncing');
      await refreshService({ skipAutoStart: true });
    } catch (e) {
      setService({ installed: true, running: false, message: `后台服务自动恢复失败：${String(e)}（可在设置中重试或修复）` });
    } finally {
      recoveryRef.current = false;
      setServiceRecovery(null);
    }
  }, [refreshService]);

  // 初始化服务状态查询
  useEffect(() => {
    void refreshService();
  }, [refreshService]);

  // 看门狗：每 20 秒检查后台服务存活与响应状态
  useEffect(() => {
    if (!service?.installed) return;
    const timer = window.setInterval(async () => {
      if (!isWindowVisible || recoveryRef.current) return;
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
      } catch {
        /* sc query hiccup */
      }
    }, 20_000);
    return () => {
      clearInterval(timer);
    };
  }, [service?.installed, isWindowVisible, runServiceRecovery]);

  return {
    service,
    serviceChecking,
    serviceBusy,
    setServiceBusy,
    serviceResult,
    setServiceResult,
    serviceRecovery,
    serviceMode,
    serviceInstalled,
    wasServiceModeRef,
    refreshService,
    runServiceRecovery,
  };
}
