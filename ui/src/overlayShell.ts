// 蓋板網頁的呈現決策（docs/bdd/overlay-presentation.md AC-OP-04／07／09）。
// 原生殼只有在收到「已掛載」確認後才把蓋板帶到前景，所以這裡必須保證：
// 沒有轉場就不渲染、清單還沒載入就不自動關閉、在面板裡答完的回執留到使用者收起、每個轉場只確認一次。

export type OverlayShellAction = 'wait' | 'render' | 'close'

/** 「沒有題目、沒有回執要保留」到真的收起之間的緩衝（AC-OP-07）；期間面板照樣掛著，回報回執或作答中就取消收起。 */
export const OVERLAY_CLOSE_GRACE_MS = 300

export function overlayShellAction(input: {
  loaded: boolean
  questionCount: number
  hasTransition: boolean
  /** 面板正保留這次 session 在面板裡作答的回執（AC-OP-07、AC-DS-09）：直到使用者按完成或 Esc 才收起。 */
  holdingReceipt?: boolean
  /**
   * 面板有作答正在送出（送出到結果回來）：server 先發 questions_changed 再回傳結果，清單會比結果先變空——收起判斷不能早於
   * 這次作答的結果（2026-10-09 QA 第 3 輪：舊版這一刻就卸載面板，回執一閃即逝）。
   */
  answering?: boolean
}): OverlayShellAction {
  if (!input.hasTransition) return 'wait'
  if (input.questionCount > 0 || input.holdingReceipt || input.answering) return 'render'
  return input.loaded ? 'close' : 'wait'
}

/** 面板要不要掛著：收起緩衝期間（`close`）也掛著，面板才回報得了「正在顯示回執」；只有等待時不掛。 */
export function overlayShellMounted(action: OverlayShellAction): boolean {
  return action !== 'wait'
}

export function shouldAckMount(input: {
  transitionId: number | null
  ackedId: number | null
  questionCount: number
}): boolean {
  return input.transitionId !== null && input.questionCount > 0 && input.ackedId !== input.transitionId
}
