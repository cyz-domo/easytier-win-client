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
  selectedAddresses?: string[];
  onToggle?: (address: string) => void;
  currentAddress?: string;
  onSelect?: (address: string) => void;
}

export const PublicServerModal: React.FC<PublicServerModalProps> = ({
  isOpen,
  onClose,
  selectedAddresses = [],
  onToggle,
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
      // 真实延迟从低到高：有效本机延迟(>=2ms)优先，否则使用监控站真实探针延迟
      const getEffectivePing = (n: PublicNode) => {
        if (n.local_ping_ms != null && n.local_ping_ms >= 2) {
          return n.local_ping_ms;
        }
        return n.ping_ms ?? 9999;
      };
      return getEffectivePing(a) - getEffectivePing(b);
    });
  }, [nodes, activeTab]);

  const isNodeSelected = useCallback(
    (addr: string) => {
      const clean = addr.trim().toLowerCase();
      if (selectedAddresses.length > 0) {
        return selectedAddresses.some(a => a.trim().toLowerCase() === clean);
      }
      if (currentAddress) {
        return currentAddress.trim().toLowerCase() === clean;
      }
      return false;
    },
    [selectedAddresses, currentAddress]
  );

  const selectedCount = selectedAddresses.length;

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
                实时状态与双向测速 · 支持多选添加至对等节点列表{selectedCount > 0 ? ` (已添加 ${selectedCount} 个)` : ''}
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
              const isSelected = isNodeSelected(node.address);
              const proto = node.address.split('://')[0]?.toUpperCase() || 'TCP';

              return (
                <div
                  key={`${node.id}-${node.address}`}
                  className={`public-node-card ${node.is_masked ? 'masked' : ''} ${
                    isSelected ? 'selected' : ''
                  }`}
                >
                  <div className="node-info-col">
                    <div className="node-address-row">
                      <span className="node-proto-tag">{proto}</span>
                      <strong className="node-address" title={node.address}>
                        {node.address}
                      </strong>
                      {node.local_ping_ms != null && node.local_ping_ms >= 2 ? (
                        <>
                          <span
                            className="node-status-badge online"
                            title={`本机真实 TCP 握手测速延迟: ${node.local_ping_ms} ms`}
                          >
                            <span className="status-dot-sm" />
                            本机 {node.local_ping_ms} ms
                          </span>
                          {node.ping_ms != null && (
                            <span
                              className="probe-badge"
                              title={`第三方探针机房参考延迟: ${node.ping_ms} ms`}
                            >
                              机房 {node.ping_ms} ms
                            </span>
                          )}
                        </>
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
                          title={
                            node.is_online
                              ? `节点在线 · 延迟 ${node.ping_ms ?? '未知'} ms（已防代理虚假 0ms 拦截，展示真实探针延迟）`
                              : '节点离线'
                          }
                        >
                          <span className="status-dot-sm" />
                          {node.is_online
                            ? `${node.ping_ms != null ? `${node.ping_ms} ms` : '在线'}`
                            : '离线'}
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
                        className={`btn-select-node ${isSelected ? 'added' : 'primary-action'}`}
                        onClick={() => {
                          if (onToggle) {
                            onToggle(node.address);
                          } else if (onSelect) {
                            onSelect(node.address);
                            onClose();
                          }
                        }}
                        title={isSelected ? '已在当前节点列表中，点击移除' : '点击添加至节点列表（可多选）'}
                      >
                        {isSelected ? '✓ 已添加' : '＋ 添加'}
                      </button>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>

        {/* Footer info notice & action */}
        <div className="public-server-footer">
          <small style={{ flex: 1, minWidth: 240 }}>
            💡 提示：带 <span style={{ color: 'var(--ink)' }}>*</span> 的节点地址已打码，完整地址可在 EasyTier 官方 QQ 群获取（一群: 949700262，二群: 837676408，三群: 957189589）。
          </small>
          <button
            type="button"
            className="mini-button"
            style={{
              padding: '6px 18px',
              fontSize: 12,
              fontWeight: 600,
              background: 'var(--accent)',
              color: '#ffffff',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              cursor: 'pointer',
              whiteSpace: 'nowrap',
            }}
            onClick={onClose}
          >
            完成选择 {selectedCount > 0 ? `(${selectedCount})` : ''}
          </button>
        </div>
      </div>
    </div>
  );
};

