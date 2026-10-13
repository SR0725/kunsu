// 邊緣瀏海顯示的內容（PRD §19、BDD edge-notch 場景 1／10／10d／10e／11、INV-24）。純函數，可測試。
import type { Question } from '../types'
import { isTerminal } from '../types'
import { agentDisplayName } from '../agentMarks'
import { cycleTimes, forceCycleSnapshot } from '../forceCycle'
import { deferReasonsSpoken, restText, type DeferInfo, type HitShape, type RestState } from './geometry'

/** 瀏海的佇列：只算非 internal、非終態題（doctor 測試題等 internal 題不算題數、不進跑馬燈）；保留原本的強制佇列順序。 */
export function notchQueue(questions: Question[]): Question[] {
  return questions.filter((q) => !q.internal && !isTerminal(q.state))
}

/**
 * 最早一題的週期狀態 → 靜止文字的狀態（場景 10／10c／10d）。最早一題已在強制詢問（forced：蓋板或主視窗後備正在顯示它）
 * 是 asking，顯示一般待處理文字；到期但還沒詢問（warned）才是 deferred（「時間到了…」）。`defer`：undefined＝還沒讀到
 * 延後原因，null＝讀取失敗。`warnMs`＝最早一題的 force_at − warn_at（倒數分鐘補位用）。
 */
export function notchRestState(questions: Question[], nowMs: number, defer?: DeferInfo | null): RestState {
  const queue = notchQueue(questions)
  const head = queue[0]
  const snap = head ? forceCycleSnapshot(head, nowMs) : null
  const kind: RestState['kind'] =
    head?.state === 'forced' ? 'asking' : snap?.overdue ? 'deferred' : snap?.warning ? 'warning' : 'pending'
  // 倒數分鐘依這一題的警告長度補位（場景 10、FR-03）：提前警告可以在設定頁改成 10 分鐘以上。
  const times = head ? cycleTimes(head) : null
  const warnMs = times ? times.forceAt - times.warnAt : undefined
  return { kind, count: queue.length, remainingMs: snap?.remainingMs ?? 0, defer, warnMs }
}

/**
 * 實際顯示的靜止狀態（場景 10c）：剛進入到期、還沒讀到延後原因（`defer` 為 undefined）時維持前一個顯示的狀態（剛載入沒有
 * 前一個就用一般待處理文字），不先閃「時間到了，馬上跳出來」；讀到原因才換，讀取失敗（null）才說「馬上跳出來」。
 */
export function shownRestState(state: RestState, previous: RestState | null): RestState {
  if (state.kind !== 'deferred' || state.defer !== undefined) return state
  return previous ?? { kind: 'pending', count: state.count, remainingMs: state.remainingMs }
}

/** 無障礙名稱（場景 20）：「Kunsu：你有 2 則待處理通知，最早一題 2 分 41 秒後強制詢問（來自 Claude）」。 */
export function notchLabel(questions: Question[], state: RestState): string {
  const head = notchQueue(questions)[0]
  if (!head) return 'Kunsu'
  const secs = Math.ceil(state.remainingMs / 1000)
  const spoken = deferReasonsSpoken(state.defer)
  const when =
    state.kind === 'asking'
      ? '最早一題正在詢問'
      : state.kind === 'deferred'
        ? `最早一題已到期，${restText(state).replace(/^時間到了，/, '')}${spoken ? `（${spoken}）` : ''}`
        : `最早一題 ${Math.floor(secs / 60)} 分 ${secs % 60} 秒後強制詢問`
  const count = restText({ kind: 'pending', count: state.count, remainingMs: 0 })
  return `Kunsu：${count}，${when}（來自 ${agentDisplayName(head.source.agent)}）`
}

/** 沒有狀態變化時多久重算一次狀態文字（場景 19）。 */
export const CLOCK_IDLE_MS = 15_000
/** 排在 warn_at／force_at 的重算多等一點點，確保時間點已過（計時器可能早到 1 ms）。 */
export const CLOCK_MARGIN_MS = 20

/**
 * 下一次重算狀態文字的延遲（場景 10e）：15 秒與最早一題下一個 warn_at／force_at 兩者較早者；warning 期間對齊秒數
 * （m:ss 在剩餘時間跨過整秒時改變）。文字只依最早一題決定，所以只看它。
 */
export function notchClockDelay(questions: Question[], nowMs: number): number {
  const head = notchQueue(questions)[0]
  if (!head) return CLOCK_IDLE_MS
  const state = notchRestState(questions, nowMs)
  if (state.kind === 'warning') return (state.remainingMs % 1000 || 1000) + CLOCK_MARGIN_MS
  let next = CLOCK_IDLE_MS
  const times = cycleTimes(head)
  for (const at of times ? [times.warnAt, times.forceAt] : []) {
    if (at > nowMs) next = Math.min(next, at - nowMs + CLOCK_MARGIN_MS)
  }
  return next
}

