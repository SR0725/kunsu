// 錢題回執的後續動作（CONTRACT §7、§9；BDD money-approval AC-MA-18／19／20）：結果不明 → 使用者人工對帳；被擋或對帳確認沒動 → 重新授權。
import { describe, expect, it } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import type { Question } from '../types'
import { moneyActionsFor, MoneyActions } from './MoneyActions'
import type { KunsuIpc } from '../ipc'

function money(result: string | undefined, patch: Partial<Question> = {}): Question {
  return {
    id: 'pa-x',
    title: '加碼',
    question_line: '要不要加碼？',
    body_md: '',
    kind: 'choice',
    tier: 't0',
    gate: 'yellow',
    options: [
      { id: 'a', label: '加碼 20%', recommended: true },
      { id: 'b', label: '維持' },
    ],
    money: { is_money_op: true },
    state: 'answered',
    answer: { answer_id: 'a1', choice_id: 'a', channel: 'ui', answered_at: '2026-10-09T00:00:00Z', execution_result: result },
    ...patch,
  } as unknown as Question
}

describe('moneyActionsFor', () => {
  it('結果不明 → 人工對帳', () => {
    expect(moneyActionsFor(money('EXEC_UNKNOWN: executor 已用掉授權但沒有回報結果'))).toBe('reconcile')
  })
  it('被擋或對帳確認沒動 → 可重新授權', () => {
    expect(moneyActionsFor(money('EXEC_BLOCKED: prepare 失敗'))).toBe('reauthorize')
    expect(moneyActionsFor(money('RECONCILED_NOT_EXECUTED: 使用者人工對帳確認錢沒有動'))).toBe('reauthorize')
  })
  it('已執行、紅燈、非錢題、安全選項、別的通道 → 沒有動作', () => {
    expect(moneyActionsFor(money('COMMIT_OK: ok'))).toBeNull()
    expect(moneyActionsFor(money('RECONCILED_EXECUTED: 使用者人工對帳確認錢已經動了'))).toBeNull()
    expect(moneyActionsFor(money('EXEC_BLOCKED: x', { gate: 'red' }))).toBeNull()
    expect(moneyActionsFor(money('EXEC_BLOCKED: x', { money: undefined }))).toBeNull()
    const safe = money('EXEC_BLOCKED: x')
    safe.answer!.choice_id = 'b'
    expect(moneyActionsFor(safe)).toBeNull()
    const discord = money('EXEC_BLOCKED: x')
    discord.answer!.channel = 'discord'
    expect(moneyActionsFor(discord)).toBeNull()
  })
})

// 2026-10-09 錢 QA 第 11 輪 F2（AC-MA-20）：有伺服器算的 money_state 時一律依它，不看執行紀錄字串。
describe('moneyActionsFor 依伺服器的 money_state', () => {
  it('unknown → 人工對帳，即使執行紀錄是死信（8/6 型）', () => {
    expect(moneyActionsFor(money('EXEC_FAILED: 死信（重試 3 次仍失敗）', { money_state: 'unknown' }))).toBe('reconcile')
  })
  it('reauthorizable → 重新授權，即使執行紀錄沒有被擋的開頭（驗證前中斷）', () => {
    expect(moneyActionsFor(money('EXEC_BLOCKED: Kunsu 在 executor 驗證授權之前中斷，錢沒有動', { money_state: 'reauthorizable' }))).toBe(
      'reauthorize',
    )
    expect(moneyActionsFor(money(undefined, { money_state: 'reauthorizable' }))).toBe('reauthorize')
  })
  it('running／settled → 沒有動作，即使執行紀錄看起來可以', () => {
    expect(moneyActionsFor(money('EXEC_UNKNOWN: x', { money_state: 'running' }))).toBeNull()
    expect(moneyActionsFor(money('EXEC_BLOCKED: x', { money_state: 'running' }))).toBeNull()
    expect(moneyActionsFor(money('RECONCILED_NOT_EXECUTED: 舊版對帳', { money_state: 'settled' }))).toBeNull()
  })
})

describe('MoneyActions', () => {
  const ipc = {} as KunsuIpc
  it('結果不明時顯示兩個對帳按鈕與說明', () => {
    const html = renderToStaticMarkup(<MoneyActions ipc={ipc} question={money('EXEC_UNKNOWN: x')} />)
    expect(html).toContain('我核對過：錢已經動了')
    expect(html).toContain('我核對過：錢沒有動')
    expect(html).toContain('人工對帳')
  })
  it('可以重新授權時顯示重新授權按鈕', () => {
    const html = renderToStaticMarkup(<MoneyActions ipc={ipc} question={money('EXEC_BLOCKED: x')} />)
    expect(html).toContain('重新授權並執行')
  })
  it('沒有動作時什麼都不畫', () => {
    expect(renderToStaticMarkup(<MoneyActions ipc={ipc} question={money('COMMIT_OK: ok')} />)).toBe('')
  })
  it('依伺服器的 money_state 畫按鈕', () => {
    const html = renderToStaticMarkup(<MoneyActions ipc={ipc} question={money('EXEC_FAILED: 死信', { money_state: 'unknown' })} />)
    expect(html).toContain('我核對過：錢沒有動')
    expect(renderToStaticMarkup(<MoneyActions ipc={ipc} question={money('EXEC_BLOCKED: x', { money_state: 'settled' })} />)).toBe('')
  })
})
