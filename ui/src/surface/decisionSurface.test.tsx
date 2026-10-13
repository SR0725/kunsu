// QA 第 1 輪（BDD decision-surface）：面板的靜態渲染與 CSS 結構。測試環境沒有 DOM：用 renderToStaticMarkup 檢查首次渲染，
// window／document／localStorage 以最小 stub 代替；版面的實際幾何另在瀏覽器量測（見 BDD 測試對應）。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc, OverlayTransition } from '../ipc'
import type { Question } from '../types'
import { DecisionSurface } from './DecisionSurface'
import { DecisionColumn } from './DecisionColumn'
import css from '../index.css?raw'

const ipc = new Proxy({} as Record<string, unknown>, {
  get: (_t, key: string) => (key.startsWith('subscribe') ? () => () => {} : async () => null),
}) as unknown as KunsuIpc

function q(partial: Partial<Question> = {}): Question {
  return {
    id: 'q-ds',
    title: '素材 B 要不要加碼',
    question_line: '要不要 **加碼** B？',
    why_md: '每日巡檢發現 B 比較好。',
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [
      { id: 'a', label: '**加碼** 20%', recommended: true },
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

const answered = (partial: Partial<Question> = {}) =>
  q({ state: 'answered', answer: { choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:01:00Z' } as Question['answer'], ...partial })

const leftTransition: OverlayTransition = { id: 1, origin_x: 16, origin_y: 300, origin_size: 18, source: 'capsule' }
const rightTransition: OverlayTransition = { id: 2, origin_x: 1430, origin_y: 300, origin_size: 18, source: 'capsule' }

function browser({ osReduced = false, appMotion, stored }: { osReduced?: boolean; appMotion?: 'full' | 'reduced'; stored?: string } = {}) {
  vi.stubGlobal('window', {
    innerWidth: 1440,
    innerHeight: 900,
    matchMedia: (query: string) => ({ matches: query.includes('reduced-motion') ? osReduced : false, media: query }),
  })
  vi.stubGlobal('document', { documentElement: { dataset: appMotion ? { motion: appMotion } : {} }, body: { style: {} } })
  vi.stubGlobal('localStorage', { getItem: (key: string) => (key === 'kunsu:motion:v1' ? (stored ?? null) : null) })
}

const text = (html: string) => html.replace(/<[^>]*>/g, '')
const panelTag = (html: string) => html.match(/<section class="surface-panel"[^>]*>/)?.[0] ?? ''

function column(question: Question) {
  return renderToStaticMarkup(
    <DecisionColumn ipc={ipc} question={question} withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
  )
}

afterEach(() => vi.unstubAllGlobals())

describe('張開與減少動態（AC-DS-01）', () => {
  it('張開動畫還沒定案時面板 inert：點不到、Tab 不進去（QA #4）', () => {
    browser()
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} transition={leftTransition} onExit={() => {}} />)
    expect(html).toContain('data-animated="true"')
    expect(panelTag(html)).toMatch(/\sinert=""/)
  })

  it('沒有動畫（主視窗）時一開始就可以操作', () => {
    browser()
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} onExit={() => {}} />)
    expect(panelTag(html)).not.toMatch(/\sinert/)
  })

  it('App 的「連續轉場動畫」關閉（html[data-motion=reduced]）時不播形狀動畫（QA #5）', () => {
    browser({ appMotion: 'reduced' })
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} transition={leftTransition} onExit={() => {}} />)
    expect(html).toContain('data-animated="false"')
    expect(panelTag(html)).not.toMatch(/\sinert/)
  })

  it('App 設定已存但 html 還沒套上時同樣不播（QA #5）', () => {
    browser({ stored: 'reduced' })
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} transition={leftTransition} onExit={() => {}} />)
    expect(html).toContain('data-animated="false"')
  })

  it('減少動態時面板圓角朝螢幕內：瀏海在右邊就圓左邊兩個角（QA #14）', () => {
    browser({ osReduced: true })
    const right = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} transition={rightTransition} onExit={() => {}} />)
    expect(panelTag(right)).toMatch(/border-radius:28px 0 0 28px/)
    const left = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} transition={leftTransition} onExit={() => {}} />)
    expect(panelTag(left)).toMatch(/border-radius:0 28px 28px 0/)
  })
})

