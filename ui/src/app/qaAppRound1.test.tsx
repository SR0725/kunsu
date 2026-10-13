// 獨立 QA 第 1 輪：主視窗外觀與設定（BDD application-shell、settings；DESIGN「Application (dub.co language)」）。
// 照規格寫；紅的是本輪找到的缺陷（重現步驟與截圖見 QA 回報）。
import { describe, expect, it } from 'vitest'
import css from './app.css?raw'
import indexCss from '../index.css?raw'
import { dndLabel, type DndStatus } from '../dnd'
import { applyTheme, THEME_KEY, type ThemeEnv } from './theme'

// ---------------------------------------------------------------- 共用：極簡 CSS 解析與對比

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
      if (head.startsWith('@keyframes')) {
        // 影格不是選擇器
      } else if (head.startsWith('@')) walk(inner, head)
      else out.push({ selector: head, body: inner, at })
      i = j
    }
  }
  walk(text, null)
  return out
}

const appRules = parse(css)
function tokens(selector: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const r of appRules.filter((r) => r.at === null && r.selector === selector)) {
    for (const m of r.body.matchAll(/(--k-[\w-]+)\s*:\s*([^;]+)/g)) out[m[1]] = m[2].trim()
  }
  return out
}
const LIGHT = tokens('html[data-surface="app"]')
const DARK = { ...LIGHT, ...tokens('html[data-surface="app"][data-theme="dark"]') }

const hex = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
const lum = (h: string) => {
  const [r, g, b] = hex(h).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}
const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)]
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
}
/** `var(--k-x)` → token 值。 */
const resolve = (value: string, t: Record<string, string>) => value.replace(/var\((--k-[\w-]+)\)/g, (_, name: string) => t[name] ?? name)
function decl(selector: string, prop: string): string | undefined {
  for (const r of [...appRules].reverse()) {
    if (r.at !== null || !r.selector.split(',').map((s) => s.trim()).includes(selector)) continue
    const m = r.body.match(new RegExp(`(?:^|;|\\s)${prop}\\s*:\\s*([^;]+)`))
    if (m) return m[1].trim()
  }
  return undefined
}

// ---------------------------------------------------------------- 勿擾（AC-ST-08）

describe('AC-ST-08 勿擾：剩餘時間是人話（do-not-disturb BDD「人話剩餘時間」、DESIGN `剩 2 小時 14 分`）', () => {
  const at = (ms: number): DndStatus => ({
    active: true,
    starts_at: '2026-10-09T06:00:00Z',
    ends_at: new Date(Date.parse('2026-10-09T06:00:00Z') + ms).toISOString(),
    remaining_seconds: Math.round(ms / 1000),
    actor: 'ui',
  })
  const start = new Date('2026-10-09T06:00:00Z')

  it('QA1-紅：按「4 小時」一秒後，頁首與設定卡片不能寫「剩 3 小時 60 分」', () => {
    // 重現：設定 › 勿擾（或頁首「勿擾模式」）按 4 小時 → 一秒後重新繪製就顯示「剩 3 小時 60 分」（截圖 46）。
    const label = dndLabel(at(4 * 3600_000 - 1000), start)
    expect(label).not.toMatch(/小時 60 分/)
    expect(label).toMatch(/剩 (4 小時|3 小時 59 分)$/)
  })

  it('QA1-紅：任何整點快捷時長剛開始的那一分鐘都不會出現「N 小時 60 分」', () => {
    for (const hours of [1, 4, 8, 24]) {
      for (const lateMs of [1000, 15_000, 59_000]) {
        const label = dndLabel(at(hours * 3600_000 - lateMs), start)
        expect(label, `${hours} 小時、過了 ${lateMs / 1000} 秒：${label}`).not.toMatch(/小時 60 分/)
      }
    }
  })
})

// ---------------------------------------------------------------- 對比（AC-AS-08、AC-ST-15）

describe('AC-AS-08／AC-ST-15 WCAG AA：關著的開關看得到（1.4.11 非文字對比 ≥ 3:1）', () => {
  it('QA1-紅：開關「關」的軌道對卡片底色 ≥ 3:1（淺色 #D4D4D4 對 #FFFFFF 只有 1.48:1，深色 #333 對 #111 只有 1.49:1）', () => {
    // 重現：設定 › 睡眠時間，把「啟用睡眠時間」關掉——白卡片上只剩一條幾乎看不見的淺灰膠囊（截圖 17）。
    const track = decl('.k-switch', 'background')
    expect(track, '.k-switch 的底色').toBeTruthy()
    for (const [name, t] of [
      ['淺色', LIGHT],
      ['深色', DARK],
    ] as const) {
      const off = resolve(track!, t)
      const card = t['--k-panel']
      const r = ratio(off, card)
      expect(r, `${name}：關著的開關 ${off} 對卡片 ${card} = ${r.toFixed(2)}:1`).toBeGreaterThanOrEqual(3)
    }
  })
})

// ---------------------------------------------------------------- 強制介面不受影響（AC-AS-10）

describe('AC-AS-10 主視窗裡打開的決策面板跟著主視窗的主題', () => {
  function env(stored: string): ThemeEnv {
    const store = new Map<string, string>([[THEME_KEY, stored]])
    return {
      search: '',
      storage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => void store.set(k, v) },
      root: { dataset: {}, style: {} },
      matchMedia: () => ({ matches: false }),
    }
  }

  it('QA1：決策面板的 color-scheme 跟 <html> 一致（淺色主題 → light；原生控制與面板同一套顏色）', () => {
    // 重現：主視窗淺色 → 待決事項「開始處理 N 題」→ 黑色面板裡 getComputedStyle(textarea).colorScheme === 'light'
    // （改版前整個 app 是 :root { color-scheme: dark }）。原生控制（textarea 縮放把手、自動填入、拼字底線、
    // 沒自訂的捲軸）會用淺色樣式畫在黑底上；蓋板視窗裡同一個面板仍是 dark，兩邊不一致。
    const e = env('light')
    applyTheme(e)
    const htmlScheme = (e.root!.style as Record<string, string>).colorScheme
    const surfaceRules = parse(indexCss).concat(appRules).filter((r) => /\.surface-stage\b/.test(r.selector) && !/\s/.test(r.selector.trim()))
    const surfaceDeclaresLight = surfaceRules.some((r) => /color-scheme\s*:\s*light/.test(r.body))
    expect(htmlScheme === 'light' && surfaceDeclaresLight, `html color-scheme=${htmlScheme}，.surface-stage 應宣告 color-scheme: light`).toBe(true)
  })
})
