import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { defaultConfig, NetworkConfig, validateConfig } from './network-config';
import { decodeTOML, encodeTOML } from './toml-codec';
import { ConfigEditor } from './ConfigEditor';
import { NodeStatus, PeerColumn, PeerInfo, PEER_COLUMNS, RefreshInterval, RouteInfo, formatBytes, parseNodeJSON, parsePeerJSON, parseRouteJSON } from './status-data';

type Status = 'running' | 'stopped' | 'starting' | 'stopping' | 'failed';
type Tab = 'status' | 'peers' | 'routes' | 'config' | 'logs' | 'settings';

interface Instance {
  id: string;
  name: string;
  status: Status;
  rpcPort: number;
  config: NetworkConfig;
}

const load = <T,>(k: string, d: T): T => {
  try { return JSON.parse(localStorage.getItem(k) || JSON.stringify(d)) as T; } catch { return d; }
};

function nextRpcPort(instances: Instance[]): number {
  const used = new Set(instances.map(i => i.rpcPort));
  for (let p = 15888; p < 16888; p++) if (!used.has(p)) return p;
  return 0; // random
}

interface runtimeInfo { available: boolean; version: string; core_path?: string }

export default function App() {
  const [instances, setInstances] = useState<Instance[]>(() =>
    load('easytier.instances.v2', [{ id: crypto.randomUUID(), name: '我的网络', status: 'stopped', rpcPort: 15888, config: defaultConfig() }]));
  const [activeId, setActiveId] = useState<string>(() => load('easytier.active.v2', ''));
  const [tab, setTab] = useState<Tab>('status');
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [runtime, setRuntime] = useState<runtimeInfo | null>(null);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [routes, setRoutes] = useState<RouteInfo[]>([]);
  const [node, setNode] = useState<NodeStatus | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [secretVisible, setSecretVisible] = useState(false);
  const [visibleCols, setVisibleCols] = useState<PeerColumn[]>(() =>
    load('easytier.peer-cols.v1', PEER_COLUMNS.filter(c => c.defaultOn).map(c => c.key)));
  const [refreshSecs, setRefreshSecs] = useState<RefreshInterval>(() => load('easytier.refresh.v1', 3));
  const [showDisplaySettings, setShowDisplaySettings] = useState(false);
  const logTimer = useRef<number | null>(null);

  const current = useMemo(() => instances.find(i => i.id === activeId) ?? instances[0], [instances, activeId]);

  useEffect(() => { localStorage.setItem('easytier.instances.v2', JSON.stringify(instances)); }, [instances]);
  useEffect(() => { if (current) localStorage.setItem('easytier.active.v2', current.id); }, [current]);
  useEffect(() => { invoke<runtimeInfo>('detect_runtime', {}).then(v => setRuntime({ ...v, core_path: v.core_path ?? 'core/easytier-core.exe' })).catch(() => setRuntime(null)); }, []);

  useEffect(() => { localStorage.setItem('easytier.peer-cols.v1', JSON.stringify(visibleCols)); }, [visibleCols]);
  useEffect(() => { localStorage.setItem('easytier.refresh.v1', JSON.stringify(refreshSecs)); }, [refreshSecs]);

  // Poll peer/route status while an instance is running.
  useEffect(() => {
    if (current?.status !== 'running') { setPeers([]); setRoutes([]); setNode(null); setStatusError(null); return; }
    let alive = true;
    const refresh = async () => {
      try {
        const [peerText, routeText, nodeText] = await Promise.all([
          invoke<string>('run_cli', { args: ['--rpc-portal', `127.0.0.1:${current.rpcPort}`, '--output', 'json', 'peer'] }),
          invoke<string>('run_cli', { args: ['--rpc-portal', `127.0.0.1:${current.rpcPort}`, '--output', 'json', 'route'] }),
          invoke<string>('run_cli', { args: ['--rpc-portal', `127.0.0.1:${current.rpcPort}`, '--output', 'json', 'node'] }),
        ]);
        if (!alive) return;
        setPeers(parsePeerJSON(peerText));
        setRoutes(parseRouteJSON(routeText));
        setNode(parseNodeJSON(nodeText));
        setStatusError(null);
      } catch (e) {
        if (alive) setStatusError(String(e));
      }
    };
    void refresh();
    const t = window.setInterval(refresh, refreshSecs * 1000);
    return () => { alive = false; window.clearInterval(t); };
  }, [current?.id, current?.status, current?.rpcPort, refreshSecs]);

  // Append runtime messages to the log pane.
  const addLog = (line: string) => setLogs(xs => [...xs.slice(-300), `${new Date().toLocaleTimeString()}  ${line}`]);
  useEffect(() => { if (statusError) addLog(`状态查询失败：${statusError}`); }, [statusError]);

  const patchConfig = (patch: Partial<NetworkConfig>) =>
    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, config: { ...i.config, ...patch } } : i)));

  const addInstance = () => {
    const id = crypto.randomUUID();
    setInstances(xs => [...xs, { id, name: '新网络', status: 'stopped', rpcPort: nextRpcPort(xs), config: defaultConfig() }]);
    setActiveId(id);
    setTab('config');
  };

  const removeInstance = (id: string) => {
    if (instances.length === 1) return;
    setInstances(xs => xs.filter(i => i.id !== id));
    if (activeId === id) setActiveId(instances.find(i => i.id !== id)!.id);
  };

  const renameInstance = (name: string) =>
    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, name } : i)));

  const toggle = async () => {
    const running = current.status === 'running';
    setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: running ? 'stopping' : 'starting' } : i)));
    try {
      if (running) {
        await invoke('stop_instance', { id: current.id });
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: 'stopped' } : i)));
        addLog(`[${current.name}] 网络已停止`);
      } else {
        const errors = validateConfig(current.config);
        if (errors.length) throw new Error(errors[0].message);
        const toml = encodeTOML(current.config);
        await invoke('start_instance', { id: current.id, config: toml, rpcPortal: `127.0.0.1:${current.rpcPort}` });
        setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: 'running' } : i)));
        addLog(`[${current.name}] 网络已启动，RPC ${current.rpcPort}`);
      }
    } catch (e) {
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, status: 'failed' } : i)));
      addLog(`[${current.name}] 操作失败：${String(e)}`);
      alert(`网络操作失败：${String(e)}`);
    }
  };

  const importToml = async (text: string) => {
    try {
      const config = decodeTOML(text);
      patchConfig(config);
      addLog('TOML 配置导入成功');
      alert('配置导入成功');
    } catch (e) {
      addLog(`TOML 导入失败：${String(e)}`);
      alert(`导入失败：${String(e)}`);
    }
  };

  const exportToml = async (copy = false) => {
    const hasSecret = !!current.config.network_secret;
    if (copy && hasSecret && !secretVisible) {
      if (!confirm('配置中包含网络密钥，确定复制到剪贴板吗？')) return;
    }
    const text = encodeTOML(current.config, true);
    try {
      if (copy) await navigator.clipboard.writeText(text);
      else {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(new Blob([text], { type: 'application/toml' }));
        a.download = `${current.name || 'easytier'}.toml`;
        a.click();
        URL.revokeObjectURL(a.href);
      }
      addLog(copy ? 'TOML 已复制到剪贴板' : 'TOML 已导出为文件');
    } catch (e) { alert(`导出失败：${String(e)}`); }
  };

  const openTomlFile = async () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.toml,text/plain';
    input.onchange = async () => {
      const f = input.files?.[0];
      if (f) await importToml(await f.text());
    };
    input.click();
  };

  if (!current) return null;
  const running = current.status === 'running';
  const statusText = { running: '网络运行中', stopped: '网络已停止', starting: '正在启动…', stopping: '正在停止…', failed: '启动失败' }[current.status];

  const navItems: [Tab, string][] = [['status', '状态总览'], ['peers', '组网成员'], ['routes', '路由信息'], ['config', '组网配置'], ['logs', '运行日志'], ['settings', '设置']];

  return (
    <main className="app-shell">
      <aside>
        <div className="brand"><span className="brand-mark">E</span><div><strong>EasyTier</strong><small>Windows Client</small></div></div>
        <div className="section-label">网络实例</div>
        <nav>
          {instances.map(i => (
            <button className={i.id === current.id ? 'nav-item active' : 'nav-item'} onClick={() => { setActiveId(i.id); setTab('status'); }} key={i.id}>
              <span className={i.status === 'running' ? 'dot on' : i.status === 'failed' ? 'dot err' : 'dot'} />{i.name}
              <span className="chevron">›</span>
            </button>
          ))}
        </nav>
        <button className="add-button" onClick={addInstance}>＋ 新建实例</button>
        <div className="sidebar-bottom">
          <button className="quiet" onClick={() => setTab('settings')}>⚙　设置</button>
          <span className="version">{runtime?.version ? `核心 ${runtime.version.split(' ').pop()}` : '核心未检测'} · 客户端 0.1.0</span>
        </div>
      </aside>

      <section className="content">
        <header>
          <div>
            <p className="eyebrow">{navItems.find(([t]) => t === tab)?.[1]}</p>
            <h1>{current.name}</h1>
          </div>
          <div className="header-actions">
            {running && <span className="rpc-badge">RPC :{current.rpcPort}</span>}
            <button className={running ? 'stop' : 'primary'} onClick={() => void toggle()} disabled={current.status === 'starting' || current.status === 'stopping'}>
              {running ? '停止网络' : '启动网络'}
            </button>
          </div>
        </header>

        <nav className="tabbar">
          {navItems.map(([t, label]) => (
            <button key={t} className={tab === t ? 'tab active' : 'tab'} onClick={() => setTab(t)}>{label}</button>
          ))}
        </nav>

        {tab === 'status' && (
          <>
            <div className="status-card">
              <div className="status-icon">{running ? '✓' : '−'}</div>
              <div>
                <span className="card-label">当前状态</span>
                <h2>{statusText}</h2>
                <p>{running ? `RPC 管理端口 127.0.0.1:${current.rpcPort}${node?.version ? ` · 核心 ${node.version}` : ''}` : '启动网络后，设备将加入 EasyTier 虚拟网络。'}</p>
                {running && !node?.ipv4_addr && (
                  <p className="warn-inline">⚠ 本机尚未获得虚拟 IPv4 —— 请在「组网配置 → 基础配置」勾选 DHCP 或手动填写虚拟 IPv4，然后重启网络（TUN 需要管理员权限运行客户端）。</p>
                )}
              </div>
              <div className="status-meta"><span>本机虚拟地址</span><b>{node?.ipv4_addr || (current.config.dhcp ? '等待分配' : current.config.virtual_ipv4 || '—')}</b></div>
            </div>
            <div className="metrics">
              <article><span>组网成员</span><strong>{running ? peers.length : '—'}</strong><small>{running ? '在线节点' : '未运行'}</small></article>
              <article><span>路由条目</span><strong>{running ? routes.length : '—'}</strong><small>{running ? '已知网段' : '未运行'}</small></article>
              <article><span>收发总量</span><strong>{running ? formatBytes(peers.reduce((s, p) => s + (Number(p.rx_bytes) || 0) + (Number(p.tx_bytes) || 0), 0)) : '—'}</strong><small>所有成员合计</small></article>
            </div>
            <div className="section-heading"><div><h2>快速操作</h2><p>常用配置与信息入口</p></div></div>
            <div className="config-grid">
              <button className="config-card" onClick={() => setTab('config')}><span className="config-icon">◎</span><div><b>组网配置</b><small>网络名称、密钥、地址与高级参数</small></div><span>›</span></button>
              <button className="config-card" onClick={() => setTab('peers')}><span className="config-icon">⇄</span><div><b>组网成员</b><small>查看在线节点与连接质量</small></div><span>›</span></button>
              <button className="config-card" onClick={() => setTab('logs')}><span className="config-icon">≡</span><div><b>运行日志</b><small>启动、停止与错误记录</small></div><span>›</span></button>
            </div>
          </>
        )}

        {tab === 'peers' && (
          <div className="card">
            <div className="card-title-row">
              <h3 className="card-title">组网成员{running ? `（${peers.length}）` : ''}</h3>
              <div className="title-actions">
                <span className="hint-inline">刷新 {refreshSecs}s</span>
                <button className="ghost" onClick={() => setShowDisplaySettings(x => !x)}>显示设置</button>
              </div>
            </div>
            {showDisplaySettings && (
              <div className="display-settings">
                <div className="display-cols">
                  <span className="field-label">数据项</span>
                  <div className="display-col-list">
                    {PEER_COLUMNS.map(c => (
                      <label key={c.key} className="check-item">
                        <input type="checkbox" checked={visibleCols.includes(c.key)}
                          onChange={e => setVisibleCols(xs => e.target.checked ? [...xs, c.key] : xs.filter(x => x !== c.key))} />
                        {c.label}
                      </label>
                    ))}
                  </div>
                </div>
                <div className="display-refresh">
                  <span className="field-label">刷新</span>
                  <select className="field-input narrow" value={refreshSecs}
                    onChange={e => setRefreshSecs(parseInt(e.target.value, 10) as RefreshInterval)}>
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
            {running && statusError && <p className="list-empty err">状态查询失败：{statusError}</p>}
            {running && !statusError && (
              <table className="data-table">
                <thead><tr>
                  <th>节点 ID</th>
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
                </tr></thead>
                <tbody>
                  {peers.map((p, i) => {
                    const route = routes.find(r => r.hostname === p.hostname);
                    const isLocal = p.cost === 'Local';
                    const nodeType = isLocal ? '本机' : (route && (route.path_len ?? 0) > 1 ? '服务节点' : '普通节点');
                    return (
                      <tr key={String(p.id ?? i)}>
                        <td>{String(p.id ?? '—')}<div className="cell-sub">{nodeType}</div></td>
                        {visibleCols.includes('ipv4') && <td>{p.ipv4 || '—'}</td>}
                        {visibleCols.includes('cidr') && <td>{p.cidr || '—'}</td>}
                        {visibleCols.includes('hostname') && <td>{p.hostname || '—'}</td>}
                        {visibleCols.includes('cost') && <td>{p.cost || '—'}</td>}
                        {visibleCols.includes('proto') && <td>{p.tunnel_proto || '—'}</td>}
                        {visibleCols.includes('latency') && <td>{p.lat_ms || '—'}</td>}
                        {visibleCols.includes('loss') && <td>{p.loss_rate || '—'}</td>}
                        {visibleCols.includes('rx') && <td>{formatBytes(p.rx_bytes)}</td>}
                        {visibleCols.includes('tx') && <td>{formatBytes(p.tx_bytes)}</td>}
                        {visibleCols.includes('nat') && <td>{p.nat_type || '—'}</td>}
                        {visibleCols.includes('version') && <td>{p.version || '—'}</td>}
                        {visibleCols.includes('relay') && <td>{route && (route.path_len ?? 0) > 1 ? route.next_hop_hostname : '—'}</td>}
                        {visibleCols.includes('routes') && <td>{route?.proxy_cidrs && route.proxy_cidrs !== '' ? String(route.proxy_cidrs) : '—'}</td>}
                      </tr>
                    );
                  })}
                  {peers.length === 0 && <tr><td colSpan={14} className="list-empty">暂无在线成员</td></tr>}
                </tbody>
              </table>
            )}
          </div>
        )}

        {tab === 'routes' && (
          <div className="card">
            <h3 className="card-title">路由信息{running ? `（${routes.length}）` : ''}</h3>
            {!running && <p className="list-empty">网络未运行，启动后此处显示路由表。</p>}
            {running && (
              <table className="data-table">
                <thead><tr><th>虚拟地址</th><th>主机名</th><th>下一跳</th><th>跳数</th><th>路径延迟</th><th>子网代理</th><th>版本</th></tr></thead>
                <tbody>
                  {routes.map((r, i) => (
                    <tr key={i}>
                      <td>{r.ipv4 || '—'}</td>
                      <td>{r.hostname || '—'}</td>
                      <td>{r.next_hop_hostname || r.next_hop_ipv4 || '—'}</td>
                      <td>{r.path_len ?? '—'}</td>
                      <td>{r.path_latency ? `${r.path_latency} ms` : '—'}</td>
                      <td>{r.proxy_cidrs && String(r.proxy_cidrs) !== '' ? String(r.proxy_cidrs) : '—'}</td>
                      <td>{r.version || '—'}</td>
                    </tr>
                  ))}
                  {routes.length === 0 && <tr><td colSpan={7} className="list-empty">暂无路由</td></tr>}
                </tbody>
              </table>
            )}
          </div>
        )}

        {tab === 'config' && (
          <div className="card config-card-wide">
            <div className="card-title-row">
              <input className="title-input" value={current.name} onChange={e => renameInstance(e.target.value)} />
              <div className="title-actions">
                <button className="ghost" onClick={() => void openTomlFile()}>导入 TOML</button>
                <button className="ghost" onClick={async () => { try { await importToml(await navigator.clipboard.readText()); } catch (e) { alert(`读取剪贴板失败：${String(e)}`); } }}>剪贴板导入</button>
                <button className="ghost" onClick={() => void exportToml()}>导出 TOML</button>
                <button className="ghost" onClick={() => void exportToml(true)}>复制</button>
                <button className="mini-button danger" onClick={() => removeInstance(current.id)}>删除实例</button>
              </div>
            </div>
            <ConfigEditor config={current.config} onChange={patchConfig} showAdvanced={showAdvanced} onToggleAdvanced={() => setShowAdvanced(x => !x)} />
            <details className="toml-preview">
              <summary>查看当前 TOML</summary>
              <pre>{encodeTOML(current.config)}</pre>
            </details>
          </div>
        )}

        {tab === 'logs' && (
          <div className="card">
            <div className="card-title-row">
              <h3 className="card-title">运行日志</h3>
              <button className="ghost" onClick={() => setLogs([])}>清空</button>
            </div>
            <div className="log-pane" ref={el => { if (el && logTimer.current == null) el.scrollTop = el.scrollHeight; }}>
              {logs.length === 0 ? <p className="list-empty">暂无日志</p> : logs.map((l, i) => <div className="log-line" key={i}>{l}</div>)}
            </div>
          </div>
        )}

        {tab === 'settings' && (
          <>
            <div className="card">
              <h3 className="card-title">运行时</h3>
              <table className="kv-table">
                <tbody>
                  <tr><th>核心路径</th><td>{runtime?.core_path ?? 'core/easytier-core.exe'}</td></tr>
                  <tr><th>核心版本</th><td>{runtime?.version ?? '未检测'}</td></tr>
                  <tr><th>运行时可用</th><td>{runtime?.available ? '是' : '否（请检查 core 目录）'}</td></tr>
                </tbody>
              </table>
            </div>
            <div className="card">
              <h3 className="card-title">RPC 管理</h3>
              <p className="hint">每个实例使用独立 RPC 端口，避免多实例冲突。启动网络后可通过 easytier-cli 连接该端口查询状态。</p>
              <table className="kv-table">
                <tbody>
                  {instances.map(i => (
                    <tr key={i.id}>
                      <th>{i.name}</th>
                      <td className="inline">
                        <input className="field-input narrow" type="number" value={i.rpcPort} min={1024} max={65535}
                          onChange={e => setInstances(xs => xs.map(x => (x.id === i.id ? { ...x, rpcPort: parseInt(e.target.value, 10) || x.rpcPort } : x)))} />
                        <span className="hint-inline">{i.status === 'running' ? '运行中' : '已停止'}</span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="card">
              <h3 className="card-title">关于</h3>
              <p className="hint">EasyTier Windows Client 0.1.0 · Tauri 2 + React · 设计文档见仓库 docs/ 目录。</p>
              <label className="toggle-row"><span className="field-label">显示密钥（导出时提醒）</span>
                <button className={secretVisible ? 'switch on' : 'switch'} role="switch" aria-checked={secretVisible} onClick={() => setSecretVisible(v => !v)}><span className="knob" /></button>
              </label>
            </div>
          </>
        )}
      </section>
    </main>
  );
}
