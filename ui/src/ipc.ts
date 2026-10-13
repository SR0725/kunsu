// IPC 抽象層（CONTRACT §9）：偵測 window.__TAURI__ → @tauri-apps/api invoke；
// 否則載入 mock adapter（src/mock.ts），pnpm dev 免 Tauri 可全功能開發。
// command 名照契約：list_questions / get_question / answer_question / dismiss_question /
// edit_option / open_deep_link / open_overlay_from_capsule / resize_capsule / kill_switch。
// answer_question 是唯一作答入口（簽章在 Rust 端產，UI 永不碰 HMAC key）。

import type { EvidenceBundle, ItemDecision, Question } from './types'
import type { RedlineResult } from './redline'
import type { DndStatus } from './dnd'
import type { DeferInfo } from './edge/geometry'
import { settingsTauriIpc, type SettingsIpc } from './app/settings/settingsIpc'

/** 作答載荷。reject = 「這題不該問我／退回重擬」（server 端 cancelled 語義＋品質計分）。 */
export type AnswerPayload =
  | { question_id: string; type: 'choice'; choice_id: string; edited_params?: string; note?: string; revision?: number; content_hash?: string }
  | { question_id: string; type: 'free_text'; free_text: string; revision?: number; content_hash?: string }
  | { question_id: string; type: 'reject'; reason?: string }
  /** 題組逐項決定（CONTRACT §14）；instruction＝整體指示（全部退回時就是那段指示）。 */
  | { question_id: string; type: 'batch'; items: ItemDecision[]; instruction?: string; revision?: number; content_hash?: string }

/** 答後執行流事件（prepare → guard → api → ledger）。 */
export type ExecStep = 'prepare' | 'guard' | 'api' | 'ledger'

export interface ExecEvent {
  question_id: string
  step: ExecStep
  status: 'start' | 'ok' | 'fail'
  /** 等寬字體串流行。 */
  line?: string
  /** 最後一步 ok 時附收據。 */
  receipt?: Receipt
}

export interface Receipt {
  /** 「✅ 已暫停 TW-3」等一行結論。 */
  summary: string
  decision_ref?: string
  prediction_ref?: string
  /** prediction verify 日期（MM-DD）。 */
  prediction_verify?: string
  discord_logged: boolean
}

export interface EditOptionResult {
  question: Question
  redline: RedlineResult
}

export interface CapsuleDragGeometry {
  /** Tauri 實體螢幕座標。 */
  x: number
  y: number
  /** CSS screenX/screenY → 實體座標倍率。 */
  scale_factor: number
}

/** 膠囊 WebView 內的 Orb 幾何；Rust 會換算為 overlay 的 screen-space 座標。 */
export interface CapsuleOrbAnchor {
  local_x: number
  local_y: number
  size: number
}

/** 每次 overlay 開啟都會收到新的 id，確保隱藏期間不會預播動畫。 */
export interface OverlayTransition {
  id: number
  origin_x: number
  origin_y: number
  origin_size: number
  source: 'capsule' | 'system' | 'preview'
}

export type CapsuleCorner = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right'
export type CapsuleDock =
  | CapsuleCorner
  | 'left-top'
  | 'left-bottom'
  | 'right-top'
  | 'right-bottom'

export function isCapsuleDock(value: string | null): value is CapsuleDock {
  return (
    value === 'top-left' ||
    value === 'top-right' ||
    value === 'bottom-left' ||
    value === 'bottom-right' ||
    value === 'left-top' ||
    value === 'left-bottom' ||
    value === 'right-top' ||
    value === 'right-bottom'
  )
}

/** 舊的 overlay bloom 仍只需要四象限；邊界落點在此投影到最近的角落方向。 */
export function capsuleDockCorner(dock: CapsuleDock): CapsuleCorner {
  if (dock === 'top-left' || dock === 'top-right' || dock === 'bottom-left' || dock === 'bottom-right') {
    return dock
  }
  if (dock === 'left-top') return 'top-left'
  if (dock === 'left-bottom') return 'bottom-left'
  if (dock === 'right-top') return 'top-right'
  return 'bottom-right'
}

