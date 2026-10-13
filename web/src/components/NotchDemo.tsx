// 邊緣瀏海示範（AC-WEB-02）：拖曳沿邊吸附、滑過伸長跑題目、點開決策面板、回答後題數減一。全部假資料。
import { useEffect, useRef, useState } from 'react'
import { Check, RotateCcw } from 'lucide-react'
import { snapToEdge, type Edge } from '../lib/notch'

interface Q {
  agent: string
  project: string
  q: string
  why: string
  options: string[]
  rec: number
}

const QUESTIONS: Q[] = [
  { agent: 'Claude', project: 'landing-page', q: '首屏標題要用哪一版？', why: 'A 版在小流量測試點擊率高 12%，但字比較多。決定後 Claude 會改好並部署到預覽環境。', options: ['A：AI 卡住時叫醒你', 'B：別讓 AI 等你', '兩版都上，跑一週 A/B'], rec: 0 },
  { agent: 'Codex', project: 'api', q: '要把舊的 v1 端點一起拿掉嗎？', why: '還有 3% 的請求打 v1。拿掉要先通知兩個客戶。', options: ['先保留，下個月再拿', '現在拿掉'], rec: 0 },
  { agent: 'Claude', project: 'ads', q: '把「早鳥」廣告預算從 500 調到 600？', why: 'ROAS 連三天 3.2。動錢的題目會先讓你核對金額。', options: ['調到 600', '維持 500'], rec: 0 },
]

const SHORT = 132
const LONG = 300
const THICK = 30

