import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { serviceRequest } from '../../service-client';
import { appAlert } from '../../dialogs';
import {
  AutoStartClientStatus,
  Instance,
  KernelUpdateInfo,
  KernelUpdateProgress,
  KERNEL_PHASE_TEXT,
  KERNEL_PROXIES,
  runtimeInfo,
} from '../../types';
import { ServiceStatus } from '../../service-client';

interface SettingsTabProps {
  current: Instance;
  instances: Instance[];
  setInstances: React.Dispatch<React.SetStateAction<Instance[]>>;
  service: ServiceStatus | null;
  serviceMode: boolean;
  serviceBusy: boolean;
  setServiceBusy: React.Dispatch<React.SetStateAction<boolean>>;
  serviceResult: { ok: boolean; text: string } | null;
  setServiceResult: React.Dispatch<React.SetStateAction<{ ok: boolean; text: string } | null>>;
  refreshService: (opts?: { skipAutoStart?: boolean }) => Promise<void>;
  runtime: runtimeInfo | null;
  kernelInfo: KernelUpdateInfo | null;
  kernelUpdate: KernelUpdateProgress | null;
  kernelProxy: string;
  setKernelProxy: (proxy: string) => void;
  customKernelProxy: string;
  setCustomKernelProxy: (url: string) => void;
  availableKernelVersions: string[];
  selectedKernelVersion: string;
  setSelectedKernelVersion: (ver: string) => void;
  checkKernelUpdate: () => Promise<void>;
  switchKernel: (targetVer?: string) => Promise<void>;
  cancelKernelUpdate: () => Promise<void>;
  clientAutoStart: AutoStartClientStatus;
  updateClientAutoStart: (enabled: boolean, startMinimized: boolean) => Promise<void>;
  secretVisible: boolean;
  setSecretVisible: React.Dispatch<React.SetStateAction<boolean>>;
}

