// 邊緣瀏海的內容（BDD edge-notch AC-EN-01、10、11；QA 第 1 輪）。
import { describe, expect, it } from 'vitest'
import type { Question } from '../types'
import {
  CLOCK_IDLE_MS,
  CLOCK_MARGIN_MS,
  notchClockDelay,
  notchHiddenCleanup,
  notchHitReport,
  notchLabel,
  notchQueue,
  notchRestState,
  plainLine,
  shownRestState,
} from './content'
import { restText } from './geometry'

function q(id: string, extra: Partial<Question> = {}): Question {
  return {
    id,
    title: id,
    question_line: `${id} 的題目`,
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [{ id: 'a', label: 'A' }],
    allow_free_text: false,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'none' },
    source: { agent: 'claude', cwd: '/w' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-09',
    last_seen: '2026-10-09T00:00:00Z',
    ...extra,
  } as Question
}

describe('AC-EN-01／10 瀏海只算非 internal、非終態題', () => {
  it('internal 題（doctor 測試題）不算進題數也不進跑馬燈，順序不變', () => {
    const list = [q('a'), q('doctor', { internal: true }), q('b', { state: 'warned' }), q('done', { state: 'answered' }), q('c')]
    expect(notchQueue(list).map((x) => x.id)).toEqual(['a', 'b', 'c'])
    expect(notchQueue([q('doctor', { internal: true })])).toEqual([])
  })
})

describe('AC-EN-11 跑馬燈顯示純文字', () => {
  const cases: [string, string][] = [
    ['**要你決定：**用一句話', '要你決定：用一句話'],
    ['__粗體__與*斜體*與_底線斜體_', '粗體與斜體與底線斜體'],
    ['~~刪掉~~保留', '刪掉保留'],
    ['`ask_create` 失敗要不要重試？', 'ask_create 失敗要不要重試？'],
    ['看 [這份報告](https://example.com/r?a=1) 再決定', '看 這份報告 再決定'],
    ['![圖表](https://example.com/c.png)說明', '圖表說明'],
    ['<b>粗</b> &amp; <i>細</i> &lt;3', '粗 & 細 <3'],
    ['# 標題', '標題'],
    ['- 第一項', '第一項'],
    ['1. 第一步', '第一步'],
    ['> 引用', '引用'],
    ['第一行\n第二行', '第一行 第二行'],
    ['snake_case_name 與 2*3*4 不是強調', 'snake_case_name 與 2*3*4 不是強調'],
    ['\\*字面星號\\*', '*字面星號*'],
    ['  前後空白  ', '前後空白'],
    // QA 第 2 輪：只去掉真正的 HTML 標籤
    ['`<div>` 要不要留', '<div> 要不要留'],
    ['寄到 <alex@example.com> 嗎', '寄到 alex@example.com 嗎'],
    ['前<!-- 註解 -->後', '前後'],
    ['a < b 且 c > d', 'a < b 且 c > d'],
    ['<br/>換行與 </p>結尾', '換行與 結尾'],
  ]
  for (const [md, plain] of cases) {
    it(`${JSON.stringify(md)} → ${JSON.stringify(plain)}`, () => {
      expect(plainLine(md)).toBe(plain)
    })
  }
})

// ---------------------------------------------------------------- QA 第 2 輪（2026-10-09）

const NOW = Date.parse('2026-10-09T10:00:00Z')
const at = (ms: number) => new Date(NOW + ms).toISOString()
/** 一題 force_at 在 forceIn 毫秒後（負數＝已到期），warn_at 在它前 3 分鐘。 */
function timed(id: string, forceIn: number, extra: Partial<Question> = {}): Question {
  return q(id, {
    created_at: at(forceIn - 15 * 60_000),
    cycle_started_at: at(forceIn - 15 * 60_000),
    warn_at: at(forceIn - 3 * 60_000),
    force_at: at(forceIn),
    ...extra,
  })
}

describe('AC-EN-10 已在詢問時不說「時間到了」', () => {
  it('最早一題正在強制詢問（forced）時顯示一般待處理文字；到期還沒詢問（warned）才說時間到了', () => {
    const asking = [timed('a', -5_000, { state: 'forced' }), timed('b', 600_000)]
    const s1 = notchRestState(asking, NOW)
    expect(s1.kind).toBe('asking')
    expect(restText(s1)).toBe('你有 2 則待處理通知')
    expect(notchLabel(asking, s1)).toBe('Kunsu：你有 2 則待處理通知，最早一題正在詢問（來自 Claude）')
    // 時鐘稍有落差（還沒到 force_at 就已 forced）也算正在詢問
    expect(notchRestState([timed('a', 2_000, { state: 'forced' })], NOW).kind).toBe('asking')

    const deferred = [timed('a', -5_000, { state: 'warned' })]
    expect(notchRestState(deferred, NOW).kind).toBe('deferred')
    expect(restText(notchRestState(deferred, NOW, { reasons: ['camera_on'] }))).toBe('時間到了，鏡頭關掉就跳出來')
    expect(notchLabel(deferred, notchRestState(deferred, NOW, { reasons: ['camera_on'] }))).toContain('最早一題已到期')

    expect(notchRestState([timed('a', 120_000, { state: 'warned' })], NOW).kind).toBe('warning')
    expect(notchRestState([timed('a', 600_000)], NOW).kind).toBe('pending')
    expect(restText({ kind: 'asking', count: 1, remainingMs: 0 })).toBe('你有一則待處理通知')
  })
})

