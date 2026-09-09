import React from 'react';
import { Instance, Tab } from '../../types';
import { NodeStatus, PeerInfo, RouteInfo, formatBytes } from '../../status-data';
import { IconCopy, IconGlobe, IconTerminal, IconUsers } from '../../icons';
import { TrafficAreaChart, TrafficSample, formatSpeed } from '../../TrafficAreaChart';

interface StatusTabProps {
  current: Instance;
  running: boolean;
  statusText: string;
  node: NodeStatus | null;
  peers: PeerInfo[];
  routes: RouteInfo[];
  instanceTraffic: { rx: number; tx: number };
  clearTraffic: () => void;
  trafficHistory: TrafficSample[];
  currentRxSpeed: number;
  currentTxSpeed: number;
  peakSpeed: number;
  copyText: (text: string, label?: string) => Promise<void>;
  setTab: (tab: Tab) => void;
}

export const StatusTab: React.FC<StatusTabProps> = ({
  current,
  running,
  statusText,
  node,
  peers,
  routes,
  instanceTraffic,
  clearTraffic,
  trafficHistory,
  currentRxSpeed,
  currentTxSpeed,
  peakSpeed,
  copyText,
  setTab,
}) => {
  return (
    <>
      <div
        className={`status-card status-${
          running
            ? 'connected'
            : current.status === 'failed'
            ? 'error'
            : current.status === 'starting' || current.status === 'stopping'
            ? 'connecting'
            : 'muted'
        }`}
      >
        <div className="status-icon">
          {running ? '✓' : current.status === 'failed' ? '!' : current.status === 'starting' || current.status === 'stopping' ? '…' : '−'}
        </div>
        <div>
          <span className="card-label">当前状态</span>
          <h2>{statusText}</h2>
          <p>
            {running
              ? `RPC 管理端口 ${
                  current.remoteManageEnabled
                    ? `0.0.0.0（已允许远程管理）:${current.rpcPort}`
                    : `127.0.0.1:${current.rpcPort}`
                }${node?.version ? ` · 核心 ${node.version}` : ''}`
              : '启动网络后，设备将加入 EasyTier 虚拟网络。'}
          </p>
          {running && !node?.ipv4_addr && (
            <p className="warn-inline">
              ⚠ 本机尚未获得虚拟 IPv4 —— 请在「组网配置 → 基础配置」勾选 DHCP 或手动填写虚拟 IPv4，然后重启网络（TUN 需要管理员权限运行客户端）。
            </p>
          )}
        </div>
        <div className="status-meta">
          <span>本机虚拟地址</span>
          <b
            style={{
              cursor: node?.ipv4_addr || current.config.virtual_ipv4 ? 'pointer' : 'default',
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
            }}
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
        <article>
          <span>组网成员</span>
          <strong>{running ? peers.length : '—'}</strong>
          <small>{running ? '在线节点' : '未运行'}</small>
        </article>
        <article>
          <span>路由条目</span>
          <strong>{running ? routes.length : '—'}</strong>
          <small>{running ? '已知网段' : '未运行'}</small>
        </article>
        <article>
          <span>
            本次累计
            <button
              type="button"
              className="mini-button"
              style={{ marginLeft: 8, padding: '2px 8px', fontSize: 10 }}
              onClick={clearTraffic}
              title="清零当前累计统计"
            >
              清零
            </button>
          </span>
          <strong>{running ? formatBytes(instanceTraffic.rx + instanceTraffic.tx) : '—'}</strong>
          <small>会话累计（重连不丢失）</small>
        </article>
      </div>

      {running && (
        <div className="card traffic-card">
          <div className="card-title-row">
            <div className="traffic-title-left">
              <h3 className="card-title">实时网络流量</h3>
              <span className="hint-inline">动态面积图 · 资源自适应采样</span>
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

      <div className="section-heading">
        <div>
          <h2>快速操作</h2>
          <p>常用配置与信息入口</p>
        </div>
      </div>
      <div className="config-grid">
        <button className="config-card" onClick={() => setTab('config')}>
          <span className="config-icon">
            <IconGlobe size={15} />
          </span>
          <div>
            <b>组网配置</b>
            <small>网络名称、密钥、地址与高级参数</small>
          </div>
          <span>›</span>
        </button>
        <button className="config-card" onClick={() => setTab('peers')}>
          <span className="config-icon">
            <IconUsers size={15} />
          </span>
          <div>
            <b>组网成员</b>
            <small>查看在线节点与连接质量</small>
          </div>
          <span>›</span>
        </button>
        <button className="config-card" onClick={() => setTab('logs')}>
          <span className="config-icon">
            <IconTerminal size={15} />
          </span>
          <div>
            <b>运行日志</b>
            <small>启动、停止与错误记录</small>
          </div>
          <span>›</span>
        </button>
      </div>
    </>
  );
};
