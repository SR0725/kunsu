// 畫布裡的 `kunsu` 模組（DESIGN「Evidence canvas」）：Kunsu 色票、圖表主題與幾個現成元件，
// 讓 agent 不用挑字型與顏色，畫出來就跟決策面板一致。
import type { CSSProperties, ReactElement, ReactNode } from 'react'
import { ResponsiveContainer } from 'recharts'

export const colors = {
  card: '#0b0c0e',
  card2: '#111317',
  line: '#1d2026',
  line2: '#2a2e35',
  text: '#f2f3f5',
  text2: '#c3c7ce',
  muted: '#8b919b',
  faint: '#5f6570',
  accent: '#3987e5',
  good: '#83c494',
  bad: '#ef8d86',
  warn: '#e0b35a',
} as const

/** 類別色（已對 #0b0c0e 背景驗證對比）：藍、橘、綠、黃。 */
export const series = ['#3987e5', '#d95926', '#199e70', '#c98500'] as const

const tick = { fill: '#8b919b', fontSize: 12, fontFamily: 'Geist, -apple-system, "PingFang TC", sans-serif' }

/** Recharts 主題：直接展開到元件上，例如 `<CartesianGrid {...chart.grid} />`、`<Line {...chart.line} />`。 */
export const chart = {
  margin: { top: 8, right: 12, bottom: 0, left: 0 },
  grid: { stroke: '#1f2227', strokeWidth: 1, vertical: false },
  axis: { stroke: '#1f2227', tick, tickLine: false, axisLine: false, tickMargin: 8 },
  /** Y 軸：寬度跟著數字、1-2-5 漂亮刻度、萬以上縮寫。 */
  yAxis: {
    stroke: '#1f2227',
    tick,
    tickLine: false,
    axisLine: false,
    tickMargin: 6,
    width: 'auto' as const,
    tickCount: 5,
    niceTicks: 'snap125' as const,
    tickFormatter: (v: number) => (Math.abs(v) >= 1e4 ? `${(v / 1e4).toLocaleString('zh-TW', { maximumFractionDigits: 1 })}萬` : v.toLocaleString('zh-TW')),
  },
  tooltip: {
    cursor: { stroke: '#2a2e35', strokeWidth: 1, fill: 'rgba(255,255,255,0.03)' },
    contentStyle: {
      background: '#111317',
      border: '1px solid #2a2e35',
      borderRadius: 10,
      boxShadow: '0 8px 24px rgba(0,0,0,.45)',
      padding: '8px 10px',
      fontSize: 12,
    },
    labelStyle: { color: '#8b919b', marginBottom: 4 },
    itemStyle: { color: '#f2f3f5', padding: 0 },
  },
  line: { type: 'monotone' as const, strokeWidth: 2, dot: false, activeDot: { r: 3, strokeWidth: 0 } },
  area: { type: 'monotone' as const, strokeWidth: 2, fillOpacity: 0.12 },
  bar: { radius: [4, 4, 0, 0] as [number, number, number, number], maxBarSize: 28 },
  legend: { iconType: 'circle' as const, iconSize: 8, wrapperStyle: { fontSize: 12, color: '#8b919b' } },
}

/** 數字格式（台灣用法）。 */
export const fmt = {
  num: (n: number, digits = 0) => n.toLocaleString('zh-TW', { minimumFractionDigits: digits, maximumFractionDigits: digits }),
  pct: (n: number, digits = 1) => `${(n * 100).toFixed(digits)}%`,
  twd: (n: number) => `NT$${Math.round(n).toLocaleString('zh-TW')}`,
  compact: (n: number) => (Math.abs(n) >= 1e8 ? `${(n / 1e8).toFixed(1)}億` : Math.abs(n) >= 1e4 ? `${(n / 1e4).toFixed(1)}萬` : n.toLocaleString('zh-TW')),
}

const font = 'Geist, -apple-system, BlinkMacSystemFont, "PingFang TC", "Noto Sans TC", sans-serif'

/** 卡片。 */
export function Panel({ title, note, children, style }: { title?: ReactNode; note?: ReactNode; children?: ReactNode; style?: CSSProperties }) {
  return (
    <section
      style={{
        border: `1px solid ${colors.line}`,
        borderRadius: 14,
        background: colors.card,
        padding: '14px 16px',
        fontFamily: font,
        color: colors.text,
        minWidth: 0,
        ...style,
      }}
    >
      {(title || note) && (
        <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 12, marginBottom: 10 }}>
          {title && <h3 style={{ margin: 0, fontSize: 13, fontWeight: 600, color: colors.text2, letterSpacing: '.01em' }}>{title}</h3>}
          {note && <span style={{ fontSize: 12, color: colors.muted }}>{note}</span>}
        </header>
      )}
      {children}
    </section>
  )
}

