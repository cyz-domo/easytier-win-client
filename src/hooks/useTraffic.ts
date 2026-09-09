import { useCallback, useMemo, useRef, useState, useEffect } from 'react';
import { Instance, InstanceSnapshot } from '../types';
import { parseHumanBytes } from '../status-data';
import { TrafficSample } from '../TrafficAreaChart';

export function useTraffic(
  instances: Instance[],
  current: Instance | undefined,
  isWindowVisible: boolean,
  tab: string
) {
  const [trafficTotals, setTrafficTotals] = useState<Record<string, { rx: number; tx: number }>>({});
  const lastPeerCounters = useRef<Record<string, { rx: number; tx: number }>>({});

  // 30 个数据点的平滑流量图表历史，纯内存维护，消除对 sessionStorage 的频繁序列化开销
  const [trafficHistory, setTrafficHistory] = useState<TrafficSample[]>(() =>
    Array.from({ length: 30 }, (_, i) => ({ time: Date.now() - (29 - i) * 1000, rxSpeed: 0, txSpeed: 0 }))
  );
  const [currentRxSpeed, setCurrentRxSpeed] = useState(0);
  const [currentTxSpeed, setCurrentTxSpeed] = useState(0);
  const lastTotalBytesRef = useRef<{ rx: number; tx: number; time: number } | null>(null);

  // 当实例停止或重启时，清空该实例的累计流量
  const prevStatuses = useRef<Record<string, string>>({});
  useEffect(() => {
    for (const i of instances) {
      const prev = prevStatuses.current[i.id];
      if (prev === 'running' && i.status !== 'running') {
        clearInstanceTraffic(i.id);
      }
      prevStatuses.current[i.id] = i.status;
    }
  }, [instances]);

  const clearInstanceTraffic = useCallback((instanceId: string) => {
    const prefix = `${instanceId}:`;
    setTrafficTotals(m => {
      const next: Record<string, { rx: number; tx: number }> = {};
      for (const [k, v] of Object.entries(m)) {
        if (!k.startsWith(prefix)) next[k] = v;
      }
      return next;
    });
    for (const k of Object.keys(lastPeerCounters.current)) {
      if (k.startsWith(prefix)) delete lastPeerCounters.current[k];
    }
  }, []);

  // 累加单次轮询获取到的所有运行实例的节点流量，并计算当前选中实例的瞬时速率
  const recordPollResults = useCallback(
    (results: Array<readonly [string, InstanceSnapshot | null]>) => {
      const updates: Record<string, { rx: number; tx: number }> = {};
      let activeTotalRx = 0;
      let activeTotalTx = 0;
      let hasActiveSnapshot = false;

      for (const [id, snapshot] of results) {
        if (!snapshot) continue;
        const isActive = current?.id === id;
        if (isActive) hasActiveSnapshot = true;

        for (const p of snapshot.peers) {
          if (p.cost === 'Local') continue;
          const rx = parseHumanBytes(p.rx_bytes);
          const tx = parseHumanBytes(p.tx_bytes);

          if (isActive) {
            activeTotalRx += rx;
            activeTotalTx += tx;
          }

          const normalizedIp = (p.ipv4 ?? '').split('/')[0].trim();
          const key = `${id}:${p.hostname || p.id || 'peer'}:${normalizedIp}`;
          updates[key] = { rx, tx };
        }
      }

      // 1. 批量更新累计流量
      if (Object.keys(updates).length > 0) {
        setTrafficTotals(m => {
          let changed = false;
          const nextM = { ...m };
          for (const [key, { rx, tx }] of Object.entries(updates)) {
            const cur = nextM[key];
            const last = lastPeerCounters.current[key];
            lastPeerCounters.current[key] = { rx, tx };

            if (!cur) {
              nextM[key] = { rx, tx };
              changed = true;
            } else if (last) {
              const drx = rx >= last.rx ? rx - last.rx : rx;
              const dtx = tx >= last.tx ? tx - last.tx : tx;
              if (drx > 0 || dtx > 0) {
                nextM[key] = { rx: cur.rx + drx, tx: cur.tx + dtx };
                changed = true;
              }
            } else {
              const drx = rx > cur.rx ? rx - cur.rx : 0;
              const dtx = tx > cur.tx ? tx - cur.tx : 0;
              if (drx > 0 || dtx > 0) {
                nextM[key] = { rx: cur.rx + drx, tx: cur.tx + dtx };
                changed = true;
              }
            }
          }
          return changed ? nextM : m;
        });
      }

      // 2. 统一速率计算：消除独立 1 秒轮询循环，在主轮询结果到达时计算瞬时速率
      if (hasActiveSnapshot && current?.status === 'running' && isWindowVisible && tab === 'status') {
        const now = Date.now();
        const last = lastTotalBytesRef.current;
        lastTotalBytesRef.current = { rx: activeTotalRx, tx: activeTotalTx, time: now };
        if (last) {
          const dt = Math.max(0.4, (now - last.time) / 1000);
          const rSpeed = activeTotalRx >= last.rx ? (activeTotalRx - last.rx) / dt : 0;
          const tSpeed = activeTotalTx >= last.tx ? (activeTotalTx - last.tx) / dt : 0;
          setCurrentRxSpeed(rSpeed);
          setCurrentTxSpeed(tSpeed);
          setTrafficHistory(prev => [...prev.slice(1), { time: now, rxSpeed: rSpeed, txSpeed: tSpeed }]);
        }
      } else if (!isWindowVisible || current?.status !== 'running' || tab !== 'status') {
        setCurrentRxSpeed(0);
        setCurrentTxSpeed(0);
        lastTotalBytesRef.current = null;
      }
    },
    [current?.id, current?.status, isWindowVisible, tab]
  );

  const trafficPrefix = `${current?.id ?? ''}:`;
  const instanceTraffic = useMemo(() => {
    let rx = 0, tx = 0;
    for (const [key, v] of Object.entries(trafficTotals)) {
      if (key.startsWith(trafficPrefix)) {
        rx += v.rx;
        tx += v.tx;
      }
    }
    return { rx, tx };
  }, [trafficTotals, trafficPrefix]);

  const peakSpeed = useMemo(
    () => Math.max(0, ...trafficHistory.map(s => Math.max(s.rxSpeed, s.txSpeed))),
    [trafficHistory]
  );

  return {
    trafficTotals,
    instanceTraffic,
    trafficHistory,
    currentRxSpeed,
    currentTxSpeed,
    peakSpeed,
    clearInstanceTraffic,
    recordPollResults,
  };
}
