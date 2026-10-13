import { describe, expect, it } from 'vitest'
import { shouldOpenCapsuleAfterPointer } from './capsuleGesture'

describe('capsule pointer gesture', () => {
  it('opens on a completed click but not after drag or cancellation', () => {
    expect(shouldOpenCapsuleAfterPointer(false, false)).toBe(true)
    expect(shouldOpenCapsuleAfterPointer(true, false)).toBe(false)
    expect(shouldOpenCapsuleAfterPointer(false, true)).toBe(false)
  })
})
