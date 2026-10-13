// 浮動決策膠囊（Tauri 視窗本體由 src-tauri 管）。
// 只有待決題時存在；整顆膠囊移動超過門檻即拖曳，單擊正文展開蓋板。
//
// 視窗貼合（CONTRACT §9 resize_capsule）：在 Tauri capsule 殼裡，本元件用
// ResizeObserver 量測固定膠囊，透過 onMeasure 回報 → Rust setFrame。
// 膠囊沒有 hover/tooltip surface；只有拖曳與點擊展開兩種行為。

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CapsuleDragController, CapsuleOrbAnchor } from '../ipc'
import type { Question } from '../types'
import { isTerminal } from '../types'
import { shouldOpenCapsuleAfterPointer } from './capsuleGesture'
import { ThinkingOrb } from './ThinkingOrb'
import { ForceCycleRing } from './ForceCycleRing'
import { compareForceQueue, forceCycleSnapshot } from '../forceCycle'

export interface CapsuleBarProps {
  questions: Question[]
  onOpen: (anchor: CapsuleOrbAnchor) => void
  /** Tauri capsule 殼：回報內容實際尺寸（CSS px），讓視窗貼合內容。 */
  onMeasure?: (w: number, h: number) => void
  /** 建立手動視窗拖曳 session；不依賴 macOS startDragging。 */
  onDragStart?: (screenX: number, screenY: number) => Promise<CapsuleDragController>
}

const DRAG_THRESHOLD = 4

interface DragGesture {
  pointerId: number
  startX: number
  startY: number
  lastX: number
  lastY: number
  dragged: boolean
  controller?: Promise<CapsuleDragController>
}

export function CapsuleBar({ questions, onOpen, onMeasure, onDragStart }: CapsuleBarProps) {
  const [dragging, setDragging] = useState(false)
  const [nowMs, setNowMs] = useState(Date.now())
  const boxRef = useRef<HTMLDivElement>(null)
  const orbRef = useRef<HTMLSpanElement>(null)
  const gestureRef = useRef<DragGesture | null>(null)

  const openFromOrb = useCallback(() => {
    const rect = orbRef.current?.getBoundingClientRect()
    if (!rect) return
    onOpen({
      local_x: rect.left + rect.width / 2,
      local_y: rect.top + rect.height / 2,
      size: Math.min(rect.width, rect.height),
    })
  }, [onOpen])

  const finishDrag = (event: React.PointerEvent<HTMLDivElement>, cancelled = false) => {
    const gesture = gestureRef.current
    if (!gesture || event.pointerId !== gesture.pointerId) return
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    gestureRef.current = null
    setDragging(false)
    if (!gesture.dragged || !gesture.controller) {
      if (shouldOpenCapsuleAfterPointer(gesture.dragged, cancelled)) openFromOrb()
      return
    }
    event.preventDefault()
    event.stopPropagation()
    void gesture.controller
      .then((controller) => {
        controller.move(gesture.lastX, gesture.lastY)
        return controller.end()
      })
      .then(() => undefined)
      .catch((error) => console.error('膠囊拖曳結束失敗', error))
  }

  const { t0, t1plus, urgent } = useMemo(() => {
    const active = questions.filter((q) => !isTerminal(q.state))
    const t0s = active.filter((q) => q.tier === 't0')
    const others = active.filter((q) => q.tier !== 't0')
    const urgentQ = [...active].sort((a, b) => compareForceQueue(a, b))[0] ?? null
    return { t0: t0s.length, t1plus: others.length, urgent: urgentQ }
  }, [questions])
  const forceSnapshot = urgent ? forceCycleSnapshot(urgent, nowMs) : null

  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])

  // 尺寸回報：固定膠囊一變就量測回報（視窗貼合內容）。
  useEffect(() => {
    if (!onMeasure) return
    const el = boxRef.current
    if (!el) return
    const report = () => {
      const r = el.getBoundingClientRect()
      // +8：留一點呼吸邊，避免子像素捨入裁到邊框
      onMeasure(Math.ceil(r.width) + 8, Math.ceil(r.height) + 8)
    }
    report()
    const ro = new ResizeObserver(report)
    ro.observe(el)
    return () => ro.disconnect()
  }, [onMeasure, urgent])

  if (!urgent) return null

  return (
    <div
      className="capsule-stage fixed left-1 top-1 z-40 select-none"
    >
      <div ref={boxRef} className="capsule-stack">
        <div
          className={`decision-capsule ${t0 > 0 ? 'decision-capsule--urgent' : ''}`}
          style={{ '--capsule-height': '44px' } as React.CSSProperties}
          data-dragging={dragging ? 'true' : 'false'}
          onPointerDown={(event) => {
            if (event.button !== 0 || !onDragStart) return
            event.currentTarget.setPointerCapture(event.pointerId)
            gestureRef.current = {
              pointerId: event.pointerId,
              startX: event.screenX,
              startY: event.screenY,
              lastX: event.screenX,
              lastY: event.screenY,
              dragged: false,
            }
          }}
          onPointerMove={(event) => {
            const gesture = gestureRef.current
            if (!gesture || event.pointerId !== gesture.pointerId || !onDragStart) return
            gesture.lastX = event.screenX
            gesture.lastY = event.screenY
            if (!gesture.dragged) {
              const distance = Math.hypot(event.screenX - gesture.startX, event.screenY - gesture.startY)
              if (distance < DRAG_THRESHOLD) return
              gesture.dragged = true
              setDragging(true)
              gesture.controller = new Promise((resolve, reject) => {
                requestAnimationFrame(() => {
                  const current = gestureRef.current
                  if (!current || current.pointerId !== gesture.pointerId) {
                    reject(new Error('drag gesture ended before window geometry was ready'))
                    return
                  }
                  onDragStart(current.lastX, current.lastY).then(resolve, reject)
                })
              })
            }
            event.preventDefault()
            void gesture.controller
              ?.then((controller) => controller.move(event.screenX, event.screenY))
              .catch((error) => console.error('膠囊拖曳啟動失敗', error))
          }}
          onPointerUp={finishDrag}
          onPointerCancel={(event) => finishDrag(event, true)}
        >
          <span className="capsule-drag-handle">
            <span ref={orbRef} className="capsule-orb-anchor">
              <ForceCycleRing question={urgent} variant="capsule" />
              <ThinkingOrb size={25} />
            </span>
          </span>
          <button
            type="button"
            onClick={(event) => {
              // Pointer click 由外層 gesture 的 pointerup 統一處理；detail=0
              // 保留鍵盤 Enter / Space 的可及性。
              if (event.detail === 0) openFromOrb()
            }}
            className="capsule-open"
            aria-label="開啟決策介面"
          >
            <span className="capsule-count tabular">{t0 + t1plus}</span>
            <span className="capsule-label">
              {forceSnapshot?.warning || forceSnapshot?.overdue
                ? forceSnapshot.label
                : '待你決定'}
            </span>
          </button>
        </div>
      </div>
    </div>
  )
}