describe('頁首與按鈕（AC-DS-04、AC-DS-11）', () => {
  it('「稍後再問」以看得到的文字寫出 15 分鐘後再問（QA #7）', () => {
    browser()
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} onExit={() => {}} />)
    const head = html.slice(html.indexOf('<header'), html.indexOf('</header>'))
    expect(text(head)).toContain('15 分鐘後再問')
  })

  it('已回答的題目頁首沒有倒數或警告色殘留（QA #12，回歸）', () => {
    browser()
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[answered()]} onExit={() => {}} />)
    const head = html.slice(html.indexOf('<header'), html.indexOf('</header>'))
    expect(head).not.toContain('surface-cycle')
    expect(head).not.toContain('data-warning')
    expect(text(head)).not.toMatch(/已回答|已到期|強制詢問/)
  })

  it('確認題的主按鈕是「確認」（QA #8）', () => {
    browser()
    const html = column(q({ kind: 'confirm', options: [] }))
    const foot = text(html.slice(html.indexOf('<footer')))
    expect(foot).toContain('確認')
    expect(foot).not.toContain('確認執行')
  })

  it('確認題的補充說明是選填，不是必填的「你的回答」（AC-DS-15）', () => {
    browser()
    const html = text(column(q({ kind: 'confirm', options: [] })))
    expect(html).toContain('要你決定')
    expect(html).toContain('補充說明（選填）')
    expect(html).not.toContain('你的回答')
  })
})

describe('回執與失敗畫面（AC-DS-09、AC-DS-15）', () => {
  it('回執的題目與選項經 Markdown 渲染，不出現 **（QA #11）', () => {
    browser()
    const html = column(answered())
    expect(text(html)).toContain('已送出')
    expect(text(html)).not.toContain('**')
    expect(html).toContain('<strong>加碼</strong>')
  })

  it('執行失敗畫面的題目同樣經 Markdown 渲染（QA #11）', () => {
    browser()
    const html = column(answered({ execution_blocked: 'prepare 失敗' }))
    expect(text(html)).toContain('執行沒有完成')
    expect(text(html)).not.toContain('**')
  })

  it('確認題的回執寫「確認」（QA #6）', () => {
    browser()
    const html = column(
      q({
        kind: 'confirm',
        options: [],
        state: 'answered',
        answer: { choice_id: 'confirm', channel: 'ui', answered_at: '2026-10-09T00:01:00Z' } as Question['answer'],
      }),
    )
    const final = html.slice(html.indexOf('surface-final'), html.indexOf('surface-receipt'))
    expect(text(final)).toContain('確認')
    expect(text(final)).not.toContain('已作答')
  })
})

// ---- CSS 結構（版面幾何另在瀏覽器量測）----------------------------------------------------------

/** 取出 `@media <query> { … }`（或其他 at-rule，例如 `@container surface (…)`）的內容（大括號配對）。 */
function mediaBlocks(source: string, query: string, atRule = '@media'): string[] {
  const out: string[] = []
  let from = 0
  for (;;) {
    const at = source.indexOf(`${atRule} ${query}`, from)
    if (at < 0) return out
    const open = source.indexOf('{', at)
    let depth = 0
    let i = open
    for (; i < source.length; i++) {
      if (source[i] === '{') depth++
      else if (source[i] === '}' && --depth === 0) break
    }
    out.push(source.slice(open + 1, i))
    from = i
  }
}

/** 去掉所有 @media／@container 區塊，只留最外層規則。 */
function topLevel(source: string): string {
  let out = source
  for (;;) {
    const at = out.search(/@(media|container)\b/)
    if (at < 0) return out
    const open = out.indexOf('{', at)
    let depth = 0
    let i = open
    for (; i < out.length; i++) {
      if (out[i] === '{') depth++
      else if (out[i] === '}' && --depth === 0) break
    }
    out = out.slice(0, at) + out.slice(i + 1)
  }
}

