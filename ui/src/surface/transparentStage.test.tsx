// 決策面板張開時，面板以外的螢幕全程透明：沒有變暗遮罩、沒有模糊層（AC-DS-01，2026-10-09 作者）。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { DecisionSurface } from './DecisionSurface'
import css from '../index.css?raw'

// 測試環境沒有 DOM：面板渲染時只讀螢幕大小與減少動態偏好。
const g = globalThis as unknown as { window?: unknown }
g.window ??= { innerWidth: 1440, innerHeight: 900, matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) }

const ipc = new Proxy({} as Record<string, unknown>, {
  get: (_t, key: string) => (key.startsWith('subscribe') ? () => () => {} : async () => null),
}) as unknown as KunsuIpc

const question = {
  id: 'q-stage',
  title: 't',
  question_line: '要不要加碼？',
  body_md: '',
  kind: 'choice',
  tier: 't1',
  requested_tier: 't1',
  evidence: [],
  options: [
    { id: 'a', label: '加碼' },
    { id: 'b', label: '維持' },
  ],
  allow_free_text: true,
  default_action: { on_expire: 'noop_log' },
  wake_on_answer: { mode: 'none' },
  source: { agent: 'claude', cwd: '/w' },
  state: 'forced',
  snooze_count: 0,
  forced_today_used: false,
  first_proposed: '2026-10-09',
  last_seen: '2026-10-09T00:00:00Z',
  created_at: '2026-10-09T00:00:00Z',
  next_transition_at: '2026-10-09T00:00:00Z',
} as unknown as Question

describe('面板以外完全透明', () => {
  it('不畫任何遮罩層', () => {
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[question]} transition={null} onExit={() => {}} />)
    expect(html).toContain('surface-stage')
    expect(html).not.toContain('surface-backdrop')
  })

  it('舞台本身沒有背景色或模糊', () => {
    expect(css).not.toContain('.surface-backdrop')
    const stage = css.match(/\.surface-stage\s*\{[^}]*\}/)?.[0] ?? ''
    expect(stage).not.toMatch(/background|backdrop-filter/)
  })
})

// AC-DS-11（2026-10-09 作者）：面板打開時頁首不顯示倒數；15 分鐘週期只在背景計算。
describe('面板不顯示倒數', () => {
  it('頁首沒有倒數圓環、倒數文字或計時公告', () => {
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[question]} transition={null} onExit={() => {}} />)
    expect(html).not.toContain('強制詢問')
    expect(html).not.toContain('已到期')
    expect(html).not.toContain('role="timer"')
    expect(html).not.toContain('surface-cycle')
    expect(html).toContain('稍後再問')
  })
})
