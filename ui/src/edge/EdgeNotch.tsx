// 邊緣瀏海（PRD §19、DESIGN「Edge notch」、BDD edge-notch）。舞台視窗貼滿螢幕、預設穿透；這裡只畫黑色帶狀區，
// 回報命中區給原生層。閒置時不跑任何動畫 frame；只有移動、hover 跑馬燈與文字淡入時才用 rAF。
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { EdgeInfo, KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { AgentMarkIcon } from '../components/AgentMarkIcon'
import { readReducedMotion, subscribeReducedMotion } from '../motion'
import {
  notchClockDelay,
  notchHiddenCleanup,
  notchHitReport,
  notchLabel,
  notchRestState,
  plainLine,
  shownRestState,
} from './content'
import {
  contentAllowed,
  hoverBegan,
  hoverStale,
  initialPrivacy,
  privacyFadeIn,
  privacyPushed,
  privacyRead,
  stageIdentity,
  type PrivacyGate,
} from './hover'
import {
  DRAG_THRESHOLD,
  FONT,
  HOVER_DELAY,
  HOVER_EXTRA,
  HOVER_LEAVE,
  ICON,
  MARQUEE_GAP,
  MARQUEE_PAUSE,
  MARQUEE_SPEED,
  T,
  TEXT_FADE_MS,
  bandGeometry,
  centerToStart,
  clamp,
  cornerAt,
  dragCenter,
  edgeOfS,
  grabAfterCollapse,
  grabByDirection,
  grabStart,
  growDirection,
  headAnchoredCenter,
  hoverTarget,
  makeStage,
  marqueeText,
  mod,
  nearestEdge,
  notchFrameNeeded,
  pieces,
  predictPoint,
  preferredHead,
  resizeCenter,
  restLength,
  restoreCenter,
  restText,
  SAVED_MEMORY_INIT,
  type DeferInfo,
  type NotchMotion,
  type RestState,
  type SavedMemory,
  savePosition,
  saveStillCurrent,
  savedAfterRead,
  savedAfterRelease,
  savedAfterReset,
  settleRelease,
  stepMotion,
  textPath,
  tokenize,
  type Edge,
  type HeadEnd,
  type Pt,
} from './geometry'

const FONT_FAMILY = '"Geist Sans", "PingFang TC", "Noto Sans TC", sans-serif'
const TEXT_COLOR = '#b4bac4'
const WARN_COLOR = '#ef8d86'
/** 數字等寬（場景 10、PRD FR-03）：量寬以最寬數字計算（`tokenize`），繪製用 tabular-nums，倒數每秒換字不伸縮、不晃。 */
const TABULAR = { fontVariantNumeric: 'tabular-nums' } as const

let measureCtx: CanvasRenderingContext2D | null = null
function measure(s: string): number {
  measureCtx ??= document.createElement('canvas').getContext('2d')
  if (!measureCtx) return s.length * 7
  measureCtx.font = `${FONT}px ${FONT_FAMILY}`
  return measureCtx.measureText(s).width
}

interface Motion extends NotchMotion {
  head: HeadEnd
  textFadeStart: number
  hover: boolean
  hoverStart: number
  growDir: 1 | -1
  edge: Edge
}

interface Gesture {
  id: number
  start: Pt
  last: Pt
  t: number
  vx: number
  vy: number
  dragged: boolean
  grabOffset: number
  /** 按在 L 形兩臂重疊的角落方格裡：移動滿拖曳門檻時才依移動方向選臂（場景 4）。 */
  arms: [Edge, Edge] | null
}

/** 螢幕本身（場景 3d）：有螢幕識別就用它，否則名稱。 */
const displayKey = (i: EdgeInfo) => (i.display_id != null ? `#${i.display_id}` : i.display)

/** questions 由 App 傳入時已排除 internal 與終態題（`notchQueue`）。 */
export function EdgeNotch({ ipc, questions }: { ipc: KunsuIpc; questions: Question[] }) {
  const [info, setInfo] = useState<EdgeInfo | null>(null)
  const [, setFrame] = useState(0)
  const [nowMs, setNowMs] = useState(() => Date.now())
  // 減少動態：系統或 app 內設定，即時跟隨（場景 18）。處理函式讀 ref，畫面靠 setFrame 重繪。
  const reduced = useRef(readReducedMotion())
  // 隱私守門（場景 12）：每次 hover 拿到最新狀態前一律當作隱私中；不放進 info，改變不會移動瀏海（場景 11c）。
  const privacy = useRef<PrivacyGate>(initialPrivacy)
  // 記憶體中的「上次位置」：換螢幕時取那個螢幕的保存值，存檔時同步更新（場景 3b）；設定頁回到預設位置時清掉（場景 3e）。
  const mem = useRef<SavedMemory>(SAVED_MEMORY_INIT)
  // 目前位置是用哪個靜止長度算的（文字長度改變時頭端不動，場景 10b）。
  const restLenAt = useRef(0)
  // 停在角落：靜止長度補上轉角弧長（場景 10b）。只在放開與還原時改變，拖曳中經過角落不算。
  const [corner, setCorner] = useState(false)
  // 設定頁「回到預設位置」（kunsu://edge_position_reset）時加一，讓下方的定位重跑一次。
  const [resetTick, setResetTick] = useState(0)
  const motion = useRef<Motion | null>(null)
  const gesture = useRef<Gesture | null>(null)
  const raf = useRef(0)
  const lastFrame = useRef(0)
  const hoverTimer = useRef(0)
  const pending = useRef<'enter' | 'leave' | null>(null)
  const lastInside = useRef(0)
  const staleTimer = useRef(0)
  const lastHit = useRef('')
  const saveTimer = useRef(0)
  // 最後一次畫出瀏海時的靜止長度：拖曳中所有題目結案、瀏海不再畫出時，放開以它定位頭端保存（場景 14、INV-24）。
  const drawnLen = useRef(0)
  const svgRef = useRef<SVGSVGElement>(null)

  const width = info?.width
  const height = info?.height
  const stage = useMemo(() => (width && height ? makeStage(width, height) : null), [width, height])
  const identity = info ? stageIdentity(info) : ''
  // 到期被延後時擋住的原因（場景 10c）：進入 deferred 就讀，之後每 2 秒重讀；離開 deferred 清掉。undefined＝還沒讀到
  // （維持前一個靜止文字），null＝讀取失敗或 2 秒內沒有回應（「時間到了，馬上跳出來」）。
  const [defer, setDefer] = useState<DeferInfo | null | undefined>(undefined)
  const deferred = notchRestState(questions, nowMs).kind === 'deferred'
  useEffect(() => {
    if (!deferred) {
      setDefer(undefined)
      return
    }
    let alive = true
    const read = () =>
      void ipc
        .deferReasons()
        .then((d) => alive && setDefer(d))
        .catch(() => alive && setDefer(null))
    read()
    const timer = window.setInterval(read, 2000)
    const fallback = window.setTimeout(() => alive && setDefer((d) => (d === undefined ? null : d)), 2000)
    return () => {
      alive = false
      window.clearInterval(timer)
      window.clearTimeout(fallback)
    }
  }, [deferred, ipc])
  // 實際顯示的狀態：剛到期還沒讀到原因時維持前一個顯示的狀態（場景 10c）。
  const lastShown = useRef<RestState | null>(null)
  const state = shownRestState(notchRestState(questions, nowMs, defer), lastShown.current)
  useEffect(() => {
    lastShown.current = state
  })
  const warning = state.kind === 'warning'
  const rest = restText(state)
  const restLen = useMemo(() => restLength(rest, measure, corner), [rest, corner])
  const lines = useMemo(() => questions.map((q) => plainLine(q.question_line)), [questions])
  const redraw = useCallback(() => setFrame((n) => n + 1), [])

  useEffect(() => {
    let alive = true
    const load = () =>
      void ipc.edgeInfo().then((i) => {
        if (!alive) return
        // 換了螢幕才換成那台螢幕的保存值；同一台螢幕讀到 null 不是重設（延後的寫檔可能還沒發生或寫檔失敗，場景 3b）。
        mem.current = savedAfterRead(mem.current, displayKey(i), i.saved)
        setInfo(i)
      })
    load()
    window.addEventListener('resize', load)
    const offStage = ipc.subscribeEdgeStage(load)
    return () => {
      alive = false
      window.removeEventListener('resize', load)
      offStage()
    }
  }, [ipc])

  // 設定頁「回到預設位置」（場景 3e）：先取消還沒寫出去的存檔，再清掉記憶體位置（排定的寫檔作廢），以預設位置重新定位。
  useEffect(
    () =>
      ipc.subscribeEdgePositionReset(() => {
        window.clearTimeout(saveTimer.current)
        mem.current = savedAfterReset(mem.current)
        setResetTick((n) => n + 1)
      }),
    [ipc],
  )

  // 狀態文字的時鐘（場景 10e）：題目清單一更新就重算；下一次排在 15 秒與最早一題 warn_at／force_at 兩者較早者，
  // warning 期間對齊秒數每秒更新（m:ss）。
  useEffect(() => setNowMs(Date.now()), [questions])
  useEffect(() => {
    const id = window.setTimeout(() => setNowMs(Date.now()), notchClockDelay(questions, nowMs))
    return () => window.clearTimeout(id)
  }, [questions, nowMs])

  const kick = useCallback(() => {
    if (raf.current) return
    lastFrame.current = performance.now()
    const step = (now: number) => {
      raf.current = 0
      const m = motion.current
      if (!m || !stage) return
      // rAF 時間戳可能早於 kick（觸發動畫的事件）：負的時間差當 0，這一幀不移動（場景 6）。
      const dt = clamp((now - lastFrame.current) / 1000, 0, 0.033)
      lastFrame.current = now
      // 中心與長度同一個彈簧；減少動態時任何目標直接到位、速度歸零（場景 18）。停下來就不再要 frame（場景 19）。
      const moving = stepMotion(stage, m, dt, reduced.current)
      setFrame((n) => n + 1)
      const marquee = m.hover && !m.dragging && contentAllowed(privacy.current)
      if (notchFrameNeeded(moving, now - m.textFadeStart, marquee, reduced.current)) raf.current = requestAnimationFrame(step)
    }
    raf.current = requestAnimationFrame(step)
  }, [stage])

  useEffect(
    () => () => {
      cancelAnimationFrame(raf.current)
      window.clearTimeout(hoverTimer.current)
      window.clearInterval(staleTimer.current)
      window.clearTimeout(saveTimer.current)
    },
    [],
  )

  // 只有螢幕大小或螢幕本身改變（含第一次），或保存的位置被刪掉（回到預設位置）才決定位置：還原記憶體中的上次位置，壞值或沒有就用預設位置。直邊保存的是
  // 頭端，依目前文字長度往尾端展開；角落另補轉角弧長。
  useEffect(() => {
    if (!stage || !info) return
    const straight = restLength(rest, measure)
    const sc = restoreCenter(stage, mem.current.saved, straight, info.menubar)
    const atCorner = cornerAt(stage, sc) !== null
    const len = atCorner ? restLength(rest, measure, true) : straight
    setCorner(atCorner)
    motion.current = {
      s: sc,
      sTarget: sc,
      v: 0,
      len,
      lenTarget: len,
      lenV: 0,
      head: preferredHead(pieces(stage, centerToStart(sc, len), len - T)),
      textFadeStart: -1e9,
      hover: false,
      hoverStart: 0,
      growDir: 1,
      dragging: false,
      edge: edgeOfS(stage, sc),
    }
    restLenAt.current = len
    window.clearInterval(staleTimer.current)
    setFrame((n) => n + 1)
    // restLen 變化另外處理（頭端不動），隱私等其他狀態改變不重設位置。
  }, [stage, identity, resetTick]) // eslint-disable-line react-hooks/exhaustive-deps

  // 文字長度變了（題數、狀態）：頭端不動、尾端沿彈簧伸縮；hover 中保留延長量；角落 L 形維持置中。
  useEffect(() => {
    const m = motion.current
    if (!m || !stage) return
    const old = restLenAt.current
    restLenAt.current = restLen
    if (old && old !== restLen && !m.dragging) {
      const shift = m.hover ? (m.growDir * HOVER_EXTRA) / 2 : 0
      const restCenter = mod(m.sTarget - shift, stage.P)
      m.sTarget = mod(resizeCenter(stage, restCenter, old, restLen, m.head) + shift, stage.P)
    }
    m.lenTarget = restLen + (m.hover ? HOVER_EXTRA : 0)
    if (reduced.current) {
      m.s = m.sTarget
      m.len = m.lenTarget
      m.v = 0
      m.lenV = 0
    }
    kick()
  }, [restLen, kick, stage])

  // 減少動態設定改變：立即生效（正在動的直接到位）。
  useEffect(
    () =>
      subscribeReducedMotion((r) => {
        reduced.current = r
        const m = motion.current
        if (r && m) {
          m.s = m.sTarget
          m.len = m.lenTarget
          m.v = 0
          m.lenV = 0
        }
        redraw()
        kick()
      }),
    [kick, redraw],
  )

  // shell 推送隱私 guard 改變：hover 中命中立即改回狀態文字；hover 中解除才讓題目淡入，沒有 hover 時靜止文字不重新淡入
  // （場景 11c）。
  useEffect(
    () =>
      ipc.subscribeEdgePrivacy((active) => {
        const before = contentAllowed(privacy.current)
        privacy.current = privacyPushed(privacy.current, active)
        const m = motion.current
        if (m && privacyFadeIn(before, contentAllowed(privacy.current), m.hover && !m.dragging)) m.textFadeStart = performance.now()
        redraw()
        kick()
      }),
    [ipc, kick, redraw],
  )

  const setHead = (m: Motion, head: HeadEnd) => {
    if (head === m.head) return
    m.head = head
    m.textFadeStart = performance.now()
  }

  /** 放開。`fromLen`：瀏海已不再畫出（拖曳中所有題目結案）時最後畫出來的靜止長度，以頭端為錨保存（場景 14）。 */
  const settle = (fromLen?: number) => {
    const m = motion.current
    if (!m || !stage || !info) return
    // 吸附位置與放開後的最終長度一起決定（場景 7）：角落用角落長度、直邊用直邊長度，24 px 以最終長度計算。記錄的靜止長度
    // 同步成最終長度，之後長度 effect 不會再移動中心。
    const drawnAt = fromLen ? { len: fromLen, head: m.head } : undefined
    const r = settleRelease(stage, m.sTarget, restLength(rest, measure), restLength(rest, measure, true), drawnAt)
    m.sTarget = r.center
    m.lenTarget = r.len
    restLenAt.current = r.len
    setHead(m, preferredHead(pieces(stage, centerToStart(r.center, r.len), r.len - T)))
    if (reduced.current) {
      m.s = m.sTarget
      m.len = m.lenTarget
      m.v = 0
      m.lenV = 0
    }
    kick()
    setCorner(r.corner)
    // 記憶體中的位置立即更新；寫檔延後 400 ms 合併。直邊保存頭端（用最終長度定位）。排定之後按了回到預設位置，這次寫檔
    // 作廢（場景 3e）。
    const pos = savePosition(stage, r.center, r.len)
    mem.current = savedAfterRelease(mem.current, pos)
    const gen = mem.current.resetGen
    window.clearTimeout(saveTimer.current)
    saveTimer.current = window.setTimeout(() => {
      if (!saveStillCurrent(mem.current, gen)) return
      void ipc.edgeSave(info.display, info.display_id ?? null, pos).catch(() => {})
    }, 400)
  }

  const localPoint = (event: React.PointerEvent): Pt => {
    const rect = svgRef.current?.getBoundingClientRect()
    return { x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0) }
  }

  const endHover = () => {
    window.clearInterval(staleTimer.current)
    const m = motion.current
    if (!m || !stage || !m.hover) return
    m.hover = false
    // 收回以頭端為錨（場景 11）：頭端不動、尾端縮回靜止長度。
    m.sTarget = headAnchoredCenter(stage, m.sTarget, m.lenTarget, restLen, m.head)
    m.lenTarget = restLen
    if (reduced.current) {
      m.s = m.sTarget
      m.len = m.lenTarget
      m.v = 0
      m.lenV = 0
    }
    kick()
  }

  const startHover = () => {
    const m = motion.current
    if (!m || !stage || m.dragging || m.hover) return
    reduced.current = readReducedMotion()
    // 一律往尾端延長，頭端永遠不動；空間不足時尾端沿周長轉過角落成 L（場景 11）。
    m.growDir = growDirection(m.head)
    m.hover = true
    m.hoverStart = performance.now()
    m.lenTarget = restLen + HOVER_EXTRA
    m.sTarget = hoverTarget(stage, m, m.growDir, true)
    if (reduced.current) {
      m.s = m.sTarget
      m.len = m.lenTarget
      m.v = 0
      m.lenV = 0
    }
    // 隱私：這次 hover 拿到最新狀態前只顯示狀態文字；讀不到就維持隱私中。
    privacy.current = hoverBegan(privacy.current)
    const epoch = privacy.current.epoch
    void ipc
      .edgePrivacy()
      .then((active) => {
        const before = contentAllowed(privacy.current)
        privacy.current = privacyRead(privacy.current, epoch, active)
        const mm = motion.current
        // 回應回來時已不在 hover（或已開始拖曳）：畫的是靜止文字，不重新淡入（場景 11c）。
        if (mm && privacyFadeIn(before, contentAllowed(privacy.current), mm.hover && !mm.dragging)) {
          mm.textFadeStart = performance.now()
          mm.hoverStart = performance.now()
        }
        redraw()
        kick()
      })
      .catch(() => {})
    // 逾時收回（場景 11b）：只在 hover 中檢查，閒置時不跑計時器。
    lastInside.current = performance.now()
    window.clearInterval(staleTimer.current)
    staleTimer.current = window.setInterval(() => {
      const mm = motion.current
      if (hoverStale(!!mm?.hover && !mm.dragging, lastInside.current, performance.now())) hoverFns.current.end()
    }, 1000)
    kick()
  }

  // 計時器回呼一律透過 ref 拿最新的處理函式（避免用到排程當下的舊文字長度）。
  const hoverFns = useRef({ start: startHover, end: endHover })
  hoverFns.current = { start: startHover, end: endHover }

  /** 游標在瀏海上（網頁指標事件或原生通知）：120 ms 後開始 hover；hover 中則取消待收回。 */
  const pointerInside = () => {
    lastInside.current = performance.now()
    const m = motion.current
    if (!m || m.dragging || gesture.current) return
    if (m.hover) {
      if (pending.current === 'leave') {
        window.clearTimeout(hoverTimer.current)
        pending.current = null
      }
      return
    }
    if (pending.current === 'enter') return
    window.clearTimeout(hoverTimer.current)
    pending.current = 'enter'
    hoverTimer.current = window.setTimeout(() => {
      pending.current = null
      hoverFns.current.start()
    }, HOVER_DELAY)
  }

  /** 游標離開瀏海：取消待開始的 hover；hover 中則 260 ms 後收回。 */
  const pointerOutside = () => {
    if (gesture.current) return
    const m = motion.current
    if (pending.current === 'enter') {
      window.clearTimeout(hoverTimer.current)
      pending.current = null
    }
    if (!m?.hover || pending.current === 'leave') return
    pending.current = 'leave'
    hoverTimer.current = window.setTimeout(() => {
      pending.current = null
      hoverFns.current.end()
    }, HOVER_LEAVE)
  }

  // 原生層的進出通知（WebKit 在穿透切換時可能漏送 pointerleave／pointerenter）。用 ref 拿最新的處理函式。
  const pointerRef = useRef({ inside: pointerInside, outside: pointerOutside })
  pointerRef.current = { inside: pointerInside, outside: pointerOutside }
  useEffect(() => ipc.subscribeEdgePointer((inside) => (inside ? pointerRef.current.inside() : pointerRef.current.outside())), [ipc])

  const finishGesture = (cancelled: boolean, event?: React.PointerEvent, fromLen?: number) => {
    const g = gesture.current
    const m = motion.current
    if (!g || !m) return
    gesture.current = null
    if (event && (event.currentTarget as Element).hasPointerCapture?.(event.pointerId)) {
      ;(event.currentTarget as Element).releasePointerCapture(event.pointerId)
    }
    m.dragging = false
    if (!g.dragged) {
      if (!cancelled) openDecision()
      return
    }
    settle(fromLen)
  }
  const finishRef = useRef(finishGesture)
  finishRef.current = finishGesture

  const geometry = (() => {
    const m = motion.current
    if (!m || !stage) return null
    const lc = Math.max(1, m.len - T)
    const start = centerToStart(m.s, m.len)
    const ps = pieces(stage, start, lc)
    return { geo: bandGeometry(stage, start, lc), tp: textPath(stage, ps, m.head) }
  })()

  // 命中區回報（只在實際改變時）：就是畫出來的那組多邊形；沒畫出瀏海（沒有待決題、幾何還沒算好）就回報空命中區
  // （場景 14、INV-24）。
  const hitReport = notchHitReport(questions, geometry?.geo.hit ?? null, motion.current?.dragging ?? false)
  const drawn = hitReport.shapes.length > 0
  useEffect(() => {
    const key = JSON.stringify([hitReport.shapes, hitReport.dragging])
    if (key === lastHit.current) return
    lastHit.current = key
    void ipc.edgeHit(hitReport.shapes, hitReport.dragging)
  })

  /**
   * 瀏海沒畫出來（拖曳中所有題目在別處結案），或原生層結束接管（`edge_drag_end`：網頁漏收 pointerup，或拖曳／按著時舞台
   * 被勿擾、⌘⇧K 隱藏）：進行中的拖曳當作放開、hover 收回、不再拖曳（INV-24）。沒畫出來時以最後畫出來的長度定位頭端保存。
   */
  const cleanUp = (nativeEnded: boolean) => {
    const m = motion.current
    const c = notchHiddenCleanup(
      drawn,
      { gesture: gesture.current !== null, dragging: m?.dragging ?? false, hover: m?.hover ?? false },
      nativeEnded,
    )
    if (!drawn || c.finishGesture) {
      window.clearTimeout(hoverTimer.current)
      pending.current = null
    }
    if (c.finishGesture) finishRef.current(true, undefined, drawn ? undefined : drawnLen.current)
    if (c.endHover) hoverFns.current.end()
    if (m) m.dragging = c.dragging
  }
  const cleanUpRef = useRef(cleanUp)
  cleanUpRef.current = cleanUp
  // 原生層結束接管（只訂閱一次，用 ref 拿最新的處理函式）。重繪一次，命中區回報立即改成不拖曳（舞台隱藏時不跑動畫 frame）。
  useEffect(
    () =>
      ipc.subscribeEdgeDragEnd(() => {
        cleanUpRef.current(true)
        redraw()
      }),
    [ipc, redraw],
  )
  useEffect(() => {
    if (drawn) drawnLen.current = restLen
    else cleanUp(false)
  })

  function openDecision() {
    const icon = geometry?.tp.at(0)
    if (!icon) return
    void ipc.openOverlayFromCapsule({ local_x: icon.x, local_y: icon.y, size: ICON })
  }

  if (!stage || !info || !geometry || !drawn) return null
  const m = motion.current!
  const { geo, tp } = geometry
  const now = performance.now()
  const still = reduced.current
  const textAlpha = still ? 1 : clamp((now - m.textFadeStart) / TEXT_FADE_MS, 0, 1)
  const marquee = m.hover && !m.dragging
  const showContent = marquee && contentAllowed(privacy.current)
  const label = notchLabel(questions, state)

  // 文字：靜止時是狀態文字；hover 時跑馬燈（拿到這次 hover 的最新隱私狀態且未命中才顯示題目；減少動態時截斷第一題）。
  const startD = ICON / 2 + 7
  const endD = tp.total + T / 2 - 12
  let text = rest
  if (marquee) {
    text = marqueeText(lines, !showContent, rest)
    if (still && showContent && lines[0]) text = lines[0]
  }
  let toks = tokenize(text, measure)
  let offset = 0
  const width2 = toks.reduce((n, t) => n + t.adv, 0)
  if (marquee && width2 > endD - startD) {
    if (still) {
      // 截斷並附「…」
      const out = []
      let used = 0
      for (const t of toks) {
        if (used + t.adv > endD - startD - (FONT + 2)) break
        out.push(t)
        used += t.adv
      }
      toks = [...out, { s: '…', kind: 'cjk', adv: FONT }]
    } else {
      const elapsed = Math.max(0, now - m.hoverStart - MARQUEE_PAUSE) / 1000
      offset = (elapsed * MARQUEE_SPEED) % (width2 + MARQUEE_GAP)
      toks = [...toks, { s: ' ', kind: 'space', adv: MARQUEE_GAP }, ...toks]
    }
  }
  const glyphs: { s: string; x: number; y: number; rot: number; opacity: number }[] = []
  let d = startD - offset
  for (const tk of toks) {
    const mid = d + tk.adv / 2
    d += tk.adv
    if (tk.kind === 'space' || mid < startD - 2 || mid > endD + 2) continue
    const fade = Math.min(1, (mid - startD + 2) / 10, (endD + 2 - mid) / 10)
    if (fade <= 0) continue
    const q = tp.at(mid)
    let rot = 0
    if (tk.kind === 'cjk') {
      if (q.arc) rot = q.arc.turn * 14 * Math.sin(Math.PI * q.arc.t)
    } else if (q.arc) rot = q.arc.fromVertical ? 90 * (1 - q.arc.t) : 90 * q.arc.t
    else rot = q.vertical ? 90 : 0
    glyphs.push({ s: tk.s, x: q.x, y: q.y, rot, opacity: fade * textAlpha })
  }
  const icon = tp.at(0)
  const color = warning && !showContent ? WARN_COLOR : TEXT_COLOR

  return (
    <svg ref={svgRef} className="edge-stage" width={stage.W} height={stage.H} viewBox={`0 0 ${stage.W} ${stage.H}`}>
      <g
        className="edge-band"
        role="button"
        aria-label={label}
        data-dragging={m.dragging ? 'true' : 'false'}
        onPointerDown={(event) => {
          if (event.button !== 0) return
          reduced.current = readReducedMotion()
          const p = localPoint(event)
          ;(event.currentTarget as Element).setPointerCapture(event.pointerId)
          // 抓取偏移與拖曳用同一條邊：游標底下那一臂（場景 4），L 形抓哪一臂、離角落多近都不跳。按在兩臂重疊的角落方格裡
          // （grab.arms）時先不決定，移動滿拖曳門檻再依方向選。
          const grab = grabStart(stage, m.s, p, m.len)
          gesture.current = {
            id: event.pointerId,
            start: p,
            last: p,
            t: performance.now(),
            vx: 0,
            vy: 0,
            dragged: false,
            grabOffset: grab.offset,
            arms: grab.arms,
          }
          m.edge = grab.edge
        }}
        onPointerMove={(event) => {
          const g = gesture.current
          if (!g) {
            pointerInside()
            return
          }
          if (event.pointerId !== g.id) return
          const p = localPoint(event)
          const t = performance.now()
          const dt = Math.max(1, t - g.t) / 1000
          const a = 0.35
          g.vx = g.vx * (1 - a) + ((p.x - g.last.x) / dt) * a
          g.vy = g.vy * (1 - a) + ((p.y - g.last.y) / dt) * a
          g.last = p
          g.t = t
          if (!g.dragged && Math.hypot(p.x - g.start.x, p.y - g.start.y) >= DRAG_THRESHOLD) {
            g.dragged = true
            m.dragging = true
            window.clearTimeout(hoverTimer.current)
            window.clearInterval(staleTimer.current)
            pending.current = null
            if (g.arms) {
              // 角落方格裡按下（場景 4）：依這段移動的方向選臂，不加遲滯；抓取偏移在選定的邊上以按下位置量。
              const pick = grabByDirection(stage, m.s, g.start, p, g.arms)
              m.edge = pick.edge
              g.grabOffset = pick.offset
            }
            if (m.hover) {
              // hover 立即收回、以頭端為錨；抓取偏移以收回後的位置計算，第一幀頭端不跳（場景 4）。
              const c = grabAfterCollapse(stage, m.edge, g.start, m.s, m.len, restLen, m.head)
              g.grabOffset = c.offset
              m.hover = false
              m.sTarget = c.center
              m.lenTarget = restLen
            }
            if (reduced.current) {
              // 減少動態：長度與位置直接到位（場景 18）
              m.len = m.lenTarget
              m.lenV = 0
            }
          }
          if (!g.dragged) return
          m.edge = nearestEdge(stage, predictPoint(stage, p, { x: g.vx, y: g.vy }), m.edge)
          m.sTarget = dragCenter(stage, m.edge, p, g.grabOffset)
          if (reduced.current) {
            m.s = m.sTarget
            m.v = 0
          }
          kick()
        }}
        onPointerUp={(event) => finishGesture(false, event)}
        onPointerCancel={(event) => finishGesture(true, event)}
        onPointerEnter={() => pointerInside()}
        onPointerLeave={() => pointerOutside()}
      >
        {geo.shapes.map((path, i) => (
          <path key={i} d={path} fill="#000" />
        ))}
        <g transform={`translate(${(icon.x - ICON / 2).toFixed(2)} ${(icon.y - ICON / 2).toFixed(2)})`}>
          <AgentMarkIcon agent={questions[0].source.agent} size={ICON} />
        </g>
        {glyphs.map((g, i) => (
          <text
            key={i}
            x={g.x.toFixed(2)}
            y={g.y.toFixed(2)}
            textAnchor="middle"
            dominantBaseline="central"
            fontSize={FONT}
            fontFamily={FONT_FAMILY}
            fill={color}
            style={TABULAR}
            opacity={g.opacity.toFixed(2)}
            transform={`rotate(${g.rot.toFixed(2)} ${g.x.toFixed(2)} ${g.y.toFixed(2)})`}
            aria-hidden="true"
          >
            {g.s}
          </text>
        ))}
      </g>
    </svg>
  )
}
