// 主視窗外觀（BDD settings 場景 7、application-shell 場景 8／10；DESIGN「Application (dub.co language)」Theme）：
// 三選一存在這台 Mac（kunsu:theme:v1），首次繪製前套到 <html data-theme>；只套在主視窗，蓋板與瀏海維持黑色系統。
import indexHtml from '../../index.html?raw'
import { describe, expect, it } from 'vitest'
import {
  DEFAULT_THEME_PREF,
  THEME_KEY,
  applyTheme,
  bindTheme,
  isApplicationView,
  parseThemePref,
  resolveTheme,
  setThemePref,
  type ThemeEnv,
} from './theme'

function fakeEnv(opts: { search?: string; stored?: string | null; systemDark?: boolean } = {}) {
  const store = new Map<string, string>()
  if (opts.stored != null) store.set(THEME_KEY, opts.stored)
  const dataset: Record<string, string | undefined> = {}
  const style: Record<string, string> = {}
  let systemDark = opts.systemDark ?? false
  const mediaListeners = new Set<(e: { matches: boolean }) => void>()
  const windowListeners = new Map<string, Set<(e: unknown) => void>>()
  const env: ThemeEnv = {
    search: opts.search ?? '',
    storage: {
      getItem: (k) => store.get(k) ?? null,
      setItem: (k, v) => void store.set(k, v),
    },
    root: { dataset, style },
    matchMedia: () => ({
      get matches() {
        return systemDark
      },
      addEventListener: (_t, cb) => void mediaListeners.add(cb),
      removeEventListener: (_t, cb) => void mediaListeners.delete(cb),
    }),
    addEventListener: (t, cb) => {
      if (!windowListeners.has(t)) windowListeners.set(t, new Set())
      windowListeners.get(t)!.add(cb)
    },
    removeEventListener: (t, cb) => void windowListeners.get(t)?.delete(cb),
    nextFrame: (cb) => cb(),
  }
  return {
    env,
    dataset,
    style,
    store,
    setSystemDark(value: boolean) {
      systemDark = value
      mediaListeners.forEach((cb) => cb({ matches: value }))
    },
    fireStorage(key: string, newValue: string | null) {
      if (newValue === null) store.delete(key)
      else store.set(key, newValue)
      windowListeners.get('storage')?.forEach((cb) => cb({ key, newValue }))
    },
    listenerCount: () => mediaListeners.size + (windowListeners.get('storage')?.size ?? 0),
  }
}

describe('外觀偏好', () => {
  it('預設淺色；只接受 system／light／dark，其餘一律當預設', () => {
    expect(DEFAULT_THEME_PREF).toBe('light')
    expect(parseThemePref('dark')).toBe('dark')
    expect(parseThemePref('system')).toBe('system')
    expect(parseThemePref('light')).toBe('light')
    expect(parseThemePref(null)).toBe('light')
    expect(parseThemePref('purple')).toBe('light')
  })

  it('跟隨系統時依 macOS 深淺決定', () => {
    expect(resolveTheme('system', true)).toBe('dark')
    expect(resolveTheme('system', false)).toBe('light')
    expect(resolveTheme('dark', false)).toBe('dark')
    expect(resolveTheme('light', true)).toBe('light')
  })

  it('只有主視窗算 application view；蓋板、瀏海與畫布檢查視窗不是', () => {
    expect(isApplicationView('')).toBe(true)
    expect(isApplicationView('?empty')).toBe(true)
    expect(isApplicationView('?view=overlay')).toBe(false)
    expect(isApplicationView('?view=capsule')).toBe(false)
    expect(isApplicationView('?view=canvas-check')).toBe(false)
  })
})

