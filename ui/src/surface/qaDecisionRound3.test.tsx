// QA 第 3 輪（BDD decision-surface、overlay-presentation）：第 2 輪修正（merge d3a0d2d）之後的獨立 QA。
// 測試環境沒有 DOM：靜態渲染（renderToStaticMarkup）、純函數與原始碼結構；需要真瀏覽器的行為（effect 時序、焦點）
// 另見 QA 報告裡的瀏覽器重現步驟（vite mock 模式，`?view=overlay`）。
// 標「QA3-紅」的測試在目前 main 會失敗，是要修的缺口；其餘是本輪確認成立的行為的回歸保護。
//
// 瀏覽器重現（AC-DS-09、AC-OP-07(c)，本檔無法用靜態渲染重現，只記步驟）：
//   1. `pnpm --dir ui exec vite`，開 `/?view=overlay`（mock），等面板張開。
//   2. console：把目前題目以外的題都標成在其他地方結案
//      `for (const id of ['mail-reply-1009','pa-creative-b-scale-1008','pa-course-b-bait-restart-0731','pa-nova-kongko-batch-0731',
//        'pa-acme-creative-approved-0731','pa-nova-bets-restructure','pa-acme-ad1j-leads-migration']) __kunsuMock.closeElsewhere(id)`
//   3. 選「維持現狀」按送出。期望：回執留在畫面上直到「完成 ⏎」或 Esc。實際：50 ms 內面板就消失，約 330 ms 後送出
//      `kunsu://overlay/close`，回執從沒出現。原因：清單更新（作答時 server 先發 questions_changed）讓 OverlayShell 看到
//      「0 題、holdingReceipt=false」→ `overlayShellAction` 回 close → OverlayShell 立刻 `return null` 卸載面板；
//      holdingReceipt 要等面板的 passive effect 回報，但面板已經被卸載，永遠回報不到。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc, OverlayTransition } from '../ipc'
import type { Question } from '../types'
import { DecisionSurface } from './DecisionSurface'
import { DecisionColumn } from './DecisionColumn'
import { escapeAction, holdsReceipt, runLater } from './decision'
import { overlayShellAction } from '../overlayShell'
import { mergeQuestionSession } from '../components/overlaySession'
import appSource from '../App.tsx?raw'

// 記下每個 RichContent 拿到的 props（照樣渲染真正的元件），檢查連結有沒有「打開」的出口。
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
    id: 'q-qa3',
    title: '素材 B 要不要加碼',
    question_line: '要不要加碼 B？',
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

function browser() {
  vi.stubGlobal('window', { innerWidth: 1440, innerHeight: 900, matchMedia: () => ({ matches: false }) })
  vi.stubGlobal('document', { documentElement: { dataset: {} }, body: { style: {} } })
  vi.stubGlobal('localStorage', { getItem: () => null })
}

const transition: OverlayTransition = { id: 1, origin_x: 16, origin_y: 300, origin_size: 18, source: 'capsule' }
const opener = (children: string) => richCalls.filter((call) => call.children === children)
const expectOpenable = (children: string) => {
  const calls = opener(children)
  expect(calls.length, `找不到渲染「${children}」的 RichContent`).toBeGreaterThan(0)
  for (const call of calls) expect(typeof call.onOpenLink, `「${children}」的連結第二次點擊要能打開`).toBe('function')
}

afterEach(() => {
  vi.unstubAllGlobals()
  richCalls.length = 0
})

// =====================================================================================================