function rules(source: string): { selector: string; body: string }[] {
  const flat = source.replace(/\/\*[\s\S]*?\*\//g, '')
  return [...flat.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector, body }))
}

/** 一段 CSS 裡 selector（逗號分開的任一段）以 `needle` 結尾的所有規則宣告（合併）。 */
function decls(source: string, needle: string): string {
  return rules(source)
    .filter(({ selector }) => selector.split(',').some((part) => part.trim().endsWith(needle)))
    .map(({ body }) => body)
    .join(';')
}

function paddingBottom(body: string): string | null {
  const explicit = body.match(/padding-bottom:\s*([^;]+)/)
  if (explicit) return explicit[1].trim()
  const shorthand = [...body.matchAll(/(?:^|;)\s*padding:\s*([^;]+)/g)].pop()
  if (!shorthand) return null
  const parts = shorthand[1].trim().split(/\s+/)
  return parts.length === 1 ? parts[0] : parts.length === 2 ? parts[0] : parts[2]
}

describe('窄螢幕與操作列（AC-DS-14）', () => {
  // 2026-10-09 作者核准（QA 第 2 輪）：單欄以面板本身的寬度判斷（container query），不再看視窗寬度。
  const narrow = mediaBlocks(css, 'surface (width < 1100px)', '@container').join('\n')

  it('寬度 < 1100 px：單欄、整個內容區一起捲動、列高跟內容走（QA #3）', () => {
    const body = decls(narrow, '.surface-body[data-layout="split"]')
    expect(body).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\)/)
    expect(body).toMatch(/grid-auto-rows:\s*max-content/)
    expect(body).toMatch(/overflow-y:\s*auto/)
  })

  it('窄版證據與決定欄撐開自己的高度，不各自捲動（QA #3）', () => {
    expect(decls(narrow, '[data-layout="split"] .surface-evidence')).toMatch(/overflow:\s*visible/)
    expect(decls(narrow, '[data-layout="split"] .surface-decision')).toMatch(/overflow:\s*visible/)
  })

  it('窄版操作列固定在面板底部，內容區預留它的高度（QA #3）', () => {
    const foot = decls(narrow, '[data-layout="split"] .surface-foot')
    expect(foot).toMatch(/position:\s*absolute/)
    expect(foot).toMatch(/bottom:\s*0/)
    expect(paddingBottom(decls(narrow, '[data-layout="split"] .surface-decision'))).not.toBeNull()
  })

  it('任何寬度下捲動內容不從操作列下方露出：決定欄底部不留白，留白算在操作列自己身上（QA #13）', () => {
    expect(paddingBottom(decls(topLevel(css), '.surface-decision'))).toMatch(/^0(px)?$/)
    expect(paddingBottom(decls(topLevel(css), '.surface-foot'))).toMatch(/^[1-9]\d*px$/)
  })

  it('長網址換行，決定欄不橫向捲動（QA #13）', () => {
    const decision = decls(topLevel(css), '.surface-decision')
    expect(decision).toMatch(/overflow-wrap:\s*anywhere/)
    expect(decision).toMatch(/overflow-x:\s*hidden/)
  })
})

describe('動態 CSS（AC-DS-01）', () => {
  it('系統減少動態時保留 160 ms 淡入，不被全域規則壓成瞬間（QA #5）', () => {
    const reduced = mediaBlocks(css, '(prefers-reduced-motion: reduce)').join('\n')
    expect(decls(reduced, '.surface-panel')).toMatch(/animation-duration:\s*160ms\s*!important/)
  })

  it('App 設定減少動態時同樣保留 160 ms 淡入（QA #5）', () => {
    expect(decls(css, 'html[data-motion="reduced"] .surface-stage[data-animated="false"] .surface-panel')).toMatch(
      /animation-duration:\s*160ms\s*!important/,
    )
  })

  it('內容在張開時以 45 ms 間隔依序淡入，只在張開階段（切題不重播）（QA #9）', () => {
    const stagger = rules(css).filter(({ body }) => /animation-delay:[^;]*45ms/.test(body))
    expect(stagger.length).toBeGreaterThan(0)
    for (const rule of stagger) expect(rule.selector).toContain('[data-stage="opening"]')
  })
})
