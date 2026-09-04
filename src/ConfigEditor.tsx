import { NetworkConfig, NetworkingMethod, PortForwardConfig } from './network-config';

interface FieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  disabled?: boolean;
  type?: 'text' | 'password' | 'number';
}

export function TextField({ label, value, onChange, placeholder, disabled, type = 'text' }: FieldProps) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <input className="field-input" type={type} value={value} placeholder={placeholder} disabled={disabled}
        onChange={e => onChange(e.target.value)} />
    </label>
  );
}

export function NumberField({ label, value, onChange, disabled, min, max }: FieldProps & { min?: number; max?: number; disabled?: boolean | null }) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <input className="field-input" type="number" value={value} min={min} max={max} disabled={disabled}
        onChange={e => onChange(e.target.value)} />
    </label>
  );
}

export function ToggleField({ label, value, onChange, hint }: { label: string; value: boolean | null; onChange: (v: boolean | null) => void; hint?: string }) {
  const checked = value === true;
  return (
    <label className="toggle-row" title={hint}>
      <span className="field-label">{label}</span>
      <button type="button" role="switch" aria-checked={checked} className={checked ? 'switch on' : 'switch'}
        onClick={() => {
          if (value === null) onChange(true);
          else if (value === true) onChange(false);
          else onChange(null);
        }}>
        <span className="knob" />
      </button>
    </label>
  );
}

export function StringListEditor({ title, values, onChange, placeholder }: { title: string; values: string[]; onChange: (v: string[]) => void; placeholder?: string }) {
  return (
    <div className="list-editor">
      <div className="list-editor-head">
        <span className="field-label">{title}</span>
        <button type="button" className="mini-button" onClick={() => onChange([...values, ''])}>＋ 添加</button>
      </div>
      {values.length === 0 && <p className="list-empty">暂无条目</p>}
      {values.map((v, i) => (
        <div className="list-row" key={i}>
          <input className="field-input" value={v} placeholder={placeholder} onChange={e => onChange(values.map((x, j) => (j === i ? e.target.value : x)))} />
          <button type="button" className="mini-button danger" onClick={() => onChange(values.filter((_, j) => j !== i))}>删除</button>
        </div>
      ))}
    </div>
  );
}

export interface EditorProps {
  config: NetworkConfig;
  onChange: (patch: Partial<NetworkConfig>) => void;
  showAdvanced: boolean;
  onToggleAdvanced: () => void;
}

