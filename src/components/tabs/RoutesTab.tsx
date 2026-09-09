import React from 'react';
import { RouteInfo, latencyTone } from '../../status-data';

interface RoutesTabProps {
  running: boolean;
  routes: RouteInfo[];
}

export const RoutesTab: React.FC<RoutesTabProps> = ({ running, routes }) => {
  return (
    <div className="card">
      <h3 className="card-title">路由信息{running ? `（${routes.length}）` : ''}</h3>
      {!running && <p className="list-empty">网络未运行，启动后此处显示路由表。</p>}
      {running && (
        <div className="table-scroll">
          <table className="data-table">
            <thead>
              <tr>
                <th>虚拟地址</th>
                <th>主机名</th>
                <th>下一跳</th>
                <th>跳数</th>
                <th>路径延迟</th>
                <th>子网代理</th>
                <th>版本</th>
              </tr>
            </thead>
            <tbody>
              {routes.map((r, i) => (
                <tr key={i}>
                  <td>{r.ipv4 || '—'}</td>
                  <td>{r.hostname || '—'}</td>
                  <td>{r.next_hop_hostname || r.next_hop_ipv4 || '—'}</td>
                  <td>{r.path_len ?? '—'}</td>
                  <td className={`tone-${latencyTone(r.path_latency)}`}>{r.path_latency ? `${r.path_latency} ms` : '—'}</td>
                  <td>{r.proxy_cidrs && String(r.proxy_cidrs) !== '' ? String(r.proxy_cidrs) : '—'}</td>
                  <td>{r.version || '—'}</td>
                </tr>
              ))}
              {routes.length === 0 && (
                <tr>
                  <td colSpan={7} className="list-empty">
                    暂无路由
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
