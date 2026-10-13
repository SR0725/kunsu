// 勿擾（DESIGN「Do Not Disturb」；BDD do-not-disturb、settings 場景 8）：沿用 ipc.dndGet／dndSet／dndClear 的既有語意
// （最長 24 小時、題目與週期保存），換成新外觀。頁首的工具列按鈕與設定頁的卡片共用同一份內容，不是兩套實作。
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import type { KunsuIpc } from '../ipc'
import type { DndStatus } from '../dnd'
import { dndLabel, normalizeDndMinutes } from '../dnd'
import { Button, Icon, NumberField, segmentKey } from './ui'

const DND_PRESETS = [60, 240, 480, 1440] as const
const presetLabel = (minutes: number) => `${minutes / 60} 小時`

/** `acted`：這次動作成功換了狀態才留著 true（呼叫端據此把焦點移到新按鈕）；失敗就清掉。 */
function useDndActions(ipc: KunsuIpc, onStatus: (status: DndStatus) => void, acted?: { current: boolean }) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const run = async (action: () => Promise<DndStatus>) => {
    setError('')
    setBusy(true)
    try {
      const next = await action()
      if (acted) acted.current = true
      onStatus(next)
      return true
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
      return false
    } finally {
      setBusy(false)
    }
  }
  return {
    error,
    busy,
    apply: (minutes: number) => run(() => ipc.dndSet(minutes, 'ui')),
    clear: () => run(() => ipc.dndClear('ui')),
  }
}

/** 勿擾內容：未啟用＝四個快捷時長＋自訂分鐘；啟用中＝狀態、保留說明與「提前結束」。 */
export function DndOptions({
  ipc,
  status,
  onStatus,
  idPrefix,
  onDone,
  autoFocus = false,
  statusLine = false,
}: {
  ipc: KunsuIpc
  status: DndStatus
  onStatus: (status: DndStatus) => void
  idPrefix: string
  onDone?: () => void
  /** 打開時焦點落在第一個快捷時長（啟用中則是「提前結束」）。 */
  autoFocus?: boolean
  /** 沒開時也先寫出目前狀態（設定頁的勿擾卡片，AC-ST-08）：「目前沒有開勿擾」。 */
  statusLine?: boolean
}) {
  // 自己按了快捷時長或提前結束、內容換成另一組按鈕時，焦點移到新按鈕（AC-ST-15）；工具列的彈出視窗會自己收起，不用移。
  const acted = useRef(false)
  const { error, busy, apply, clear } = useDndActions(ipc, onStatus, acted)
  const [custom, setCustom] = useState('')
  const customMinutes = normalizeDndMinutes(custom)
  const presetRefs = useRef<(HTMLButtonElement | null)[]>([])
  const endRef = useRef<HTMLButtonElement | null>(null)
  useEffect(() => {
    if (!autoFocus) return
    ;(status.active ? endRef.current : presetRefs.current[0])?.focus()
  }, [autoFocus, status.active])
  // 每次重繪後看一次（動作成功後的那一次重繪，新按鈕已經畫上）。
  useEffect(() => {
    if (!acted.current) return
    acted.current = false
    if (onDone) return
    ;(status.active ? endRef.current : presetRefs.current[0])?.focus()
  })
  const onPresetKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = presetRefs.current.findIndex((el) => el === document.activeElement)
    if (index < 0) return
    const next = segmentKey(index, DND_PRESETS.length, event.key)
    if (next === null) return
    event.preventDefault()
    presetRefs.current[next]?.focus()
  }

  if (status.active) {
    return (
      <div className="k-dnd">
        <p className="k-dnd__status tabular">
          <Icon name="bell-off" size={16} />
          {dndLabel(status)}
        </p>
        <p className="k-dnd__note">待決事項與倒數都會保留，結束後從原進度恢復；期間不顯示通知、瀏海或強制詢問。</p>
        <div className="k-dnd__row">
          <Button
            variant="secondary"
            ref={endRef}
            aria-disabled={busy || undefined}
            onClick={busy ? undefined : () => void clear().then((ok) => ok && onDone?.())}
          >
            提前結束
          </Button>
        </div>
        {error && (
          <p role="alert" className="k-field__error">
            {error}
          </p>
        )}
      </div>
    )
  }

  return (
    <div className="k-dnd">
      {statusLine && (
        <p className="k-dnd__status" data-active="false">
          <Icon name="bell" size={16} />
          目前沒有開勿擾
        </p>
      )}
      <div className="k-dnd__presets" role="group" aria-label="快捷時長" onKeyDown={onPresetKey}>
        {DND_PRESETS.map((minutes, i) => (
          <Button
            key={minutes}
            variant="secondary"
            aria-disabled={busy || undefined}
            ref={(el: HTMLButtonElement | null) => {
              presetRefs.current[i] = el
            }}
            onClick={busy ? undefined : () => void apply(minutes).then((ok) => ok && onDone?.())}
          >
            {presetLabel(minutes)}
          </Button>
        ))}
      </div>
      <form
        className="k-dnd__custom"
        onSubmit={(event) => {
          event.preventDefault()
          if (customMinutes !== null) void apply(customMinutes).then((ok) => ok && onDone?.())
        }}
      >
        <NumberField
          id={`${idPrefix}-custom`}
          label="自訂時長"
          unit="分鐘"
          value={custom}
          onChange={setCustom}
          hint="1–1440 分鐘（最長 24 小時）"
          error={custom.trim() !== '' && customMinutes === null ? '請輸入 1–1440 之間的整數分鐘' : undefined}
        />
        <Button type="submit" variant="primary" disabled={customMinutes === null || busy}>
          啟用勿擾
        </Button>
      </form>
      {error && (
        <p role="alert" className="k-field__error">
          {error}
        </p>
      )}
    </div>
  )
}

/** 頁首右端的勿擾按鈕：未啟用寫「勿擾模式」；啟用中變成狀態小標（勿擾至 18:30 · 剩 2 小時 14 分）。 */
export function DndToolbarButton({ ipc, status, onStatus }: { ipc: KunsuIpc; status: DndStatus; onStatus: (status: DndStatus) => void }) {
  const [open, setOpen] = useState(false)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node
      if (!panelRef.current?.contains(target) && !buttonRef.current?.contains(target)) setOpen(false)
    }
    window.addEventListener('pointerdown', onPointer)
    return () => window.removeEventListener('pointerdown', onPointer)
  }, [open])

  const close = () => {
    setOpen(false)
    buttonRef.current?.focus()
  }

  return (
    <div className="k-popover-anchor">
      <button
        ref={buttonRef}
        type="button"
        className="k-btn k-dnd-button"
        data-variant="secondary"
        data-size="md"
        data-active={status.active ? 'true' : 'false'}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="bell-off" size={16} />
        <span className="tabular">{dndLabel(status)}</span>
      </button>
      {open && (
        <div
          ref={panelRef}
          className="k-popover"
          role="dialog"
          aria-label="勿擾模式設定"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault()
              event.stopPropagation()
              close()
            }
          }}
        >
          <p className="k-popover__title">{status.active ? '勿擾中' : '暫停所有 Kunsu 提醒'}</p>
          {!status.active && <p className="k-popover__desc">不顯示通知、瀏海或強制詢問；題目與倒數都保留。</p>}
          <DndOptions ipc={ipc} status={status} onStatus={onStatus} idPrefix="toolbar-dnd" onDone={close} autoFocus />
        </div>
      )}
    </div>
  )
}
