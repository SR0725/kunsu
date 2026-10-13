// 減少動態偏好（DESIGN「Motion」、BDD edge-notch 場景 18）：系統設定或 app 內設定任一要求就減少動態，兩者改變都
// 即時生效。app 內設定存在 localStorage（所有視窗共用），本視窗的 `<html data-motion>` 給 CSS 用。

export const MOTION_KEY = 'kunsu:motion:v1'
/** 決策面板用的舊名（同一個鍵）。 */
export const MOTION_STORAGE_KEY = MOTION_KEY
const QUERY = '(prefers-reduced-motion: reduce)'

interface MediaQueryLike {
  matches: boolean
  addEventListener?: (type: 'change', cb: (e: { matches: boolean }) => void) => void
  removeEventListener?: (type: 'change', cb: (e: { matches: boolean }) => void) => void
}

/** 可注入的環境（測試用假物件；正式環境是 window／document）。 */
export interface MotionEnv {
  matchMedia?: (query: string) => MediaQueryLike
  storage?: { getItem(key: string): string | null }
  root?: { dataset: Record<string, string | undefined> }
  addEventListener?: (type: string, cb: (e: unknown) => void) => void
  removeEventListener?: (type: string, cb: (e: unknown) => void) => void
  /** 本視窗的 `data-motion` 屬性改變時呼叫 cb；回傳取消函式。 */
  observeMotionAttribute?: (cb: () => void) => () => void
}

function browserEnv(): MotionEnv {
  if (typeof window === 'undefined') return {}
  let storage: MotionEnv['storage']
  try {
    storage = typeof localStorage !== 'undefined' ? localStorage : undefined
  } catch {
    storage = undefined
  }
  const root = typeof document !== 'undefined' ? document.documentElement : undefined
  return {
    matchMedia: window.matchMedia ? (q) => window.matchMedia(q) : undefined,
    storage,
    root,
    addEventListener: (t, cb) => window.addEventListener(t, cb as EventListener),
    removeEventListener: (t, cb) => window.removeEventListener(t, cb as EventListener),
    observeMotionAttribute:
      root && typeof MutationObserver !== 'undefined'
        ? (cb) => {
            const mo = new MutationObserver(cb)
            mo.observe(root, { attributes: true, attributeFilter: ['data-motion'] })
            return () => mo.disconnect()
          }
        : undefined,
  }
}

function appSetting(env: MotionEnv): string | undefined {
  let stored: string | null = null
  try {
    stored = env.storage?.getItem(MOTION_KEY) ?? null
  } catch {
    stored = null
  }
  return stored ?? env.root?.dataset.motion
}

/** 現在是否該減少動態（每次呼叫都重新讀，不快取）。 */
export function readReducedMotion(env: MotionEnv = browserEnv()): boolean {
  const system = env.matchMedia?.(QUERY).matches === true
  return system || appSetting(env) === 'reduced'
}

/** 訂閱減少動態偏好的變化（系統設定、其他視窗改 app 內設定、本視窗改 data-motion）；值改變才呼叫 cb。 */
export function subscribeReducedMotion(cb: (reduced: boolean) => void, env: MotionEnv = browserEnv()): () => void {
  let last = readReducedMotion(env)
  const check = () => {
    const now = readReducedMotion(env)
    if (now === last) return
    last = now
    cb(now)
  }
  const mql = env.matchMedia?.(QUERY)
  const onMedia = () => check()
  mql?.addEventListener?.('change', onMedia)
  const onStorage = (e: unknown) => {
    const ev = e as { key?: string | null; newValue?: string | null }
    if (ev.key !== MOTION_KEY) return
    // 其他視窗改了設定：同步本視窗給 CSS 用的屬性。
    if (env.root) env.root.dataset.motion = ev.newValue === 'reduced' ? 'reduced' : 'full'
    check()
  }
  env.addEventListener?.('storage', onStorage)
  const stopObserving = env.observeMotionAttribute?.(check)
  return () => {
    mql?.removeEventListener?.('change', onMedia)
    env.removeEventListener?.('storage', onStorage)
    stopObserving?.()
  }
}

/**
 * 本視窗的 `<html data-motion>` 跟著 app 內設定走（DESIGN「Reduced motion」）：載入時依存好的設定套上，之後其他視窗
 * 改了設定（storage 事件）也即時同步——蓋板視窗以前只在載入時讀一次。回傳解除綁定。
 */
export function bindMotionAttribute(env: MotionEnv = browserEnv()): () => void {
  if (env.root) env.root.dataset.motion = appSetting(env) === 'reduced' ? 'reduced' : 'full'
  return subscribeReducedMotion(() => {}, env)
}

/** 決策面板（DESIGN「Decision surface」Reduced motion、AC-DS-01）：蓋板視窗剛載入時 html 的 data-motion 可能還沒套上，
 * 所以每次都直接讀設定。 */
export function reducedMotion(): boolean {
  return readReducedMotion()
}
