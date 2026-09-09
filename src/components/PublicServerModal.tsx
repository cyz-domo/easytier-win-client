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
  uptime_pct?: number | null;
  can_relay: boolean;
  is_masked: boolean;
  description: string;
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
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'domestic' | 'overseas'>('domestic');

  const fetchNodes = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<PublicNode[]>('fetch_public_nodes');
      setNodes(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      if (nodes.length === 0) {
        void fetchNodes();
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
      // 延迟从低到高
      const pingA = a.ping_ms ?? 9999;
      const pingB = b.ping_ms ?? 9999;
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
                实时状态与延迟测速 · 数据源自社区监控
              </small>
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <button
              type="button"
              className="ghost"
              style={{ fontSize: 12, padding: '4px 8px', display: 'inline-flex', alignItems: 'center', gap: 4 }}
              onClick={() => void fetchNodes()}
              disabled={loading}
              title="刷新实时在线状态与延迟"
            >
              <span className={loading ? 'icon-spin' : ''} style={{ display: 'inline-flex' }}>
                <IconRefresh size={13} />
              </span>
              <span>{loading ? '刷新中…' : '刷新'}</span>
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
            <div className="public-node-empty">正在拉取公共节点列表与实时延迟…</div>
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
                      <span
                        className={`node-status-badge ${
                          node.is_online ? 'online' : 'offline'
                        }`}
                      >
                        <span className="status-dot-sm" />
                        {node.is_online
                          ? `${node.ping_ms != null ? `${node.ping_ms} ms` : '在线'}`
                          : '离线'}
                      </span>
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