export function NotchDemo() {
  const desk = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ edge: Edge; along: number }>({ edge: 'top', along: 0.5 })
  const [drag, setDrag] = useState<{ x: number; y: number } | null>(null)
  const [hover, setHover] = useState(false)
  const [left, setLeft] = useState(QUESTIONS.length)
  const [open, setOpen] = useState(false)
  const [pick, setPick] = useState<number | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const [size, setSize] = useState({ w: 1000, h: 575 })
  const start = useRef<{ x: number; y: number; moved: boolean } | null>(null)

  useEffect(() => {
    const el = desk.current
    if (!el) return
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const q = QUESTIONS[QUESTIONS.length - left]
  const horizontal = pos.edge === 'top' || pos.edge === 'bottom'
  const len = hover && !drag ? Math.min(LONG, (horizontal ? size.w : size.h) - 40) : SHORT
  const top0 = 26 // 選單列下方
  let style: React.CSSProperties
  if (drag) {
    style = { left: drag.x - (horizontal ? len / 2 : THICK / 2), top: drag.y - (horizontal ? THICK / 2 : len / 2), width: horizontal ? len : THICK, height: horizontal ? THICK : len, borderRadius: 14 }
  } else if (horizontal) {
    const x = Math.min(Math.max(pos.along * size.w - len / 2, 8), size.w - len - 8)
    style = { left: x, top: pos.edge === 'top' ? top0 : size.h - THICK, width: len, height: THICK }
  } else {
    const y = Math.min(Math.max(pos.along * size.h - len / 2, top0 + 8), size.h - len - 8)
    style = { top: y, left: pos.edge === 'left' ? 0 : size.w - THICK, width: THICK, height: len }
  }

  const local = (e: React.PointerEvent) => {
    const r = desk.current!.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }

  const onDown = (e: React.PointerEvent) => {
    start.current = { ...local(e), moved: false }
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch {
      /* 有些指標（例如自動化）不能捕捉；拖曳照樣能用 */
    }
  }
  const onMove = (e: React.PointerEvent) => {
    if (!start.current) return
    const p = local(e)
    if (!start.current.moved && Math.hypot(p.x - start.current.x, p.y - start.current.y) < 5) return
    start.current.moved = true
    setDrag(p)
  }
  const onUp = (e: React.PointerEvent) => {
    const s = start.current
    start.current = null
    if (!s) return
    if (s.moved) {
      const p = local(e)
      setPos(snapToEdge(p.x, p.y, size.w, size.h))
      setDrag(null)
    } else {
      setOpen(true)
      setPick(null)
      setDone(null)
    }
  }

  const send = () => {
    if (pick == null) return
    setDone(q.options[pick])
  }
  const next = () => {
    setLeft((n) => n - 1)
    setOpen(false)
    setDone(null)
    setPick(null)
    setHover(false)
  }

  return (
    <div className="desk" ref={desk}>
      <div className="desk__menubar" aria-hidden="true">
        <b>Claude Code</b>
        <span>檔案</span>
        <span>編輯</span>
        <span className="r">23:41</span>
      </div>
      <div className="desk__win" aria-hidden="true">
        <div className="dots"><span /><span /><span /></div>
        <div>$ claude</div>
        <div className="term-ok">✓ 改好 hero 的兩個版本</div>
        <div className="term-wait">… 等待你決定：首屏標題要用哪一版？</div>
        <i style={{ width: '80%' }} />
        <i style={{ width: '64%' }} />
        <i style={{ width: '72%' }} />
      </div>

      {left > 0 && (
        <div
          className="notch"
          role="button"
          tabIndex={0}
          aria-label={`還有 ${left} 題在等你。拖曳可以移到別的邊，按一下或 Enter 打開。`}
          data-edge={pos.edge}
          data-dragging={drag ? 'true' : 'false'}
          style={style}
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerEnter={() => setHover(true)}
          onPointerLeave={() => setHover(false)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              setOpen(true)
            }
          }}
        >
          <img src="/kunsu-mark.png" alt="" draggable={false} />
          <span className="notch__count">{left}</span>
          {horizontal && hover && !drag && (
            <span className="notch__marquee" aria-hidden="true">
              <span>{QUESTIONS.slice(QUESTIONS.length - left).map((x) => x.q).join('　·　')}</span>
            </span>
          )}
        </div>
      )}

      {left === 0 && (
        <div className="panel-scrim" style={{ background: 'transparent' }}>
          <div className="panel" style={{ textAlign: 'center' }}>
            <div className="panel__q">沒有事在等你了</div>
            <p className="panel__why">三個 AI 都拿到答案，接著做了。</p>
            <button className="btn" onClick={() => setLeft(QUESTIONS.length)}>
              <RotateCcw size={14} /> 再玩一次
            </button>
          </div>
        </div>
      )}

      {open && q && (
        <div className="panel-scrim" onClick={(e) => e.target === e.currentTarget && setOpen(false)}>
          <div className="panel" role="dialog" aria-modal="true" aria-label={q.q}>
            <div className="panel__meta">
              <span className="chip chip--blue">{q.agent}</span>
              <span>{q.project}</span>
              <span style={{ marginLeft: 'auto' }}>第 {QUESTIONS.length - left + 1} 題，共 {QUESTIONS.length} 題</span>
            </div>
            <div className="panel__q">{q.q}</div>
            {done ? (
              <>
                <div className="receipt" role="status">
                  <div><Check size={16} /> 已回答「{done}」</div>
                  <div><Check size={16} /> 記進帳本</div>
                  <div><Check size={16} /> 交給 {q.agent}，原本那段對話接著做</div>
                </div>
                <div className="panel__foot">
                  <span />
                  <button className="btn btn--primary" onClick={next} autoFocus>
                    {left > 1 ? '下一題' : '完成'}
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="panel__why">{q.why}</p>
                {q.options.map((o, i) => (
                  <button key={o} className="opt" aria-pressed={pick === i} onClick={() => setPick(i)}>
                    <kbd>{i + 1}</kbd>
                    <span>{o}</span>
                    {i === q.rec && <span className="tag">AI 建議</span>}
                  </button>
                ))}
                <div className="panel__foot">
                  <button className="later" onClick={() => setOpen(false)}>稍後再問</button>
                  <button className="btn btn--primary" disabled={pick == null} onClick={send}>
                    送出決定
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      <div className="desk__hint">拖拖看黑色的瀏海，或點一下回答</div>
    </div>
  )
}
