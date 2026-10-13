import { describe, expect, it } from 'vitest'
import { overlayShellAction, shouldAckMount } from './overlayShell'

describe('overlay shell 呈現決策（AC-OP-04／09）', () => {
  it('沒有轉場時只等待，絕不渲染空白蓋板', () => {
    expect(overlayShellAction({ loaded: true, questionCount: 3, hasTransition: false })).toBe('wait')
  })

  it('題目清單尚未載入時不自動關閉', () => {
    expect(overlayShellAction({ loaded: false, questionCount: 0, hasTransition: true })).toBe('wait')
  })

  it('清單已載入且確實沒有題目才關閉', () => {
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true })).toBe('close')
  })

  // QA 第 1 輪 #2（AC-OP-07、AC-DS-09）：在面板裡答完最後一題，回執要留著，直到完成或 Esc。
  it('面板保留這次 session 在面板裡作答的回執時，清單空了也不關閉', () => {
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true, holdingReceipt: true })).toBe('render')
  })

  it('沒有回執要保留時，題目全由其他通道結案仍自動關閉', () => {
    expect(overlayShellAction({ loaded: true, questionCount: 0, hasTransition: true, holdingReceipt: false })).toBe('close')
  })

  it('有轉場且有題目才渲染', () => {
    expect(overlayShellAction({ loaded: true, questionCount: 1, hasTransition: true })).toBe('render')
    expect(overlayShellAction({ loaded: false, questionCount: 2, hasTransition: true })).toBe('render')
  })
})

describe('掛載確認（AC-OP-04）', () => {
  it('每個轉場只回報一次，且必須有題目內容', () => {
    expect(shouldAckMount({ transitionId: 4, ackedId: null, questionCount: 1 })).toBe(true)
    expect(shouldAckMount({ transitionId: 4, ackedId: 4, questionCount: 1 })).toBe(false)
    expect(shouldAckMount({ transitionId: 5, ackedId: 4, questionCount: 1 })).toBe(true)
    expect(shouldAckMount({ transitionId: 5, ackedId: null, questionCount: 0 })).toBe(false)
    expect(shouldAckMount({ transitionId: null, ackedId: null, questionCount: 3 })).toBe(false)
  })
})