describe('AC-DS-13 每一個 Markdown 連結都接上同一個打開入口（第 3 輪：選項）', () => {
  // CONTRACT §9：options 支援 Markdown（選項標題與說明沒有長度上限；question_line 有 24 字上限，實務上放不下連結，不列）。
  // RichContent 沒有 onOpenLink 時，第一次點顯示「（再按一次用瀏覽器開啟 網域）」，第二次點什麼都不做——提示說可以開、
  // 實際打不開（DESIGN「Markdown body」：提示只出現在 Kunsu 開得了的連結）。瀏覽器實測（mock 蓋板，選項說明放
  // `[試算表](https://sheets.example.com/s)`）：第一次點出現提示、同時把選項選起來；第二次點沒有打開任何東西、選項又被取消
  // ——連結在選項按鈕裡，點擊會冒泡成選取／取消選取。
  it('QA3-紅：選項標題與說明裡的連結', () => {
    browser()
    const label = '照 [提案](https://docs.example.com/p) 執行'
    const detail = '細節見 [試算表](https://sheets.example.com/s)'
    renderToStaticMarkup(
      <DecisionColumn
        ipc={ipc}
        question={q({ options: [{ id: 'a', label, detail, recommended: true }, { id: 'b', label: '維持' }] })}
        withWhy={false}
        isLast={false}
        active={false}
        onUpdated={() => {}}
        onNext={() => {}}
      />,
    )
    expectOpenable(label)
    expectOpenable(detail)
  })

  it('QA3-紅：回執裡你選的選項', () => {
    browser()
    const label = '照 [提案](https://docs.example.com/p2) 執行'
    renderToStaticMarkup(
      <DecisionColumn
        ipc={ipc}
        question={q({
          options: [{ id: 'a', label, recommended: true }, { id: 'b', label: '維持' }],
          state: 'answered',
          answer: { choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:01:00Z' } as Question['answer'],
        })}
        withWhy={false}
        isLast
        active={false}
        onUpdated={() => {}}
        onNext={() => {}}
      />,
    )
    expectOpenable(label)
  })
})

describe('AC-DS-16（與 AC-BR-09）主視窗的作答 flow 也要跟著最新清單', () => {
  // 面板靠 `latest`（最新完整清單）把 session 裡的題換成最新資料：被其他通道結案的就地唯讀、草稿重新註冊時 rebase。
  // 蓋板（OverlayShell）有傳；主視窗（「開始處理 N 題」、待決清單點一題、蓋板就緒逾時的後備）只傳開啟當下的 snapshot
  // （`view.questions`），之後清單怎麼變面板都看不到。瀏覽器實測（mock 主視窗）：`__kunsuMock.closeElsewhere` 目前題目後
  // 面板仍是「要你決定」＋3 個選項；題組重新註冊後卡片沒有「AI 更新了這一項」，送出帶的仍是舊雜湊（server 會拒收「請重新
  // 開啟面板」，重開後使用者的修改全部消失）。
  it('QA3-紅：App.tsx 裡每一個 <DecisionSurface> 都拿到最新清單（latest）', () => {
    const elements = [...appSource.matchAll(/<DecisionSurface\b([\s\S]*?)\/>/g)].map((m) => m[1])
    expect(elements.length).toBeGreaterThanOrEqual(2)
    for (const props of elements) expect(props, `<DecisionSurface${props.replace(/\s+/g, ' ')}/>`).toMatch(/\blatest=/)
  })
})

describe('AC-DS-10／AC-DS-16 回看已結案的題（主視窗「歷史紀錄」點一題）', () => {
  // 主視窗從歷史紀錄打開已結案的題（`openFlow([question])`）：面板只把「在這次 session 裡從未結案變成結案」的題標成
  // 「已在其他地方結案」，一打開就已經是終態的題不算。瀏覽器實測（mock 主視窗→歷史紀錄）：過期的「今天最重要的三件事？」
  // 顯示「要你回應」＋回答框＋「送出回答」（送出一定被 server 以已結案拒收）；出題的 AI 取消的「這週三個目標進度如何？」
  // 顯示「已退回重擬／已退回出題者重擬」，像是使用者自己推掉的（使用者推掉的題 server 會記 `rejected`，UI 型別也有）。
  it('QA3-紅：過期（沒有答案）的題回看時是唯讀畫面，沒有回答框與送出', () => {
    browser()
    const html = renderToStaticMarkup(
      <DecisionColumn ipc={ipc} question={q({ kind: 'text', options: [], state: 'expired' })} withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
    )
    expect(html).not.toContain('<textarea')
    expect(html.replace(/<[^>]*>/g, '')).not.toMatch(/要你回應|要你決定|送出回答/)
  })

  it('QA3-紅：出題的 AI 取消的題回看時不寫「已退回重擬」（那是使用者自己推掉的說法）', () => {
    browser()
    const html = renderToStaticMarkup(
      <DecisionColumn ipc={ipc} question={q({ state: 'cancelled' })} withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
    )
    expect(html.replace(/<[^>]*>/g, '')).not.toContain('退回重擬')
  })

  it('回歸：使用者推掉的題（rejected）回看時寫「已退回重擬」', () => {
    browser()
    const html = renderToStaticMarkup(
      <DecisionColumn
        ipc={ipc}
        question={q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z', reason: '不該問我' } } as Partial<Question>)}
        withWhy={false}
        isLast
        active={false}
        onUpdated={() => {}}
        onNext={() => {}}
      />,
    )
    expect(html.replace(/<[^>]*>/g, '')).toContain('已退回重擬')
  })
})

describe('AC-OP-06 稍後再問失敗後重試：每一題只記一次 dismissal', () => {
  // 面板的稍後再問會逐題 dismissal（目前題目＋session 裡所有 forced 題）；中途失敗就停、顯示錯誤，使用者重試時面板重新算一次
  // 範圍：目前題目還沒結案，所以又在裡面。瀏覽器實測（mock，第 2 次 dismissal 失敗後再按 Esc）：TW-3 被 dismissal 兩次
  // （cycle_number、snooze_count 各 +2，帳本兩筆 force_cycle_dismissed），creative-b 一次。
  it('QA3-紅：第一次只成功一半，重試時不會再對已成功的題記一次 dismissal', async () => {
    const calls: string[] = []
    let locked = true
    const dismiss = async (id: string) => {
      calls.push(id)
      if (id === 'q-forced-2' && locked) throw new Error('資料庫暫時鎖住了')
    }
    const ids = ['q-current', 'q-forced-2'] // DecisionSurface.later：目前題目（未結案）＋session 裡的 forced 題
    // 2026-10-09 作者決定「記住這次已成功的題、重試只做失敗的」：記憶由面板持有、跨重試共用（`done`），兩次呼叫傳同一個。
    const done = new Map<string, number | null>()
    const first = await runLater({ ids, dismiss, close: () => {}, done })
    expect(first.ok).toBe(false)
    locked = false
    const retry = await runLater({ ids, dismiss, close: () => {}, done })
    expect(retry.ok).toBe(true)
    expect(calls.filter((id) => id === 'q-current'), `實際呼叫順序：${calls.join(' → ')}`).toHaveLength(1)
  })

  it('回歸：失敗不通知原生、不收起；成功才通知原生（再收起）', async () => {
    const events: string[] = []
    const failed = await runLater({
      ids: ['a'],
      dismiss: async () => {
        throw new Error('x')
      },
      ack: () => events.push('ack'),
      close: () => void events.push('close'),
    })
    expect(failed.ok).toBe(false)
    expect(events).toEqual([])
    await runLater({ ids: ['a', 'b'], dismiss: async (id) => void events.push(`dismiss ${id}`), ack: () => events.push('ack'), close: () => void events.push('close') })
    expect(events).toEqual(['dismiss a', 'dismiss b', 'ack', 'close'])
  })

  it('回歸：行內區塊裡的按鈕上按 Esc＝稍後再問；輸入框裡只收起編輯；收起中交給原生', () => {
    expect(escapeAction({ stage: 'open', composing: false, local: true, field: false })).toEqual({ later: true, ack: true })
    expect(escapeAction({ stage: 'open', composing: false, local: true, field: true })).toEqual({ later: false, ack: true })
    expect(escapeAction({ stage: 'opening', composing: false, local: false, field: false })).toEqual({ later: true, ack: true })
    expect(escapeAction({ stage: 'closing', composing: false, local: false, field: false })).toEqual({ later: false, ack: false })
  })
})

describe('AC-OP-07 蓋板 session 增減（回歸）', () => {
  it('session 開著時才到期的強制題接到尾端；已在 session 的題原地更新、不重複；internal 與未到期的不插進來', () => {
    const a = q({ id: 'a', state: 'forced' })
    const b = q({ id: 'b', state: 'pending' })
    const session = [a, b]
    const latest = [
      q({ id: 'a', state: 'answered', answer: { choice_id: 'a', channel: 'discord', answered_at: '2026-10-09T00:02:00Z' } as Question['answer'] }),
      q({ id: 'b', state: 'pending' }),
      q({ id: 'c', state: 'forced' }),
      q({ id: 'doctor', state: 'forced', internal: true }),
      q({ id: 'later', state: 'pending' }),
    ]
    const merged = mergeQuestionSession(session, latest)
    expect(merged.map((item) => `${item.id}:${item.state}`)).toEqual(['a:answered', 'b:pending', 'c:forced'])
  })

  it('保留回執只在面板正顯示這次在面板送出的回執時成立', () => {
    const answered = q({ id: 'a', state: 'answered', answer: { choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:01:00Z' } as Question['answer'] })
    expect(holdsReceipt(answered, new Set(['a']))).toBe(true)
    expect(holdsReceipt(answered, new Set())).toBe(false)
    expect(holdsReceipt(q({ id: 'b' }), new Set(['a']))).toBe(false)
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true, holdingReceipt: true })).toBe('render')
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true, holdingReceipt: false })).toBe('close')
  })

  it('頁首的佇列跟著 session：第 N 題，共 M 題', () => {
    browser()
    const html = renderToStaticMarkup(
      <DecisionSurface ipc={ipc} questions={[q({ id: 'a' }), q({ id: 'b', state: 'pending' }), q({ id: 'c', state: 'forced' })]} transition={transition} onExit={() => {}} />,
    )
    expect(html.replace(/<[^>]*>/g, '')).toContain('第 1 題，共 3 題')
  })
})
