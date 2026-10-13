// AC-OP-02（決策面板 QA 第 2 輪）：題目自動到期或從選單列開啟時沒有點擊座標，蓋板要從瀏海圖示中心展開，不是固定的
// 左上角。瀏海頁面在命中區更新後量來源圖示的中心回報給原生（變動超過 0.5 px 才送）。
import { describe, expect, it, vi } from 'vitest'
import type { KunsuIpc } from './ipc'
import { anchorFromRect, anchorMoved, withNotchAnchorReport } from './notchAnchor'

describe('瀏海圖示中心回報（AC-OP-02）', () => {
  it('圖示外框的中心與大小（舞台本地 CSS px）', () => {
    expect(anchorFromRect({ left: 1490, top: 300, width: 18, height: 18 })).toEqual({ x: 1499, y: 309, size: 18 })
  })

  it('變動超過 0.5 px 才算移動；第一次一定送', () => {
    const a = { x: 10, y: 10, size: 18 }
    expect(anchorMoved(null, a)).toBe(true)
    expect(anchorMoved(a, { x: 10.4, y: 9.6, size: 18 })).toBe(false)
    expect(anchorMoved(a, { x: 10.6, y: 10, size: 18 })).toBe(true)
    expect(anchorMoved(a, { x: 10, y: 10, size: 20 })).toBe(true)
  })

  it('每次回報命中區後量一次圖示並回報；沒移動不重送；量不到（瀏海沒畫出來）就不送', async () => {
    const edgeHit = vi.fn(async () => {})
    const edgeAnchor = vi.fn(async () => {})
    const ipc = { edgeHit, edgeAnchor, isMock: false } as unknown as KunsuIpc
    let rect: { left: number; top: number; width: number; height: number } | null = { left: 0, top: 300, width: 18, height: 18 }
    const tasks: (() => void)[] = []
    const wrapped = withNotchAnchorReport(ipc, () => (rect ? anchorFromRect(rect) : null), (task) => void tasks.push(task))

    await wrapped.edgeHit([[0, 0, 32, 0, 32, 100]], false)
    expect(edgeHit).toHaveBeenCalledWith([[0, 0, 32, 0, 32, 100]], false)
    expect(edgeAnchor).not.toHaveBeenCalled()
    tasks.shift()!()
    expect(edgeAnchor).toHaveBeenCalledWith({ x: 9, y: 309, size: 18 })

    await wrapped.edgeHit([[0, 0, 32, 0, 32, 101]], false)
    tasks.shift()!()
    expect(edgeAnchor).toHaveBeenCalledTimes(1)

    rect = { left: 1890, top: 1100, width: 18, height: 18 }
    await wrapped.edgeHit([[1888, 1080, 1920, 1080]], false)
    tasks.shift()!()
    expect(edgeAnchor).toHaveBeenLastCalledWith({ x: 1899, y: 1109, size: 18 })

    rect = null
    await wrapped.edgeHit([], false)
    tasks.shift()!()
    expect(edgeAnchor).toHaveBeenCalledTimes(2)
  })

  it('拖曳中不量（起點只在靜止時有用；拖曳每一幀都會回報命中區），放開後的更新才量', async () => {
    const edgeAnchor = vi.fn(async () => {})
    const ipc = { edgeHit: vi.fn(async () => {}), edgeAnchor, isMock: false } as unknown as KunsuIpc
    const tasks: (() => void)[] = []
    const wrapped = withNotchAnchorReport(ipc, () => ({ x: 9, y: 309, size: 18 }), (task) => void tasks.push(task))
    await wrapped.edgeHit([[0, 0, 32, 0]], true)
    await wrapped.edgeHit([[0, 5, 32, 5]], true)
    expect(tasks).toHaveLength(0)
    await wrapped.edgeHit([[0, 9, 32, 9]], false)
    expect(tasks).toHaveLength(1)
    tasks.shift()!()
    expect(edgeAnchor).toHaveBeenCalledTimes(1)
  })
})
