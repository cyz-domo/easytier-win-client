import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface RemoteInstanceInfo {
  instance_id: string;
  hostname: string;
  virtual_ip: string;
}

interface RemoteConfigEditorProps {
  host: string;
  /** Target instance's RPC portal port (multi-instance hosts use several). */
  port: number;
  candidatePorts?: number[];
  onPortChange?: (port: number) => void;
  onSaved?: (patched: { hostname: string; ipv4Addr: string; ipv4Len: number; proxyCidrs: string[]; exitNodes: string[] }, port: number) => void;
  /** Unique per mount: state resets when the key changes. */
  onClose?: () => void;
}

interface EditState {
  hostname: string;
  ipv4Addr: string;
  ipv4Len: number;
  proxyCidrs: string[];
  exitNodes: string[];
}

type Phase = 'discover' | 'load' | 'ready' | 'saving' | 'saved';

const STR: Record<string, string> = {
  discover: '正在发现远端实例…',
  load: '正在读取远端配置…',
  saving: '正在下发配置…',
};

function normalizeIp(ip: string): string {
  return ip.split('/')[0].trim();
}

function parseIpv4Cidr(cidrStr: string) {
  const [ip, lenStr] = cidrStr.trim().split('/');
  const parts = ip.split('.').map(n => parseInt(n, 10) | 0);
  if (parts.length !== 4 || parts.some(n => Number.isNaN(n) || n < 0 || n > 255)) return null;
  const len = parseInt(lenStr ?? '24', 10) || 24;
  return {
    address: { addr: ((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]) >>> 0 },
    network_length: len,
  };
}

function parseExitNode(ipStr: string) {
  const parts = ipStr.trim().split('.').map(n => parseInt(n, 10) | 0);
  if (parts.length !== 4 || parts.some(n => Number.isNaN(n) || n < 0 || n > 255)) return null;
  return {
    addr: {
      Ipv4: ((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]) >>> 0,
    },
  };
}

/** Map get_config response fields to the editable subset. */
function toEditState(config: Record<string, unknown>): EditState {
  const ipv4 = typeof config.virtual_ipv4 === 'string' ? config.virtual_ipv4 : '';
  const [addr, len] = ipv4.split('/');
  return {
    hostname: typeof config.hostname === 'string' ? config.hostname : '',
    ipv4Addr: addr ?? '',
    ipv4Len: parseInt(len ?? '24', 10) || 24,
    proxyCidrs: Array.isArray(config.proxy_cidrs) ? config.proxy_cidrs.map(String) : [],
    exitNodes: Array.isArray(config.exit_nodes) ? config.exit_nodes.map(String) : [],
  };
}

/**
 * Editable body shared by the dialog and the tab panel. Loads the remote
 * config, tracks unsaved edits, and patches only the fields the remote
 * patch_config protocol supports.
 */
