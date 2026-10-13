// QA 錢的安全第 11 輪（BDD money-approval AC-MA-17／18／19／20、conversation-continuity AC-CC-13／14）：回執的人工對帳與重新授權。
// 測試環境沒有 DOM：靜態渲染（renderToStaticMarkup）、純函數與原始碼結構。需要真瀏覽器的行為（state 更新、焦點）以暫用 harness
// 在 vite（port 1594）實測：真的 <DecisionSurface>，假 IPC 照伺服器的事件順序（reactor.rs money_chain／blocked：prepare start →
// prepare ok → api start → api fail → 才寫執行紀錄 → questions_changed）推進，`latest` 餵最新的題目。實測結果記在各測試的註解。
// 標「QA11-紅」的測試在目前 main 會失敗，是要修的缺口；其餘是本輪確認成立的行為的回歸保護。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { DecisionColumn } from './DecisionColumn'
import { moneyActionsFor } from './MoneyActions'
import moneyActionsSource from './MoneyActions.tsx?raw'

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

/**
 * shell `to_ui`（src-tauri/src/ipc.rs）送給網頁的已答黃燈錢題：執行紀錄以 EXEC_BLOCKED:／EXEC_FAILED: 開頭時附 execution_blocked。
 * 2026-10-09 主線決定 F2：shell 另附伺服器算的 `money_state`（`moneyState` 參數；不給＝舊 shell，網頁退回看執行紀錄字串）。
 */
function answeredMoney(result: string | undefined, moneyState?: Question['money_state']): Question {
  const q = {
    id: 'pa-qa11',
    title: 'iOS 實驗室加碼',
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
  } as unknown as Question
  const failure = result?.match(/^(?:EXEC_BLOCKED|EXEC_FAILED):\s*(.*)$/s)
  if (failure) (q as { execution_blocked?: string }).execution_blocked = failure[1]
  if (moneyState) q.money_state = moneyState
  return q
}

const column = (question: Question) =>
  renderToStaticMarkup(
    <DecisionColumn ipc={ipc} question={question} withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
  )
const text = (html: string) => html.replace(/<[^>]*>/g, '')

describe('AC-MA-18 結果不明的錢題，使用者在回執有「錢已經動了／錢沒有動」', () => {
  it('回歸：回看執行紀錄以 EXEC_UNKNOWN 開頭的錢題，回執有兩個對帳鈕與說明', () => {
    browser()
    const html = text(column(answeredMoney('EXEC_UNKNOWN: executor 已用掉授權但 commit 沒有回報結果')))
    expect(html).toContain('我核對過：錢已經動了')
    expect(html).toContain('我核對過：錢沒有動')
    expect(html).not.toContain('執行沒有完成')
  })

  // 8/6 那類舊題（pa-ios-cbo-scale-6000）：approval 已被 executor 消耗、沒有錢鏈結果、交付進死信，執行紀錄是 `EXEC_FAILED: 死信…`。
  // 伺服器以 nonce 判斷「結果不明」（agent 讀到結果不明、重新授權被拒、重建列人工對帳；server 測試
  // `qa11_dead_lettered_consumed_approval_gives_ray_the_reconcile_buttons`），但 shell 只送執行紀錄，網頁只看 `EXEC_UNKNOWN` 開頭：
  // 回執進「執行沒有完成」畫面，寫「修好原因後可以在這裡重新授權」，卻沒有重新授權鈕（伺服器也會拒絕）、沒有對帳鈕。這題永遠解除不了。
  // 主線決定 F2：伺服器的判斷經 shell 的 `money_state` 送到網頁（這題伺服器算出 unknown），網頁依它給按鈕。
  it('QA11-紅：8/6 型（EXEC_FAILED 死信、伺服器判定結果不明）回看時有對帳鈕，不寫「可以在這裡重新授權」', () => {
    browser()
    const q = answeredMoney('EXEC_FAILED: 死信（重試 3 次仍失敗）：spawn 失敗', 'unknown')
    const html = text(column(q))
    const offered = html.includes('我核對過：錢沒有動') || html.includes('重新授權並執行')
    expect(
      offered,
      `畫面：${html.slice(0, 160)}…（moneyActionsFor=${moneyActionsFor(q)}）——寫著可以重新授權，卻沒有任何按鈕`,
    ).toBe(true)
    // 主線決定 F2：結果不明的失敗畫面標題是「結果不明」，請使用者先核對再對帳；不寫可以重新授權。
    expect(html).toContain('結果不明')
    expect(html).toContain('請先到廣告後台核對再對帳')
    expect(html).not.toContain('修好原因後可以在這裡重新授權')
  })
})