describe('AC-EN-10／19 狀態文字準時', () => {
  it('下一次重算排在 15 秒與最早一題 warn_at／force_at 兩者較早者；warning 期間對齊秒數', () => {
    expect(CLOCK_IDLE_MS).toBe(15_000)
    // pending：warn_at 在 4 秒後 → 4 秒後（加一點餘裕）就重算，不等 15 秒
    expect(notchClockDelay([timed('a', 3 * 60_000 + 4_000)], NOW)).toBe(4_000 + CLOCK_MARGIN_MS)
    // pending：都還很遠 → 15 秒
    expect(notchClockDelay([timed('a', 10 * 60_000)], NOW)).toBe(CLOCK_IDLE_MS)
    // warning：顯示 2:42（剩 161.4 秒）→ 0.4 秒後變 2:41；剛好整秒 → 1 秒後
    expect(notchClockDelay([timed('a', 161_400, { state: 'warned' })], NOW)).toBe(400 + CLOCK_MARGIN_MS)
    expect(notchClockDelay([timed('a', 161_000, { state: 'warned' })], NOW)).toBe(1_000 + CLOCK_MARGIN_MS)
    // warning 最後不到一秒：force_at 一到就重算（進入到期）
    expect(notchClockDelay([timed('a', 300, { state: 'warned' })], NOW)).toBe(300 + CLOCK_MARGIN_MS)
    // 已到期、沒題目：沒有下一個時間點 → 15 秒
    expect(notchClockDelay([timed('a', -5_000, { state: 'warned' })], NOW)).toBe(CLOCK_IDLE_MS)
    expect(notchClockDelay([], NOW)).toBe(CLOCK_IDLE_MS)
    // 只看最早一題（文字只依它決定）
    expect(notchClockDelay([timed('a', 10 * 60_000), timed('b', 11 * 60_000)], NOW)).toBe(CLOCK_IDLE_MS)
  })
})

describe('inv_edge_notch_click_through', () => {
  it('沒有待決題、只剩 internal 或終態題、幾何還沒算好時不畫，回報空命中區且不接管', () => {
    const hit = [[0, 0, 32, 0, 32, 200, 0, 200]]
    const none = { shapes: [], dragging: false }
    expect(notchHitReport([], hit, true)).toEqual(none)
    expect(notchHitReport([q('doctor', { internal: true })], hit, true)).toEqual(none)
    expect(notchHitReport([q('done', { state: 'answered' }), q('gone', { state: 'cancelled' })], hit, false)).toEqual(none)
    expect(notchHitReport([q('a')], null, true)).toEqual(none)
    // 有畫出來：回報的就是畫出來的那組多邊形
    expect(notchHitReport([q('a')], hit, true)).toEqual({ shapes: hit, dragging: true })
    expect(notchHitReport([q('doctor', { internal: true }), q('a')], hit, false)).toEqual({ shapes: hit, dragging: false })
  })

  it('拖曳中瀏海不再畫出：清掉網頁拖曳與 hover 狀態，之後再出現也不回報拖曳中（QA 第 3 輪）', () => {
    const hit = [[0, 0, 32, 0, 32, 200, 0, 200]]
    const busy = { gesture: true, dragging: true, hover: true }
    // 瀏海還畫著：什麼都不清
    expect(notchHiddenCleanup(true, busy)).toEqual({ finishGesture: false, endHover: false, dragging: true })
    // 拖曳中所有題目在別處結案（不畫）：手勢當作放開結束、hover 收回、不再拖曳；這一刻回報空命中區、不拖曳
    const after = notchHiddenCleanup(false, busy)
    expect(after).toEqual({ finishGesture: true, endHover: true, dragging: false })
    expect(notchHitReport([], hit, busy.dragging)).toEqual({ shapes: [], dragging: false })
    // 之後新題目讓瀏海再出現：回報的是畫出來的形狀，但不是拖曳中（原生層不會接管別的 app 的拖曳）
    expect(notchHitReport([q('new')], hit, after.dragging)).toEqual({ shapes: hit, dragging: false })
    // 沒有進行中的手勢或 hover：不必做事
    expect(notchHiddenCleanup(false, { gesture: false, dragging: false, hover: false })).toEqual({
      finishGesture: false,
      endHover: false,
      dragging: false,
    })
  })

  it('拖曳或按著時舞台被隱藏：原生發 edge_drag_end，網頁照沒畫出瀏海清理，之後再出現也不回報拖曳中（QA 第 4 輪）', () => {
    const hit = [[0, 0, 32, 0, 32, 200, 0, 200]]
    // 勿擾、⌘⇧K 讓舞台在拖曳中隱藏：網頁那邊瀏海還算畫著（題目還在），但原生層已解除接管並發 edge_drag_end。
    const dragging = { gesture: true, dragging: true, hover: false }
    expect(notchHiddenCleanup(true, dragging, true)).toEqual({ finishGesture: true, endHover: false, dragging: false })
    // 之後瀏海再出現、幾何改變（倒數、題數）：回報不拖曳，原生層不會重新接管
    expect(notchHitReport([q('a')], hit, false)).toEqual({ shapes: hit, dragging: false })
    // 只是按著還沒拖（原生按下鎖定）：手勢結束（以取消結束，不算點擊）；hover 中也一併收回
    expect(notchHiddenCleanup(true, { gesture: true, dragging: false, hover: true }, true)).toEqual({
      finishGesture: true,
      endHover: true,
      dragging: false,
    })
    // 一般情況（瀏海畫著、原生沒有結束拖曳）：什麼都不清
    expect(notchHiddenCleanup(true, dragging)).toEqual({ finishGesture: false, endHover: false, dragging: true })
    expect(notchHiddenCleanup(true, dragging, false)).toEqual({ finishGesture: false, endHover: false, dragging: true })
    // 網頁已自己收到 pointerup（手勢結束、不拖曳）後才收到原生的 edge_drag_end：放開後游標停在瀏海上開始的 hover 不被收回
    expect(notchHiddenCleanup(true, { gesture: false, dragging: false, hover: true }, true)).toEqual({
      finishGesture: false,
      endHover: false,
      dragging: false,
    })
  })
})

