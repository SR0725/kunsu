// 回答延續原對話的回執文案與「打開原對話」連結（PRD §20、AC-CC-13）。
// 連結放行規則與 kunsu-core `is_conversation_link` 及 shell open_deep_link 一致。
import type { Question } from './types'

export interface Handoff {
  text: string
  done: boolean
  pulse: boolean
  tone: 'ok' | 'muted' | 'danger'
}

/** 依 server 的交付紀錄決定接手狀態；沒有紀錄回 null（沿用舊文案）。 */
export function handoffFromDelivery(q: Question): Handoff | null {
  const d = q.answer?.delivery
  if (!d) return null
  switch (d.state) {
    case 'live_waiter':
      return { text: '已交給等待中的原對話', done: true, pulse: false, tone: 'ok' }
    case 'resumed':
      if (d.reason === '已排入原對話，等待 AI 接手' && !q.answer?.report) return { text: d.reason, done: false, pulse: false, tone: 'muted' }
      return { text: '已在原對話追加處理', done: true, pulse: false, tone: 'ok' }
    case 'picked_up':
      return { text: '原對話已自行取走答案', done: true, pulse: false, tone: 'ok' }
    case 'held':
      return { text: '原對話正開著，等它空下來就接續', done: false, pulse: false, tone: 'muted' }
    case 'resuming':
      return { text: '正在原對話追加處理…', done: false, pulse: true, tone: 'muted' }
    case 'pending':
      return { text: '等待中的原對話將取走答案', done: false, pulse: true, tone: 'muted' }
    case 'fresh':
      return { text: d.reason ? `已開新對話處理（${d.reason}）` : '已開新對話處理', done: true, pulse: false, tone: 'ok' }
    case 'no_continuation':
      return { text: '沒有原對話可接續，答案已寫入決策帳本', done: true, pulse: false, tone: 'muted' }
    case 'dead':
      return { text: `交付失敗，已進死信${d.reason ? `（${d.reason}）` : ''}`, done: false, pulse: false, tone: 'danger' }
  }
}

/**
 * 回執還要不要等：AI 已回報，或交付不會再有回報（沒有原對話、交付失敗）才算定案。已交出去（追加、開新對話、
 * 交給等待者）但 AI 還沒回報時繼續等 AI 回饋。沒有交付紀錄的現行題不輪詢。
 */
export function deliverySettled(q: Question): boolean {
  if (q.answer?.report) return true
  const state = q.answer?.delivery?.state
  if (state === undefined) return q.wake_on_answer?.mode !== 'continue'
  return state === 'no_continuation' || state === 'dead'
}

/**
 * 決策欄回執還要不要輪詢（AC-CC-13、AC-MA-20）：交付定案，而且錢鏈列已結束（`money_state` 不是 running）——錢的結果定案後按鈕才會
 * 跟著出現。
 */
export function receiptSettled(q: Question): boolean {
  return deliverySettled(q) && q.money_state !== 'running'
}

/** 回執輪詢間隔：前 2 分鐘每 2 秒，之後每 10 秒，30 分鐘後停止（追加一輪可能跑很久）。 */
export function receiptPollDelay(elapsedMs: number): number | null {
  if (elapsedMs < 2 * 60_000) return 2000
  if (elapsedMs < 30 * 60_000) return 10_000
  return null
}

/**
 * 回執輪詢（AC-CC-13）：起算時間固定在開始輪詢的那一刻，新資料不重設；交付定案或 30 分鐘後停止。
 * 回傳 stop()。時鐘與計時器用全域的（測試以 fake timers 控制）。
 */
