// 決策面板改用主視窗的設計語言（DESIGN「Unified design language」；BDD decision-surface AC-DS-18～21、application-shell AC-AS-10）。
// 面板、題組與蓋板跟著「外觀」設定走淺色／深色；顏色全部來自 --s-* token；瀏海維持純黑。
import { describe, expect, it } from 'vitest'
import indexCss from '../index.css?raw'
import batchCss from './batch.css?raw'
import surfaceTsx from './DecisionSurface.tsx?raw'
import indexHtml from '../../index.html?raw'
import { applyTheme, THEME_KEY, type ThemeEnv } from '../app/theme'

interface Rule {
  selector: string
  body: string
  at: string | null
}

function parse(source: string): Rule[] {
  const text = source.replace(/\/\*[\s\S]*?\*\//g, '')
  const out: Rule[] = []
  const walk = (chunk: string, at: string | null) => {
    let i = 0
    while (i < chunk.length) {
      const open = chunk.indexOf('{', i)
      if (open < 0) break
      const head = chunk.slice(i, open).trim()
      let depth = 1
      let j = open + 1
      while (j < chunk.length && depth > 0) {
        if (chunk[j] === '{') depth++
        else if (chunk[j] === '}') depth--
        j++
      }
      const inner = chunk.slice(open + 1, j - 1)
      if (head.startsWith('@keyframes') || head.startsWith('@font-face')) {
        // 不是選擇器
      } else if (head.startsWith('@')) walk(inner, head)
      else out.push({ selector: head, body: inner, at })
      i = j
    }
  }
  walk(text, null)
  return out
}

const rules = parse(indexCss).concat(parse(batchCss))
const selectorsOf = (r: Rule) => r.selector.split(',').map((s) => s.trim()).filter(Boolean)
function decl(selector: string, prop: string): string | undefined {
  for (const r of [...rules].reverse()) {
    if (r.at !== null || !selectorsOf(r).includes(selector)) continue
    const m = r.body.match(new RegExp(`(?:^|;|\\s)${prop.replace(/[-]/g, '\\-')}\\s*:\\s*([^;]+)`))
    if (m) return m[1].trim()
  }
  return undefined
}
const LIGHT = '.surface-stage'
const DARK = 'html[data-theme="dark"] .surface-stage'
const TOKEN_RULES = new Set([LIGHT, DARK])

describe('AC-DS-18 面板用主視窗的 token，跟著淺色／深色', () => {
  it('淺色宣告 color-scheme: light，深色宣告 color-scheme: dark', () => {
    expect(decl(LIGHT, 'color-scheme')).toBe('light')
    expect(decl(DARK, 'color-scheme')).toBe('dark')
  })
  it('面板色與主視窗底色：淺 #FFFFFF／#F5F5F5，深 #111111／#0A0A0A', () => {
    expect(decl(LIGHT, '--s-panel')?.toUpperCase()).toBe('#FFFFFF')
    expect(decl(LIGHT, '--s-stage')?.toUpperCase()).toBe('#F5F5F5')
    expect(decl(DARK, '--s-panel')?.toUpperCase()).toBe('#111111')
    expect(decl(DARK, '--s-stage')?.toUpperCase()).toBe('#0A0A0A')
  })
  it('主要文字與框線沿用主視窗：淺 #171717／#E5E5E5，深 #FAFAFA／#262626', () => {
    expect(decl(LIGHT, '--s-ink')?.toUpperCase()).toBe('#171717')
    expect(decl(LIGHT, '--s-hair')?.toUpperCase()).toBe('#E5E5E5')
    expect(decl(DARK, '--s-ink')?.toUpperCase()).toBe('#FAFAFA')
    expect(decl(DARK, '--s-hair')?.toUpperCase()).toBe('#262626')
  })
  it('主視窗裡舞台是主視窗底色、蓋板裡透明；面板與作答列是面板色', () => {
    expect(decl('html[data-surface="app"] .surface-stage', 'background')).toBe('var(--s-stage)')
    expect(decl(LIGHT, 'background')).toBeUndefined()
    expect(decl('.surface-panel[data-solid="true"]', 'background')).toBe('var(--s-panel)')
    expect(decl('.surface-foot', 'background')).toBe('var(--s-panel)')
  })
})

describe('AC-DS-19 蓋板跟著外觀設定，瀏海與畫布檢查不動', () => {
  function env(search: string, stored: string, systemDark = false): ThemeEnv & { root: { dataset: Record<string, string | undefined>; style: Record<string, string> } } {
    const store = new Map([[THEME_KEY, stored]])
    return {
      search,
      storage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => void store.set(k, v) },
      root: { dataset: {}, style: {} },
      matchMedia: () => ({ matches: systemDark }),
    }
  }
  it('蓋板：標 data-theme 與 color-scheme，不標 data-surface', () => {
    const e = env('?view=overlay', 'dark')
    applyTheme(e)
    expect(e.root.dataset.theme).toBe('dark')
    expect(e.root.dataset.surface).toBeUndefined()
    expect(e.root.style.colorScheme).toBe('dark')
    const s = env('?view=overlay', 'system', true)
    applyTheme(s)
    expect(s.root.dataset.theme).toBe('dark')
  })
  it('瀏海與畫布檢查：什麼都不標', () => {
    for (const search of ['?view=capsule', '?view=canvas-check']) {
      const e = env(search, 'dark')
      applyTheme(e)
      expect(e.root.dataset.theme).toBeUndefined()
      expect(e.root.dataset.surface).toBeUndefined()
    }
  })
  it('開機腳本：蓋板首次繪製前就標 data-theme，不標 data-surface', () => {
    const boot = indexHtml.match(/<script id="kunsu-theme-boot">([\s\S]*?)<\/script>/)![1]
    const dataset: Record<string, string> = {}
    const style: Record<string, string> = {}
    const win = {
      location: { search: '?view=overlay' },
      localStorage: { getItem: (k: string) => (k === THEME_KEY ? 'dark' : null) },
      matchMedia: () => ({ matches: false }),
    }
    new Function('window', 'document', 'localStorage', boot)(win, { documentElement: { dataset, style } }, win.localStorage)
    expect(dataset.theme).toBe('dark')
    expect(dataset.surface).toBeUndefined()
    expect(style.colorScheme).toBe('dark')
  })
})

