// 錢 QA 第 11 輪（2026-10-09 主線決定 F2／F3／F9／F10；BDD money-approval 場景 14、AC-MA-20；CONTRACT §9；DESIGN「Money follow-up
// in the receipt」）：回執與失敗畫面依伺服器算的 `money_state`、即時更新、再確認的焦點、錢那一行的圖示。
// 測試環境沒有 DOM：靜態渲染、純函數與原始碼結構；焦點與即時更新另在 vite mock（port 1597）以真瀏覽器實測。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { moneyOutcome, receiptSettled } from '../delivery'
import { DecisionColumn } from './DecisionColumn'
import { failureCopy, phaseAfterExecFailure } from './decision'
import moneyActionsSource from './MoneyActions.tsx?raw'
import columnSource from './DecisionColumn.tsx?raw'

const ipc = new Proxy({} as Record<string, unknown>, {
  get: (_t, key: string) => (key.startsWith('subscribe') ? () => () => {} : async () => null),
}) as unknown as KunsuIpc

function browser() {
  vi.stubGlobal('window', { innerWidth: 1440, innerHeight: 900, matchMedia: () => ({ matches: false }) })
  vi.stubGlobal('document', { documentElement: { dataset: {} }, body: { style: {} } })
  vi.stubGlobal('localStorage', { getItem: () => null })
}

afterEach(() => {
  vi.unstubAllGlobals()
})

