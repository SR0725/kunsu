// 執行畫布前的殼層許可（BDD evidence-canvas 場景 9、9b；AC-EC-09、09b）。殼層可能已停用這一版（卡住過）；
// 許可只對拿到它的那一題那一版有效，換版要重新問；問不到（IPC 失敗）就當作不許可。
import { failureText, type FrameState } from './frameState'

export type PermissionValue = 'granted' | 'refused' | 'unavailable'
export type Permission = 'pending' | PermissionValue

export interface HeldPermission {
  key: string
  value: PermissionValue
}

export function permissionKey(questionId: string, revision: number): string {
  return JSON.stringify([questionId, revision])
}

/** 目前這一題這一版的許可：不需要問殼層（檢查模式、沒有 IPC）就直接可以；拿到的許可不是這一版的就還在等。 */
export function currentPermission(needs: boolean, held: HeldPermission | null, key: string): Permission {
  if (!needs) return 'granted'
  return held !== null && held.key === key ? held.value : 'pending'
}

/** 問殼層：只有明確回 true 才可以執行；IPC 失敗（含同步丟出）＝無法確認，不執行。 */
export async function askPermission(ask: () => Promise<boolean>): Promise<PermissionValue> {
  try {
    return (await ask()) === true ? 'granted' : 'refused'
  } catch {
    return 'unavailable'
  }
}

export function refusalText(value: 'refused' | 'unavailable'): string {
  return value === 'refused' ? '圖表載入失敗：畫布沒有回應' : '圖表載入失敗：無法確認畫布可以執行'
}

/** 沙盒 runtime 的載入狀態。 */
export type RuntimeLoad = 'loading' | 'ready' | 'failed'

/**
 * 什麼時候向殼層登記（`canvas_active`）：要問殼層、有畫布，而且 runtime 已經載入成功、真的要執行畫布。殼層一答應就開始
 * 每秒心跳，面板要等 runtime 載入、執行畫布後才會回應；runtime 還在讀或讀不到時就登記，3 秒後會被誤判卡住（終止網頁行程、
 * 重新載入、停用這一版，QA 第 5 輪）。
 */
export function shouldRegister(needs: boolean, hasCanvas: boolean, runtime: RuntimeLoad): boolean {
  return needs && hasCanvas && runtime === 'ready'
}

/** 畫布區要顯示的失敗原因（沒有失敗＝null）：殼層拒絕或問不到 → runtime 讀不到 → 畫布本身的錯誤。 */
export function canvasFailure(state: FrameState, runtimeError: string | null, permission: Permission): string | null {
  if (permission === 'refused' || permission === 'unavailable') return refusalText(permission)
  if (runtimeError !== null) return `圖表載入失敗：${runtimeError}`
  return state.phase === 'failed' ? failureText(state) : null
}

/** runtime 讀取失敗的原因：只取錯誤訊息（`String(error)` 會多一個「Error:」前綴）。 */
export function runtimeErrorText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error)
  return text.trim() === '' ? '畫布 runtime 讀取失敗' : text
}
