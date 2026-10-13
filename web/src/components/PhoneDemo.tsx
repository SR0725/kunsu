// 手機 App 示範（AC-WEB-04）：一問一答自動播放、可以自己點答案與撤回、往下拉圓圈填滿才出現歷史紀錄。
import { useEffect, useRef, useState } from 'react'
import { phoneDemo, type PhoneState } from '../lib/phoneDemo'

const reduced = () => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

function Mark({ agent }: { agent: 'claude' | 'codex' }) {
  return agent === 'claude' ? (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-label="Claude"><path fill="#d97757" d="M12 2l1.6 6.2L20 6l-4.4 4.8L22 12l-6.4 1.2L20 18l-6.4-2.2L12 22l-1.6-6.2L4 18l4.4-4.8L2 12l6.4-1.2L4 6l6.4 2.2z" /></svg>
  ) : (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-label="Codex"><circle cx="12" cy="12" r="10" fill="#5b5bd6" /><path d="M8 10l3 2-3 2M12.5 15h3.5" stroke="#fff" strokeWidth="1.8" fill="none" strokeLinecap="round" /></svg>
  )
}

const HISTORY = [
  ['這週決策紀錄放進 W41？', '放進 W41'],
  ['landing page hero 換新文案？', '換'],
]

export function PhoneDemo() {
  const [s, setS] = useState<PhoneState>(phoneDemo.initial)
  const [pull, setPull] = useState(0)
  const [pulling, setPulling] = useState(false)
  const [touched, setTouched] = useState(false)
  const [visible, setVisible] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const startY = useRef<number | null>(null)

  // 只在看得到時自動播放；使用者一碰就停
  useEffect(() => {
    const el = root.current
    if (!el) return
    const io = new IntersectionObserver(([e]) => setVisible(e.isIntersecting), { threshold: 0.4 })
    io.observe(el)
    return () => io.disconnect()
  }, [])

  useEffect(() => {
    if (touched || !visible || reduced()) return
    const t = window.setTimeout(() => {
      if (s.undoable) setS(phoneDemo.commit)
      else if (s.current) setS((x) => phoneDemo.answer(x, x.current?.recommended ?? 0))
      else setS(phoneDemo.initial())
    }, s.undoable ? 1800 : s.current ? 2600 : 3500)
    return () => window.clearTimeout(t)
  }, [s, touched, visible])

  // 撤回窗 5 秒
  useEffect(() => {
    if (!s.undoable || !touched) return
    const t = window.setTimeout(() => setS(phoneDemo.commit), 5000)
    return () => window.clearTimeout(t)
  }, [s, touched])

  const progress = phoneDemo.progress(pull)
  const n = s.current ? phoneDemo.questions.length - s.answered.length : 0

  return (
    <div ref={root}>
      <div className="phone">
        <div
          className="phone__screen"
          onPointerDown={(e) => {
            setTouched(true)
            startY.current = e.clientY
          }}
          onPointerMove={(e) => {
            if (startY.current == null) return
            const d = e.clientY - startY.current
            if (d > 0) {
              setPulling(true)
              setPull(d * 0.6)
            }
          }}
          onPointerUp={() => {
            if (pulling) setS((x) => phoneDemo.release(x, pull))
            startY.current = null
            setPulling(false)
            setPull(0)
          }}
          onPointerCancel={() => {
            startY.current = null
            setPulling(false)
            setPull(0)
          }}
        >
          <div className="phone__island" />
          <div className="phone__head">
            <img src="/kunsu-mark.png" alt="" />
            軍師 · Claude
            <span className="on"><i />Mac 已連線</span>
          </div>
          {pull > 6 && (
            <div className="phone__ring" aria-hidden="true">
              <svg width="22" height="22" viewBox="0 0 22 22">
                <circle cx="11" cy="11" r="9" fill={progress >= 1 ? 'var(--ink)' : 'none'} stroke="var(--line-2)" strokeWidth="2" />
                <circle cx="11" cy="11" r="9" fill="none" stroke="var(--ink)" strokeWidth="2" strokeLinecap="round" strokeDasharray={`${progress * 56.5} 56.5`} transform="rotate(-90 11 11)" />
              </svg>
              {progress >= 1 ? <b>放開看歷史紀錄</b> : <span>向下拖出歷史紀錄</span>}
            </div>
          )}
          {s.undoable && touched && (
            <div className="ph-undo" role="status">
              <div>
                已送出「{s.answered.at(-1)?.answer}」
                <small>5 秒內可撤回，撤回前不會送到 Mac</small>
                <div className="bar"><i /></div>
              </div>
              <button onClick={() => setS(phoneDemo.undo)}>撤回</button>
            </div>
          )}
          <div className="phone__scroll" data-pulling={pulling} style={{ transform: `translateY(${pull}px)` }}>
            {s.historyShown && (
              <>
                <div className="ph-day">今晚</div>
                {HISTORY.map(([q, a]) => (
                  <span key={q} style={{ display: 'contents' }}>
                    <div className="ph-q" data-old="true"><span className="av"><Mark agent="claude" /></span><div className="bub">{q}</div></div>
                    <div className="ph-a">{a}</div>
                  </span>
                ))}
                <div className="ph-day">這次打開</div>
              </>
            )}
            <div className="ph-hello">
              <h4>晚安</h4>
              <p>{n > 0 ? `有 ${n} 件事在等你` : '沒有事在等你，可以睡了'}</p>
            </div>
            {s.answered.map((a) => {
              const q = phoneDemo.questions.find((x) => x.id === a.id)!
              return (
                <span key={a.id} style={{ display: 'contents' }}>
                  <div className="ph-q" data-old="true"><span className="av"><Mark agent={q.agent} /></span><div className="bub">{q.question}</div></div>
                  <div className="ph-a">{a.answer}</div>
                  <div className="ph-status">✓✓ 已交給 {q.agent === 'claude' ? 'Claude' : 'Codex'}</div>
                </span>
              )
            })}
            {s.current && (
              <>
                <div className="ph-q" key={s.current.id}>
                  <span className="av"><Mark agent={s.current.agent} /></span>
                  <div className="bub">{s.current.question}<small>{s.current.why}</small></div>
                </div>
                {n > 1 && <div className="ph-pending">尚有 {n - 1} 個待辦事項要處理</div>}
              </>
            )}
          </div>
          <div className="ph-dock">
            {s.current ? (
              <>
                <div className="ph-chips"><span>為什麼？</span><span>前因後果？</span><span>等下再說</span></div>
                <div className="ph-tiles">
                  {s.current.options.map((o, i) => (
                    <button key={o} className="ph-tile" data-rec={i === s.current!.recommended} onClick={() => { setTouched(true); setS((x) => phoneDemo.answer(phoneDemo.commit(x), i)) }}>
                      {o}
                    </button>
                  ))}
                </div>
              </>
            ) : (
              <div className="ph-chips"><span>大家在做什麼？</span><span>開新任務</span></div>
            )}
            <div className="ph-input">{s.current ? '打字問軍師這題的事…' : '跟軍師說…'}</div>
          </div>
        </div>
      </div>
      <p className="phone-caption">點答案試試看，或在畫面上往下拉</p>
    </div>
  )
}