export const SettingsTab: React.FC<SettingsTabProps> = ({
  current,
  instances,
  setInstances,
  service,
  serviceMode,
  serviceBusy,
  setServiceBusy,
  serviceResult,
  setServiceResult,
  refreshService,
  runtime,
  kernelInfo,
  kernelUpdate,
  kernelProxy,
  setKernelProxy,
  customKernelProxy,
  setCustomKernelProxy,
  availableKernelVersions,
  selectedKernelVersion,
  setSelectedKernelVersion,
  checkKernelUpdate,
  switchKernel,
  cancelKernelUpdate,
  clientAutoStart,
  updateClientAutoStart,
  secretVisible,
  setSecretVisible,
}) => {
  return (
    <>
      <div className="card service-card">
        <div className="card-title-row">
          <h3 className="card-title">后台服务</h3>
          <span className={serviceMode ? 'service-state online' : 'service-state'}>
            {serviceMode ? '服务模式' : '兼容模式'}
          </span>
        </div>
        <p className="hint">
          {service?.message ||
            (service?.installed
              ? service?.running
                ? '服务正在运行。'
                : '服务已安装但尚未运行。'
              : '未检测到 EasyTierService。')}
        </p>
        {serviceResult && (
          <p className={serviceResult.ok ? 'hint' : 'list-empty err'} style={{ marginTop: 6 }}>
            {serviceResult.ok ? '✓ ' : '✗ '}
            {serviceResult.text}
          </p>
        )}
        <div className="service-actions">
          {!service?.installed && (
            <button
              className="primary"
              disabled={serviceBusy}
              onClick={async () => {
                setServiceBusy(true);
                setServiceResult(null);
                try {
                  await invoke('install_service');
                  setServiceResult({ ok: true, text: '后台服务已安装并启动。' });
                  await refreshService({ skipAutoStart: true });
                } catch (e) {
                  setServiceResult({ ok: false, text: `安装服务失败：${String(e)}` });
                } finally {
                  setServiceBusy(false);
                }
              }}
            >
              {serviceBusy ? '正在安装…' : '安装后台服务'}
            </button>
          )}
          {service?.installed && !service?.running && (
            <button
              className="primary"
              disabled={serviceBusy}
              onClick={async () => {
                setServiceBusy(true);
                try {
                  await invoke('start_service');
                  await refreshService();
                } catch (e) {
                  await appAlert(`启动服务失败：${String(e)}`);
                } finally {
                  setServiceBusy(false);
                }
              }}
            >
              启动服务
            </button>
          )}
          {service?.installed && (
            <button
              className="ghost"
              disabled={serviceBusy}
              onClick={async () => {
                setServiceBusy(true);
                setServiceResult(null);
                try {
                  await invoke('repair_service');
                  setServiceResult({ ok: true, text: '后台服务已修复并启动。' });
                  await refreshService();
                } catch (e) {
                  setServiceResult({ ok: false, text: `修复服务失败：${String(e)}` });
                } finally {
                  setServiceBusy(false);
                }
              }}
            >
              修复服务
            </button>
          )}
        </div>
      </div>

      <div className="card">
        <h3 className="card-title">运行时</h3>
        <table className="kv-table">
          <tbody>
            <tr>
              <th>核心路径</th>
              <td>{runtime?.core_path ?? 'core/easytier-core.exe'}</td>
            </tr>
            <tr>
              <th>核心版本</th>
              <td>{runtime?.version ?? '未检测'}</td>
            </tr>
            <tr>
              <th>运行时可用</th>
              <td>{runtime?.available ? '是' : '否（请检查 core 目录）'}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div className="card">
        <div className="card-title-row">
          <h3 className="card-title">EasyTier 内核版本管理与切换</h3>
          <span className="hint-inline">
            当前版本: {kernelInfo?.current_version || runtime?.version || '未检测'}
          </span>
        </div>
        <div className="kernel-update-controls">
          <label className="field">
            <span className="field-label">GitHub 下载线路</span>
            <select
              className="field-input"
              value={kernelProxy}
              onChange={e => setKernelProxy(e.target.value)}
              disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}
            >
              {KERNEL_PROXIES.map(p => (
                <option value={p.value} key={p.value}>
                  {p.label}
                </option>
              ))}
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
          <label className="field">
            <span className="field-label">目标核心版本</span>
            <select
              className="field-input"
              value={selectedKernelVersion}
              onChange={e => setSelectedKernelVersion(e.target.value)}
              disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}
            >
              {availableKernelVersions.length === 0 ? (
                <option value="">
                  {kernelInfo?.latest_version ? `v${kernelInfo.latest_version} (最新稳定版)` : '点击「获取版本列表」'}
                </option>
              ) : (
                availableKernelVersions.map(ver => (
                  <option value={ver} key={ver}>
                    {ver}
                    {ver === `v${kernelInfo?.latest_version}` || ver === kernelInfo?.latest_version
                      ? ' (最新稳定版)'
                      : ''}
                  </option>
                ))
              )}
            </select>
          </label>
          <div className="kernel-update-actions">
            <button
              className="ghost"
              onClick={() => void checkKernelUpdate()}
              disabled={!!kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase)}
            >
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
              {selectedKernelVersion
                ? `切换/安装 ${selectedKernelVersion}`
                : kernelInfo?.update_available
                ? `更新到 v${kernelInfo.latest_version}`
                : '重新安装当前版本'}
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
            <small className="kernel-progress-msg">
              {kernelUpdate.message}
              {kernelUpdate.percent != null ? ` · ${kernelUpdate.percent}%` : ''}
            </small>
          </div>
        )}
        {kernelInfo?.error && <p className="list-empty err">检查失败：{kernelInfo.error}</p>}
        {kernelInfo && !kernelInfo.error && !kernelInfo.update_available && !selectedKernelVersion && (
          <p className="hint">当前已是最新正式版本。可通过上方下拉选择历史版本进行自由切换。</p>
        )}
        {kernelUpdate?.phase === 'failed' && (
          <p className="list-empty err">操作失败：{kernelUpdate.error || kernelUpdate.message}</p>
        )}
        {kernelUpdate?.phase === 'cancelled' && <p className="hint">已取消内核更新操作。</p>}
        {kernelUpdate?.phase === 'completed' && (
          <p className="hint">内核切换/更新完成，原来运行中的网络已尝试自动恢复。</p>
        )}
      </div>

      <div className="card">
        <h3 className="card-title">系统自启动</h3>
        <p className="hint">配置 EasyTier 客户端随 Windows 开机启动，实现桌面登录后即刻在后台就绪。</p>

        {clientAutoStart.path_mismatch && (
          <div className="warn-inline" style={{ marginBottom: 12, padding: 10, borderRadius: 6 }}>
            <div>
              ⚠ <b>自启动路径失效警告</b>：检测到当前可执行文件位置已变更。
              <br />
              <small style={{ opacity: 0.8 }}>
                注册表中原路径：{clientAutoStart.registered_path || '未知'}
              </small>
            </div>
            <button
              type="button"
              className="mini-button primary"
              style={{ marginTop: 8 }}
              onClick={() => void updateClientAutoStart(true, clientAutoStart.start_minimized)}
            >
              更新为当前路径并重新激活
            </button>
          </div>
        )}

        <table className="kv-table">
          <tbody>
            <tr>
              <th>开机自启客户端</th>
              <td className="inline">
                <button
                  className={clientAutoStart.enabled ? 'switch on' : 'switch'}
                  role="switch"
                  aria-checked={clientAutoStart.enabled}
                  title="随 Windows 开机自启动客户端"
                  onClick={() =>
                    void updateClientAutoStart(!clientAutoStart.enabled, clientAutoStart.start_minimized)
                  }
                >
                  <span className="knob" />
                </button>
                <span className="hint-inline">{clientAutoStart.enabled ? '已开启' : '已关闭'}</span>
              </td>
            </tr>
            {clientAutoStart.enabled && (
              <tr>
                <th>静默启动至托盘</th>
                <td className="inline">
                  <button
                    className={clientAutoStart.start_minimized ? 'switch on' : 'switch'}
                    role="switch"
                    aria-checked={clientAutoStart.start_minimized}
                    title="开机自启后静默最小化到系统托盘，不弹出主窗口"
                    onClick={() => void updateClientAutoStart(true, !clientAutoStart.start_minimized)}
                  >
                    <span className="knob" />
                  </button>
                  <span className="hint-inline">
                    {clientAutoStart.start_minimized ? '仅托盘运行（静默）' : '显示主界面'}
                  </span>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="card">
        <p className="hint">
          每个实例使用独立 RPC 端口，避免多实例冲突。启动网络后可通过 easytier-cli 连接该端口查询状态。
        </p>
        <table className="kv-table">
          <tbody>
            {instances.map(i => (
              <tr key={i.id}>
                <th>{i.name}</th>
                <td className="inline">
                  <input
                    className="field-input narrow"
                    type="number"
                    value={i.rpcPort}
                    min={1024}
                    max={65535}
                    onChange={e =>
                      setInstances(xs =>
                        xs.map(x =>
                          x.id === i.id ? { ...x, rpcPort: parseInt(e.target.value, 10) || x.rpcPort } : x
                        )
                      )
                    }
                  />
                  <span className="hint-inline">{i.status === 'running' ? '运行中' : '已停止'}</span>
                  <button
                    className={i.autoStart ? 'switch on' : 'switch'}
                    role="switch"
                    aria-checked={i.autoStart ?? false}
                    title="程序启动后自动连接本网络（若已安装后台服务，开机免登录静默连接）"
                    onClick={async () => {
                      const next = !(i.autoStart ?? false);
                      if (serviceMode) {
                        try {
                          await serviceRequest('set_auto_start', { instance_id: i.id, auto_start: next });
                        } catch (e) {
                          await appAlert(`更新自动启动失败：${String(e)}`);
                          return;
                        }
                      }
                      setInstances(xs => xs.map(x => (x.id === i.id ? { ...x, autoStart: next } : x)));
                    }}
                  >
                    <span className="knob" />
                  </button>
                  <span
                    className="hint-inline"
                    title="程序启动后自动连接本网络（服务模式下开机免登录自启）"
                  >
                    自动启动
                  </span>
                  <button
                    className={i.remoteManageEnabled ? 'switch on' : 'switch'}
                    role="switch"
                    aria-checked={i.remoteManageEnabled ?? false}
                    title="允许虚拟网内其他设备修改本实例配置（RPC 监听 0.0.0.0）"
                    onClick={() =>
                      setInstances(xs =>
                        xs.map(x =>
                          x.id === i.id ? { ...x, remoteManageEnabled: !(x.remoteManageEnabled ?? false) } : x
                        )
                      )
                    }
                  >
                    <span className="knob" />
                  </button>
                  <span className="hint-inline">允许远程管理</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {current?.remoteManageEnabled && (
          <label className="field">
            <span className="field-label">RPC 白名单（当前实例，逗号分隔 CIDR，环回地址自动附加）</span>
            <input
              className="field-input"
              value={(current.rpcWhitelistCidrs ?? []).join(',')}
              placeholder="10.126.126.0/24"
              onChange={e =>
                setInstances(xs =>
                  xs.map(x =>
                    x.id === current.id
                      ? { ...x, rpcWhitelistCidrs: e.target.value.split(',').map(s => s.trim()).filter(Boolean) }
                      : x
                  )
                )
              }
            />
            <span className="hint-inline">
              开启后 RPC 监听 0.0.0.0，会暴露到物理局域网；白名单务必只填 EasyTier 虚拟网段。
            </span>
          </label>
        )}
      </div>

      <div className="card">
        <h3 className="card-title">关于</h3>
        <p className="hint">EasyTier Windows Client 0.1.0 · Tauri 2 + React · 设计文档见仓库 docs/ 目录。</p>
        <label className="toggle-row">
          <span className="field-label">显示密钥（导出时提醒）</span>
          <button
            className={secretVisible ? 'switch on' : 'switch'}
            role="switch"
            aria-checked={secretVisible}
            onClick={() => setSecretVisible(v => !v)}
          >
            <span className="knob" />
          </button>
        </label>
      </div>
    </>
  );
};
