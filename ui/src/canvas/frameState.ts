// 畫布載入狀態（AC-EC-09）：錯誤或 2 秒內沒畫完 → 隱藏並顯示原因；決定不受影響。
import type { CanvasIssue, CanvasMessage } from './messages'

export type FrameState =
  | { phase: 'loading'; height?: number }
  | { phase: 'ready'; height: number }
  | { phase: 'failed'; reason: string; issues: CanvasIssue[] }

export type FrameEvent = { type: 'message'; message: CanvasMessage } | { type: 'timeout' }

export const READY_TIMEOUT_MS = 2000
/** 註冊檢查與正式顯示同一個時限：檢查過了，使用者那邊也畫得出來。 */
export const CHECK_TIMEOUT_MS = READY_TIMEOUT_MS
export const initialFrame: FrameState = { phase: 'loading' }

export function frameReducer(state: FrameState, event: FrameEvent): FrameState {
  if (state.phase === 'failed') return state
  if (event.type === 'timeout') {
    return state.phase === 'loading'
      ? { phase: 'failed', reason: `${READY_TIMEOUT_MS / 1000} 秒內沒有畫完`, issues: [] }
      : state
  }
  const m = event.message
  switch (m.type) {
    case 'error':
      return { phase: 'failed', reason: m.issues[0]?.message || '畫布執行出錯', issues: m.issues }
    case 'ready':
      return { phase: 'ready', height: m.height }
    case 'resize':
      return state.phase === 'ready' ? { phase: 'ready', height: m.height } : { phase: 'loading', height: m.height }
  }
}

export function failureText(state: FrameState): string {
  if (state.phase !== 'failed') return ''
  const first = state.issues[0]
  const where = first?.line ? `（${first.file} 第 ${first.line} 行）` : ''
  return `圖表載入失敗：${state.reason}${where}`
}
