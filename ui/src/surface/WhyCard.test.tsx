// 為什麼問你：why 裡的 ./assets/ 圖片顯示快照（AC-EC-08）。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { DraftAssetsContext } from '../components/RichContent'
import { WhyCard } from './WhyCard'

const ipc = { openDeepLink: async () => {} } as unknown as KunsuIpc
const question = {
  id: 'ec-why',
  title: 't',
  question_line: '要不要加碼？',
  why_md: '看這張圖：\n\n![趨勢](./assets/b.png)',
  body_md: '',
  kind: 'choice',
  tier: 't1',
  requested_tier: 't1',
  evidence: [],
  options: [],
  allow_free_text: true,
  default_action: { on_expire: 'noop_log' },
  wake_on_answer: { mode: 'continue' },
  source: { agent: 'claude', cwd: '/w' },
  state: 'pending',
  snooze_count: 0,
  forced_today_used: false,
  first_proposed: '2026-10-08',
  last_seen: '2026-10-08T00:00:00Z',
  created_at: '2026-10-08T00:00:00Z',
  next_transition_at: '2026-10-08T00:00:00Z',
  draft: { revision: 1, content_hash: 'sha256:x', registered_at: '2026-10-08T00:00:00Z' },
} as unknown as Question

describe('WhyCard', () => {
  it('why 引用的資產顯示快照', () => {
    const html = renderToStaticMarkup(
      <DraftAssetsContext.Provider value={{ './assets/b.png': 'data:image/png;base64,AAAA' }}>
        <WhyCard ipc={ipc} question={question} />
      </DraftAssetsContext.Provider>,
    )
    expect(html).toContain('data:image/png;base64,AAAA')
  })
})