export function startReceiptPolling({
  fetch,
  onUpdate,
  firstDelayMs = 1200,
  settled = deliverySettled,
}: {
  fetch: () => Promise<Question | null | undefined>
  onUpdate: (q: Question) => void
  firstDelayMs?: number
  /** 什麼時候停（預設交付定案；決策欄另等錢鏈列結束：`receiptSettled`）。 */
  settled?: (q: Question) => boolean
}): () => void {
  const started = Date.now()
  let stopped = false
  let timer: ReturnType<typeof setTimeout> | undefined
  const next = () => {
    const delay = receiptPollDelay(Date.now() - started)
    if (!stopped && delay !== null) timer = setTimeout(tick, delay)
  }
  const tick = () => {
    if (stopped) return
    fetch().then(
      (updated) => {
        if (stopped) return
        if (!updated) return next()
        onUpdate(updated)
        if (!settled(updated)) next()
      },
      // 讀取失敗（app 忙、IPC 逾時）：照退避重試，不停止。
      () => next(),
    )
  }
  timer = setTimeout(tick, firstDelayMs)
  return () => {
    stopped = true
    if (timer !== undefined) clearTimeout(timer)
  }
}

const SAFE = /^[A-Za-z0-9-]{1,80}$/
const SAFE_THREAD = /^[A-Za-z0-9_-]{1,80}$/

export function isConversationLink(link: string): boolean {
  const claude = 'claude://code/continue?session=local_'
  const codex = 'codex://threads/'
  if (link.startsWith(claude)) return SAFE.test(link.slice(claude.length))
  if (link.startsWith(codex)) return SAFE_THREAD.test(link.slice(codex.length))
  return false
}

/** 打開原對話的 deep link：先用 server 寫入的連結，否則由來源身分推得。 */
export function conversationLink(q: Question): string | null {
  const fromServer = q.answer?.delivery?.conversation_link
  if (fromServer && isConversationLink(fromServer)) return fromServer
  const s = q.source
  if (s.host === 'claude_desktop' && s.host_session_id) {
    const link = `claude://code/continue?session=${s.host_session_id}`
    return isConversationLink(link) ? link : null
  }
  if ((s.host === 'codex_app' || s.host === 'codex_cli') && s.conversation_id) {
    const link = `codex://threads/${s.conversation_id}`
    return isConversationLink(link) ? link : null
  }
  return null
}

/** 錢那一行的圖示（AC-MA-20、F10）：已執行 ✓、乾跑或沒有執行 –、結果不明 !、還在跑 …——文字另外寫明，不只靠顏色。 */
export type MoneyIcon = '✓' | '–' | '!' | '…'

/**
 * 錢題回執的錢的結果一行（AC-CC-13／14、AC-MA-20）：依伺服器的 `money_state` 與執行紀錄開頭判斷；被擋走失敗畫面，交付失敗由交付
 * 狀態說明。錢鏈列還沒跑完時：這一輪的錢鏈結果已寫入（COMMIT_OK／EXEC_UNKNOWN／乾跑，只是交付還在跑）照結果說，否則（還沒有結果，
 * 或還是上一輪的被擋／對帳紀錄）寫「還在跑」。
 */
export function moneyOutcome(q: Question): { text: string; tone: 'ok' | 'muted'; icon: MoneyIcon } | null {
  if (!q.money?.is_money_op) return null
  const r = q.answer?.execution_result?.trimStart() ?? ''
  const thisRound = /^(COMMIT_OK|EXEC_UNKNOWN|DRY_RUN|APPROVAL_SIGNED)/.test(r)
  if (q.money_state === 'running' && !thisRound) return { text: '錢：錢鏈還在跑', tone: 'muted', icon: '…' }
  if (q.money_state === 'unknown' || r.startsWith('EXEC_UNKNOWN'))
    return { text: '錢：結果不明，可能已經動過，請人工對帳', tone: 'muted', icon: '!' }
  if (r.startsWith('COMMIT_OK')) return { text: '錢：已執行完成', tone: 'ok', icon: '✓' }
  if (r.startsWith('RECONCILED_EXECUTED')) return { text: '錢：已執行完成（你人工對帳確認）', tone: 'ok', icon: '✓' }
  if (r.startsWith('RECONCILED_NOT_EXECUTED')) return { text: '錢：沒有執行（你人工對帳確認）', tone: 'muted', icon: '–' }
  if (r.startsWith('DRY_RUN') || r.startsWith('APPROVAL_SIGNED')) return { text: '錢：乾跑，未實際執行', tone: 'muted', icon: '–' }
  return null
}
