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
  const text = typeof value === 'string' ? value : (value as { text?: string } | null | undefined)?.text || '';
  return text
    .split(/\r?\n/)
    .map(s => s.trim())
    .filter(Boolean);
}

function formatLogLine(line: string, view: 'runtime' | 'network'): React.ReactNode {
  if (view === 'runtime') {
    // 运行日志典型格式: "[17:15:20] [fn2] text..." 或 "[17:15:20] text..."
    const match = line.match(/^(\[\d{1,2}:\d{2}:\d{2}\])\s*(?:(\[[^\]]+\])\s*)?(.*)$/);
    if (match) {
      const [, time, tag, rest] = match;
      const isSuccess = rest.includes('✓') || rest.includes('成功') || rest.includes('已启动') || rest.includes('已连接');
      const isError = rest.includes('✗') || rest.includes('失败') || rest.includes('异常') || rest.includes('错误');
      const isWarn = rest.includes('⚠') || rest.includes('警告') || rest.includes('冲突');
      const textClass = isSuccess ? 'log-text-success' : isError ? 'log-text-error' : isWarn ? 'log-text-warn' : '';

      return (
        <span className="log-line-content">
          <span className="log-time">{time}</span>
          {tag && <span className="log-tag">{tag.slice(1, -1)}</span>}
          <span className={textClass}>{rest}</span>
        </span>
      );
    }
  } else {
    // 组网日志典型格式: "2026-09-09T17:11:53... INFO CORE... [instance-id] message"
    const isoMatch = line.match(/^(\d{4}-\d{2}-\d{2}T(\d{2}:\d{2}:\d{2})(?:\.\d+)?(?:[+-]\d{2}:\d{2}|Z)?)\s+([A-Z]+)\s+(.*)$/);
    if (isoMatch) {
      const [, , timeOnly, level, rest] = isoMatch;
      const levelClass =
        level === 'ERROR' ? 'log-level-error' : level === 'WARN' ? 'log-level-warn' : 'log-level-info';
      return (
        <span className="log-line-content">
          <span className="log-time">[{timeOnly}]</span>
          <span className={`log-level ${levelClass}`}>{level}</span>
          <span>{rest}</span>
        </span>
      );
    }
    // 异常堆栈或错误行
    if (line.toLowerCase().includes('error') || line.includes('TunnelError')) {
      return <span className="log-line-content log-text-error">{line}</span>;
    }
  }

  return <span className="log-line-content">{line}</span>;
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
              <span className="log-line-num">{i + 1}</span>
              {formatLogLine(l, logView)}
            </div>
          ))
        )}
      </div>
    </div>
  );
};