/**
 * 回報給原生層的命中區（INV-24）：只有真的畫出瀏海（佇列有題、幾何已算好）才回報畫出來的多邊形；沒畫就回報空命中區且
 * 不接管，整個舞台穿透。
 */
export function notchHitReport(
  questions: Question[],
  hit: HitShape[] | null,
  dragging: boolean,
): { shapes: HitShape[]; dragging: boolean } {
  if (!hit || notchQueue(questions).length === 0) return { shapes: [], dragging: false }
  return { shapes: hit, dragging }
}

/**
 * 瀏海沒畫出來時要清掉的網頁狀態（INV-24）：拖曳中所有題目在別處結案，SVG 已卸載、收不到 pointerup。進行中的手勢當作放開
 * 結束（照場景 7 吸附）、hover 收回、不再拖曳；不清掉的話之後瀏海再出現會回報「拖曳中」，原生層可能接管別的 app 的拖曳。
 *
 * `nativeEnded`：原生層已結束接管並發 `kunsu://edge_drag_end`（滑鼠鍵已放開但網頁漏收 pointerup，或拖曳／按著時舞台被
 * 勿擾、⌘⇧K 隱藏）。網頁還有手勢或還在拖曳時，瀏海在網頁這邊還畫著也一樣清理（QA 第 4 輪）；只是按著還沒拖的手勢以取消
 * 結束，不算點擊。網頁已自己收到 pointerup（沒有手勢、不拖曳）才收到的通知不動任何狀態，放開後開始的 hover 照常。
 */
export function notchHiddenCleanup(
  drawn: boolean,
  s: { gesture: boolean; dragging: boolean; hover: boolean },
  nativeEnded = false,
): { finishGesture: boolean; endHover: boolean; dragging: boolean } {
  const ended = nativeEnded && (s.gesture || s.dragging)
  if (drawn && !ended) return { finishGesture: false, endHover: false, dragging: s.dragging }
  return { finishGesture: s.gesture, endHover: s.hover, dragging: false }
}

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', '#39': "'" }

/** CommonMark 自動連結：`<scheme:…>`（scheme 2–32 字）與 `<信箱>`。 */
const URI_AUTOLINK = /<([A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*)>/g
const EMAIL_AUTOLINK =
  /<([A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*)>/g
/** 真正的 HTML 標籤：`<` 後面緊接英文字母（開始標籤）或 `</`＋字母（結束標籤）。 */
const HTML_TAG = /<\/?[A-Za-z][A-Za-z0-9-]*(?:\s[^<>]*)?\/?>/g

/**
 * 跑馬燈用的純文字（場景 11）：`question_line` 可能含 Markdown 與安全 HTML，瀏海逐字排在路徑上，不能顯示符號。
 * 只處理一行摘要會用到的語法：強調、刪除線、行內程式碼、連結、自動連結、圖片、HTML 標籤／註解與實體、標題／清單／引用前綴、
 * 跳脫字元。只去掉真正的 HTML 標籤（`<` 後接英文字母、`</`、`<!`）；不是標籤的 `<`、`>` 是題目內容，照原文
 * （`CPA<200 且 ROAS>3`）；自動連結顯示網址；行內程式碼裡的字照原文。
 */
export function plainLine(md: string): string {
  // 跳脫字元、行內程式碼與自動連結的內容先換成私用區佔位符，最後才還原成字面文字，不受後面的規則影響。
  const kept: string[] = []
  const keep = (text: string) => {
    kept.push(text)
    return `\uE000${kept.length - 1}\uE001`
  }
  let s = md.replace(/\\([\\`*_{}[\]()#+\-.!~>|<])/g, (_m, ch: string) => keep(ch))
  s = s
    .split(/\r?\n/)
    .map((line) => line.replace(/^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/, ''))
    .join(' ')
  s = s
    .replace(/`([^`]*)`/g, (_m, code: string) => keep(code))
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(URI_AUTOLINK, (_m, url: string) => keep(url))
    .replace(EMAIL_AUTOLINK, (_m, mail: string) => keep(mail))
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<![^<>]*>/g, '')
    .replace(HTML_TAG, '')
    .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, '$2')
    .replace(/~~(?=\S)([\s\S]*?\S)~~/g, '$1')
    // 單一 * 或 _ 的強調：兩側不能緊貼英數（2*3*4、snake_case 保持原樣），中文旁邊可以。
    .replace(/(^|[^\w*])\*(?=[^\s*])([^*]*?[^\s*])\*(?![\w*])/g, '$1$2')
    .replace(/(^|\W)_(?=[^\s_])([^_]*?[^\s_])_(?!\w)/g, '$1$2')
    .replace(/&(#39|amp|lt|gt|quot|apos|nbsp);/g, (_m, name: string) => ENTITIES[name])
  s = s.replace(/\uE000(\d+)\uE001/g, (_m, i: string) => kept[Number(i)])
  return s.replace(/\s+/g, ' ').trim()
}