export interface CapsuleDragController {
  move(screenX: number, screenY: number): void
  end(): Promise<CapsuleDock>
}

/** 邊緣瀏海舞台資訊（隱私狀態另外讀，不放在這裡快取）。 */
export interface EdgeInfo {
  width: number
  height: number
  /** 預設位置要避開的上方高度：選單列、實體瀏海與系統選單列標準高度三者最大。 */
  menubar: number
  /** 螢幕名稱（備援識別）。 */
  display: string
  /** 螢幕識別（CGDirectDisplayID）；取不到為 null。 */
  display_id?: number | null
  saved: unknown
}

/** 主視窗設定與狀態（settings_get／settings_set／settings_reset／app_status／edge_reset_position／open_privacy_settings）見
 * CONTRACT §15 與 app/settings/settingsIpc.ts。 */
export interface KunsuIpc extends SettingsIpc {
  listQuestions(): Promise<Question[]>
  getQuestion(id: string): Promise<Question | null>
  /** 邊緣瀏海舞台資訊（PRD §19）：螢幕大小、選單列高度、螢幕名稱與保存的位置。 */
  edgeInfo(): Promise<EdgeInfo>
  /** 回報瀏海命中區（畫出來的多邊形，扁平 [x0,y0,x1,y1,…]）與拖曳狀態；原生層據此切換點擊穿透。 */
  edgeHit(shapes: number[][], dragging: boolean): Promise<void>
  /** 回報瀏海來源圖示中心（舞台本地 CSS px）；沒有點擊座標時蓋板從這裡展開（AC-OP-02）。 */
  edgeAnchor(anchor: { x: number; y: number; size: number }): Promise<void>
  /** 保存瀏海位置（螢幕名稱＋螢幕識別＋位置；格式不合會被拒收）。 */
  edgeSave(display: string, displayId: number | null, position: unknown): Promise<void>
  /** 當下重新偵測的隱私 guard（分享螢幕、攝影機、麥克風、前景全螢幕任一）。 */
  edgePrivacy(): Promise<boolean>
  /** 到期被延後時擋住的原因（BDD edge-notch 場景 10c）。 */
  deferReasons(): Promise<DeferInfo>
  /** 原生層結束接管（滑鼠鍵已放開但網頁漏收 pointerup，或拖曳／按著時舞台被勿擾、⌘⇧K 隱藏）；網頁照沒畫出瀏海清理。 */
  subscribeEdgeDragEnd(cb: () => void): () => void
  /** shell 主動推送隱私 guard 改變（瀏海可見時每 500 ms 偵測）。 */
  subscribeEdgePrivacy(cb: (active: boolean) => void): () => void
  /** 原生層的游標進出命中區通知（在命中區內每 1 秒重送 inside）。 */
  subscribeEdgePointer(cb: (inside: boolean) => void): () => void
  /** 螢幕配置改變、舞台重新貼合後通知網頁重讀 edgeInfo（只重新貼合與重讀；不代表保存的位置被重設）。 */
  subscribeEdgeStage(cb: () => void): () => void
  /**
   * 設定頁「回到預設位置」：shell 刪掉保存檔後通知（`kunsu://edge_position_reset`，BDD edge-notch 場景 3e）。網頁取消待寫的
   * 存檔、清掉記憶體位置、回到預設位置。
   */
  subscribeEdgePositionReset(cb: () => void): () => void
  /** 畫布顯示失敗的診斷事件（BDD evidence-canvas 場景 9）。 */
  canvasFailure(questionId: string, revision: number, reason: string): Promise<void>
  /** 某題某版本的證據畫布與資產（PRD §24）；版本不存在回 null。 */
  getEvidence(questionId: string, revision: number): Promise<EvidenceBundle | null>
  /** 面板掛上／卸下畫布；畫布作用中回應原生心跳（看門狗，AC-EC-09b）。 */
  /** 回 false＝這一版已被停用（卡住過），不要執行。 */
  canvasActive(questionId: string, revision: number): Promise<boolean>
  canvasInactive(questionId: string, revision: number): Promise<void>
  subscribeCanvasPing(cb: () => void): () => void
  surfacePong(): Promise<void>
  /** 唯一作答入口；回寫後的完整題目。 */
  answerQuestion(payload: AnswerPayload): Promise<Question>
  /** 被擋或對帳確認沒動的黃燈錢題重新授權（重跑 prepare→比對→簽章→commit，答案不變）。 */
  reauthorizeQuestion(id: string): Promise<Question>
  /** 使用者人工對帳：結果不明的錢題標「錢已經動了／錢沒有動」（CONTRACT §7，只有 UI 通道）。 */
  resolveMoneyOutcome(questionId: string, executed: boolean): Promise<Question>
  dismissQuestion(id: string): Promise<Question>
  /** 鉛筆行內編輯：newTo = 編輯後目標值 → params_line 重算＋紅線重檢。 */
  editOption(id: string, optionId: string, newTo: number): Promise<EditOptionResult>
  openDeepLink(target: string): Promise<void>
  /** 以膠囊 Orb 的精確座標開啟蓋板並啟動 shared-particle handoff。 */
  openOverlayFromCapsule(anchor: CapsuleOrbAnchor): Promise<void>
  /** 膠囊視窗貼合內容（CSS px；只在 Tauri capsule 殼有效，web/mock no-op）。 */
  resizeCapsule(w: number, h: number): Promise<void>
  /** 建立手動拖曳 session；整顆膠囊超過位移門檻後由 pointer gesture 觸發。 */
  startCapsuleDrag(screenX: number, screenY: number): Promise<CapsuleDragController>
  /** 全域 kill-switch：4h 免打擾，落 fatigue event。 */
  killSwitch(): Promise<void>
  dndGet(): Promise<DndStatus>
  dndSet(durationMinutes: number, actor?: string): Promise<DndStatus>
  dndClear(actor?: string): Promise<DndStatus>
  /** 訂閱某題的執行流串流；回傳退訂函數。 */
  subscribeExecution(questionId: string, cb: (e: ExecEvent) => void): () => void
  /** 題目清單變更通知（膠囊/清單刷新用）。 */
  subscribeQuestions(cb: () => void): () => void
  /** 訂閱每次蓋板開啟的原生 transition 幾何。 */
  subscribeOverlayTransition(cb: (transition: OverlayTransition) => void): () => void
  subscribeOpenSettings(cb: () => void): () => void
  /** 主視窗：蓋板就緒逾時後備，打開作答流程（AC-OP-05）。 */
  subscribeOpenFlow(cb: () => void): () => void
  /** 蓋板網頁就緒：回傳目前待播的轉場（主動拉取，避免事件遺失；AC-OP-04）。 */
  overlayReady(): Promise<OverlayTransition | null>
  /** 該轉場的題目、輸入與關閉控制都已渲染（AC-OP-04）。 */
  overlayMounted(id: number): Promise<void>
  /** 網頁已接手 Esc，解除原生看門狗（AC-OP-06）。 */
  overlayEscapeAck(): Promise<void>
  /** 蓋板目前顯示的題目；原生 Esc 後備的 dismissal 也要算它（AC-OP-06、AC-EC-09b）。 */
  overlayCurrent(questionId: string | null): Promise<void>
  /** true = mock adapter（開發模式 banner 用）。 */
  readonly isMock: boolean
}

