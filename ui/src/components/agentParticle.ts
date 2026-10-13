export const THINKING_ORB_DOT_COUNT = 18

export interface ThinkingOrbDot {
  x: number
  y: number
  radius: number
  opacity: number
}

/** 膠囊 SVG 與全螢幕 handoff 共用同一組 18 點幾何。 */
export function thinkingOrbDot(index: number): ThinkingOrbDot {
  const slot = ((index % THINKING_ORB_DOT_COUNT) + THINKING_ORB_DOT_COUNT) % THINKING_ORB_DOT_COUNT
  const angle = (slot / THINKING_ORB_DOT_COUNT) * Math.PI * 2
  const ring = slot % 3
  const radius = 34 + ring * 9
  return {
    x: 50 + Math.cos(angle) * radius,
    y: 50 + Math.sin(angle) * radius * (0.48 + ring * 0.08),
    radius: slot % 3 === 0 ? 2.7 : 2.1,
    opacity: 0.35 + ring * 0.22,
  }
}

export function particleSeedFromOrb(
  index: number,
  origin: { x: number; y: number; size: number },
): { x: number; y: number } {
  const dot = thinkingOrbDot(index)
  return {
    x: origin.x + ((dot.x - 50) / 100) * origin.size,
    y: origin.y + ((dot.y - 50) / 100) * origin.size,
  }
}

