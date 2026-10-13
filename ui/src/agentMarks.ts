// 來源 AI 圖示（docs/bdd/overlay-presentation.md AC-OP-11）。
// Claude 星芒取自本機 Claude App 內建 AppLogo 的官方向量；Codex 依 Codex App 圖示簡化為
// 八瓣雲加 `>_`。蓋板粒子、決策面板頁首、邊緣瀏海與主 app 標記全部共用這一份。

import type { AgentKind } from './types'

export const CLAUDE_SPARK = {
  viewBox: 248,
  color: '#D97757',
  path: 'M52.4285 162.873L98.7844 136.879L99.5485 134.602L98.7844 133.334H96.4921L88.7237 132.862L62.2346 132.153L39.3113 131.207L17.0249 130.026L11.4214 128.844L6.2 121.873L6.7094 118.447L11.4214 115.257L18.171 115.847L33.0711 116.911L55.485 118.447L71.6586 119.392L95.728 121.873H99.5485L100.058 120.337L98.7844 119.392L97.7656 118.447L74.5877 102.732L49.4995 86.1905L36.3823 76.62L29.3779 71.7757L25.8121 67.2858L24.2839 57.3608L30.6515 50.2716L39.3113 50.8623L41.4763 51.4531L50.2636 58.1879L68.9842 72.7209L93.4357 90.6804L97.0015 93.6343L98.4374 92.6652L98.6571 91.9801L97.0015 89.2625L83.757 65.2772L69.621 40.8192L63.2534 30.6579L61.5978 24.632C60.9565 22.1032 60.579 20.0111 60.579 17.4246L67.8381 7.49965L71.9133 6.19995L81.7193 7.49965L85.7946 11.0443L91.9074 24.9865L101.714 46.8451L116.996 76.62L121.453 85.4816L123.873 93.6343L124.764 96.1155H126.292V94.6976L127.566 77.9197L129.858 57.3608L132.15 30.8942L132.915 23.4505L136.608 14.4708L143.994 9.62643L149.725 12.344L154.437 19.0788L153.8 23.4505L150.998 41.6463L145.522 70.1215L141.957 89.2625H143.994L146.414 86.7813L156.093 74.0206L172.266 53.698L179.398 45.6635L187.803 36.802L193.152 32.5484H203.34L210.726 43.6549L207.415 55.1159L196.972 68.3492L188.312 79.5739L175.896 96.2095L168.191 109.585L168.882 110.689L170.738 110.53L198.755 104.504L213.91 101.787L231.994 98.7149L240.144 102.496L241.036 106.395L237.852 114.311L218.495 119.037L195.826 123.645L162.07 131.592L161.696 131.893L162.137 132.547L177.36 133.925L183.855 134.279H199.774L229.447 136.524L237.215 141.605L241.8 147.867L241.036 152.711L229.065 158.737L213.019 154.956L175.45 145.977L162.587 142.787H160.805V143.85L171.502 154.366L191.242 172.089L215.82 195.011L217.094 200.682L213.91 205.172L210.599 204.699L188.949 188.394L180.544 181.069L161.696 165.118H160.422V166.772L164.752 173.152L187.803 207.771L188.949 218.405L187.294 221.832L181.308 223.959L174.813 222.777L161.187 203.754L147.305 182.486L136.098 163.345L134.745 164.2L128.075 235.42L125.019 239.082L117.887 241.8L111.902 237.31L108.718 229.984L111.902 215.452L115.722 196.547L118.779 181.541L121.58 162.873L123.291 156.636L123.14 156.219L121.773 156.449L107.699 175.752L86.304 204.699L69.3663 222.777L65.291 224.431L58.2867 220.768L58.9235 214.27L62.8713 208.48L86.304 178.705L100.44 160.155L109.551 149.507L109.462 147.967L108.959 147.924L46.6977 188.512L35.6182 189.93L30.7788 185.44L31.4156 178.115L33.7079 175.752L52.4285 162.873Z',
} as const

/** Codex 標記，viewBox 0 0 24 24。 */
export const CODEX_MARK = {
  viewBox: 24,
  gradient: ['#b9a8f6', '#6c78f5', '#3430ef'] as const,
  lobes: Array.from({ length: 8 }, (_, i) => {
    const angle = (i / 8) * Math.PI * 2 + Math.PI / 8
    return { cx: 12 + Math.cos(angle) * 6.2, cy: 12 + Math.sin(angle) * 6.2, r: 4.6 }
  }),
  core: { cx: 12, cy: 12, r: 7.2 },
  prompt: 'M8.4 9.2 L10.9 12 L8.4 14.8 M12.6 14.8 H16',
} as const

export type AgentMarkSource =
  | { kind: 'vector'; agent: AgentKind }
  | { kind: 'glyph'; text: string }

/** visual_symbol 只能覆寫粒子輪廓（最多 4 個字元）；其餘一律用官方向量。 */
export function agentMarkSource(agent: AgentKind, visualSymbol?: string): AgentMarkSource {
  const trimmed = visualSymbol?.trim()
  if (trimmed) return { kind: 'glyph', text: [...trimmed].slice(0, 4).join('') }
  return { kind: 'vector', agent }
}

export function agentDisplayName(agent: AgentKind): string {
  return agent === 'claude' ? 'Claude' : agent === 'codex' ? 'Codex' : 'AI Agent'
}

/** 在 canvas 上以 (x, y) 為中心畫出 size×size 的來源圖示（粒子取樣與靜態 fallback 共用）。 */
export function drawAgentMark(
  context: CanvasRenderingContext2D,
  agent: AgentKind,
  x: number,
  y: number,
  size: number,
  fill?: string,
): void {
  context.save()
  if (agent === 'claude') {
    const scale = size / CLAUDE_SPARK.viewBox
    context.translate(x - size / 2, y - size / 2)
    context.scale(scale, scale)
    context.fillStyle = fill ?? CLAUDE_SPARK.color
    context.fill(new Path2D(CLAUDE_SPARK.path))
  } else if (agent === 'codex') {
    const scale = size / CODEX_MARK.viewBox
    context.translate(x - size / 2, y - size / 2)
    context.scale(scale, scale)
    context.fillStyle = fill ?? CODEX_MARK.gradient[1]
    for (const lobe of [...CODEX_MARK.lobes, CODEX_MARK.core]) {
      context.beginPath()
      context.arc(lobe.cx, lobe.cy, lobe.r, 0, Math.PI * 2)
      context.fill()
    }
    // 單色取樣時把 >_ 挖空，彩色顯示時畫白色筆畫。
    context.globalCompositeOperation = fill ? 'destination-out' : 'source-over'
    context.strokeStyle = fill ? '#000' : '#f2f3ff'
    context.lineWidth = 1.7
    context.lineCap = 'round'
    context.lineJoin = 'round'
    context.stroke(new Path2D(CODEX_MARK.prompt))
  } else {
    context.fillStyle = fill ?? '#c3c7ce'
    context.beginPath()
    context.arc(x, y, size * 0.32, 0, Math.PI * 2)
    context.fill()
    context.globalCompositeOperation = 'destination-out'
    context.beginPath()
    context.arc(x, y, size * 0.14, 0, Math.PI * 2)
    context.fill()
  }
  context.restore()
}
