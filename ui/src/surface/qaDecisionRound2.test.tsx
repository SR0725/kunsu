// QA 第 2 輪（BDD decision-surface、overlay-presentation）：第 1 輪修正之後、題組與 motion 模組合併進 main 之後的重現測試。
// 測試環境沒有 DOM：靜態渲染（renderToStaticMarkup）、純函數與 CSS 結構；需要真瀏覽器的行為（焦點、Enter 在連結上、
// 1100 px 寬時捲不動）另見 QA 報告裡的瀏覽器重現步驟。
// 這個檔案裡標「QA2-紅」的測試在目前 main 會失敗，是要修的缺口；其餘是第 1 輪修正的回歸保護。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc, OverlayTransition } from '../ipc'
import type { BatchItem, Question } from '../types'
import { DecisionSurface } from './DecisionSurface'
import { DecisionColumn } from './DecisionColumn'
import { BatchReview } from './BatchColumn'
import { openOutside } from './decision'
import { overlayShellAction } from '../overlayShell'
import { mergeQuestionSession } from '../components/overlaySession'
import css from '../index.css?raw'
import batchCss from './batch.css?raw'

// 記下每個 RichContent 拿到的 props（照樣渲染真正的元件），用來檢查連結有沒有「打開」的出口。
const richCalls = vi.hoisted(() => [] as { children: string; onOpenLink?: unknown }[])
vi.mock('../components/RichContent', async (importOriginal) => {
  const mod = await importOriginal<typeof import('../components/RichContent')>()
  return {
    ...mod,
    RichContent: (props: Parameters<typeof mod.RichContent>[0]) => {
      richCalls.push({ children: props.children, onOpenLink: props.onOpenLink })
      return mod.RichContent(props)
    },
  }
})

const ipc = new Proxy({} as Record<string, unknown>, {
  get: (_t, key: string) => (key.startsWith('subscribe') ? () => () => {} : async () => null),
}) as unknown as KunsuIpc

function q(partial: Partial<Question> = {}): Question {
  return {
    id: 'q-qa2',
    title: '素材 B 要不要加碼',
    question_line: '要不要 **加碼** B？',
    why_md: '每日巡檢發現 B 比較好。',
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [
      { id: 'a', label: '加碼 20%', recommended: true },
      { id: 'b', label: '維持' },
    ],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'continue' },
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

const ITEMS: BatchItem[] = [
  { id: 'i1', title: '陳同學', context_md: '老師好，第 5 章打不開。', proposal: '已經修好了。', editable: true, default: 'approve', content_hash: 'sha256:1' },
  { id: 'i2', title: '王同學', context_md: '請問發票。', proposal: '已寄出。', editable: true, default: 'approve', content_hash: 'sha256:2' },
]

const answeredBatch = (partial: Partial<Question> = {}) =>
  q({
    id: 'mail-qa2',
    kind: 'batch',
    options: [],
    items: ITEMS,
    question_line: '今天 **2 封信**的回覆',
    state: 'answered',
    answer: {
      answer_id: 'a1',
      channel: 'ui',
      answered_at: '2026-10-09T01:05:00Z',
      items: [
        { item_id: 'i1', action: 'approve', text: '已經修好了。' },
        { item_id: 'i2', action: 'approve', text: '已寄出。' },
      ],
      delivery: { state: 'resumed', updated_at: '2026-10-09T01:06:00Z' },
      report: { status: 'done', summary_md: '已送出 2 封，見 [寄件備份](https://mail.example.com/sent/1)。', reported_at: '2026-10-09T01:07:00Z' },
    } as Question['answer'],
    ...partial,
  })

function browser() {
  vi.stubGlobal('window', {
    innerWidth: 1440,
    innerHeight: 900,
    matchMedia: () => ({ matches: false }),
  })
  vi.stubGlobal('document', { documentElement: { dataset: {} }, body: { style: {} } })
  vi.stubGlobal('localStorage', { getItem: () => null })
}

const text = (html: string) => html.replace(/<[^>]*>/g, '')
const transition: OverlayTransition = { id: 1, origin_x: 16, origin_y: 300, origin_size: 18, source: 'capsule' }

afterEach(() => {
  vi.unstubAllGlobals()
  richCalls.length = 0
})

// ---- CSS 小工具（與第 1 輪的 decisionSurface.test.tsx 相同做法）--------------------------------------
function rules(source: string): { selector: string; body: string }[] {
  const flat = source.replace(/\/\*[\s\S]*?\*\//g, '')
  return [...flat.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body }))
}

