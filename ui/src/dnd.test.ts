import { describe, expect, it } from 'vitest'
import { dndLabel, normalizeDndMinutes } from './dnd'

describe('do not disturb UI contract', () => {
  it('accepts the inclusive 1..1440 minute boundary and rejects invalid durations', () => {
    expect(normalizeDndMinutes('1')).toBe(1)
    expect(normalizeDndMinutes('1440')).toBe(1440)
    expect(normalizeDndMinutes('0')).toBeNull()
    expect(normalizeDndMinutes('1441')).toBeNull()
    expect(normalizeDndMinutes('1.5')).toBeNull()
    expect(normalizeDndMinutes('nope')).toBeNull()
  })

  it('uses explicit text for active status instead of color alone', () => {
    const label = dndLabel({
      active: true,
      starts_at: '2026-08-12T18:00:00Z',
      ends_at: '2026-08-12T20:14:00Z',
      remaining_seconds: 8040,
      actor: 'ui',
    }, new Date('2026-08-12T18:00:00Z'))
    expect(label).toContain('勿擾至')
    expect(label).toContain('剩 2 小時 14 分')
  })
})