describe('AC-EN-10 剛到期時不先閃「馬上跳出來」（QA 第 3 輪）', () => {
  it('剛進入到期、讀到延後原因前維持前一個靜止文字；剛載入用一般待處理文字；讀取失敗才說馬上跳出來（QA 第 3 輪）', () => {
    const before = notchRestState([timed('a', 1_000, { state: 'warned' }), timed('b', 600_000)], NOW)
    expect(restText(before)).toBe('0:01 後強制詢問')
    const overdue = [timed('a', -500, { state: 'warned' }), timed('b', 600_000)]
    // 還沒讀到延後原因（undefined）：維持前一個靜止文字
    const unread = notchRestState(overdue, NOW, undefined)
    expect(unread.kind).toBe('deferred')
    expect(unread.defer).toBeUndefined()
    expect(restText(shownRestState(unread, before))).toBe('0:01 後強制詢問')
    expect(notchRestState(overdue, NOW).defer).toBeUndefined()
    // 剛載入就已到期（沒有前一個）：一般待處理文字
    expect(restText(shownRestState(unread, null))).toBe('你有 2 則待處理通知')
    // 讀到原因：換成原因文字
    expect(restText(shownRestState(notchRestState(overdue, NOW, { reasons: ['mic_on'] }), before))).toBe('時間到了，麥克風關掉就跳出來')
    // 讀取失敗（null）或沒有 guard 命中：馬上跳出來
    expect(restText(shownRestState(notchRestState(overdue, NOW, null), before))).toBe('時間到了，馬上跳出來')
    expect(restText(shownRestState(notchRestState(overdue, NOW, { reasons: [] }), before))).toBe('時間到了，馬上跳出來')
    // 不是到期狀態：照當下狀態
    const pending = notchRestState([timed('b', 600_000)], NOW)
    expect(shownRestState(pending, before)).toBe(pending)
  })
})

describe('AC-EN-10 倒數分鐘依警告長度補位（QA 第 5 輪）', () => {
  it('靜止狀態帶著最早一題的警告長度（force_at − warn_at），倒數依它補位（QA 第 5 輪）', () => {
    // 預設 3 分鐘
    const three = notchRestState([timed('a', 179_000, { state: 'warned' })], NOW)
    expect(three.warnMs).toBe(180_000)
    expect(restText(three)).toBe('2:59 後強制詢問')
    // 設定頁把提前警告改成 10 分鐘（之後新開始的週期）：warn_at 在 force_at 前 10 分鐘
    const tenWarn = (forceIn: number) => timed('a', forceIn, { state: 'warned', warn_at: at(forceIn - 10 * 60_000) })
    expect(notchRestState([tenWarn(600_000)], NOW).warnMs).toBe(600_000)
    expect(restText(notchRestState([tenWarn(600_000)], NOW))).toBe('10:00 後強制詢問')
    expect(restText(notchRestState([tenWarn(599_000)], NOW))).toBe('09:59 後強制詢問')
    // 只看最早一題：後面那題的警告長度不影響
    expect(notchRestState([timed('a', 179_000, { state: 'warned' }), tenWarn(700_000)], NOW).warnMs).toBe(180_000)
    // 沒有題目：沒有警告長度
    expect(notchRestState([], NOW).warnMs).toBeUndefined()
  })
})
