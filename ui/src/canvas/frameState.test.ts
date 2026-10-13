// 畫布載入狀態（AC-EC-09）：錯誤或 2 秒內沒畫完 → 隱藏並顯示原因，決定不受影響。
import { describe, expect, it } from 'vitest'
import { READY_TIMEOUT_MS, failureText, frameReducer, initialFrame } from './frameState'

describe('frameReducer', () => {
  it('ready 之後跟著 resize 調整高度', () => {
    let s = frameReducer(initialFrame, { type: 'message', message: { type: 'resize', height: 100 } })
    expect(s.phase).toBe('loading')
    s = frameReducer(s, { type: 'message', message: { type: 'ready', height: 300, overflowX: 0, tall: false } })
    expect(s).toEqual({ phase: 'ready', height: 300 })
    s = frameReducer(s, { type: 'message', message: { type: 'resize', height: 480 } })
    expect(s).toEqual({ phase: 'ready', height: 480 })
    expect(frameReducer(s, { type: 'timeout' })).toEqual(s)
  })

  it('錯誤或逾時 → 失敗，之後的 ready 不再生效', () => {
    const issues = [{ file: 'canvas.tsx', line: 2, column: 5, code: 'canvas_runtime', message: 'x is not defined', hint: '' }]
    let s = frameReducer(initialFrame, { type: 'message', message: { type: 'error', issues } })
    expect(s.phase).toBe('failed')
    expect(failureText(s)).toBe('圖表載入失敗：x is not defined（canvas.tsx 第 2 行）')
    s = frameReducer(s, { type: 'message', message: { type: 'ready', height: 300, overflowX: 0, tall: false } })
    expect(s.phase).toBe('failed')

    const timedOut = frameReducer(initialFrame, { type: 'timeout' })
    expect(failureText(timedOut)).toBe(`圖表載入失敗：${READY_TIMEOUT_MS / 1000} 秒內沒有畫完`)
  })
})

describe('時限一致', () => {
  it('註冊檢查與正式顯示用同一個時限（檢查過了，使用者那邊也畫得出來）', async () => {
    const { CHECK_TIMEOUT_MS } = await import('./frameState')
    expect(CHECK_TIMEOUT_MS).toBe(READY_TIMEOUT_MS)
  })
})