export function RemoteConfigEditor({ host, port, candidatePorts, onPortChange, onSaved, onClose }: RemoteConfigEditorProps) {
  const [phase, setPhase] = useState<Phase>('discover');
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<RemoteInstanceInfo | null>(null);
  const [edit, setEdit] = useState<EditState | null>(null);
  const [snapshot, setSnapshot] = useState<EditState | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  const discoverAndLoad = useCallback(async () => {
    setError(null);
    setPhase('discover');
    const portsToTry = candidatePorts && candidatePorts.length > 0
      ? Array.from(new Set([port, ...candidatePorts]))
      : [port];

    let foundInfo: RemoteInstanceInfo | null = null;
    let effectivePort = port;
    let lastErr: unknown = null;

    const probe = async (p: number) => {
      const disc = await invoke<RemoteInstanceInfo>('remote_config_discover', { host, port: p, virtualIp: host });
      return { disc, port: p };
    };

    try {
      const winner = await Promise.any(portsToTry.map(p => probe(p)));
      foundInfo = winner.disc;
      effectivePort = winner.port;
    } catch (e) {
      lastErr = e;
    }

    if (!foundInfo) {
      setError(String(lastErr || '未能通过已知 RPC 端口连接到目标设备'));
      setPhase('discover');
      return;
    }

    if (effectivePort !== port && onPortChange) {
      onPortChange(effectivePort);
    }

    setInfo(foundInfo);
    setPhase('load');
    try {
      const cfg = await invoke<Record<string, unknown>>('remote_config_load', { host, port: effectivePort, instanceId: foundInfo.instance_id });
      const config = (cfg.config ?? cfg) as Record<string, unknown>;
      const state = toEditState(config);
      setEdit(state);
      setSnapshot(state);
      setPhase('ready');
    } catch (e) {
      setError(String(e));
      setPhase('load');
    }
  }, [host, port, candidatePorts, onPortChange]);

  useEffect(() => { void discoverAndLoad(); }, [discoverAndLoad, reloadKey]);

  const dirty = useMemo(() => edit != null && snapshot != null && JSON.stringify(edit) !== JSON.stringify(snapshot), [edit, snapshot]);

  const save = async () => {
    if (!edit || !info) return;
    setError(null);
    setPhase('saving');
    try {
      const patch: Record<string, unknown> = {};
      if (edit.hostname !== snapshot?.hostname) patch.hostname = edit.hostname;
      if (edit.ipv4Addr !== snapshot?.ipv4Addr || edit.ipv4Len !== snapshot?.ipv4Len) {
        const parts = normalizeIp(edit.ipv4Addr).split('.').map(n => parseInt(n, 10) | 0);
        if (parts.length !== 4 || parts.some(n => Number.isNaN(n) || n < 0 || n > 255)) {
          throw new Error(`虚拟 IPv4 格式不正确：${edit.ipv4Addr}`);
        }
        patch.ipv4 = {
          address: { addr: ((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]) >>> 0 },
          network_length: edit.ipv4Len,
        };
      }
      if (JSON.stringify(edit.proxyCidrs) !== JSON.stringify(snapshot?.proxyCidrs)) {
        const validProxies = edit.proxyCidrs.map(parseIpv4Cidr).filter(Boolean);
        patch.proxy_networks = [
          { action: 2 }, // Clear
          ...validProxies.map(c => ({ action: 0, cidr: c })),
        ];
      }
      if (JSON.stringify(edit.exitNodes) !== JSON.stringify(snapshot?.exitNodes)) {
        const validNodes = edit.exitNodes.map(parseExitNode).filter(Boolean);
        patch.exit_nodes = [
          { action: 2 }, // Clear
          ...validNodes.map(n => ({ action: 0, node: n })),
        ];
      }
      if (Object.keys(patch).length === 0) { setPhase('ready'); return; }
      await invoke('remote_config_patch', { host, port, instanceId: info.instance_id, patch });
      setSnapshot(edit);
      onSaved?.(edit, port);
      setPhase('saved');
      setTimeout(() => setPhase(p => (p === 'saved' ? 'ready' : p)), 1600);
    } catch (e) {
      setError(String(e));
      setPhase('ready');
    }
  };

  const busy = phase === 'saving';

  return (
    <div className="remote-editor">
      {info && <p className="hint">实例 {info.instance_id} · 主机名 {info.hostname || '—'}</p>}
      {(phase === 'discover' || phase === 'load') && !error && <p className="list-empty">{STR[phase]}</p>}
      {phase === 'saving' && <p className="list-empty">{STR[phase]}</p>}
      {phase === 'saved' && <p className="list-empty">配置已下发并在远端生效。</p>}
      {error && (
        <p className="list-empty err">
          {error}　<button type="button" className="ghost" onClick={() => setReloadKey(k => k + 1)}>重试</button>
          {onClose && <button type="button" className="ghost" onClick={onClose}>关闭</button>}
        </p>
      )}
      {edit && snapshot && (
        <div className="editor">
          <section className="card">
            <h3 className="card-title">基础</h3>
            <div className="grid-2">
              <label className="field">
                <span className="field-label">主机名</span>
                <input className="field-input" value={edit.hostname}
                  onChange={e => setEdit({ ...edit, hostname: e.target.value })} />
              </label>
              <label className="field">
                <span className="field-label">虚拟 IPv4</span>
                <div className="inline">
                  <input className="field-input" value={edit.ipv4Addr} placeholder="10.144.144.10"
                    onChange={e => setEdit({ ...edit, ipv4Addr: e.target.value })} />
                  <span className="slash">/</span>
                  <input className="field-input narrow" type="number" min={1} max={32} value={edit.ipv4Len}
                    onChange={e => setEdit({ ...edit, ipv4Len: parseInt(e.target.value, 10) || 24 })} />
                </div>
              </label>
            </div>
          </section>
          <section className="card">
            <h3 className="card-title">路由与出口</h3>
            <div className="list-editor">
              <div className="list-editor-head">
                <span className="field-label">代理网络（子网代理）</span>
                <button type="button" className="mini-button" onClick={() => setEdit({ ...edit, proxyCidrs: [...edit.proxyCidrs, ''] })}>＋ 添加</button>
              </div>
              {edit.proxyCidrs.length === 0 && <p className="list-empty">暂无条目</p>}
              {edit.proxyCidrs.map((v, i) => (
                <div className="list-row" key={i}>
                  <input className="field-input" value={v} placeholder="10.0.0.0/24"
                    onChange={e => setEdit({ ...edit, proxyCidrs: edit.proxyCidrs.map((x, j) => (j === i ? e.target.value : x)) })} />
                  <button type="button" className="mini-button danger" onClick={() => setEdit({ ...edit, proxyCidrs: edit.proxyCidrs.filter((_, j) => j !== i) })}>删除</button>
                </div>
              ))}
            </div>
            <div className="list-editor">
              <div className="list-editor-head">
                <span className="field-label">出口节点</span>
                <button type="button" className="mini-button" onClick={() => setEdit({ ...edit, exitNodes: [...edit.exitNodes, ''] })}>＋ 添加</button>
              </div>
              {edit.exitNodes.length === 0 && <p className="list-empty">暂无条目</p>}
              {edit.exitNodes.map((v, i) => (
                <div className="list-row" key={i}>
                  <input className="field-input" value={v} placeholder="10.144.144.1"
                    onChange={e => setEdit({ ...edit, exitNodes: edit.exitNodes.map((x, j) => (j === i ? e.target.value : x)) })} />
                  <button type="button" className="mini-button danger" onClick={() => setEdit({ ...edit, exitNodes: edit.exitNodes.filter((_, j) => j !== i) })}>删除</button>
                </div>
              ))}
            </div>
            <p className="hint">网络名称、密钥、监听器等字段只能在对应设备本机上修改，此处不做展示。</p>
          </section>
        </div>
      )}
      <div className="kernel-update-actions">
        <button type="button" className="primary" onClick={() => void save()}
          disabled={busy || !edit || !dirty || phase === 'saved'}>
          {dirty ? (phase === 'saving' ? '正在下发…' : '保存并下发') : '无修改'}
        </button>
      </div>
    </div>
  );
}

