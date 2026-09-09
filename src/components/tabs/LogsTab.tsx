import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { serviceRequest } from '../../service-client';
import { Instance } from '../../types';
import { RefreshInterval } from '../../status-data';

interface LogsTabProps {
  current: Instance;
  logsByInstance: Record<string, string[]>;
  setLogsByInstance: React.Dispatch<React.SetStateAction<Record<string, string[]>>>;
  serviceMode: boolean;
  refreshSecs: RefreshInterval;
  isWindowVisible: boolean;
}

function normalizeNetworkLogs(value: unknown): string[] {
  if (Array.isArray(value)) return value.map(v => String(v));
  if (typeof value === 'string') {
    return value
      .split('\n')
      .map(s => s.trim())
      .filter(Boolean);
  }
  return [];
}

export const LogsTab: React.FC<LogsTabProps> = ({
  current,
  logsByInstance,
  setLogsByInstance,
  serviceMode,
  refreshSecs,
  isWindowVisible,
}) => {
  const [logView, setLogView] = useState<'runtime' | 'network'>('runtime');
  const [networkLogs, setNetworkLogs] = useState<string[]>([]);
  const logTimer = useRef<number | null>(null);

  const clearNetworkLogs = () => setNetworkLogs([]);

  useEffect(() => {
    if (current?.status !== 'running' || !isWindowVisible) {
      if (current?.status !== 'running') clearNetworkLogs();
      return;
    }
    let alive = true;
    let timer: number | null = null;
    const refresh = async () => {
      try {
        const value = serviceMode
          ? await serviceRequest<string[] | string>('get_network_logs', { instance_id: current.id })
          : await invoke<string[] | string>('get_network_logs', { id: current.id });
        if (!alive) return;
        setNetworkLogs(normalizeNetworkLogs(value));
      } catch {
        /* log bridge may be temporarily unavailable */
      } finally {
        if (alive) timer = window.setTimeout(() => void refresh(), refreshSecs * 1000);
      }
    };
    void refresh();
    return () => {
      alive = false;
      if (timer != null) window.clearTimeout(timer);
    };
  }, [current?.id, current?.status, serviceMode, refreshSecs, isWindowVisible]);

  const activeLogs = logView === 'runtime' ? logsByInstance[current.id] ?? [] : networkLogs;

  return (
    <div className="card">
      <div className="card-title-row">
        <div className="segmented" role="tablist" aria-label="日志类型">
          <button className={logView === 'runtime' ? 'seg active' : 'seg'} onClick={() => setLogView('runtime')}>
            运行日志
          </button>
          <button className={logView === 'network' ? 'seg active' : 'seg'} onClick={() => setLogView('network')}>
            组网日志
          </button>
        </div>
        <button
          className="ghost"
          onClick={() =>
            logView === 'runtime'
              ? setLogsByInstance(m => ({ ...m, [current.id]: [] }))
              : clearNetworkLogs()
          }
        >
          清空
        </button>
      </div>
      <div
        className="log-pane"
        ref={el => {
          if (el && logTimer.current == null) el.scrollTop = el.scrollHeight;
        }}
      >
        {activeLogs.length === 0 ? (
          <p className="list-empty">暂无{logView === 'runtime' ? '运行' : '组网'}日志</p>
        ) : (
          activeLogs.map((l, i) => (
            <div className="log-line" key={i}>
              {l}
            </div>
          ))
        )}
      </div>
    </div>
  );
};