declare global {
  interface Window {
    __TAURI__?: unknown
    __TAURI_INTERNALS__?: unknown
  }
}

export function isTauri(): boolean {
  return (
    typeof window !== 'undefined' &&
    (window.__TAURI__ !== undefined || window.__TAURI_INTERNALS__ !== undefined)
  )
}

export function capsuleDragTarget(
  origin: CapsuleDragGeometry,
  startScreenX: number,
  startScreenY: number,
  screenX: number,
  screenY: number,
): { x: number; y: number } {
  return {
    x: Math.round(origin.x + (screenX - startScreenX) * origin.scale_factor),
    y: Math.round(origin.y + (screenY - startScreenY) * origin.scale_factor),
  }
}

function makeTauriIpc(): KunsuIpc {
  // 動態載入：web 開發（mock）路徑不需要 @tauri-apps/api runtime。
  const core = import('@tauri-apps/api/core')
  const event = import('@tauri-apps/api/event')
  const invoke = async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    const { invoke } = await core
    return invoke<T>(cmd, args)
  }
  return {
    isMock: false,
    ...settingsTauriIpc(invoke),
    listQuestions: () => invoke('list_questions'),
    getQuestion: (id) => invoke('get_question', { id }),
    getEvidence: (questionId, revision) => invoke('get_evidence', { question_id: questionId, revision }),
    canvasFailure: (questionId, revision, reason) => invoke('canvas_failure', { question_id: questionId, revision, reason }),
    canvasActive: (questionId, revision) => invoke('canvas_active', { question_id: questionId, revision }),
    canvasInactive: (questionId, revision) => invoke('canvas_inactive', { question_id: questionId, revision }),
    surfacePong: () => invoke('surface_pong'),
    subscribeCanvasPing: (cb) => listenVoid(event, 'kunsu://canvas_ping', cb),
    edgeInfo: () => invoke('edge_info'),
    edgeHit: (shapes, dragging) => invoke('edge_hit', { shapes, dragging }),
    edgeAnchor: (anchor) => invoke('edge_anchor', { x: anchor.x, y: anchor.y, size: anchor.size }),
    edgeSave: (display, displayId, position) => invoke('edge_save', { display, display_id: displayId, position }),
    edgePrivacy: () => invoke('edge_privacy'),
    deferReasons: () => invoke('defer_reasons'),
    subscribeEdgeDragEnd: (cb) => listenVoid(event, 'kunsu://edge_drag_end', cb),
    subscribeEdgePrivacy: (cb) => listenPayload<{ active: boolean }>(event, 'kunsu://edge_privacy', (p) => cb(p.active)),
    subscribeEdgePointer: (cb) => listenPayload<{ inside: boolean }>(event, 'kunsu://edge_pointer', (p) => cb(p.inside)),
    subscribeEdgeStage: (cb) => listenVoid(event, 'kunsu://edge_stage_changed', cb),
    subscribeEdgePositionReset: (cb) => listenVoid(event, 'kunsu://edge_position_reset', cb),
    answerQuestion: (payload) => invoke('answer_question', { payload }),
    reauthorizeQuestion: (id) => invoke('reauthorize_question', { id }),
    resolveMoneyOutcome: (questionId, executed) => invoke('resolve_money_outcome', { question_id: questionId, executed }),
    dismissQuestion: (id) => invoke('dismiss_question', { id }),
    editOption: (id, optionId, newTo) =>
      invoke('edit_option', { id, option_id: optionId, new_to: newTo }),
    openDeepLink: (target) => invoke('open_deep_link', { target }),
    openOverlayFromCapsule: (anchor) => invoke('open_overlay_from_capsule', { anchor }),
    resizeCapsule: (w, h) => invoke('resize_capsule', { w, h }),
    startCapsuleDrag: async (startScreenX, startScreenY) => {
      const origin = await invoke<CapsuleDragGeometry>('capsule_drag_geometry')
      const moves = new Set<Promise<void>>()
      let ended = false
      return {
        move(screenX, screenY) {
          if (ended) return
          const target = capsuleDragTarget(origin, startScreenX, startScreenY, screenX, screenY)
          const request = invoke<void>('move_capsule', target)
          moves.add(request)
          void request.finally(() => moves.delete(request)).catch((error) => {
            console.error('膠囊視窗移動失敗', error)
          })
        },
        async end() {
          if (ended) {
            const stored = localStorage.getItem('kunsu:capsule-dock:v2')
            return isCapsuleDock(stored) ? stored : 'top-left'
          }
          ended = true
          await Promise.all([...moves])
          const reducedMotion =
            document.documentElement.dataset.motion === 'reduced' ||
            window.matchMedia('(prefers-reduced-motion: reduce)').matches
          const dock = await invoke<CapsuleDock>('snap_capsule', {
            reduced_motion: reducedMotion,
          })
          localStorage.setItem('kunsu:capsule-dock:v2', dock)
          localStorage.setItem('kunsu:capsule-corner:v1', capsuleDockCorner(dock))
          return dock
        },
      }
    },
    killSwitch: () => invoke('kill_switch'),
    dndGet: () => invoke('dnd_get'),
    dndSet: (durationMinutes, actor = 'ui') =>
      invoke('dnd_set', { duration_minutes: durationMinutes, actor }),
    dndClear: (actor = 'ui') => invoke('dnd_clear', { actor }),
    subscribeExecution: (questionId, cb) => {
      let unlisten: (() => void) | null = null
      let dead = false
      void event.then(({ listen }) =>
        listen<ExecEvent>('kunsu://execution', (ev) => {
          if (ev.payload.question_id === questionId) cb(ev.payload)
        }).then((un) => {
          if (dead) un()
          else unlisten = un
        }),
      )
      return () => {
        dead = true
        unlisten?.()
      }
    },
    subscribeQuestions: (cb) => {
      let unlisten: (() => void) | null = null
      let dead = false
      void event.then(({ listen }) =>
        listen('kunsu://questions_changed', () => cb()).then((un) => {
          if (dead) un()
          else unlisten = un
        }),
      )
      return () => {
        dead = true
        unlisten?.()
      }
    },
    subscribeOverlayTransition: (cb) => {
      let unlisten: (() => void) | null = null
      let dead = false
      void event.then(({ listen }) =>
        listen<OverlayTransition>('kunsu://overlay_transition', (ev) => cb(ev.payload)).then((un) => {
          if (dead) un()
          else unlisten = un
        }),
      )
      return () => {
        dead = true
        unlisten?.()
      }
    },
    subscribeOpenSettings: (cb) => listenVoid(event, 'kunsu://open_settings', cb),
    subscribeOpenFlow: (cb) => listenVoid(event, 'kunsu://open_flow', cb),
    overlayReady: () => invoke<OverlayTransition | null>('overlay_ready'),
    overlayMounted: (id) => invoke<void>('overlay_mounted', { id }),
    overlayEscapeAck: () => invoke<void>('overlay_escape_ack'),
    overlayCurrent: (questionId) => invoke<void>('overlay_current', { question_id: questionId }),
  }
}

function listenVoid(
  event: Promise<typeof import('@tauri-apps/api/event')>,
  name: string,
  cb: () => void,
): () => void {
  let unlisten: (() => void) | null = null
  let dead = false
  void event.then(({ listen }) =>
    listen(name, () => cb()).then((un) => {
      if (dead) un()
      else unlisten = un
    }),
  )
  return () => {
    dead = true
    unlisten?.()
  }
}

function listenPayload<T>(
  event: Promise<typeof import('@tauri-apps/api/event')>,
  name: string,
  cb: (payload: T) => void,
): () => void {
  let unlisten: (() => void) | null = null
  let dead = false
  void event.then(({ listen }) =>
    listen<T>(name, (e) => cb(e.payload)).then((un) => {
      if (dead) un()
      else unlisten = un
    }),
  )
  return () => {
    dead = true
    unlisten?.()
  }
}

let cached: KunsuIpc | null = null

/** 取得 IPC adapter（單例）。 */
export async function getIpc(): Promise<KunsuIpc> {
  if (cached) return cached
  if (isTauri()) {
    cached = makeTauriIpc()
  } else {
    const { makeMockIpc } = await import('./mock')
    cached = makeMockIpc()
  }
  return cached
}
