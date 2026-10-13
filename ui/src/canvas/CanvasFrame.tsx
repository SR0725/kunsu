// 證據畫布（PRD §24、DESIGN「Evidence canvas」、AC-EC-06/07/09）：沙盒 iframe，只收白名單訊息；
// 2 秒內沒畫完或出錯 → 隱藏並顯示原因，決策介面完全不受影響。
import { createContext, useContext, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { CANVAS_SANDBOX, buildSrcdoc, newToken } from './srcdoc'
import { frameMessage, layoutIssues } from './messages'
import { failureOnce } from './hardening'
import { READY_TIMEOUT_MS, failureText, frameReducer, initialFrame, type FrameState } from './frameState'
import { loadCanvasRuntime } from './useEvidence'
import {
  askPermission,
  canvasFailure,
  currentPermission,
  permissionKey,
  refusalText,
  runtimeErrorText,
  shouldRegister,
  type HeldPermission,
  type RuntimeLoad,
} from './permission'
import type { EvidenceBundle } from '../types'

/** 註冊檢查頁用：同一個面板改成檢查模式（回報版面問題、等久一點）並把結果交出去。 */
export interface CanvasCheckMode {
  timeoutMs: number
  onSettled: (state: FrameState) => void
}
export const CanvasCheckContext = createContext<CanvasCheckMode | null>(null)

export function CanvasFrame({
  question,
  bundle,
  mode: modeProp,
  timeoutMs: timeoutProp,
  onSettled: onSettledProp,
  onFailure,
  ipc,
}: {
  question: Question
  bundle: EvidenceBundle
  /** 正式顯示時向原生殼登記畫布並回應心跳（看門狗，AC-EC-09b）。 */
  ipc?: KunsuIpc
  mode?: 'display' | 'check'
  timeoutMs?: number
  onSettled?: (state: FrameState) => void
  /** 正式顯示失敗時（記診斷事件）。 */
  onFailure?: (reason: string) => void
}) {
  const check = useContext(CanvasCheckContext)
  const mode = modeProp ?? (check ? 'check' : 'display')
  const timeoutMs = timeoutProp ?? check?.timeoutMs ?? READY_TIMEOUT_MS
  const onSettled = onSettledProp ?? check?.onSettled
  const canvas = bundle.canvas
  const frameRef = useRef<HTMLIFrameElement>(null)
  const [runtime, setRuntime] = useState<string | null>(null)
  const [runtimeError, setRuntimeError] = useState<string | null>(null)
  const [state, dispatch] = useReducer(frameReducer, initialFrame)
  const revision = question.draft?.revision ?? 0
  // 每一次渲染（題目＋版本＋內容）一個 token：內容變了就換，舊文件被換掉時送出的 canvas_navigation（帶舊 token）不會被
  // 當成這一次的畫布換頁。
  const renderKey = JSON.stringify([question.id, revision, question.draft?.content_hash ?? ''])
  const token = useMemo(() => {
    void renderKey
    return newToken()
  }, [renderKey])

  useEffect(() => {
    let alive = true
    loadCanvasRuntime()
      .then((code) => alive && setRuntime(code))
      .catch((e: unknown) => alive && setRuntimeError(runtimeErrorText(e)))
    return () => {
      alive = false
    }
  }, [])

  // 執行畫布前先問殼層：這一版若已被看門狗停用（卡住過），任何視窗都不再執行（AC-EC-09b）。許可以「題目＋版本」為準：
  // 換版後在新版拿到許可前不執行；問不到（IPC 失敗）視同不許可（AC-EC-09）。runtime 載入成功、真的要執行畫布時才問——
  // 殼層一答應就開始心跳，runtime 讀不到時面板永遠不回，會被誤判卡住（QA 第 5 輪）。
  const needsPermission = mode === 'display' && Boolean(ipc)
  const runtimeLoad: RuntimeLoad = runtimeError !== null ? 'failed' : runtime !== null ? 'ready' : 'loading'
  const [held, setHeld] = useState<HeldPermission | null>(null)
  const permission = currentPermission(needsPermission, held, permissionKey(question.id, revision))
  useEffect(() => {
    if (!shouldRegister(needsPermission, Boolean(canvas), runtimeLoad) || !ipc) return
    const id = question.id
    const key = permissionKey(id, revision)
    let alive = true
    void askPermission(() => ipc.canvasActive(id, revision)).then((value) => {
      if (alive) setHeld({ key, value })
    })
    return () => {
      alive = false
      void ipc.canvasInactive(id, revision).catch(() => {})
    }
  }, [needsPermission, ipc, canvas, question.id, revision, runtimeLoad])

  const srcdoc = useMemo(
    () =>
      canvas && runtime && permission === 'granted'
        ? buildSrcdoc({
            runtime,
            kind: canvas.kind,
            source: canvas.source,
            assets: bundle.assets,
            token,
          })
        : null,
    [canvas, runtime, bundle.assets, token, permission],
  )

  // 只收這個 iframe、這次 token 的 resize／ready／error。檢查時 ready 的版面數據轉成版面問題。
  useEffect(() => {
    if (!srcdoc) return
    const file = `canvas.${canvas?.kind ?? 'tsx'}`
    const onMessage = (event: MessageEvent) => {
      const message = frameMessage(event.source, frameRef.current?.contentWindow ?? null, event.data, token)
      if (!message) return
      const issues = mode === 'check' ? layoutIssues(message, file) : []
      dispatch({
        type: 'message',
        message: issues.length > 0 ? { type: 'error', issues } : message,
      })
    }
    window.addEventListener('message', onMessage)
    const timer = window.setTimeout(() => dispatch({ type: 'timeout' }), timeoutMs)
    return () => {
      window.removeEventListener('message', onMessage)
      window.clearTimeout(timer)
    }
  }, [srcdoc, token, timeoutMs, mode, canvas?.kind])

  // 看門狗（AC-EC-09b）：正式顯示期間回應原生心跳；卡住時殼層停用這一版、終止卡住的網頁行程並重新載入。
  useEffect(() => {
    if (!srcdoc || mode !== 'display' || !ipc) return
    return ipc.subscribeCanvasPing(() => void ipc.surfacePong().catch(() => {}))
  }, [srcdoc, mode, ipc])

  // 焦點不留在畫布裡：Esc、Tab 與快捷鍵永遠回到 Kunsu 原生介面（畫布只負責顯示）。
  useEffect(() => {
    const onBlur = () => {
      window.setTimeout(() => {
        // 只處理「焦點進了這個畫布」：拉回決定欄的第一個可操作元件（沒有就回面板本身）。
        if (document.activeElement !== frameRef.current) return
        frameRef.current?.blur()
        const dialog = frameRef.current?.closest<HTMLElement>('[role="dialog"]')
        const target = dialog?.querySelector<HTMLElement>('[role="radio"], textarea') ?? dialog
        target?.focus({ preventScroll: true })
      }, 0)
    }
    window.addEventListener('blur', onBlur)
    return () => window.removeEventListener('blur', onBlur)
  }, [])

  // canvas_failed 每個題目／版本／這次顯示只記一次；runtime 本身載入失敗也記（AC-EC-09）。
  const onFailureRef = useRef(onFailure)
  useEffect(() => {
    onFailureRef.current = onFailure
  })
  const reportFailure = useMemo(() => failureOnce((reason) => onFailureRef.current?.(reason)), [])
  const failureKey = `${question.id}@${revision}@${token}`
  useEffect(() => {
    if (state.phase !== 'loading') onSettled?.(state)
    if (mode !== 'display') return
    if (state.phase === 'failed') reportFailure(failureKey, failureText(state))
    else if (runtimeError) reportFailure(failureKey, `圖表載入失敗：${runtimeError}`)
    // 停用的版本由殼層的看門狗記過了；問不到殼層才在這裡記。
    else if (permission === 'unavailable') reportFailure(failureKey, refusalText('unavailable'))
  }, [state, onSettled, mode, runtimeError, reportFailure, failureKey, permission])

  useEffect(() => {
    if (runtimeError) console.error('畫布 runtime 載入失敗', runtimeError)
  }, [runtimeError])

  if (!canvas) return null
  const failure = canvasFailure(state, runtimeError, permission)
  const failed = failure !== null
  const height = state.phase === 'ready' ? state.height : state.phase === 'loading' && state.height ? state.height : 160
  return (
    <section
      className="surface-card surface-canvas"
      aria-label="AI 提供的資料：圖表"
      data-phase={failed ? 'failed' : state.phase}
      data-mode={mode}
    >
      <div className="surface-canvas__caption">
        <span className="surface-canvas__dot" aria-hidden="true" />
        AI 提供的資料
      </div>
      {failed ? (
        <p className="surface-canvas__failed" role="status">
          {failure}
        </p>
      ) : (
        <div className="surface-canvas__viewport" style={{ height }}>
          {srcdoc && (
            <iframe
              ref={frameRef}
              className="surface-canvas__frame"
              title="AI 提供的資料：圖表"
              sandbox={CANVAS_SANDBOX}
              srcDoc={srcdoc}
              tabIndex={-1}
              data-ready={state.phase === 'ready' ? 'true' : 'false'}
              style={{ height }}
            />
          )}
        </div>
      )}
    </section>
  )
}
