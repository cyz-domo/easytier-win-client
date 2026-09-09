import React, { useEffect, useState, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { IconGlobe, IconRefresh } from '../icons';

export interface PublicNode {
  id: number;
  name: string;
  address: string;
  category: 'domestic' | 'overseas';
  is_online: boolean;
  ping_ms?: number | null;
  local_ping_ms?: number | null;
  uptime_pct?: number | null;
  can_relay: boolean;
  is_masked: boolean;
  description: string;
}

export interface PublicNodesResponse {
  nodes: PublicNode[];
  is_fallback: boolean;
  updated_at: number;
}

interface PublicServerModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSelect: (address: string) => void;
  currentAddress?: string;
}

export const PublicServerModal: React.FC<PublicServerModalProps> = ({
  isOpen,
  onClose,
  onSelect,
  currentAddress,
}) => {
  const [nodes, setNodes] = useState<PublicNode[]>([]);
  const [isFallback, setIsFallback] = useState(false);
  const [loading, setLoading] = useState(false);
  const [pinging, setPinging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'domestic' | 'overseas'>('domestic');

  const fetchNodes = useCallback(async (force = false) => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<PublicNodesResponse | PublicNode[]>('fetch_public_nodes', {
        forceRefresh: force,
      });
      if (Array.isArray(data)) {
        setNodes(data);
        setIsFallback(false);
      } else if (data && Array.isArray(data.nodes)) {
        setNodes(data.nodes);
        setIsFallback(Boolean(data.is_fallback));
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const pingNodes = useCallback(async () => {
    setPinging(true);
    try {
      const unmaskedAddrs = nodes.filter(n => !n.is_masked && n.is_online).map(n => n.address);
      if (unmaskedAddrs.length === 0) return;
      const pingResults = await invoke<Record<string, number | null>>('ping_public_nodes', {
        addresses: unmaskedAddrs,
      });
      setNodes(prev =>
        prev.map(n => {
          if (n.address in pingResults) {
            return { ...n, local_ping_ms: pingResults[n.address] };
          }
          return n;
        })
      );
    } catch (e) {
      console.error('ping error', e);
    } finally {
      setPinging(false);
    }
  }, [nodes]);

  useEffect(() => {
    if (isOpen) {
      if (nodes.length === 0) {
        void fetchNodes(false);
      }
    }
  }, [isOpen, nodes.length, fetchNodes]);

  // ESC key to close
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen, onClose]);

  const filteredNodes = useMemo(() => {
    const list = nodes.filter(n => n.category === activeTab);
    return list.sort((a, b) => {
      // 官方节点置顶
      if (a.id === 0) return -1;
      if (b.id === 0) return 1;
      // 打码节点排在后方
      if (a.is_masked !== b.is_masked) return a.is_masked ? 1 : -1;
      // 在线节点优先
      if (a.is_online !== b.is_online) return a.is_online ? -1 : 1;
      // 本机真实延迟从低到高优先，无本机延迟则回退机房延迟
      const pingA = a.local_ping_ms ?? (a.ping_ms != null ? a.ping_ms + 200 : 9999);
      const pingB = b.local_ping_ms ?? (b.ping_ms != null ? b.ping_ms + 200 : 9999);
      return pingA - pingB;
    });
  }, [nodes, activeTab]);

  if (!isOpen) return null;

  return (
    <div
      className="modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="选择公共服务器"
      onMouseDown={e => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal-card public-server-modal-card">
        {/* Header */}
        <div className="card-title-row" style={{ marginBottom: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ color: 'var(--accent)', display: 'inline-flex' }}>
              <IconGlobe size={20} />
            </span>
            <div>
              <h3 className="card-title" style={{ margin: 0, fontSize: 16 }}>
                EasyTier 公共节点选择器
              </h3>
              <small style={{ color: 'var(--ink-3)', fontSize: 11 }}>
                实时状态与双向测速 · 支持本机 TCP 直连握手与社区监控探针
              </small>
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <button
              type="button"
              className="ghost"
              style={{ fontSize: 12, padding: '4px 8px', display: 'inline-flex', alignItems: 'center', gap: 4 }}
              onClick={() => void pingNodes()}
              disabled={pinging || loading || nodes.length === 0}
              title="重新向当前所有在线节点发起本机真实 TCP 握手测速"
            >
              <span className={pinging ? 'icon-spin' : ''} style={{ display: 'inline-flex' }}>
                ⚡
              </span>
              <span>{pinging ? '测速中…' : '本机测速'}</span>
            </button>
            <button
              type="button"
              className="ghost"
              style={{ fontSize: 12, padding: '4px 8px', display: 'inline-flex', alignItems: 'center', gap: 4 }}
              onClick={() => void fetchNodes(true)}
              disabled={loading || pinging}
              title="强制穿透缓存，从监控站刷新最新节点列表"
            >
              <span className={loading ? 'icon-spin' : ''} style={{ display: 'inline-flex' }}>
                <IconRefresh size={13} />
              </span>
              <span>{loading ? '刷新中…' : '刷新列表'}</span>
            </button>
            <button
              type="button"
              className="ghost"
              style={{ fontSize: 16, padding: '2px 8px', lineHeight: 1 }}
              onClick={onClose}
              aria-label="关闭"
            >
              ✕
            </button>
          </div>
        </div>

        {/* Fallback Banner Notice */}
        {isFallback && (
          <div className="fallback-notice-banner">
            <span>⚠️ 无法连接到第三方监控站，当前已自动切换至内置推荐高可用节点（已测速）。</span>
            <button
              type="button"
              className="ghost"
              style={{ fontSize: 11, padding: '2px 6px', color: 'var(--accent)', cursor: 'pointer' }}
              onClick={() => void fetchNodes(true)}
              disabled={loading}
            >
              重试在线拉取
            </button>
          </div>
        )}

        {/* Tabs */}
        <div className="public-server-tabs">
          <button
            type="button"
            className={activeTab === 'domestic' ? 'tab-btn active' : 'tab-btn'}
            onClick={() => setActiveTab('domestic')}
          >
            🇨🇳 国内公共节点 ({nodes.filter(n => n.category === 'domestic').length})
          </button>
          <button
            type="button"
            className={activeTab === 'overseas' ? 'tab-btn active' : 'tab-btn'}
            onClick={() => setActiveTab('overseas')}
          >
            🌐 海外公共节点 ({nodes.filter(n => n.category === 'overseas').length})
          </button>
        </div>

        {/* Node list container */}
        <div className="public-node-list">
          {loading && nodes.length === 0 ? (
            <div className="public-node-empty">正在拉取公共节点列表并进行本机 TCP 握手测速…</div>
          ) : error && nodes.length === 0 ? (
            <div className="public-node-empty" style={{ color: 'var(--danger)' }}>
              获取节点列表失败：{error}
              <button
                type="button"
                className="mini-button"
                style={{ marginTop: 8 }}
                onClick={() => void fetchNodes()}
              >
                重试
              </button>
            </div>
          ) : filteredNodes.length === 0 ? (
            <div className="public-node-empty">该分类下暂无可用节点</div>
          ) : (
            filteredNodes.map(node => {
              const isCurrent = currentAddress?.trim() === node.address.trim();
              const proto = node.address.split('://')[0]?.toUpperCase() || 'TCP';

              return (
                <div
                  key={`${node.id}-${node.address}`}
                  className={`public-node-card ${node.is_masked ? 'masked' : ''} ${
                    isCurrent ? 'selected' : ''
                  }`}
                >
                  <div className="node-info-col">
                    <div className="node-address-row">
                      <span className="node-proto-tag">{proto}</span>
                      <strong className="node-address" title={node.address}>
                        {node.address}
                      </strong>
                      {node.local_ping_ms != null ? (
                        <span
                          className="node-status-badge online"
                          title={`本机直接 TCP 握手测速延迟: ${node.local_ping_ms} ms`}
                        >
                          <span className="status-dot-sm" />
                          本机 {node.local_ping_ms} ms
                        </span>
                      ) : node.is_masked ? (
                        <span className="node-status-badge masked" title="地址包含*掩码，需加群获取完整地址测速">
                          <span className="status-dot-sm" />
                          需加群测速
                        </span>
                      ) : (
                        <span
                          className={`node-status-badge ${
                            node.is_online ? 'online' : 'offline'
                          }`}
                          title={`第三方探针机房延迟: ${node.ping_ms ?? '未知'} ms`}
                        >
                          <span className="status-dot-sm" />
                          {node.is_online
                            ? `${node.ping_ms != null ? `机房 ${node.ping_ms} ms` : '在线'}`
                            : '离线'}
                        </span>
                      )}

                      {node.local_ping_ms != null && node.ping_ms != null && (
                        <span
                          className="probe-badge"
                          title={`第三方探针机房延迟: ${node.ping_ms} ms`}
                        >
                          机房 {node.ping_ms} ms
                        </span>
                      )}
                    </div>

                    <div className="node-desc-row">
                      <span className="node-description" title={node.description || node.name}>
                        {node.description || node.name}
                      </span>
                    </div>

                    <div className="node-badges-row">
                      <span
                        className={`relay-badge ${
                          node.can_relay ? 'relay-ok' : 'relay-no'
                        }`}
                      >
                        {node.can_relay ? '✓ 可中转' : '✕ 禁中转'}
                      </span>
                      {node.uptime_pct != null && (
                        <span className="uptime-badge">
                          24h 在线率 {node.uptime_pct}%
                        </span>
                      )}
                      {node.is_masked && (
                        <span className="masked-badge">需加群解锁</span>
                      )}
                    </div>
                  </div>

                  <div className="node-action-col">
                    {node.is_masked ? (
                      <button
                        type="button"
                        className="btn-select-node disabled"
                        disabled
                        title="包含*的节点为受保护节点，请加入 EasyTier 官方 QQ 群获取完整地址"
                      >
                        需加群
                      </button>
                    ) : (
                      <button
                        type="button"
                        className={`btn-select-node ${isCurrent ? 'current' : 'primary-action'}`}
                        onClick={() => {
                          onSelect(node.address);
                          onClose();
                        }}
                      >
                        {isCurrent ? '已选' : '选择'}
                      </button>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>

        {/* Footer info notice */}
        <div className="public-server-footer">
          <small>
            💡 提示：带 <span style={{ color: 'var(--ink)' }}>*</span> 的节点地址已打码，完整地址可在 EasyTier 官方 QQ 群获取（一群: 949700262，二群: 837676408，三群: 957189589）。
          </small>
        </div>
      </div>
    </div>
  );
};