describe('AC-DS-20 面板與題組不寫死顏色', () => {
  const literal = /#[0-9a-fA-F]{3,8}\b|rgba?\(/
  it('只有兩處 token 規則可以寫顏色值', () => {
    const offenders = rules
      .filter((r) => /\.surface-|\.batch-/.test(r.selector) && !TOKEN_RULES.has(r.selector))
      .filter((r) => literal.test(r.body.replace(/var\(--[\w-]+,\s*[^)]*\)/g, '')))
      .map((r) => r.selector)
    expect(offenders).toEqual([])
  })
  it('展開形狀用面板 token 填色，不是 #000', () => {
    expect(surfaceTsx).not.toMatch(/fill="#000"/)
    expect(surfaceTsx).toMatch(/var\(--s-panel\)/)
  })
})

describe('AC-DS-21 證據畫布在淺色面板裡有自己的深色底', () => {
  it('畫布框底色是 --s-canvas-bg，兩種主題都是深色', () => {
    expect(decl('.surface-canvas__viewport', 'background-color')).toBe('var(--s-canvas-bg)')
    const dark = (v?: string) => !!v && parseInt(v.replace('#', '').slice(0, 2), 16) < 0x30
    expect(dark(decl(LIGHT, '--s-canvas-bg'))).toBe(true)
    expect(dark(decl(DARK, '--s-canvas-bg'))).toBe(true)
  })
})
