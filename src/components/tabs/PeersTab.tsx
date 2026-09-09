import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Instance } from '../../types';
import {
  PeerColumn,
  PeerInfo,
  PEER_COLUMNS,
  RefreshInterval,
  RouteInfo,
  formatBytes,
  latencyTone,
  routeTone,
} from '../../status-data';
import { IconCopy, IconGear, IconPencil, IconSliders } from '../../icons';
import { getCandidatePortsForIp, saveNodeRpcPort } from '../../types';

interface PeersTabProps {
  current: Instance;
  instances: Instance[];
  peers: PeerInfo[];
  routes: RouteInfo[];
  visiblePeers: PeerInfo[];
  running: boolean;
  visibleCols: PeerColumn[];
  setVisibleCols: React.Dispatch<React.SetStateAction<PeerColumn[]>>;
  showDisplaySettings: boolean;
  setShowDisplaySettings: React.Dispatch<React.SetStateAction<boolean>>;
  showPeerNodes: boolean;
  setShowPeerNodes: React.Dispatch<React.SetStateAction<boolean>>;
  refreshSecs: RefreshInterval;
  setRefreshSecs: React.Dispatch<React.SetStateAction<RefreshInterval>>;
  pollEpoch: number;
  setPollEpoch: React.Dispatch<React.SetStateAction<number>>;
  setRemoteConfigTarget: (target: { host: string; port: number; candidatePorts?: number[] } | null) => void;
  showToast: (msg: string) => void;
  copyText: (text: string, label?: string) => Promise<void>;
}