function money(result: string | undefined, patch: Partial<Question> = {}): Question {
  const q = {
    id: 'pa-r11',
    title: '加碼',
    question_line: '要不要加碼？',
    body_md: '',
    kind: 'choice',
    tier: 't0',
    requested_tier: 't0',
    gate: 'yellow',
    evidence: [],
    params_line: 'adset 1 日預算 3,000 → 3,600',
    options: [
      { id: 'a', label: '加碼 20%', recommended: true },
      { id: 'b', label: '維持' },
    ],
    allow_free_text: true,
    money: { is_money_op: true, executor_prepare_cmd: 'python3 executor.py prepare-budget --entity 1 --new-budget 3600 --product x' },
    default_action: { on_expire: 'escalate_discord' },
    wake_on_answer: { mode: 'fresh' },
    source: { agent: 'claude', cwd: '/w' },
    state: 'answered',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-09',
    last_seen: '2026-10-09T00:00:00Z',
    created_at: '2026-10-09T00:00:00Z',
    next_transition_at: '2026-10-09T00:00:00Z',
    answer: { answer_id: 'a1', choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:01:00Z', execution_result: result },
    ...patch,
  } as unknown as Question
  const failure = result?.match(/^(?:EXEC_BLOCKED|EXEC_FAILED):\s*(.*)$/s)
  if (failure) q.execution_blocked = failure[1]
  return q
}

const column = (question: Question) =>
  renderToStaticMarkup(
    <DecisionColumn ipc={ipc} question={question} withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
  )
const text = (html: string) => html.replace(/<[^>]*>/g, '')

describe('F2 失敗畫面的文字依 money_state', () => {
  it('可以重新授權：寫「修好原因後可以在這裡重新授權」並有按鈕', () => {
    browser()
    const html = text(column(money('EXEC_BLOCKED: Kunsu 在 executor 驗證授權之前中斷，錢沒有動', { money_state: 'reauthorizable' })))
    expect(html).toContain('執行沒有完成')
    expect(html).toContain('修好原因後可以在這裡重新授權')
    expect(html).toContain('重新授權並執行')
  })

  it('結果不明：標題「結果不明」，請使用者先核對再對帳，有兩個對帳鈕，不寫可以重新授權', () => {
    browser()
    const html = text(column(money('EXEC_FAILED: 死信（重試 3 次仍失敗）', { money_state: 'unknown' })))
    expect(html).toContain('結果不明')
    expect(html).toContain('executor 已用掉授權但沒有回報結果，錢可能已經動過，請先到廣告後台核對再對帳')
    expect(html).toContain('我核對過：錢已經動了')
    expect(html).toContain('我核對過：錢沒有動')
    expect(html).not.toContain('修好原因後可以在這裡重新授權')
    expect(html).not.toContain('執行沒有完成')
  })

  it('已定案（例如已執行完成後交付死信）：不寫可以重新授權、沒有按鈕', () => {
    browser()
    const html = text(column(money('EXEC_FAILED: 死信', { money_state: 'settled' })))
    expect(html).not.toContain('修好原因後可以在這裡重新授權')
    expect(html).not.toContain('重新授權並執行')
    expect(html).not.toContain('我核對過')
  })

  it('非錢題的執行失敗不提重新授權', () => {
    browser()
    const q = money('EXEC_FAILED: 死信', { money: undefined, gate: undefined })
    const html = text(column(q))
    expect(html).toContain('執行沒有完成')
    expect(html).not.toContain('修好原因後可以在這裡重新授權')
  })

  it('failureCopy：純函數', () => {
    expect(failureCopy(money('EXEC_FAILED: x', { money_state: 'unknown' })).label).toBe('結果不明')
    expect(failureCopy(money('EXEC_BLOCKED: x', { money_state: 'reauthorizable' })).note).toContain('修好原因後可以在這裡重新授權')
    expect(failureCopy(money('EXEC_BLOCKED: x', { money_state: 'settled' })).note).not.toContain('重新授權')
    // 舊 shell（沒有 money_state）：被擋的黃燈 UI 執行型答案照舊可以重新授權。
    expect(failureCopy(money('EXEC_BLOCKED: x')).note).toContain('修好原因後可以在這裡重新授權')
  })
})

describe('F3 即時：按鈕跟著最新的題目；執行失敗事件後重新讀題', () => {
  it('執行失敗事件後依重新讀到的題目決定：結果不明 → 回執；還在跑 → 等；其他 → 失敗畫面', () => {
    expect(phaseAfterExecFailure(money('EXEC_UNKNOWN: x', { money_state: 'unknown' }))).toBe('receipt')
    expect(phaseAfterExecFailure(money(undefined, { money_state: 'running' }))).toBe('wait')
    expect(phaseAfterExecFailure(money('EXEC_BLOCKED: x', { money_state: 'reauthorizable' }))).toBe('failed')
    expect(phaseAfterExecFailure(money('EXEC_BLOCKED: x'))).toBe('failed')
    expect(phaseAfterExecFailure(null)).toBe('failed')
  })

  it('回執在錢鏈還在跑時繼續輪詢', () => {
    const settledDelivery = money('COMMIT_OK: ok', { money_state: 'running' })
    expect(receiptSettled(settledDelivery)).toBe(false)
    expect(receiptSettled(money('COMMIT_OK: ok', { money_state: 'settled' }))).toBe(true)
    expect(receiptSettled(money('COMMIT_OK: ok'))).toBe(true)
  })

  it('決策欄收到執行失敗事件後重新讀題（不直接切到失敗畫面）', () => {
    const handler = columnSource.slice(columnSource.indexOf('subscribeExecution('), columnSource.indexOf('subscribeExecution(') + 900)
    expect(handler).toMatch(/getQuestion\(/)
    expect(handler).toMatch(/phaseAfterExecFailure\(/)
  })

  it('MoneyActions 送出後把新題目交回決策欄（prop 隨之更新），本地只留剛送出的暫態', () => {
    expect(moneyActionsSource).not.toMatch(/useState\(\s*question\s*\)/)
    expect(moneyActionsSource).toMatch(/onUpdated\?\.\(/)
    expect(columnSource).toMatch(/<MoneyActions[^>]*onUpdated=/)
  })
})

describe('F9 再確認一次的焦點', () => {
  it('打開確認時焦點移到「確定」，取消回到原按鈕，送出成功後移到「完成／下一題」', () => {
    expect(moneyActionsSource).toMatch(/confirmRef\.current\?\.focus\(/)
    expect(moneyActionsSource).toMatch(/openerRef\.current\?\.focus\(/)
    expect(moneyActionsSource).toMatch(/focusAfter\?\.current\?\.focus\(/)
    expect(columnSource).toMatch(/<MoneyActions[^>]*focusAfter=\{nextRef\}/)
  })

  // 瀏覽器實測（vite mock，port 1597）發現：再確認時把原按鈕卸載，取消後 openerRef 指到已卸載的元素，焦點掉到 <body>。改成只藏起來
  // （hidden），取消後回到同一顆按鈕；此條是修正後補的回歸保護（實作前沒有先紅）。
  it('再確認時原按鈕只藏起來不卸載（取消後焦點回得去）', () => {
    expect(moneyActionsSource.match(/hidden=\{Boolean\(open\)\}/g)?.length ?? 0).toBeGreaterThanOrEqual(2)
    expect(moneyActionsSource).not.toMatch(/action === 'reconcile' && !open/)
  })
})

describe('F10 錢那一行的圖示依狀態、附文字', () => {
  it('moneyOutcome 的圖示：已執行 ✓、乾跑或沒有執行 –、結果不明 !、還在跑 …', () => {
    expect(moneyOutcome(money('COMMIT_OK: ok'))).toMatchObject({ icon: '✓', text: '錢：已執行完成' })
    expect(moneyOutcome(money('RECONCILED_EXECUTED: x'))).toMatchObject({ icon: '✓' })
    expect(moneyOutcome(money('RECONCILED_NOT_EXECUTED: x'))).toMatchObject({ icon: '–', text: '錢：沒有執行（你人工對帳確認）' })
    expect(moneyOutcome(money('DRY_RUN(internal): prepare 未執行'))).toMatchObject({ icon: '–' })
    expect(moneyOutcome(money('EXEC_UNKNOWN: x'))).toMatchObject({ icon: '!', text: '錢：結果不明，可能已經動過，請人工對帳' })
    // 伺服器說結果不明（8/6 型）時，就算執行紀錄是死信也寫結果不明。
    expect(moneyOutcome(money('EXEC_FAILED: 死信', { money_state: 'unknown' }))).toMatchObject({ icon: '!' })
    expect(moneyOutcome(money(undefined, { money_state: 'running' }))).toMatchObject({ icon: '…', text: '錢：錢鏈還在跑' })
  })

  it('回執上錢那一行用 moneyOutcome 的圖示（不是一律 ✓）', () => {
    browser()
    const html = column(money('RECONCILED_NOT_EXECUTED: 使用者人工對帳確認錢沒有動', { money_state: 'reauthorizable' }))
    const row = html.match(/<span class="surface-rc__ic"[^>]*>([^<]*)<\/span>\s*<span>錢：沒有執行/)
    expect(row, '找不到錢的那一行').not.toBeNull()
    expect(row![1]).toBe('–')
    const running = column(money(undefined, { money_state: 'running' }))
    expect(text(running)).toContain('錢：錢鏈還在跑')
  })
})
