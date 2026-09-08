import { useMemo, useState, useRef, MouseEvent } from 'react';

export interface TrafficSample {
  time: number;
  rxSpeed: number;
  txSpeed: number;
}

export interface TrafficAreaChartProps {
  history: TrafficSample[];
  currentRxSpeed: number;
  currentTxSpeed: number;
  peakSpeed: number;
}

export function formatSpeed(bytesPerSec: number): string {
  if (!bytesPerSec || bytesPerSec < 0.1) return '0 B/s';
  if (bytesPerSec < 1024) return `${bytesPerSec.toFixed(0)} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  if (bytesPerSec < 1024 * 1024 * 1024) return `${(bytesPerSec / (1024 * 1024)).toFixed(2)} MB/s`;
  return `${(bytesPerSec / (1024 * 1024 * 1024)).toFixed(2)} GB/s`;
}

function getBezierPath(points: Array<[number, number]>): string {
  if (points.length === 0) return '';
  let d = `M ${points[0][0].toFixed(1)},${points[0][1].toFixed(1)}`;
  for (let i = 0; i < points.length - 1; i++) {
    const p0 = points[i];
    const p1 = points[i + 1];
    const dx = p1[0] - p0[0];
    const cp1x = p0[0] + dx * 0.45;
    const cp1y = p0[1];
    const cp2x = p1[0] - dx * 0.45;
    const cp2y = p1[1];
    d += ` C ${cp1x.toFixed(1)},${cp1y.toFixed(1)} ${cp2x.toFixed(1)},${cp2y.toFixed(1)} ${p1[0].toFixed(1)},${p1[1].toFixed(1)}`;
  }
  return d;
}

export function TrafficAreaChart({ history }: TrafficAreaChartProps) {
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);

  // SVG dimensions
  const W = 620;
  const H = 140;
  const padLeft = 68;
  const padRight = 14;
  const padTop = 14;
  const padBottom = 26;

  const chartW = W - padLeft - padRight;
  const chartH = H - padTop - padBottom;
  const bottomY = padTop + chartH;

  // Compute adaptive max speed
  const maxVal = useMemo(() => {
    let m = 2048; // minimum scale: 2 KB/s
    for (const item of history) {
      if (item.rxSpeed > m) m = item.rxSpeed;
      if (item.txSpeed > m) m = item.txSpeed;
    }
    return m * 1.18; // 18% headroom for pleasant visuals
  }, [history]);

  // Generate coordinates for rx and tx
  const { rxPoints, txPoints, rxLine, txLine, rxArea, txArea } = useMemo(() => {
    const n = Math.max(1, history.length);
    const stepX = chartW / Math.max(1, n - 1);

    const rxPts: Array<[number, number]> = [];
    const txPts: Array<[number, number]> = [];

    history.forEach((sample, i) => {
      const x = padLeft + i * stepX;
      const yRx = bottomY - (Math.min(sample.rxSpeed, maxVal) / maxVal) * chartH;
      const yTx = bottomY - (Math.min(sample.txSpeed, maxVal) / maxVal) * chartH;
      rxPts.push([x, yRx]);
      txPts.push([x, yTx]);
    });

    const rLine = getBezierPath(rxPts);
    const tLine = getBezierPath(txPts);

    const rArea = rxPts.length
      ? `${rLine} L ${rxPts[rxPts.length - 1][0].toFixed(1)},${bottomY} L ${rxPts[0][0].toFixed(1)},${bottomY} Z`
      : '';
    const tArea = txPts.length
      ? `${tLine} L ${txPts[txPts.length - 1][0].toFixed(1)},${bottomY} L ${txPts[0][0].toFixed(1)},${bottomY} Z`
      : '';

    return {
      rxPoints: rxPts,
      txPoints: txPts,
      rxLine: rLine,
      txLine: tLine,
      rxArea: rArea,
      txArea: tArea,
    };
  }, [history, maxVal, chartW, chartH, bottomY, padLeft]);

  const handleMouseMove = (e: MouseEvent<SVGSVGElement>) => {
    if (!svgRef.current || history.length === 0) return;
    const rect = svgRef.current.getBoundingClientRect();
    const clientX = e.clientX - rect.left;
    const svgX = (clientX / rect.width) * W;

    if (svgX < padLeft || svgX > padLeft + chartW) {
      setHoverIndex(null);
      return;
    }
    const stepX = chartW / Math.max(1, history.length - 1);
    const idx = Math.round((svgX - padLeft) / stepX);
    if (idx >= 0 && idx < history.length) {
      setHoverIndex(idx);
    } else {
      setHoverIndex(null);
    }
  };

  const handleMouseLeave = () => {
    setHoverIndex(null);
  };

  const hoveredSample = hoverIndex !== null ? history[hoverIndex] : null;
  const hoveredRxPt = hoverIndex !== null ? rxPoints[hoverIndex] : null;
  const hoveredTxPt = hoverIndex !== null ? txPoints[hoverIndex] : null;

  return (
    <div className="traffic-chart-wrapper">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${W} ${H}`}
        className="traffic-svg"
        preserveAspectRatio="none"
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      >
        <defs>
          <linearGradient id="rx-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#0891b2" stopOpacity="0.45" />
            <stop offset="70%" stopColor="#0891b2" stopOpacity="0.10" />
            <stop offset="100%" stopColor="#0891b2" stopOpacity="0.0" />
          </linearGradient>
          <linearGradient id="tx-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#10b981" stopOpacity="0.42" />
            <stop offset="70%" stopColor="#10b981" stopOpacity="0.08" />
            <stop offset="100%" stopColor="#10b981" stopOpacity="0.0" />
          </linearGradient>
        </defs>

        {/* Horizontal grid lines & Y labels */}
        {[0, 0.5, 1].map(ratio => {
          const y = bottomY - ratio * chartH;
          const val = ratio * maxVal;
          return (
            <g key={ratio}>
              <line
                x1={padLeft}
                y1={y}
                x2={padLeft + chartW}
                y2={y}
                stroke="rgba(148, 163, 184, 0.18)"
                strokeDasharray={ratio === 0 ? undefined : '3,3'}
                strokeWidth={ratio === 0 ? '1.5' : '1'}
              />
              <text
                x={padLeft - 8}
                y={y + 3.5}
                textAnchor="end"
                fontSize="9.5"
                fill="rgba(100, 116, 139, 0.75)"
                fontFamily="inherit"
              >
                {formatSpeed(val)}
              </text>
            </g>
          );
        })}

        {/* Time markers along X axis */}
        {[
          { label: '-30s', x: padLeft },
          { label: '-20s', x: padLeft + chartW * 0.333 },
          { label: '-10s', x: padLeft + chartW * 0.666 },
          { label: '现在', x: padLeft + chartW },
        ].map((t, idx) => (
          <text
            key={idx}
            x={t.x}
            y={bottomY + 16}
            textAnchor={idx === 0 ? 'start' : idx === 3 ? 'end' : 'middle'}
            fontSize="9"
            fill="rgba(148, 163, 184, 0.7)"
            fontFamily="inherit"
          >
            {t.label}
          </text>
        ))}

        {/* Area Fills */}
        {rxArea && <path d={rxArea} fill="url(#rx-fill)" />}
        {txArea && <path d={txArea} fill="url(#tx-fill)" />}

        {/* Lines */}
        {rxLine && (
          <path
            d={rxLine}
            fill="none"
            stroke="#0891b2"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        )}
        {txLine && (
          <path
            d={txLine}
            fill="none"
            stroke="#10b981"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        )}

        {/* Hover vertical bar & dots */}
        {hoverIndex !== null && hoveredRxPt && hoveredTxPt && (
          <g>
            <line
              x1={hoveredRxPt[0]}
              y1={padTop}
              x2={hoveredRxPt[0]}
              y2={bottomY}
              stroke="rgba(15, 23, 42, 0.35)"
              strokeWidth="1"
              strokeDasharray="2,2"
            />
            <circle
              cx={hoveredRxPt[0]}
              cy={hoveredRxPt[1]}
              r="3.5"
              fill="#0891b2"
              stroke="#fff"
              strokeWidth="2"
            />
            <circle
              cx={hoveredTxPt[0]}
              cy={hoveredTxPt[1]}
              r="3.5"
              fill="#10b981"
              stroke="#fff"
              strokeWidth="2"
            />
          </g>
        )}
      </svg>

      {/* Hover tooltip overlay */}
      {hoverIndex !== null && hoveredSample && hoveredRxPt && (
        <div
          className="traffic-chart-tooltip"
          style={{
            left: `${(hoveredRxPt[0] / W) * 100}%`,
          }}
        >
          <div className="tooltip-time">
            {hoverIndex === history.length - 1 ? '此刻' : `-${history.length - 1 - hoverIndex}s`}
          </div>
          <div className="tooltip-row rx">
            <span className="dot on" style={{ width: 6, height: 6, background: '#0891b2' }} />
            <span>下载: {formatSpeed(hoveredSample.rxSpeed)}</span>
          </div>
          <div className="tooltip-row tx">
            <span className="dot on" style={{ width: 6, height: 6, background: '#10b981' }} />
            <span>上传: {formatSpeed(hoveredSample.txSpeed)}</span>
          </div>
        </div>
      )}
    </div>
  );
}