/** 自動換行的格線（每格至少 `min` px）。 */
export function Grid({ children, min = 150, gap = 10 }: { children?: ReactNode; min?: number; gap?: number }) {
  return <div style={{ display: 'grid', gridTemplateColumns: `repeat(auto-fit, minmax(${min}px, 1fr))`, gap }}>{children}</div>
}

/**
 * 變化量：▲／▼＋幅度＋好／差文字（不只靠顏色）。
 * `value` 是比例（0.12 = 12%）；`good='up'` 表示上升是好事，`'down'` 表示下降是好事。
 */
export function Delta({ value, good = 'up', digits }: { value: number; good?: 'up' | 'down'; digits?: number }) {
  if (!Number.isFinite(value) || value === 0) {
    return <span style={{ fontSize: 12, color: colors.muted }}>— 持平</span>
  }
  const up = value > 0
  const better = up === (good === 'up')
  const color = better ? colors.good : colors.bad
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4, fontSize: 12, fontWeight: 500, color, fontVariantNumeric: 'tabular-nums' }}>
      <span aria-hidden="true">{up ? '▲' : '▼'}</span>
      {(Math.abs(value) * 100).toFixed(digits ?? (Math.abs(value) >= 0.1 ? 0 : 1))}%
      <span style={{ border: `1px solid ${color}55`, borderRadius: 5, padding: '0 4px', fontSize: 11 }}>{better ? '好' : '差'}</span>
    </span>
  )
}

/** 迷你走勢線。 */
export function Sparkline({ data, color = colors.accent, height = 28 }: { data: number[]; color?: string; height?: number }) {
  const points = data.filter((n) => Number.isFinite(n))
  if (points.length < 2) return null
  const min = Math.min(...points)
  const max = Math.max(...points)
  const span = max - min || 1
  const w = 100
  const coords = points.map((v, i) => [(i / (points.length - 1)) * w, height - 3 - ((v - min) / span) * (height - 6)])
  const last = coords[coords.length - 1]
  return (
    <svg viewBox={`0 0 ${w} ${height}`} preserveAspectRatio="none" style={{ width: '100%', height, display: 'block', overflow: 'visible' }} aria-hidden="true">
      <polyline points={coords.map((c) => c.join(',')).join(' ')} fill="none" stroke={color} strokeWidth={1.5} vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
      {/* 零長度線段＋圓頭：不受 preserveAspectRatio=none 拉伸，永遠是正圓。 */}
      <polyline points={`${last[0]},${last[1]} ${last[0]},${last[1]}`} stroke={color} strokeWidth={5} strokeLinecap="round" vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

/** 指標卡：名稱、數值、變化與走勢。 */
export function Stat({
  label,
  value,
  unit,
  delta,
  good = 'up',
  spark,
  note,
}: {
  label: ReactNode
  value: ReactNode
  unit?: ReactNode
  delta?: number
  good?: 'up' | 'down'
  spark?: number[]
  note?: ReactNode
}) {
  return (
    <Panel style={{ padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 12, color: colors.muted }}>{label}</div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
        <span style={{ fontSize: 26, lineHeight: 1.1, fontWeight: 600, letterSpacing: '-.01em', fontVariantNumeric: 'tabular-nums' }}>
          {value}
          {unit && <span style={{ fontSize: 13, fontWeight: 500, color: colors.muted, marginLeft: 3 }}>{unit}</span>}
        </span>
        {delta !== undefined && <Delta value={delta} good={good} />}
      </div>
      {spark && <Sparkline data={spark} color={delta !== undefined && delta !== 0 ? ((delta > 0) === (good === 'up') ? colors.good : colors.bad) : colors.accent} />}
      {note && <div style={{ fontSize: 12, color: colors.faint }}>{note}</div>}
    </Panel>
  )
}

/** 圖表外框：固定高度＋寬度跟著容器（包住 Recharts 圖表）。 */
export function ChartFrame({ height = 220, children }: { height?: number; children: ReactNode }) {
  return (
    <div style={{ width: '100%', height }}>
      <ResponsiveContainer width="100%" height="100%">
        {children as ReactElement}
      </ResponsiveContainer>
    </div>
  )
}
