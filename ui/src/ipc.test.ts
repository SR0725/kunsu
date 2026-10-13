import { describe, expect, it } from 'vitest'
import { capsuleDockCorner, capsuleDragTarget, isCapsuleDock } from './ipc'

describe('capsuleDragTarget', () => {
  it('converts CSS pointer delta into physical window coordinates on Retina', () => {
    expect(
      capsuleDragTarget(
        { x: 16, y: 66, scale_factor: 2 },
        40,
        50,
        140,
        170,
      ),
    ).toEqual({ x: 216, y: 306 })
  })
})

describe('capsule dock placement', () => {
  it('accepts every edge orientation and rejects invalid persisted values', () => {
    for (const dock of [
      'top-left',
      'top-right',
      'bottom-left',
      'bottom-right',
      'left-top',
      'left-bottom',
      'right-top',
      'right-bottom',
    ]) {
      expect(isCapsuleDock(dock)).toBe(true)
    }
    expect(isCapsuleDock('center')).toBe(false)
    expect(isCapsuleDock(null)).toBe(false)
  })

  it('projects edge placements to an overlay animation quadrant', () => {
    expect(capsuleDockCorner('left-bottom')).toBe('bottom-left')
    expect(capsuleDockCorner('right-top')).toBe('top-right')
    expect(capsuleDockCorner('bottom-right')).toBe('bottom-right')
  })
})
