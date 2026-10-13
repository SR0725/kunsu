// 主視窗 QA 第 1 輪對決策面板的修正（2026-10-09 主線代作者決定；BDD decision-surface AC-DS-04／17／18）：
// 「稍後再問」寫出目前生效的週期長度；回看已結案的題不顯示「稍後再問」，改「關閉」。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question, SettingsState } from '../types'
import { DecisionSurface } from './DecisionSurface'
import { laterHint, readCycleMinutes } from './laterHint'
import { POLICY_DEFAULTS } from '../app/settings/schema'

// 測試環境沒有 DOM：面板渲染時只讀螢幕大小與減少動態偏好。
const g = globalThis as unknown as { window?: unknown }
g.window ??= { innerWidth: 1440, innerHeight: 900, matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) }

const ipc = new Proxy({} as Record<string, unknown>, {
  get: (_t, key: string) => (key.startsWith('subscribe') ? () => () => {} : async () => null),
}) as unknown as KunsuIpc

function q(partial: Partial<Question> = {}): Question {
  return {
    id: 'q-later',
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
    ...partial,
  } as Question
}

const head = (html: string) => html.slice(html.indexOf('<header'), html.indexOf('</header>')).replace(/<[^>]+>/g, '')
const settings = (cycle: number): SettingsState => ({
  effective: { ...POLICY_DEFAULTS, force: { cycle_min: cycle, warn_before_min: 3 } },
  defaults: POLICY_DEFAULTS,
  overridden: ['force.cycle_min'],
  local_path: '/tmp/policy.local.toml',
})

describe('「N 分鐘後再問」跟著目前生效的週期（AC-DS-04）', () => {
  it('寫出週期長度；讀不到或不合理時是 15', () => {
    expect(laterHint(30)).toBe('30 分鐘後再問')
    expect(laterHint(5)).toBe('5 分鐘後再問')
    expect(laterHint(120)).toBe('120 分鐘後再問')
    for (const bad of [undefined, null, 0, 4, 121, Number.NaN, 12.5]) expect(laterHint(bad as number | undefined), String(bad)).toBe('15 分鐘後再問')
  })

  it('面板掛載時讀設定的生效週期；讀不到退回 15', async () => {
    expect(await readCycleMinutes({ settingsGet: async () => settings(30) })).toBe(30)
    expect(await readCycleMinutes({ settingsGet: async () => Promise.reject(new Error('no')) })).toBe(15)
    expect(await readCycleMinutes({ settingsGet: async () => null as unknown as SettingsState })).toBe(15)
    expect(await readCycleMinutes({} as Pick<KunsuIpc, 'settingsGet'>)).toBe(15)
  })

  it('還沒讀到時頁首先寫 15 分鐘後再問', () => {
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} onExit={() => {}} />)
    expect(head(html)).toContain('15 分鐘後再問')
  })
})

describe('回看已結案的題沒有「稍後再問」（AC-DS-17）', () => {
  const closed = q({ state: 'expired' })
  it('review：右上是「關閉 Esc」', () => {
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[closed]} review onExit={() => {}} />)
    const top = head(html)
    expect(top).not.toContain('稍後再問')
    expect(top).not.toContain('分鐘後再問')
    expect(top).toContain('關閉')
    expect(top).toContain('Esc')
  })
  it('一般作答流程照舊有「稍後再問」', () => {
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} onExit={() => {}} />)
    expect(head(html)).toContain('稍後再問')
  })
})