function topLevel(source: string): string {
  let out = source
  for (;;) {
    const at = out.indexOf('@media')
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

/** 所有 `@media (max-width: N)` 的 N 與區塊內容。 */
function maxWidthBlocks(source: string): { max: number; body: string }[] {
  const out: { max: number; body: string }[] = []
  const re = /@media\s*\(max-width:\s*([\d.]+)px\)\s*\{/g
  for (let m = re.exec(source); m; m = re.exec(source)) {
    let depth = 1
    let i = re.lastIndex
    for (; i < source.length && depth > 0; i++) {
      if (source[i] === '{') depth++
      else if (source[i] === '}') depth--
    }
    out.push({ max: Number(m[1]), body: source.slice(re.lastIndex, i - 1) })
  }
  return out
}

// =====================================================================================================

describe('AC-DS-09 回執 Markdown：題組路徑（第 1 輪 QA #11 的修正沒有套到合併進來的題組回執）', () => {
  it('QA2-紅：題組回執的題目經 Markdown 渲染，不出現 **', () => {
    const html = renderToStaticMarkup(
      <BatchReview ipc={ipc} question={answeredBatch()} why={null} evidence={null} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
    )
    const ask = html.slice(html.indexOf('<h2 class="surface-ask"'), html.indexOf('</h2>'))
    expect(text(ask)).not.toContain('**')
    expect(ask).toContain('<strong>2 封信</strong>')
  })

  it('回歸：一般題的回執題目仍經 Markdown 渲染', () => {
    browser()
    const html = renderToStaticMarkup(
      <DecisionColumn
        ipc={ipc}
        question={q({ state: 'answered', answer: { choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:01:00Z' } as Question['answer'] })}
        withWhy={false}
        isLast
        active={false}
        onUpdated={() => {}}
        onNext={() => {}}
      />,
    )
    expect(html).toContain('<strong>加碼</strong>')
  })
})

describe('AC-DS-13 內文連結第二次點擊要能打開（蓋板裡先讓開）：回執與證據裡有些 Markdown 沒有接上打開的出口', () => {
  // RichContent 沒有 onOpenLink 時，第一次點會顯示「再按一次用瀏覽器開啟 網域」，第二次點什麼都不做（RichContent.tsx:127）。
  const opener = (children: string) => richCalls.filter((call) => call.children === children)

  it('QA2-紅：一般題回執的「AI 回饋」連結可以打開', () => {
    browser()
    const summary = '已調整預算，見 [報表](https://ads.example.com/report/1)。'
    renderToStaticMarkup(
      <DecisionColumn
        ipc={ipc}
        question={q({
          state: 'answered',
          answer: {
            choice_id: 'a',
            channel: 'ui',
            answered_at: '2026-10-09T00:01:00Z',
            report: { status: 'done', summary_md: summary, reported_at: '2026-10-09T00:02:00Z' },
          } as Question['answer'],
        })}
        withWhy={false}
        isLast
        active={false}
        onUpdated={() => {}}
        onNext={() => {}}
      />,
    )
    const calls = opener(summary)
    expect(calls.length).toBeGreaterThan(0)
    for (const call of calls) expect(typeof call.onOpenLink).toBe('function')
  })

  it('QA2-紅：題組回執的「AI 回饋」連結可以打開', () => {
    const batch = answeredBatch()
    renderToStaticMarkup(<BatchReview ipc={ipc} question={batch} why={null} evidence={null} isLast active={false} onUpdated={() => {}} onNext={() => {}} />)
    const calls = opener(batch.answer!.report!.summary_md)
    expect(calls.length).toBeGreaterThan(0)
    for (const call of calls) expect(typeof call.onOpenLink).toBe('function')
  })

  it('QA2-紅：證據行（evidence[].text）與時間軸裡的連結可以打開', () => {
    browser()
    const line = 'Meta 報表：[ROAS 2.4](https://ads.example.com/b)'
    const step = '看了 [PR #12](https://github.com/example/repo/pull/12)'
    renderToStaticMarkup(
      <DecisionSurface
        ipc={ipc}
        questions={[q({ evidence: [{ text: line, source: 'Meta API' }], why_md: `- 09:30 ${step}\n- 現在 等你決定` })]}
        transition={transition}
        onExit={() => {}}
      />,
    )
    for (const children of [line, step]) {
      const calls = opener(children)
      expect(calls.length, children).toBeGreaterThan(0)
      for (const call of calls) expect(typeof call.onOpenLink, children).toBe('function')
    }
  })

  it('回歸：證據 Markdown 內文的連結有打開的出口', () => {
    browser()
    const body = '細節見 [儀表板](https://ads.example.com/dash)'
    renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q({ body_md: body })]} transition={transition} onExit={() => {}} />)
    const calls = opener(body)
    expect(calls.length).toBeGreaterThan(0)
    for (const call of calls) expect(typeof call.onOpenLink).toBe('function')
  })
})

describe('AC-DS-13 蓋板先讓開再打開', () => {
  it('QA2-紅：蓋板讓開（稍後再問／關閉的 IPC 與收起動畫跑完、視窗隱藏）之後才打開目標', async () => {
    // 實際的讓開（DecisionSurface.later）是非同步的：dismiss IPC → 收起動畫（約 0.73 s，後備 0.9 s）→ onExit → kunsu://overlay/close。
    const events: string[] = []
    const stepAside = () =>
      new Promise<void>((resolve) =>
        setTimeout(() => {
          events.push('蓋板已隱藏')
          resolve()
        }, 0),
      )
    const open = openOutside({ inOverlay: true, stepAside, open: (href) => events.push(`打開 ${href}`) })
    open('claude://code/continue?session=local_x')
    await new Promise((resolve) => setTimeout(resolve, 5))
    expect(events).toEqual(['蓋板已隱藏', '打開 claude://code/continue?session=local_x'])
  })
})

describe('AC-DS-10 切題淡出淡入', () => {
  it('QA2-紅：新題目的內容會淡入（.surface-body 以 key={q.id} 重新掛載，只有 transition 的話新元素一出現就是不透明）', () => {
    browser()
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} onExit={() => {}} />)
    // 新掛載的內容區一開始就是 data-fading="false"（opacity 1），transition 沒有起點可以淡入。
    expect(html).toMatch(/class="surface-body"[^>]*data-fading="false"/)
    const body = rules(topLevel(css))
      .filter(({ selector }) => selector.split(',').some((part) => part.trim().startsWith('.surface-body')))
      .map(({ body }) => body)
      .join(';')
    expect(body).toMatch(/animation(-name)?:/)
  })
})

/** 所有 `@container surface (width < Npx)` 的 N 與區塊內容。 */
function containerBlocks(source: string): { max: number; body: string }[] {
  const out: { max: number; body: string }[] = []
  const re = /@container\s+surface\s*\(\s*width\s*<\s*([\d.]+)px\s*\)\s*\{/g
  for (let m = re.exec(source); m; m = re.exec(source)) {
    let depth = 1
    let i = re.lastIndex
    for (; i < source.length && depth > 0; i++) {
      if (source[i] === '{') depth++
      else if (source[i] === '}') depth--
    }
    out.push({ max: Number(m[1]), body: source.slice(re.lastIndex, i - 1) })
  }
  return out
}

describe('AC-DS-14 窄螢幕斷點：題組與面板必須是同一個斷點', () => {
  // 2026-10-09 作者核准的修法：以「面板本身的寬度」判斷單欄（CSS container query），決策面板與題組共用同一個斷點。
  // 原本的斷言是「題組的 @media max-width 不大於面板的 @media max-width」；改成 container query 後兩份樣式表都不再有
  // 視窗寬度的單欄規則，所以斷言改寫成同一條 container 條件（< 1100 px），意思不變：1100 px 整時兩邊都是兩欄。
  it('QA2-紅：題組的單欄規則只在面板真的變單欄（< 1100 px）時套用；1100 px 整時決定欄不能變成無法捲動', () => {
    const surface = containerBlocks(css).filter(({ body }) => body.includes('.surface-body[data-layout="split"]'))
    expect(surface.length, 'index.css 要有 @container surface (width < 1100px) 的單欄規則').toBeGreaterThan(0)
    expect(new Set(surface.map(({ max }) => max))).toEqual(new Set([1100]))
    // batch.css 在單欄時把決定欄改成 overflow: visible：只能在面板真的變單欄的同一個條件下套用
    // → 舊版實測 1100×800：決定欄高 2671 px、面板只有 772 px，第 3 項之後滑鼠捲不到。
    const batch = containerBlocks(batchCss).filter(({ body }) => /overflow:\s*visible/.test(body))
    expect(batch.length, 'batch.css 的單欄規則要用同一個 container 條件').toBeGreaterThan(0)
    for (const block of batch) expect(block.max, `batch.css @container surface (width < ${block.max}px)`).toBe(1100)
    // 不再看視窗寬度：兩份樣式表都沒有 @media max-width 的單欄規則。
    expect(maxWidthBlocks(css).filter(({ body }) => body.includes('.surface-body[data-layout="split"]'))).toEqual([])
    expect(maxWidthBlocks(batchCss).filter(({ body }) => /overflow:\s*visible/.test(body))).toEqual([])
    // 面板本身是 container。
    expect(rules(topLevel(css)).some(({ selector, body }) => selector === '.surface-panel' && /container(-type)?\s*:/.test(body))).toBe(true)
  })
})

describe('AC-OP-07 蓋板 session（回歸）', () => {
  it('在面板裡答完最後一題時保留回執；沒有回執要保留時，題目全由其他通道結案就收起', () => {
    // 注意：App.tsx 在 session 裡「任何一題」結案就把 holdingReceipt 設成 true、直到 exit 才清；
    // 使用者已經離開那張回執去看別題時也一樣（見 QA 報告 AC-OP-07 (b)，這一層的純函數沒有「面板正顯示回執」的輸入）。
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true, holdingReceipt: true })).toBe('render')
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true, holdingReceipt: false })).toBe('close')
  })
})

describe('PRD §5.8 多題排隊：session 開著時才變成強制的題', () => {
  it('QA2-紅：答完目前的題後，仍有已強制的題時 flow 可以前往它（不需關閉再彈出）', () => {
    const first = q({ id: 'q1', state: 'answered', answer: { choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:01:00Z' } as Question['answer'] })
    const second = q({ id: 'q2', state: 'forced' })
    // 蓋板網頁傳進面板的是「未結案」清單：q1 已答所以不在裡面，q2 是 session 開著時才到期的強制題。
    const session = mergeQuestionSession([first], [second])
    expect(session.map((item) => item.id)).toContain('q2')
  })
})
