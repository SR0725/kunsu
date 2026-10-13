// 減少動態偏好即時跟隨（BDD edge-notch AC-EN-18；QA 第 1 輪）：系統設定、app 內設定（其他視窗改 localStorage）、
// 同一視窗改 data-motion 都要立即生效。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { bindMotionAttribute, MOTION_KEY, readReducedMotion, reducedMotion, subscribeReducedMotion, type MotionEnv } from './motion'

function fakeEnv(initial: { system?: boolean; app?: string | null } = {}) {
  const mqlListeners: ((e: { matches: boolean }) => void)[] = []
  const windowListeners: Record<string, ((e: unknown) => void)[]> = {}
  const observers: (() => void)[] = []
  const store: Record<string, string> = {}
  if (initial.app) store[MOTION_KEY] = initial.app
  const mql = {
    matches: initial.system ?? false,
    addEventListener: (_t: string, cb: (e: { matches: boolean }) => void) => void mqlListeners.push(cb),
    removeEventListener: (_t: string, cb: (e: { matches: boolean }) => void) => {
      const i = mqlListeners.indexOf(cb)
      if (i >= 0) mqlListeners.splice(i, 1)
    },
  }
  const root = { dataset: {} as Record<string, string | undefined> }
  if (initial.app) root.dataset.motion = initial.app
  const env: MotionEnv = {
    matchMedia: () => mql,
    storage: {
      getItem: (k: string) => store[k] ?? null,
    },
    root,
    addEventListener: (t: string, cb: (e: unknown) => void) => void (windowListeners[t] ??= []).push(cb),
    removeEventListener: (t: string, cb: (e: unknown) => void) => {
      const list = windowListeners[t] ?? []
      const i = list.indexOf(cb)
      if (i >= 0) list.splice(i, 1)
    },
    observeMotionAttribute: (cb: () => void) => {
      observers.push(cb)
      return () => {
        const i = observers.indexOf(cb)
        if (i >= 0) observers.splice(i, 1)
      }
    },
  }
  return {
    env,
    setSystem(v: boolean) {
      mql.matches = v
      for (const cb of [...mqlListeners]) cb({ matches: v })
    },
    setAppFromOtherWindow(v: string) {
      store[MOTION_KEY] = v
      for (const cb of [...(windowListeners.storage ?? [])]) cb({ key: MOTION_KEY, newValue: v })
    },
    setAttribute(v: string) {
      root.dataset.motion = v
      store[MOTION_KEY] = v
      for (const cb of [...observers]) cb()
    },
    root,
    listenerCount: () => mqlListeners.length + (windowListeners.storage?.length ?? 0) + observers.length,
  }
}

describe('AC-EN-18 減少動態即時跟隨', () => {
  it('系統或 app 內任一設定為減少動態就算減少動態', () => {
    expect(readReducedMotion(fakeEnv().env)).toBe(false)
    expect(readReducedMotion(fakeEnv({ system: true }).env)).toBe(true)
    expect(readReducedMotion(fakeEnv({ app: 'reduced' }).env)).toBe(true)
    expect(readReducedMotion(fakeEnv({ app: 'full' }).env)).toBe(false)
  })

  it('系統設定改變立即通知', () => {
    const f = fakeEnv()
    const seen: boolean[] = []
    subscribeReducedMotion((v) => seen.push(v), f.env)
    f.setSystem(true)
    f.setSystem(false)
    expect(seen).toEqual([true, false])
  })

  it('其他視窗改了 app 內設定（storage 事件）立即通知，並同步這個視窗的 data-motion', () => {
    const f = fakeEnv()
    const seen: boolean[] = []
    subscribeReducedMotion((v) => seen.push(v), f.env)
    f.setAppFromOtherWindow('reduced')
    expect(seen).toEqual([true])
    expect(f.root.dataset.motion).toBe('reduced')
    f.setAppFromOtherWindow('full')
    expect(seen).toEqual([true, false])
    expect(f.root.dataset.motion).toBe('full')
  })

  it('同一視窗改 data-motion 也立即通知', () => {
    const f = fakeEnv()
    const seen: boolean[] = []
    subscribeReducedMotion((v) => seen.push(v), f.env)
    f.setAttribute('reduced')
    expect(seen).toEqual([true])
  })

  it('取消訂閱後不再通知、也不留下監聽', () => {
    const f = fakeEnv()
    const seen: boolean[] = []
    const off = subscribeReducedMotion((v) => seen.push(v), f.env)
    off()
    f.setSystem(true)
    f.setAppFromOtherWindow('reduced')
    expect(seen).toEqual([])
    expect(f.listenerCount()).toBe(0)
  })
})

// 決策面板 QA 第 2 輪（DESIGN「Reduced motion」）：蓋板與主視窗的 `<html data-motion>` 不只在載入時設一次，
// 在其他視窗改了「連續轉場動畫」之後也要跟著改（舊版蓋板視窗只在載入時讀一次）。
describe('bindMotionAttribute：每個視窗的 data-motion 即時跟隨 app 內設定', () => {
  it('載入時依存好的設定套上，之後其他視窗改設定也同步；解除綁定後不留監聽', () => {
    const f = fakeEnv({ app: 'reduced' })
    delete f.root.dataset.motion
    const off = bindMotionAttribute(f.env)
    expect(f.root.dataset.motion).toBe('reduced')
    f.setAppFromOtherWindow('full')
    expect(f.root.dataset.motion).toBe('full')
    off()
    expect(f.listenerCount()).toBe(0)
    f.setAppFromOtherWindow('reduced')
    expect(f.root.dataset.motion).toBe('full')
  })

  it('沒有存過設定就是 full', () => {
    const f = fakeEnv()
    bindMotionAttribute(f.env)()
    expect(f.root.dataset.motion).toBe('full')
  })
})

// QA 第 1 輪 #5（AC-DS-01）：系統「減少動態效果」或 Kunsu 設定的「連續轉場動畫」任一關閉，面板就不播形狀動畫。

function env({ os = false, dataset, stored }: { os?: boolean; dataset?: string; stored?: string | null }) {
  vi.stubGlobal('window', {
    matchMedia: (query: string) => ({ matches: query.includes('reduced-motion') ? os : false, media: query }),
  })
  vi.stubGlobal('document', { documentElement: { dataset: dataset ? { motion: dataset } : {} } })
  vi.stubGlobal('localStorage', { getItem: (key: string) => (key === 'kunsu:motion:v1' ? (stored ?? null) : null) })
}

describe('reducedMotion', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('系統減少動態', () => {
    env({ os: true })
    expect(reducedMotion()).toBe(true)
  })

  it('App 設定已套到 html[data-motion=reduced]', () => {
    env({ dataset: 'reduced' })
    expect(reducedMotion()).toBe(true)
  })

  it('App 設定已存但 html 還沒套上（蓋板視窗剛載入）', () => {
    env({ stored: 'reduced' })
    expect(reducedMotion()).toBe(true)
  })

  it('都沒關 → 播動畫', () => {
    env({ dataset: 'full', stored: 'full' })
    expect(reducedMotion()).toBe(false)
  })

  it('讀不到 localStorage 不會丟錯', () => {
    env({})
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('blocked')
      },
    })
    expect(reducedMotion()).toBe(false)
  })
})
