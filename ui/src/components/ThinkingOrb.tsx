import type { CSSProperties } from 'react'
import { THINKING_ORB_DOT_COUNT, thinkingOrbDot } from './agentParticle'

export function ThinkingOrb({ size = 28, active = true }: { size?: number; active?: boolean }) {
  const dots = Array.from({ length: THINKING_ORB_DOT_COUNT }, (_, index) => ({
    ...thinkingOrbDot(index),
    delay: `${-(index * 70)}ms`,
  }))

  return (
    <span
      className={`thinking-orb ${active ? 'thinking-orb--active' : ''}`}
      style={{ '--orb-size': `${size}px` } as CSSProperties}
      aria-hidden="true"
    >
      <svg viewBox="0 0 100 100" role="presentation">
        {dots.map((dot, index) => (
          <circle
            key={index}
            cx={dot.x}
            cy={dot.y}
            r={dot.radius}
            style={{ animationDelay: dot.delay, opacity: dot.opacity }}
          />
        ))}
      </svg>
    </span>
  )
}