describe('AC-MA-19 被擋或對帳確認沒動的黃燈錢題，回執有「重新授權並執行」', () => {
  it('回歸：回看 EXEC_BLOCKED 的錢題，失敗畫面有重新授權鈕', () => {
    browser()
    const html = text(column(answeredMoney('EXEC_BLOCKED: executor commit 失敗：非零退出（exit status: 1）')))
    expect(html).toContain('執行沒有完成')
    expect(html).toContain('重新授權並執行')
  })

  it('回歸：回看對帳確認沒動的錢題，回執有重新授權鈕；已經動了沒有', () => {
    browser()
    expect(text(column(answeredMoney('RECONCILED_NOT_EXECUTED: 使用者人工對帳確認錢沒有動')))).toContain('重新授權並執行')
    const done = text(column(answeredMoney('RECONCILED_EXECUTED: 使用者人工對帳確認錢已經動了')))
    expect(done).not.toContain('重新授權並執行')
    expect(done).not.toContain('我核對過')
  })
})

describe('AC-MA-18／19／20、AC-CC-13 即時回執：錢鏈跑完之後按鈕要出現', () => {
  // `MoneyActions` 用 `useState(question)` 把掛載當下的題目凍住，之後 `question` prop 怎麼變都不看（回執輪詢、`latest` 清單更新都
  // 會換 prop）。作答當下錢鏈還沒跑完（執行紀錄空的），所以：
  // (1) 即時回執：瀏覽器實測（harness，continue 題、交付 pending，2.5 秒後題目被重啟恢復改成 EXEC_UNKNOWN）——同一張回執上
  //     「錢：結果不明，可能已經動過，請人工對帳」出現了，對帳鈕沒有出現；
  // (2) 即時失敗畫面：實測（使用者按「批准並執行」→ api fail 事件 → 題目更新成 EXEC_BLOCKED）——畫面寫「修好原因後可以在這裡
  //     重新授權」，沒有「重新授權並執行」；同一份資料從歷史紀錄重開就有。
  // 只有重新打開這題才看得到按鈕。
  it('QA11-紅：MoneyActions 依最新的 question prop 決定按鈕（不能只看掛載當下）', () => {
    const frozen = /useState\(\s*question\s*\)/.test(moneyActionsSource)
    const synced = /useEffect\([\s\S]*?\[\s*question\s*\]\s*\)/.test(moneyActionsSource)
    expect(!frozen || synced, 'MoneyActions 以 useState(question) 凍住掛載當下的題目，沒有跟著 prop 更新').toBe(true)
  })

  // 即時失敗畫面的另一半：伺服器在「executor 已用掉授權、commit 失敗」時先送 api fail 事件（reactor.rs money_chain 在判斷 nonce 是否
  // 已消耗之前就 emit Fail），網頁收到任何 fail 事件就切到「執行沒有完成…修好原因後可以在這裡重新授權」。瀏覽器實測（harness，
  // 結果是 EXEC_UNKNOWN）：使用者看到的是「執行沒有完成」，沒有「錢可能已經動過」，也沒有對帳鈕——他可能以為沒動、自己去廣告後台
  // 再做一次。此項只能在瀏覽器重現（vitest 沒有 DOM，事件驅動的 phase 切換無法靜態渲染），列在 QA 報告。
})

describe('鍵盤（CLAUDE.md §6 UI QA、DESIGN「Keyboard and focus」）', () => {
  // 瀏覽器實測（harness，回看 EXEC_BLOCKED）：焦點在「重新授權並執行」按 Enter → 出現「確定重新授權？…確定／取消」，焦點掉到
  // <body>；再按一次 Enter（以為是確認）→ 面板的 Enter＝「完成」，面板收起，重新授權沒有送出。「我核對過：…」→「確定」同樣。
  // 確認之後（成功或失敗）焦點也掉到 <body>。
  it('QA11-紅：打開「再確認一次」時焦點移到確認區（不掉到 body）', () => {
    expect(/autoFocus|\.focus\(/.test(moneyActionsSource), 'MoneyActions 沒有任何焦點管理').toBe(true)
  })
})

describe('AC-CC-13 錢的結果一行不只靠顏色', () => {
  // 回執每一行的圖示都是 ✓：「錢：結果不明，可能已經動過，請人工對帳」「錢：沒有執行（你人工對帳確認）」也是 ✓，只靠 data-tone
  // 區分（CLAUDE.md §6：狀態不能只靠顏色表達）。
  it('QA11-紅：結果不明的那一行不是打勾', () => {
    browser()
    const html = column(answeredMoney('EXEC_UNKNOWN: x'))
    const row = html.match(/<div class="surface-rc"[^>]*>\s*<span class="surface-rc__ic">([^<]*)<\/span>\s*<span>錢：結果不明/)
    expect(row, '找不到錢的那一行').not.toBeNull()
    expect(row![1]).not.toBe('✓')
  })
})
