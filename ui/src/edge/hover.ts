// 邊緣瀏海 hover 的隱私守門與逾時收回（PRD §19、BDD edge-notch 場景 11b／11c／12）。純函數，可測試。
import type { EdgeInfo } from '../ipc'

/**
 * 隱私守門狀態。`epoch` 每次 hover 開始加一；`verified` 是最後一次確認「未命中」的 epoch；`pushed` 是這次 hover
 * 是否已收到 shell 推送（推送比 hover 開始時送出的讀取新，收到推送後就不再採用讀取回應）。
 */
export interface PrivacyGate {
  epoch: number
  verified: number
  active: boolean
  pushed: boolean
}

/** 一開始什麼都不知道：當作隱私中。 */
export const initialPrivacy: PrivacyGate = { epoch: 0, verified: -1, active: true, pushed: false }

/** hover 開始：拿到這一次的最新狀態之前一律當作隱私中。 */
export function hoverBegan(g: PrivacyGate): PrivacyGate {
  return { epoch: g.epoch + 1, verified: g.verified, active: true, pushed: false }
}

/** hover 開始時送出的 `edge_privacy` 回來了。舊 hover 的回應、或這次已收到推送之後才回來的回應都不算。 */
export function privacyRead(g: PrivacyGate, epoch: number, active: boolean): PrivacyGate {
  if (epoch !== g.epoch || g.pushed) return g
  return { ...g, active, verified: active ? g.verified : g.epoch }
}

/** shell 主動推送（瀏海可見時每 500 ms 偵測、改變才送）：一律採用，命中立即生效。 */
export function privacyPushed(g: PrivacyGate, active: boolean): PrivacyGate {
  return { ...g, active, pushed: true, verified: active ? g.verified : g.epoch }
}

/** 這一次 hover 已確認未命中，才可以顯示題目內容。 */
export function contentAllowed(g: PrivacyGate): boolean {
  return !g.active && g.verified === g.epoch
}

/**
 * 隱私狀態改變後要不要重設文字淡入（場景 11c／12）：只有 hover 中（跑馬燈從狀態文字換成題目）且內容從「不可顯示」變成
 * 「可顯示」才淡入。沒有 hover 時畫的是靜止文字，內容沒變，不重新淡入（不閃一下）。
 */
export function privacyFadeIn(before: boolean, after: boolean, hovering: boolean): boolean {
  return hovering && !before && after
}

/** 多久沒有任何「游標在瀏海上」訊號就收回 hover（原生每 1 秒重送一次）。 */
export const HOVER_STALE = 3000

export function hoverStale(hovering: boolean, lastInside: number, now: number): boolean {
  return hovering && now - lastInside > HOVER_STALE
}

/**
 * 舞台識別：只有螢幕大小或螢幕本身改變才重新決定瀏海位置（場景 11c）；選單列高度與保存值都不算。螢幕本身以螢幕識別
 * （CGDirectDisplayID）區分，兩台同名螢幕不會被當成同一台；取不到識別才用名稱（場景 3d）。
 */
export function stageIdentity(info: Pick<EdgeInfo, 'width' | 'height' | 'display'> & Partial<EdgeInfo>): string {
  const display = info.display_id != null ? `#${info.display_id}` : info.display
  return `${info.width}x${info.height}@${display}`
}
