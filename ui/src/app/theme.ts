// 主視窗外觀（DESIGN「Application (dub.co language)」Theme；BDD settings 場景 7、application-shell 場景 8／10）。
// 跟隨系統／淺色／深色存在這台 Mac（kunsu:theme:v1）；主視窗標 <html data-surface="app" data-theme>，蓋板只標 data-theme
// （2026-10-12 統一設計語言：決策面板跟主題走，AC-DS-19）。瀏海與畫布檢查視窗什麼都不標，瀏海維持純黑。
// 首次繪製前的那一次由 index.html 的內嵌腳本（id="kunsu-theme-boot"）做，邏輯與 applyTheme 相同。

export const THEME_KEY = 'kunsu:theme:v1'
export type ThemePref = 'system' | 'light' | 'dark'
export type Theme = 'light' | 'dark'
export const DEFAULT_THEME_PREF: ThemePref = 'light'
const DARK_QUERY = '(prefers-color-scheme: dark)'

interface MediaQueryLike {
  matches: boolean
  addEventListener?: (type: 'change', cb: (e: { matches: boolean }) => void) => void
  removeEventListener?: (type: 'change', cb: (e: { matches: boolean }) => void) => void
}

/** 可注入的環境（測試用假物件；正式環境是 window／document）。 */
export interface ThemeEnv {
  search?: string
  storage?: { getItem(key: string): string | null; setItem(key: string, value: string): void }
  root?: { dataset: Record<string, string | undefined>; style: Record<string, string> | CSSStyleDeclaration }
  matchMedia?: (query: string) => MediaQueryLike
  addEventListener?: (type: string, cb: (e: unknown) => void) => void
  removeEventListener?: (type: string, cb: (e: unknown) => void) => void
  /** 下一個畫面（主題切換當下停掉轉場，下一幀恢復）。 */
  nextFrame?: (cb: () => void) => void
}

function browserEnv(): ThemeEnv {
  if (typeof window === 'undefined') return {}
  let storage: ThemeEnv['storage']
  try {
    storage = typeof localStorage !== 'undefined' ? localStorage : undefined
  } catch {
    storage = undefined
  }
  return {
    search: window.location.search,
    storage,
    root: typeof document !== 'undefined' ? document.documentElement : undefined,
    matchMedia: window.matchMedia ? (q) => window.matchMedia(q) : undefined,
    addEventListener: (t, cb) => window.addEventListener(t, cb as EventListener),
    removeEventListener: (t, cb) => window.removeEventListener(t, cb as EventListener),
    // 兩幀之後（新顏色已畫上）；視窗在背景、rAF 停住時由計時器保底，轉場不會一直被關著。
    nextFrame: (cb) => {
      let done = false
      const run = () => {
        if (done) return
        done = true
        cb()
      }
      window.requestAnimationFrame(() => window.requestAnimationFrame(run))
      window.setTimeout(run, 120)
    },
  }
}

export function parseThemePref(raw: unknown): ThemePref {
  return raw === 'system' || raw === 'light' || raw === 'dark' ? raw : DEFAULT_THEME_PREF
}

export function resolveTheme(pref: ThemePref, systemDark: boolean): Theme {
  if (pref === 'system') return systemDark ? 'dark' : 'light'
  return pref
}

/** 主視窗才是 application view；蓋板（overlay）、瀏海（capsule）與畫布檢查視窗（canvas-check）不是。 */
export function isApplicationView(search: string): boolean {
  const view = new URLSearchParams(search).get('view')
  return view !== 'overlay' && view !== 'capsule' && view !== 'canvas-check'
}

export function readThemePref(env: ThemeEnv = browserEnv()): ThemePref {
  try {
    return parseThemePref(env.storage?.getItem(THEME_KEY))
  } catch {
    return DEFAULT_THEME_PREF
  }
}

const systemDark = (env: ThemeEnv) => env.matchMedia?.(DARK_QUERY).matches === true

/** 跟著外觀設定的視窗：主視窗與蓋板（統一設計語言，AC-DS-19）；瀏海與畫布檢查視窗不跟。 */
export function isThemedView(search: string): boolean {
  return isApplicationView(search) || new URLSearchParams(search).get('view') === 'overlay'
}

/** 依存好的偏好套到 <html>；主視窗另標 data-surface=app，蓋板只標 data-theme（舞台維持透明）；其他視窗回 null。 */
export function applyTheme(env: ThemeEnv = browserEnv(), pref: ThemePref = readThemePref(env)): Theme | null {
  const search = env.search ?? ''
  if (!isThemedView(search) || !env.root) return null
  const theme = resolveTheme(pref, systemDark(env))
  if (isApplicationView(search)) env.root.dataset.surface = 'app'
  if (env.root.dataset.theme !== theme) env.root.dataset.theme = theme
  ;(env.root.style as Record<string, string>).colorScheme = theme
  return theme
}

/** 改外觀：存在這台 Mac、立即套用；切換當下停掉轉場（主題切換是瞬間的，DESIGN「Motion」）。 */
export function setThemePref(pref: ThemePref, env: ThemeEnv = browserEnv()): Theme | null {
  try {
    env.storage?.setItem(THEME_KEY, pref)
  } catch {
    // 存不了也照樣換（只是下次開不會記得）
  }
  if (env.root && isApplicationView(env.search ?? '')) {
    env.root.dataset.themeSwitching = 'true'
    const theme = applyTheme(env, pref)
    const clear = () => {
      if (env.root) delete env.root.dataset.themeSwitching
    }
    if (env.nextFrame) env.nextFrame(clear)
    else clear()
    return theme
  }
  return null
}

/** 主視窗開著時：跟隨系統的深淺、其他主視窗改了偏好都即時同步。回傳解除。 */
export function bindTheme(env: ThemeEnv = browserEnv()): () => void {
  applyTheme(env)
  const mql = env.matchMedia?.(DARK_QUERY)
  const onMedia = () => {
    if (readThemePref(env) === 'system') applyTheme(env)
  }
  const onStorage = (e: unknown) => {
    const ev = e as { key?: string | null }
    if (ev.key === THEME_KEY || ev.key === null) applyTheme(env)
  }
  mql?.addEventListener?.('change', onMedia)
  env.addEventListener?.('storage', onStorage)
  return () => {
    mql?.removeEventListener?.('change', onMedia)
    env.removeEventListener?.('storage', onStorage)
  }
}
