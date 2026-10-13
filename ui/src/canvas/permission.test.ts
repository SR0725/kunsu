// 執行畫布前的殼層許可（BDD evidence-canvas AC-EC-09、09b）：許可以「題目＋版本」為準；IPC 失敗視同不許可。
import { describe, expect, it } from 'vitest'
import frameSource from './CanvasFrame.tsx?raw'
import { askPermission, canvasFailure, currentPermission, permissionKey, refusalText, runtimeErrorText, shouldRegister } from './permission'
import { initialFrame } from './frameState'

// QA 第 5 輪（AC-EC-09、09b）：runtime 讀不到時，面板先登記了 canvas_active 卻永遠不回心跳，3 秒後被判卡住——終止網頁行程、
// 重新載入（使用者打到一半的補充說明消失）、停用這一版、原因顯示「畫布沒有回應」、canvas_failed 記兩次。
describe('runtime 載入成功、真的要執行畫布時才向殼層登記', () => {
  it('runtime 還在讀或讀不到時不登記；不需要問殼層或沒有畫布時也不登記', () => {
    expect(shouldRegister(true, true, 'loading')).toBe(false)
    expect(shouldRegister(true, true, 'failed')).toBe(false)
    expect(shouldRegister(true, true, 'ready')).toBe(true)
    expect(shouldRegister(false, true, 'ready')).toBe(false)
    expect(shouldRegister(true, false, 'ready')).toBe(false)
  })

  it('CanvasFrame 呼叫 canvas_active 的 effect 以 shouldRegister 把關，且跟著 runtime 的載入狀態重跑', () => {
    const effect = frameSource.split('useEffect(').find((part) => part.includes('canvasActive('))
    expect(effect, '找不到呼叫 canvasActive 的 effect').toBeDefined()
    expect(effect).toMatch(/if \(!shouldRegister\(needsPermission, Boolean\(canvas\), runtimeLoad\)/)
    expect(effect).toMatch(/\[[^\]]*\bruntimeLoad\b[^\]]*\]\)/)
  })

  it('runtime 讀不到：顯示讀取失敗的原因（不是「畫布沒有回應」）；殼層拒絕或問不到才顯示拒絕原因', () => {
    expect(canvasFailure(initialFrame, '畫布 runtime 讀取失敗（404）', 'pending')).toBe('圖表載入失敗：畫布 runtime 讀取失敗（404）')
    expect(canvasFailure(initialFrame, null, 'refused')).toBe(refusalText('refused'))
    expect(canvasFailure(initialFrame, null, 'unavailable')).toBe(refusalText('unavailable'))
    expect(canvasFailure(initialFrame, null, 'granted')).toBeNull()
    expect(canvasFailure({ phase: 'failed', reason: '畫布換頁了', issues: [] }, null, 'granted')).toBe('圖表載入失敗：畫布換頁了')
  })

  it('runtime 讀取失敗的原因只取訊息（不帶「Error:」前綴）', () => {
    expect(runtimeErrorText(new Error('畫布 runtime 讀取失敗（404）'))).toBe('畫布 runtime 讀取失敗（404）')
    expect(runtimeErrorText('網路斷了')).toBe('網路斷了')
    expect(runtimeErrorText(new Error(''))).toBe('畫布 runtime 讀取失敗')
    expect(frameSource).toMatch(/setRuntimeError\(runtimeErrorText\(e\)\)/)
  })
})

describe('畫布執行許可', () => {
  it('許可只對拿到它的那一題那一版有效：換版後在新版拿到許可前一律 pending（不執行）', () => {
    const v1 = permissionKey('q1', 1)
    const v2 = permissionKey('q1', 2)
    expect(v1).not.toBe(v2)
    expect(permissionKey('q1', 1)).not.toBe(permissionKey('q2', 1))
    const held = { key: v1, value: 'granted' as const }
    expect(currentPermission(true, held, v1)).toBe('granted')
    expect(currentPermission(true, held, v2)).toBe('pending')
    expect(currentPermission(true, null, v1)).toBe('pending')
    expect(currentPermission(true, { key: v2, value: 'refused' }, v2)).toBe('refused')
  })

  it('不需要問殼層（檢查模式、沒有 IPC）時直接可以執行', () => {
    expect(currentPermission(false, null, permissionKey('q1', 1))).toBe('granted')
  })

  it('殼層回 true 才可以執行；false 或其他值是拒絕；IPC 失敗視同拒絕（無法確認）', async () => {
    expect(await askPermission(async () => true)).toBe('granted')
    expect(await askPermission(async () => false)).toBe('refused')
    expect(await askPermission(async () => null as unknown as boolean)).toBe('refused')
    expect(await askPermission(() => Promise.reject(new Error('IPC 失敗')))).toBe('unavailable')
    expect(
      await askPermission(() => {
        throw new Error('同步丟出')
      }),
    ).toBe('unavailable')
  })

  it('拒絕與無法確認各有原因文字', () => {
    expect(refusalText('refused')).toBe('圖表載入失敗：畫布沒有回應')
    expect(refusalText('unavailable')).toBe('圖表載入失敗：無法確認畫布可以執行')
  })
})