interface RemoteConfigDialogProps {
  host: string;
  port: number;
  candidatePorts?: number[];
  onSaved?: (patched: { hostname: string; ipv4Addr: string; ipv4Len: number; proxyCidrs: string[]; exitNodes: string[] }, port: number) => void;
  onClose: () => void;
}

/** Overlay wrapper around RemoteConfigEditor. */
export function RemoteConfigDialog({ host, port: initialPort, candidatePorts, onSaved, onClose }: RemoteConfigDialogProps) {
  const [port, setPort] = useState(initialPort);
  return (
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-label="远程配置">
      <div className="modal-card">
        <div className="card-title-row">
          <h3 className="card-title">远程配置 — {host}</h3>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <label style={{ fontSize: 12, display: 'inline-flex', alignItems: 'center', gap: 5 }}>
              <span>RPC 端口:</span>
              <input
                type="number"
                className="field-input narrow"
                style={{ width: 75, padding: '4px 6px' }}
                value={port}
                onChange={e => setPort(parseInt(e.target.value, 10) || 15888)}
              />
            </label>
            <button type="button" className="ghost" onClick={onClose}>关闭</button>
          </div>
        </div>
        <RemoteConfigEditor
          key={`${host}:${port}`}
          host={host}
          port={port}
          candidatePorts={candidatePorts}
          onPortChange={setPort}
          onSaved={onSaved}
          onClose={onClose}
        />
      </div>
    </div>
  );
}