describe('套用到 <html>', () => {
  it('主視窗：標上 data-surface=app 與 data-theme，color-scheme 跟著主題', () => {
    const f = fakeEnv({ stored: 'dark' })
    expect(applyTheme(f.env)).toBe('dark')
    expect(f.dataset.surface).toBe('app')
    expect(f.dataset.theme).toBe('dark')
    expect(f.style.colorScheme).toBe('dark')
  })

  it('瀏海與畫布檢查：什麼都不標（瀏海維持純黑；蓋板跟主題見 AC-DS-19）', () => {
    for (const search of ['?view=capsule', '?view=canvas-check']) {
      const f = fakeEnv({ search, stored: 'light' })
      expect(applyTheme(f.env)).toBeNull()
      expect(f.dataset.surface).toBeUndefined()
      expect(f.dataset.theme).toBeUndefined()
    }
  })

  it('改設定：存進這台 Mac 並立即換主題，切換當下停掉轉場（主題切換是瞬間的）', () => {
    const f = fakeEnv()
    const frames: (() => void)[] = []
    f.env.nextFrame = (cb) => void frames.push(cb)
    expect(setThemePref('dark', f.env)).toBe('dark')
    expect(f.store.get(THEME_KEY)).toBe('dark')
    expect(f.dataset.theme).toBe('dark')
    expect(f.dataset.themeSwitching).toBe('true')
    frames.forEach((cb) => cb())
    expect(f.dataset.themeSwitching).toBeUndefined()
  })

  it('跟隨系統時 macOS 換深淺立即跟著換；其他主視窗改了設定也同步；解除後不再監聽', () => {
    const f = fakeEnv({ stored: 'system', systemDark: false })
    const stop = bindTheme(f.env)
    expect(f.dataset.theme).toBe('light')
    f.setSystemDark(true)
    expect(f.dataset.theme).toBe('dark')
    f.fireStorage(THEME_KEY, 'light')
    expect(f.dataset.theme).toBe('light')
    stop()
    expect(f.listenerCount()).toBe(0)
  })

  it('localStorage 讀不到（例外）時用預設淺色，不崩潰', () => {
    const f = fakeEnv()
    f.env.storage = {
      getItem: () => {
        throw new Error('denied')
      },
      setItem: () => {
        throw new Error('denied')
      },
    }
    expect(applyTheme(f.env)).toBe('light')
    expect(setThemePref('dark', f.env)).toBe('dark')
  })
})

describe('首次繪製前套用（AC-ST-07）', () => {
  const html = indexHtml
  const boot = html.match(/<script id="kunsu-theme-boot">([\s\S]*?)<\/script>/)?.[1] ?? ''

  it('index.html 有同步的內嵌腳本，排在 app 模組之前', () => {
    expect(boot.length, 'index.html 要有 <script id="kunsu-theme-boot">').toBeGreaterThan(0)
    expect(boot).toContain(THEME_KEY)
    expect(html.indexOf('kunsu-theme-boot')).toBeLessThan(html.indexOf('src="/src/main.tsx"'))
  })

  function runBoot(search: string, stored: string | null, systemDark = false, throws = false) {
    const dataset: Record<string, string> = {}
    const style: Record<string, string> = {}
    const window = {
      location: { search },
      localStorage: {
        getItem: (k: string) => {
          if (throws) throw new Error('denied')
          return k === THEME_KEY ? stored : null
        },
      },
      matchMedia: (q: string) => ({ matches: q.includes('dark') ? systemDark : false }),
    }
    const document = { documentElement: { dataset, style } }
    new Function('window', 'document', 'localStorage', boot)(window, document, window.localStorage)
    return { dataset, style }
  }

  it('主視窗：依存好的偏好與系統深淺標上 data-surface／data-theme', () => {
    expect(runBoot('', null).dataset).toMatchObject({ surface: 'app', theme: 'light' })
    expect(runBoot('', 'dark').dataset).toMatchObject({ surface: 'app', theme: 'dark' })
    expect(runBoot('', 'system', true).dataset).toMatchObject({ surface: 'app', theme: 'dark' })
    expect(runBoot('', 'system', false).dataset).toMatchObject({ surface: 'app', theme: 'light' })
    expect(runBoot('', 'dark').style.colorScheme).toBe('dark')
    expect(runBoot('', 'x', false, true).dataset).toMatchObject({ surface: 'app', theme: 'light' })
  })

  it('瀏海與畫布檢查視窗：完全不動（蓋板跟主題，見 AC-DS-19）', () => {
    for (const search of ['?view=capsule', '?view=canvas-check']) {
      expect(runBoot(search, 'light').dataset).toEqual({})
    }
  })
})
