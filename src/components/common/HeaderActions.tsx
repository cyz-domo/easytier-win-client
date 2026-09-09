import React from 'react';
import { IconPlay, IconStop } from '../../icons';
import {
  Instance,
  KernelUpdateProgress,
  KERNEL_PHASE_TEXT,
  ServiceRecoveryStep,
  SERVICE_RECOVERY_TEXT,
  Tab,
} from '../../types';

interface HeaderActionsProps {
  current: Instance;
  tab: Tab;
  setTab: (t: Tab) => void;
  running: boolean;
  serviceMode: boolean;
  serviceChecking: boolean;
  serviceRecovery: ServiceRecoveryStep | null;
  kernelUpdate: KernelUpdateProgress | null;
  cancelKernelUpdate: () => Promise<void>;
  toggle: () => Promise<void>;
  navItems: [Tab, string][];
}

export const HeaderActions: React.FC<HeaderActionsProps> = ({
  current,
  tab,
  setTab,
  running,
  serviceMode,
  serviceChecking,
  serviceRecovery,
  kernelUpdate,
  cancelKernelUpdate,
  toggle,
  navItems,
}) => {
  return (
    <>
      {serviceRecovery && (
        <div className="kernel-progress-banner">
          <div className="kernel-progress-head">
            <strong>后台服务恢复</strong>
            <span>{SERVICE_RECOVERY_TEXT[serviceRecovery]}</span>
          </div>
          <div className="kernel-progress-track">
            <div className="kernel-progress-bar indeterminate" />
          </div>
          <small className="kernel-progress-msg">
            恢复期间正常运行中的网络不会被重启；服务就绪后将直接接管并同步状态。
          </small>
        </div>
      )}

      {kernelUpdate && !['completed', 'failed', 'cancelled'].includes(kernelUpdate.phase) && tab !== 'settings' && (
        <div className="kernel-progress-banner">
          <div className="kernel-progress-head">
            <strong>EasyTier 内核更新中</strong>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <span style={{ cursor: 'pointer' }} onClick={() => setTab('settings')}>
                {KERNEL_PHASE_TEXT[kernelUpdate.phase] || kernelUpdate.phase} · 前往设置 ›
              </span>
              <button
                type="button"
                className="mini-button danger"
                style={{ padding: '2px 8px', fontSize: 11 }}
                onClick={e => {
                  e.stopPropagation();
                  void cancelKernelUpdate();
                }}
              >
                取消
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
          <button
            className={running ? 'stop' : 'primary'}
            onClick={() => void toggle()}
            disabled={
              current.status === 'starting' ||
              current.status === 'stopping' ||
              (!!kernelUpdate && !['completed', 'failed'].includes(kernelUpdate.phase))
            }
          >
            {running ? <IconStop size={14} /> : <IconPlay size={14} />}
            {running ? '停止网络' : '启动网络'}
          </button>
        </div>
      </header>

      <nav className="tabbar">
        {navItems.map(([t, label]) => (
          <button className={tab === t ? 'tab active' : 'tab'} onClick={() => setTab(t)} key={t}>
            {label}
          </button>
        ))}
      </nav>
    </>
  );
};
