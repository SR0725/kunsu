import { useEffect, useState } from 'react'
import { forceCycleSnapshot } from '../forceCycle'
import type { Question } from '../types'

export function ForceCycleRing({
  question,
  variant,
}: {
  question: Question
  variant: 'capsule' | 'hero'
}) {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])
  const snapshot = forceCycleSnapshot(question, now)
  if (!snapshot) return null
  const dash = `${snapshot.progress * 100} 100`
  return (
    <div
      className={`force-cycle-ring force-cycle-ring--${variant}`}
      data-warning={snapshot.warning ? 'true' : 'false'}
      data-overdue={snapshot.overdue ? 'true' : 'false'}
      role="timer"
      aria-live="off"
      aria-label={snapshot.label}
    >
      <svg viewBox="0 0 100 100" aria-hidden="true">
        <circle className="force-cycle-ring__track" cx="50" cy="50" r="46" pathLength="100" />
        <circle
          className="force-cycle-ring__value"
          cx="50"
          cy="50"
          r="46"
          pathLength="100"
          strokeDasharray={dash}
        />
      </svg>
      {variant === 'hero' && (
        <span className="force-cycle-ring__label">
          <strong>{snapshot.overdue ? '現在' : `${Math.max(1, Math.ceil(snapshot.remainingMs / 60_000))} 分`}</strong>
          <small>{snapshot.overdue ? '等待你的回答' : snapshot.warning ? '後強制詢問' : '回答時限'}</small>
        </span>
      )}
      {(snapshot.warning || snapshot.overdue) && (
        <span className="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {snapshot.overdue
            ? '回答期限已到，現在將開啟強制詢問'
            : '剩餘 3 分鐘，之後將開啟強制詢問'}
        </span>
      )}
    </div>
  )
}
