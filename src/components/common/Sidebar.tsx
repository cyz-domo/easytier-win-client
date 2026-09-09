import React from 'react';
import easytierLogo from '../../assets/easytier-logo.png';
import { IconGear, IconPlus } from '../../icons';
import { Instance, Tab, runtimeInfo } from '../../types';

interface SidebarProps {
  instances: Instance[];
  current: Instance;
  tab: Tab;
  setActiveId: (id: string) => void;
  setTab: (tab: Tab) => void;
  addInstance: () => void;
  runtime: runtimeInfo | null;
}

export const Sidebar: React.FC<SidebarProps> = ({
  instances,
  current,
  tab,
  setActiveId,
  setTab,
  addInstance,
  runtime,
}) => {
  return (
    <aside>
      <div className="brand">
        <span className="brand-mark">
          <img src={easytierLogo} alt="EasyTier" draggable={false} />
        </span>
        <div>
          <strong>EasyTier</strong>
          <small>Windows Client</small>
        </div>
      </div>
      <div className="section-label">网络实例</div>
      <nav>
        {instances.map(i => (
          <button
            className={i.id === current.id ? 'nav-item active' : 'nav-item'}
            onClick={() => {
              setActiveId(i.id);
              if (tab === 'settings') setTab('status');
            }}
            key={i.id}
          >
            <span
              className={
                i.status === 'running'
                  ? 'dot on'
                  : i.status === 'failed'
                  ? 'dot err'
                  : i.status === 'starting' || i.status === 'stopping'
                  ? 'dot connecting'
                  : 'dot'
              }
            />
            <span className="nav-name">{i.name}</span>
            <span className="chevron">›</span>
          </button>
        ))}
      </nav>
      <button className="add-button" onClick={addInstance}>
        <IconPlus size={14} /> 新建实例
      </button>
      <div className="sidebar-bottom">
        <button className="quiet" onClick={() => setTab('settings')}>
          <IconGear size={14} /> 设置
        </button>
        <div className="version-block">
          <span className="version-row">
            核心版本：{runtime?.version ? runtime.version.split(' ').pop() : '未检测'}
          </span>
          <span className="version-row">客户端版本：v0.1.0</span>
        </div>
      </div>
    </aside>
  );
};
