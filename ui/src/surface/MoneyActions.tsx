// 錢題回執的後續動作（CONTRACT §7、§9；BDD money-approval AC-MA-18／19／20；DESIGN「Money follow-up in the receipt」）：
// 結果不明 → 使用者核對後標「錢已經動了／錢沒有動」；被擋或對帳確認沒動 → 重新授權並執行。兩個都要再確認一次，只走 UI 通道（主視窗與
// 蓋板）。按鈕一律依伺服器算的 `money_state`，跟著最新的題目（prop 換了就以新的為準）；本地只保留「剛送出」的暫態。
import { useEffect, useRef, useState } from 'react'
import type { MouseEvent, RefObject } from 'react'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { moneyActionsFor } from './decision'

export { moneyActionsFor }
export type { MoneyAction } from './decision'

type Kind = 'executed' | 'not_executed' | 'reauthorize'

/** 剛送出的暫態：送出那一刻的題目（prop）、伺服器回的新題目與結果訊息。 */
interface Sent {
  id: string
  kind: Kind
  from: Question
  next: Question
  text: string
}

const DONE_TEXT: Record<Kind, string> = {
  executed: '已記錄：錢已經動了。這題算已執行完成，不會再重新授權。',
  not_executed: '已記錄：錢沒有動。需要的話可以重新授權並執行。',
  reauthorize: '已重新授權，Kunsu 會重跑一次批准與執行。',
}

const CONFIRM_TEXT: Record<Kind, string> = {
  executed: '確定錢已經動了？之後這題不能再重新授權。',
  not_executed: '確定錢沒有動？之後可以重新授權，再跑一次批准與執行。',
  reauthorize: '確定重新授權？Kunsu 會重新 prepare、比對參數並執行這筆錢。',
}

export function MoneyActions({
  ipc,
  question,
  onUpdated,
  focusAfter,
  explain = true,
}: {
  ipc: KunsuIpc
  question: Question
  /** 送出成功後把伺服器回的新題目交回決策欄（prop 隨之更新，按鈕依新的 `money_state`）。 */
  onUpdated?: (question: Question) => void
  /** 送出成功後焦點移到哪裡（回執或失敗畫面的「完成／下一題」）。 */
  focusAfter?: RefObject<HTMLButtonElement | null>
  /** 結果不明時在按鈕上方說明為什麼要對帳（失敗畫面自己已經寫了就關掉）。 */
  explain?: boolean
}) {
  const [pending, setPending] = useState<Kind | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [sent, setSent] = useState<Sent | null>(null)
  const confirmRef = useRef<HTMLButtonElement>(null)
  const openerRef = useRef<HTMLButtonElement | null>(null)
  const restoreFocus = useRef(false)

  // 打開「再確認一次」時焦點移到「確定」；取消時回到打開它的那顆按鈕（F9）。
  useEffect(() => {
    if (pending) {
      confirmRef.current?.focus({ preventScroll: true })
    } else if (restoreFocus.current) {
      restoreFocus.current = false
      openerRef.current?.focus({ preventScroll: true })
    }
  }, [pending])

  // prop 還是送出那一刻的題目時，以伺服器回的新題目為準；prop 換了（決策欄已更新、回執輪詢、清單更新）就以 prop 為準。
  const current = sent && sent.from === question ? sent.next : question
  // 舊 shell（沒有 money_state）：剛重新授權、執行紀錄還沒變之前不再給重新授權鈕（錢鏈還在跑）。
  const staleFallback =
    sent?.kind === 'reauthorize' &&
    sent.id === current.id &&
    !current.money_state &&
    (current.answer?.execution_result ?? '') === (sent.next.answer?.execution_result ?? '')
  const action = staleFallback ? null : moneyActionsFor(current)
  const done = sent && sent.id === current.id ? sent.text : ''
  // 確認區只在狀態仍允許時出現（題目在別處變了就收起）。
  const open = pending && action === (pending === 'reauthorize' ? 'reauthorize' : 'reconcile') ? pending : null
  if (!action && !done) return null

  const ask = (kind: Kind) => (event: MouseEvent<HTMLButtonElement>) => {
    openerRef.current = event.currentTarget
    setError('')
    setPending(kind)
  }

  const cancel = () => {
    restoreFocus.current = true
    setPending(null)
  }

  const run = async (kind: Kind) => {
    setBusy(true)
    setError('')
    try {
      const next =
        kind === 'reauthorize'
          ? await ipc.reauthorizeQuestion(current.id)
          : await ipc.resolveMoneyOutcome(current.id, kind === 'executed')
      setSent({ id: current.id, kind, from: question, next, text: DONE_TEXT[kind] })
      setPending(null)
      onUpdated?.(next)
      // 送出成功後焦點移到回執的「完成／下一題」（下一個畫面更新之後）。
      window.setTimeout(() => focusAfter?.current?.focus({ preventScroll: true }), 0)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
      // 「確定」送出中停用會掉焦點：失敗後回到「確定」，可以直接再試或取消。
      window.setTimeout(() => confirmRef.current?.focus({ preventScroll: true }), 0)
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="surface-money-actions" aria-label="錢的後續處理">
      {/* 再確認時原按鈕只藏起來不卸載：取消後焦點回得去同一顆按鈕（F9）。 */}
      {action === 'reconcile' && (
        <>
          {explain && (
            <p hidden={Boolean(open)}>
              executor 已經用掉授權，卻沒有回報結果，錢可能已經動過。請到廣告後台或 executor 紀錄核對後，在這裡做人工對帳。
            </p>
          )}
          <div className="surface-money-actions__row" hidden={Boolean(open)}>
            <button type="button" className="surface-money-btn" disabled={busy} onClick={ask('executed')}>
              我核對過：錢已經動了
            </button>
            <button type="button" className="surface-money-btn" disabled={busy} onClick={ask('not_executed')}>
              我核對過：錢沒有動
            </button>
          </div>
        </>
      )}
      {action === 'reauthorize' && (
        <div className="surface-money-actions__row" hidden={Boolean(open)}>
          <button type="button" className="surface-money-btn" disabled={busy} onClick={ask('reauthorize')}>
            重新授權並執行
          </button>
        </div>
      )}
      {open && (
        <div className="surface-money-actions__confirm" role="group" aria-label="再確認一次">
          <p>{CONFIRM_TEXT[open]}</p>
          <div className="surface-money-actions__row">
            <button
              ref={confirmRef}
              type="button"
              className="surface-money-btn surface-money-btn--primary"
              disabled={busy}
              onClick={() => void run(open)}
            >
              確定
            </button>
            <button type="button" className="surface-money-btn" disabled={busy} onClick={cancel}>
              取消
            </button>
          </div>
        </div>
      )}
      {done && (
        <p className="surface-money-actions__done" role="status">
          {done}
        </p>
      )}
      {error && (
        <p className="surface-money-actions__error" role="alert">
          {error}
        </p>
      )}
    </section>
  )
}