export function ConfigEditor({ config: c, onChange, showAdvanced, onToggleAdvanced }: EditorProps) {
  const method = c.networking_method;
  const setMethod = (m: NetworkingMethod) => onChange({ networking_method: m });

  const updateForward = (id: string, patch: Partial<PortForwardConfig>) =>
    onChange({ port_forwards: c.port_forwards.map(f => (f.id === id ? { ...f, ...patch } : f)) });

  return (
    <div className="editor">
      <section className="card">
        <h3 className="card-title">基础配置</h3>
        <div className="grid-2">
          <TextField label="网络名称" value={c.network_name} onChange={v => onChange({ network_name: v })} placeholder="easytier" />
          <TextField label="网络密钥" type="password" value={c.network_secret ?? ''} onChange={v => onChange({ network_secret: v })} placeholder="用于验证节点归属" />
          <TextField label="主机名" value={c.hostname ?? ''} onChange={v => onChange({ hostname: v })} placeholder="本机在虚拟网中的名称" />
          <div className="field">
            <span className="field-label">组网方式</span>
            <div className="segmented">
              {([['公共服务器', 0], ['手动', 1], ['独立', 2]] as [string, NetworkingMethod][]).map(([label, m]) => (
                <button key={m} type="button" className={method === m ? 'seg active' : 'seg'} onClick={() => setMethod(m)}>{label}</button>
              ))}
            </div>
          </div>
        </div>
        {method === 0 && (
          <TextField label="公共服务器地址" value={c.public_server_url} onChange={v => onChange({ public_server_url: v })} placeholder="tcp://public.easytier.top:11010" />
        )}
        {method === 1 && (
          <StringListEditor title="初始节点（Peer URL）" values={c.peer_urls} onChange={v => onChange({ peer_urls: v })} placeholder="tcp://192.168.1.10:11010" />
        )}
        <div className="grid-2">
          <label className="field">
            <span className="field-label">虚拟 IPv4</span>
            <div className="inline">
              <input className="field-input" value={c.virtual_ipv4} placeholder="10.144.144.10" disabled={c.dhcp}
                onChange={e => onChange({ virtual_ipv4: e.target.value })} />
              <span className="slash">/</span>
              <input className="field-input narrow" type="number" min={1} max={32} value={c.network_length} disabled={c.dhcp}
                onChange={e => onChange({ network_length: parseInt(e.target.value, 10) || 24 })} />
            </div>
          </label>
          <ToggleField label="DHCP 自动分配虚拟 IP" value={c.dhcp ? true : false} onChange={v => onChange({ dhcp: v !== false })} />
        </div>
      </section>

      <button type="button" className="advanced-toggle" onClick={onToggleAdvanced}>
        {showAdvanced ? '▾ 收起高级设置' : '▸ 展开高级设置'}
      </button>

      {showAdvanced && (
        <>
          <section className="card">
            <h3 className="card-title">监听与转发</h3>
            <StringListEditor title="监听器" values={c.listener_urls} onChange={v => onChange({ listener_urls: v })} placeholder="tcp://0.0.0.0:11010" />
            <StringListEditor title="映射监听器" values={c.mapped_listeners} onChange={v => onChange({ mapped_listeners: v })} placeholder="tcp://123.123.123.123:11223" />
            <div className="grid-2">
              <ToggleField label="SOCKS5 服务器" value={c.enable_socks5} onChange={v => onChange({ enable_socks5: v })} />
              <NumberField label="SOCKS5 端口" value={String(c.socks5_port)} onChange={v => onChange({ socks5_port: parseInt(v, 10) || 1080 })} disabled={c.enable_socks5 !== true} min={1} max={65535} />
              <ToggleField label="VPN 门户（WireGuard）" value={c.enable_vpn_portal ? true : false} onChange={v => onChange({ enable_vpn_portal: v === true })} />
              <NumberField label="VPN 门户端口" value={String(c.vpn_portal_listen_port)} onChange={v => onChange({ vpn_portal_listen_port: parseInt(v, 10) || 22022 })} disabled={!c.enable_vpn_portal} min={1} max={65535} />
            </div>
            {c.enable_vpn_portal && (
              <div className="grid-2">
                <TextField label="客户端网段地址" value={c.vpn_portal_client_network_addr} onChange={v => onChange({ vpn_portal_client_network_addr: v })} placeholder="10.14.14.0" />
                <NumberField label="客户端网段前缀" value={String(c.vpn_portal_client_network_len)} onChange={v => onChange({ vpn_portal_client_network_len: parseInt(v, 10) || 24 })} min={1} max={32} />
              </div>
            )}
            <div className="list-editor">
              <div className="list-editor-head">
                <span className="field-label">端口转发</span>
                <button type="button" className="mini-button" onClick={() => onChange({
                  port_forwards: [...c.port_forwards, { id: crypto.randomUUID(), bind_ip: '0.0.0.0', bind_port: 12345, dst_ip: '10.126.126.1', dst_port: 23456, proto: 'tcp' }],
                })}>＋ 添加</button>
              </div>
              {c.port_forwards.map(f => (
                <div className="forward-row" key={f.id}>
                  <select className="field-input narrow" value={f.proto} onChange={e => updateForward(f.id, { proto: e.target.value })}>
                    <option value="tcp">TCP</option>
                    <option value="udp">UDP</option>
                  </select>
                  <input className="field-input" value={`${f.bind_ip}:${f.bind_port}`} onChange={e => {
                    const [ip, port] = e.target.value.split(':');
                    updateForward(f.id, { bind_ip: ip ?? '', bind_port: parseInt(port, 10) || 0 });
                  }} />
                  <span className="arrow">→</span>
                  <input className="field-input" value={`${f.dst_ip}:${f.dst_port}`} onChange={e => {
                    const [ip, port] = e.target.value.split(':');
                    updateForward(f.id, { dst_ip: ip ?? '', dst_port: parseInt(port, 10) || 0 });
                  }} />
                  <button type="button" className="mini-button danger" onClick={() => onChange({ port_forwards: c.port_forwards.filter(x => x.id !== f.id) })}>删除</button>
                </div>
              ))}
            </div>
          </section>

          <section className="card">
            <h3 className="card-title">路由与子网</h3>
            <StringListEditor title="代理网络（子网代理）" values={c.proxy_cidrs} onChange={v => onChange({ proxy_cidrs: v })} placeholder="10.0.0.0/24" />
            <ToggleField label="手动路由" value={c.enable_manual_routes} onChange={v => onChange({ enable_manual_routes: v === true })} />
            <StringListEditor title="路由 CIDR" values={c.routes} onChange={v => onChange({ routes: v })} placeholder="192.168.0.0/16" />
            <StringListEditor title="出口节点" values={c.exit_nodes} onChange={v => onChange({ exit_nodes: v })} placeholder="10.144.144.1" />
            <ToggleField label="启用出口节点" value={c.enable_exit_node} onChange={v => onChange({ enable_exit_node: v })} />
            <ToggleField label="中继白名单" value={c.enable_relay_network_whitelist} onChange={v => onChange({ enable_relay_network_whitelist: v })} />
            <StringListEditor title="允许的网络" values={c.relay_network_whitelist} onChange={v => onChange({ relay_network_whitelist: v })} placeholder="net1" />
          </section>

          <section className="card">
            <h3 className="card-title">协议与性能</h3>
            <div className="grid-3">
              <ToggleField label="延迟优先" value={c.latency_first} onChange={v => onChange({ latency_first: v === true })} />
              <ToggleField label="多线程" value={c.multi_thread} onChange={v => onChange({ multi_thread: v })} />
              <ToggleField label="禁用 P2P" value={c.disable_p2p} onChange={v => onChange({ disable_p2p: v })} />
              <ToggleField label="仅 P2P" value={c.p2p_only} onChange={v => onChange({ p2p_only: v })} />
              <ToggleField label="懒建立 P2P" value={c.lazy_p2p} onChange={v => onChange({ lazy_p2p: v })} />
              <ToggleField label="需要 P2P" value={c.need_p2p} onChange={v => onChange({ need_p2p: v })} />
              <ToggleField label="禁用 TCP 打洞" value={c.disable_tcp_hole_punching} onChange={v => onChange({ disable_tcp_hole_punching: v })} />
              <ToggleField label="禁用 UDP 打洞" value={c.disable_udp_hole_punching} onChange={v => onChange({ disable_udp_hole_punching: v })} />
              <ToggleField label="禁用对称 NAT 打洞" value={c.disable_sym_hole_punching} onChange={v => onChange({ disable_sym_hole_punching: v })} />
              <ToggleField label="禁用 UPnP" value={c.disable_upnp} onChange={v => onChange({ disable_upnp: v })} />
              <ToggleField label="KCP 代理" value={c.enable_kcp_proxy} onChange={v => onChange({ enable_kcp_proxy: v })} />
              <ToggleField label="禁用 KCP 入站" value={c.disable_kcp_input} onChange={v => onChange({ disable_kcp_input: v })} />
              <ToggleField label="QUIC 代理" value={c.enable_quic_proxy} onChange={v => onChange({ enable_quic_proxy: v })} />
              <ToggleField label="禁用 QUIC 入站" value={c.disable_quic_input} onChange={v => onChange({ disable_quic_input: v })} />
              <ToggleField label="使用 smoltcp" value={c.use_smoltcp} onChange={v => onChange({ use_smoltcp: v })} />
              <ToggleField label="无 TUN 模式" value={c.no_tun} onChange={v => onChange({ no_tun: v })} hint="不创建虚拟网卡，仅通过子网代理访问" />
              <ToggleField label="绑定物理设备" value={c.bind_device} onChange={v => onChange({ bind_device: v })} />
              <ToggleField label="系统转发" value={c.proxy_forward_by_system} onChange={v => onChange({ proxy_forward_by_system: v })} />
              <ToggleField label="禁用加密" value={c.disable_encryption} onChange={v => onChange({ disable_encryption: v })} />
              <ToggleField label="禁用 IPv6" value={c.disable_ipv6} onChange={v => onChange({ disable_ipv6: v })} />
              <ToggleField label="私有模式" value={c.enable_private_mode} onChange={v => onChange({ enable_private_mode: v })} />
              <ToggleField label="转发全部 Peer RPC" value={c.relay_all_peer_rpc} onChange={v => onChange({ relay_all_peer_rpc: v })} />
              <ToggleField label="UDP 广播中继" value={c.enable_udp_broadcast_relay} onChange={v => onChange({ enable_udp_broadcast_relay: v })} hint="仅 Windows：帮助局域网游戏发现房间" />
              <ToggleField label="Magic DNS" value={c.enable_magic_dns} onChange={v => onChange({ enable_magic_dns: v })} hint="通过域名访问其他节点，会修改系统 DNS" />
            </div>
            <div className="grid-2">
              <NumberField label="MTU" value={c.mtu != null ? String(c.mtu) : ''} onChange={v => onChange({ mtu: v ? parseInt(v, 10) : null })} min={576} max={9000} disabled={undefined} />
              <NumberField label="入站限速（B/s）" value={c.instance_recv_bps_limit != null ? String(c.instance_recv_bps_limit) : ''} onChange={v => onChange({ instance_recv_bps_limit: v ? parseInt(v, 10) : null })} min={0} />
              <TextField label="TUN 接口名" value={c.dev_name} onChange={v => onChange({ dev_name: v })} placeholder="自动" />
              <TextField label="凭据文件" value={c.credential_file ?? ''} onChange={v => onChange({ credential_file: v })} placeholder="留空使用默认" />
            </div>
          </section>
        </>
      )}
    </div>
  );
}
