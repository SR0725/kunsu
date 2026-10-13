// 來源 AI 官方圖示（AC-OP-11）：決策面板頁首、邊緣瀏海、Agents／Onboarding 共用。
import { useId } from 'react'
import { CLAUDE_SPARK, CODEX_MARK } from '../agentMarks'
import type { AgentKind } from '../types'

/** `label` 給了就讓報讀器讀得到（role="img"）；沒給則是裝飾，aria-hidden。 */
export function AgentMarkIcon({ agent, size = 18, className, label }: { agent: AgentKind; size?: number; className?: string; label?: string }) {
  const gradientId = useId()
  // data-agent-mark：瀏海視窗量來源圖示中心回報給原生（AC-OP-02，ui/src/notchAnchor.ts）。
  const a11y = { 'data-agent-mark': agent, ...(label ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true as const }) }
  if (agent === 'claude') {
    return (
      <svg className={className} width={size} height={size} viewBox={`0 0 ${CLAUDE_SPARK.viewBox} ${CLAUDE_SPARK.viewBox}`} {...a11y}>
        <path d={CLAUDE_SPARK.path} fill={CLAUDE_SPARK.color} />
      </svg>
    )
  }
  if (agent === 'codex') {
    return (
      <svg className={className} width={size} height={size} viewBox="0 0 24 24" {...a11y}>
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor={CODEX_MARK.gradient[0]} />
            <stop offset=".55" stopColor={CODEX_MARK.gradient[1]} />
            <stop offset="1" stopColor={CODEX_MARK.gradient[2]} />
          </linearGradient>
        </defs>
        {[...CODEX_MARK.lobes, CODEX_MARK.core].map((lobe, index) => (
          <circle key={index} cx={lobe.cx} cy={lobe.cy} r={lobe.r} fill={`url(#${gradientId})`} />
        ))}
        <path d={CODEX_MARK.prompt} fill="none" stroke="#f2f3ff" strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    )
  }
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" {...a11y}>
      <circle cx="12" cy="12" r="7.6" fill="none" stroke="#c3c7ce" strokeWidth="2.4" />
    </svg>
  )
}