export const PeersTab: React.FC<PeersTabProps> = ({
  current,
  instances,
  peers,
  routes,
  visiblePeers,
  running,
  visibleCols,
  setVisibleCols,
  showDisplaySettings,
  setShowDisplaySettings,
  showPeerNodes,
  setShowPeerNodes,
  refreshSecs,
  setRefreshSecs,
  setPollEpoch,
  setRemoteConfigTarget,
  showToast,
  copyText,
}) => {
  const [inlineRename, setInlineRename] = useState<{
    peerId: string | number;
    originalName: string;
    currentName: string;
    host: string;
    saving: boolean;
  } | null>(null);

  const submitRename = async () => {
    if (!inlineRename || inlineRename.saving) return;
    const newName = inlineRename.currentName.trim();
    if (!newName) {
      showToast('主机名不能为空');
      setInlineRename(null);
      return;
    }
    if (newName === inlineRename.originalName) {
      setInlineRename(null);
      return;
    }

    setInlineRename(prev => (prev ? { ...prev, saving: true } : null));

    const candidatePorts = getCandidatePortsForIp(inlineRename.host, instances, current?.rpcPort);

    const probeAndPatch = async (port: number): Promise<boolean> => {
      try {
        const discover = await invoke<{ instance_ids: string[]; error?: string }>('remote_config_discover', {
          host: inlineRename.host,
          port,
          virtualIp: inlineRename.host,
        });
        const instanceId = discover.instance_ids?.[0] || 'default';
        await invoke('remote_config_patch', {
          host: inlineRename.host,
          port,
          instanceId,
          patch: { hostname: newName },
        });
        saveNodeRpcPort(inlineRename.host, port);
        return true;
      } catch {
        return false;
      }
    };

    try {
      let ok = false;
      for (const p of candidatePorts) {
        if (await probeAndPatch(p)) {
          ok = true;
          break;
        }
      }
      if (ok) {
        showToast(`✓ 已成功将对端名称修改为「${newName}」`);
        setPollEpoch(n => n + 1);
      } else {
        showToast(`修改失败：目标节点未开启远程管理或端口未在探测范围`);
      }
    } catch (e) {
      showToast(`保存出错：${String(e)}`);
    } finally {
      setInlineRename(null);
    }
  };

  return (
    <div className="card">
      <div className="card-title-row">
        <h3 className="card-title">
          组网成员
          {running
            ? `（${visiblePeers.length}${
                showPeerNodes ? '' : `，已隐藏 ${peers.length - visiblePeers.length} 个 Peer`
              }）`
            : ''}
        </h3>
        <div className="title-actions">
          <span className="hint-inline">刷新 {refreshSecs}s</span>
          <button
            type="button"
            className="ghost"
            onClick={() => setShowDisplaySettings(x => !x)}
          >
            <IconSliders size={13} /> 显示设置
          </button>
        </div>
      </div>

      {showDisplaySettings && (
        <div className="display-settings">
          <div className="display-cols">
            <span className="field-label">数据列</span>
            <div className="display-col-list">
              <label className="check-item">
                <input
                  type="checkbox"
                  checked={showPeerNodes}
                  onChange={e => setShowPeerNodes(e.target.checked)}
                />
                显示 Peer 节点
              </label>
              {PEER_COLUMNS.map(c => (
                <label key={c.key} className="check-item">
                  <input
                    type="checkbox"
                    checked={visibleCols.includes(c.key)}
                    onChange={e => {
                      const next = e.target.checked
                        ? [...visibleCols, c.key]
                        : visibleCols.filter(x => x !== c.key);
                      if (next.length === 0) {
                        showToast('至少保留一列可见');
                        return;
                      }
                      setVisibleCols(next);
                      localStorage.setItem('easytier.peer-cols.v2', JSON.stringify(next));
                    }}
                  />
                  {c.label}
                </label>
              ))}
            </div>
          </div>
          <div className="display-refresh">
            <span className="field-label">刷新</span>
            <select
              className="field-input narrow"
              value={refreshSecs}
              onChange={e => setRefreshSecs(parseInt(e.target.value, 10) as RefreshInterval)}
            >
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
      {running && !peers.length && !current && <p className="list-empty">正在获取实例状态…</p>}
      {running && (
        <div className="table-scroll">
          <table className="data-table">
            <thead>
              <tr>
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
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {visiblePeers.map((p, i) => {
                const route = routes.find(r => r.hostname === p.hostname);
                const isLocal = p.cost === 'Local';
                const nodeType = isLocal
                  ? '本机'
                  : route && (route.path_len ?? 0) > 1
                  ? '服务节点'
                  : '普通节点';
                const remoteIp = p.ipv4 || route?.ipv4?.split('/')[0];
                return (
                  <tr key={String(p.id ?? i)}>
                    {visibleCols.includes('nodeid') && (
                      <td>
                        {String(p.id ?? '—')}
                        <div className="cell-sub">{nodeType}</div>
                      </td>
                    )}
                    {visibleCols.includes('ipv4') && (
                      <td>
                        <span
                          style={{
                            cursor: p.ipv4 ? 'pointer' : 'default',
                            display: 'inline-flex',
                            alignItems: 'center',
                            gap: 4,
                          }}
                          title={p.ipv4 ? '点击复制 IP' : undefined}
                          onClick={() => {
                            if (p.ipv4) {
                              const clean = p.ipv4.split('/')[0].trim();
                              void copyText(clean, `已复制对端 IP: ${clean}`);
                            }
                          }}
                        >
                          {p.ipv4 || '—'}
                          {p.ipv4 && <IconCopy size={11} style={{ opacity: 0.5 }} />}
                        </span>
                        {!isLocal && remoteIp && <div className="cell-sub">{nodeType}</div>}
                      </td>
                    )}
                    {visibleCols.includes('cidr') && <td>{p.cidr || '—'}</td>}
                    {visibleCols.includes('hostname') && (
                      <td>
                        {inlineRename?.peerId === (p.id ?? i) ? (
                          <div className="inline-rename-cell">
                            <input
                              className="field-input inline-edit-input"
                              autoFocus
                              disabled={inlineRename.saving}
                              value={inlineRename.currentName}
                              onChange={e =>
                                setInlineRename({ ...inlineRename, currentName: e.target.value })
                              }
                              onKeyDown={e => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  void submitRename();
                                } else if (e.key === 'Escape') {
                                  setInlineRename(null);
                                }
                              }}
                              onBlur={() => {
                                if (!inlineRename.saving) setInlineRename(null);
                              }}
                            />
                            {inlineRename.saving && <span className="inline-spin">…</span>}
                          </div>
                        ) : (
                          <span
                            className={!isLocal && remoteIp ? 'peer-hostname-cell' : undefined}
                            title={
                              !isLocal && remoteIp ? '双击远程改名 (Enter 确认, Esc 取消)' : undefined
                            }
                            onDoubleClick={() => {
                              if (!isLocal && remoteIp) {
                                const cleanIp = remoteIp.split('/')[0].trim();
                                setInlineRename({
                                  peerId: p.id ?? i,
                                  originalName: p.hostname || '',
                                  currentName: p.hostname || '',
                                  host: cleanIp,
                                  saving: false,
                                });
                              }
                            }}
                          >
                            {p.hostname || '—'}
                            {!isLocal && remoteIp && <IconPencil size={11} className="rename-hint-icon" />}
                          </span>
                        )}
                      </td>
                    )}
                    {visibleCols.includes('cost') && (
                      <td>
                        <span className={`route-badge tone-${routeTone(p.cost)}`}>{p.cost || '—'}</span>
                      </td>
                    )}
                    {visibleCols.includes('proto') && <td>{p.tunnel_proto || '—'}</td>}
                    {visibleCols.includes('latency') && (
                      <td className={`tone-${latencyTone(p.lat_ms)}`}>{p.lat_ms || '—'}</td>
                    )}
                    {visibleCols.includes('loss') && <td>{p.loss_rate || '—'}</td>}
                    {visibleCols.includes('rx') && <td>{formatBytes(p.rx_bytes)}</td>}
                    {visibleCols.includes('tx') && <td>{formatBytes(p.tx_bytes)}</td>}
                    {visibleCols.includes('nat') && <td>{p.nat_type || '—'}</td>}
                    {visibleCols.includes('version') && <td>{p.version || '—'}</td>}
                    {visibleCols.includes('relay') && (
                      <td>
                        <span
                          className={`route-badge tone-${routeTone(
                            route && (route.path_len ?? 0) > 1 ? `Relay (${route.path_len})` : 'Local'
                          )}`}
                        >
                          {route && (route.path_len ?? 0) > 1 ? route.next_hop_hostname : '—'}
                        </span>
                      </td>
                    )}
                    {visibleCols.includes('routes') && (
                      <td>
                        {route?.proxy_cidrs && route.proxy_cidrs !== ''
                          ? String(route.proxy_cidrs)
                          : '—'}
                      </td>
                    )}
                    <td>
                      {!isLocal && remoteIp && (
                        <button
                          type="button"
                          className="mini-button ghost"
                          title="远程管理该节点的配置（需对端开启远程管理）"
                          onClick={() => {
                            const cleanIp = remoteIp.split('/')[0].trim();
                            const candidatePorts = getCandidatePortsForIp(cleanIp, instances, current?.rpcPort);
                            setRemoteConfigTarget({
                              host: cleanIp,
                              port: candidatePorts[0] || 15888,
                              candidatePorts,
                            });
                          }}
                        >
                          <IconGear size={12} />
                          <span>远程配置</span>
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
