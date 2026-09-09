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
    <>
      <div className="peers-header-bar">
        <div className="peers-title-group">
          <h2>组网成员</h2>
          <span className="peers-count-badge">
            {running ? `${visiblePeers.length} 个节点在线` : '未运行'}
            {!showPeerNodes && peers.length > visiblePeers.length && (
              <span className="hidden-peers-hint">（已隐藏 {peers.length - visiblePeers.length} 个公共节点）</span>
            )}
          </span>
        </div>
        <div className="peers-controls-group">
          <button
            type="button"
            className={`display-settings-btn ${showDisplaySettings ? 'active' : ''}`}
            onClick={() => setShowDisplaySettings(v => !v)}
            title="自定义显示的列与数据过滤"
          >
            <IconSliders size={13} />
            <span>显示设置</span>
          </button>
        </div>
      </div>

      {showDisplaySettings && (
        <div className="display-settings-card">
          <div className="display-settings-grid">
            <div className="display-setting-item">
              <label className="checkbox-label" title="公共服务器通常为中继节点，隐藏后界面更简洁">
                <input
                  type="checkbox"
                  checked={showPeerNodes}
                  onChange={e => setShowPeerNodes(e.target.checked)}
                />
                <span>显示公共节点 (PublicServer)</span>
              </label>
            </div>
            <div className="display-setting-item">
              <span className="setting-label-text">状态刷新频率：</span>
              <select
                className="select-compact"
                value={refreshSecs}
                onChange={e => setRefreshSecs(Number(e.target.value) as RefreshInterval)}
              >
                <option value={1}>1 秒 (实时极速)</option>
                <option value={3}>3 秒 (顺畅响应)</option>
                <option value={5}>5 秒 (推荐均衡)</option>
                <option value={10}>10 秒 (轻量节能)</option>
                <option value={30}>30 秒 (极低负载)</option>
              </select>
            </div>
          </div>
          <div className="column-toggles-section">
            <div className="column-toggles-header">
              <span className="setting-label-text">自定义表格展示列：</span>
              <button
                type="button"
                className="text-btn-mini"
                onClick={() => setVisibleCols(PEER_COLUMNS.filter(c => c.defaultOn).map(c => c.key))}
              >
                恢复默认
              </button>
            </div>
            <div className="column-checkboxes-grid">
              {PEER_COLUMNS.map(col => {
                const checked = visibleCols.includes(col.key);
                return (
                  <label key={col.key} className="col-checkbox-label">
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={e => {
                        if (e.target.checked) {
                          setVisibleCols(prev => [...prev, col.key]);
                        } else {
                          if (visibleCols.length <= 1) {
                            showToast('至少保留一列可见');
                            return;
                          }
                          setVisibleCols(prev => prev.filter(k => k !== col.key));
                        }
                      }}
                    />
                    <span>{col.label}</span>
                  </label>
                );
              })}
            </div>
          </div>
        </div>
      )}

      {!running ? (
        <div className="empty-state">
          <h3>网络未运行</h3>
          <p>请点击右上角「启动网络」后查看在线成员节点。</p>
        </div>
      ) : visiblePeers.length === 0 ? (
        <div className="empty-state">
          <h3>暂无成员在线</h3>
          <p>正在寻找组网节点，或当前网络内暂无其他在线设备。</p>
        </div>
      ) : (
        <div className="card table-card">
          <div className="table-wrapper">
            <table>
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
                  const ipOnly = (p.ipv4 ?? '').split('/')[0].trim();
                  const isLocal = p.cost === 'Local';
                  const isRenaming = inlineRename?.peerId === (p.id ?? i);
                  const isPublic = String(p.hostname ?? '').toLowerCase().startsWith('publicserver');
                  const route = routes.find(
                    r => r.ipv4 === p.ipv4 || (p.ipv4 && r.ipv4 && r.ipv4.startsWith(p.ipv4.split('/')[0]))
                  );

                  return (
                    <tr key={p.id ?? i} className={isLocal ? 'row-local' : ''}>
                      {visibleCols.includes('nodeid') && (
                        <td>
                          <code className="code-badge" title={String(p.id ?? '')}>
                            {String(p.id ?? '').slice(0, 8) || '—'}
                          </code>
                        </td>
                      )}
                      {visibleCols.includes('ipv4') && (
                        <td>
                          {ipOnly ? (
                            <span
                              className="clickable-ip"
                              title="点击复制虚拟 IP"
                              onClick={() => void copyText(ipOnly, `已复制虚拟 IP: ${ipOnly}`)}
                            >
                              <code>{ipOnly}</code>
                              <IconCopy size={11} className="copy-icon-hover" />
                            </span>
                          ) : (
                            '—'
                          )}
                        </td>
                      )}
                      {visibleCols.includes('cidr') && <td>{p.cidr || '—'}</td>}
                      {visibleCols.includes('hostname') && (
                        <td>
                          {isRenaming && inlineRename ? (
                            <div className="inline-rename-box">
                              <input
                                type="text"
                                autoFocus
                                className="inline-rename-input"
                                value={inlineRename.currentName}
                                disabled={inlineRename.saving}
                                onChange={e =>
                                  setInlineRename(prev => (prev ? { ...prev, currentName: e.target.value } : null))
                                }
                                onKeyDown={e => {
                                  if (e.key === 'Enter') void submitRename();
                                  if (e.key === 'Escape') setInlineRename(null);
                                }}
                              />
                              <button
                                type="button"
                                className="inline-rename-btn confirm"
                                disabled={inlineRename.saving}
                                onClick={() => void submitRename()}
                                title="保存"
                              >
                                {inlineRename.saving ? '…' : '✓'}
                              </button>
                              <button
                                type="button"
                                className="inline-rename-btn cancel"
                                disabled={inlineRename.saving}
                                onClick={() => setInlineRename(null)}
                                title="取消"
                              >
                                ✕
                              </button>
                            </div>
                          ) : (
                            <div
                              className="hostname-cell"
                              onDoubleClick={() => {
                                if (ipOnly && !isLocal) {
                                  setInlineRename({
                                    peerId: p.id ?? i,
                                    originalName: p.hostname || '',
                                    currentName: p.hostname || '',
                                    host: ipOnly,
                                    saving: false,
                                  });
                                }
                              }}
                              title={!isLocal && ipOnly ? '双击可修改对端主机名' : undefined}
                            >
                              <span className="hostname-text">{p.hostname || (isLocal ? '本机' : '未知')}</span>
                              {!isLocal && ipOnly && (
                                <IconPencil
                                  size={11}
                                  className="pencil-hover"
                                  onClick={() =>
                                    setInlineRename({
                                      peerId: p.id ?? i,
                                      originalName: p.hostname || '',
                                      currentName: p.hostname || '',
                                      host: ipOnly,
                                      saving: false,
                                    })
                                  }
                                />
                              )}
                            </div>
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
                        <td>
                          <span className={`latency-tag tone-${latencyTone(p.lat_ms)}`}>{p.lat_ms || '—'}</span>
                        </td>
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
                            {route && (route.path_len ?? 0) > 1 ? route.next_hop_hostname || '中继' : '—'}
                          </span>
                        </td>
                      )}
                      {visibleCols.includes('routes') && (
                        <td>
                          {route?.proxy_cidrs && route.proxy_cidrs !== '' ? String(route.proxy_cidrs) : '—'}
                        </td>
                      )}
                      <td>
                        {!isLocal && ipOnly && !isPublic && (
                          <button
                            type="button"
                            className="mini-button remote-cfg-btn"
                            title="修改该远程节点的配置（需对端开启远程管理）"
                            onClick={() => {
                              const candidatePorts = getCandidatePortsForIp(ipOnly, instances, current?.rpcPort);
                              setRemoteConfigTarget({
                                host: ipOnly,
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
        </div>
      )}
    </>
  );
};
