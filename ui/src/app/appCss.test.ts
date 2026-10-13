// 主視窗樣式表（BDD application-shell AC-AS-02／07／08／09／10；DESIGN「Application (dub.co language)」）：
// token 只掛在主視窗（html[data-surface=app]），蓋板、瀏海與決策面板的黑色系統不受影響；尺寸、圓角、窄面板、深淺 AA 對比。
import { describe, expect, it } from 'vitest'
import css from './app.css?raw'
import indexCss from '../index.css?raw'

interface Rule {
  selector: string
  body: string
  at: string | null
}

/** 極簡 CSS 解析：拆出 { } 規則，記住外層 at-rule（@container／@media）。 */
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
        // 影格（from／50%）不是選擇器
      } else if (head.startsWith('@')) walk(inner, head)
      else out.push({ selector: head, body: inner, at })
      i = j
    }
  }
  walk(text, null)
  return out
}

const rules = parse(css)
const selectorsOf = (r: Rule) => r.selector.split(',').map((s) => s.trim()).filter(Boolean)
function decl(selector: string, prop: string, at: string | null = null): string | undefined {
  const hits = rules.filter((r) => r.at === at && selectorsOf(r).includes(selector))
  for (const r of hits.reverse()) {
    const m = r.body.match(new RegExp(`(?:^|;|\\s)${prop.replace(/[-]/g, '\\-')}\\s*:\\s*([^;]+)`))
    if (m) return m[1].trim()
  }
  return undefined
}
function tokens(selector: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const r of rules.filter((r) => r.at === null && r.selector === selector)) {
    for (const m of r.body.matchAll(/(--k-[\w-]+)\s*:\s*([^;]+)/g)) out[m[1]] = m[2].trim()
  }
  return out
}

const LIGHT = 'html[data-surface="app"]'
const DARK = 'html[data-surface="app"][data-theme="dark"]'

describe('只影響主視窗（AC-AS-10）', () => {
  it('每條規則都掛在 html[data-surface="app"] 底下或是 .k- 元件', () => {
    const bad = rules.flatMap(selectorsOf).filter((s) => !s.startsWith(LIGHT) && !s.startsWith('.k-'))
    expect(bad).toEqual([])
  })

  it('不碰決策面板、瀏海；內文樣式只在 .k- 範圍裡覆寫', () => {
    const all = rules.flatMap(selectorsOf)
    expect(all.filter((s) => /\.surface-|\.edge-|\.batch-/.test(s))).toEqual([])
    expect(all.filter((s) => s.includes('.rich-content') && !s.startsWith('.k-'))).toEqual([])
  })

  it('沒有重新定義黑色系統用的 --color-*／--s-* 變數', () => {
    expect(css).not.toMatch(/--color-[\w-]+\s*:/)
    expect(css).not.toMatch(/--s-[\w-]+\s*:/)
  })

  it('舊的深色主視窗樣式已從 index.css 移除，決策面板的樣式還在', () => {
    for (const old of ['.app-window', '.app-sidebar', '.nav-button', '.question-row', '.onboarding-panel', '.routine-chip']) {
      expect(indexCss, old).not.toContain(old)
    }
    expect(indexCss).toContain('.surface-stage')
    expect(indexCss).toContain('.edge-stage')
  })
})

describe('token（DESIGN 淺色預設、深色）', () => {
  const light = tokens(LIGHT)
  const dark = tokens(DARK)

  it('淺色：視窗底、面板、邊框、文字、主按鈕、強調色', () => {
    expect(light).toMatchObject({
      '--k-bg': '#F5F5F5',
      '--k-sidebar-hover': '#EBEBEB',
      '--k-panel': '#FFFFFF',
      '--k-subtle': '#FAFAFA',
      '--k-hover': '#F5F5F5',
      '--k-text-strong': '#171717',
      '--k-text': '#404040',
      '--k-text-muted': '#6B6B6B',
      '--k-text-faint': '#A3A3A3',
      '--k-border': '#E5E5E5',
      '--k-border-strong': '#D4D4D4',
      '--k-primary': '#171717',
      '--k-primary-hover': '#262626',
      '--k-primary-text': '#FFFFFF',
      '--k-accent': '#2563EB',
      '--k-accent-fill': '#EFF4FF',
    })
  })

  it('深色：同一套名稱', () => {
    expect(dark).toMatchObject({
      '--k-bg': '#0A0A0A',
      '--k-sidebar-hover': '#1A1A1A',
      '--k-panel': '#111111',
      '--k-subtle': '#161616',
      '--k-hover': '#1C1C1C',
      '--k-text-strong': '#FAFAFA',
      '--k-text': '#D4D4D4',
      '--k-text-muted': '#A3A3A3',
      '--k-border': '#262626',
      '--k-border-strong': '#333333',
      '--k-primary': '#FAFAFA',
      '--k-primary-text': '#0A0A0A',
      '--k-accent': '#60A5FA',
    })
    for (const name of Object.keys(light)) expect(dark, `深色缺 ${name}`).toHaveProperty(name)
  })

  it('主視窗的 body 用視窗底色（不閃深色）', () => {
    expect(decl(`${LIGHT} body`, 'background')).toBe('var(--k-bg)')
  })
})

describe('WCAG AA 對比（AC-AS-08、AC-ST-15）', () => {
  const hex = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
  function rgba(value: string, under: string): string {
    if (value.startsWith('#')) return value
    const m = value.match(/rgb\(\s*(\d+)\s+(\d+)\s+(\d+)\s*\/\s*([\d.]+)\s*\)/)
    if (!m) throw new Error(`看不懂的顏色 ${value}`)
    const a = Number(m[4])
    const b = hex(under)
    const c = [Number(m[1]), Number(m[2]), Number(m[3])].map((v, i) => Math.round((v / 255) * a * 255 + b[i] * (1 - a) * 255))
    return '#' + c.map((v) => v.toString(16).padStart(2, '0')).join('').toUpperCase()
  }
  const lum = (h: string) => {
    const [r, g, b] = hex(h).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b
  }
  const ratio = (a: string, b: string) => {
    const [x, y] = [lum(a), lum(b)]
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
  }

  for (const [name, t] of [
    ['淺色', tokens(LIGHT)],
    ['深色', { ...tokens(LIGHT), ...tokens(DARK) }],
  ] as const) {
    it(`${name}：文字、強調、小標、按鈕都 ≥ 4.5:1`, () => {
      const panel = t['--k-panel']
      const pairs: [string, string, string][] = [
        ['--k-text-strong', '--k-panel', '主文字／面板'],
        ['--k-text', '--k-panel', '內文／面板'],
        ['--k-text-muted', '--k-panel', '輔助文字／面板'],
        ['--k-text-muted', '--k-subtle', '輔助文字／表頭'],
        ['--k-text-muted', '--k-hover', '輔助文字／hover 列'],
        ['--k-text-muted', '--k-bg', '側欄分組標籤／視窗底'],
        ['--k-text', '--k-sidebar-hover', '側欄 hover'],
        ['--k-accent-text', '--k-accent-fill', '選中的導覽'],
        ['--k-accent', '--k-panel', '連結'],
        ['--k-primary-text', '--k-primary', '主按鈕'],
        ['--k-badge-text', '--k-badge', '計數徽章'],
        ['--k-badge-active-text', '--k-badge-active', '選中的計數徽章'],
        ['--k-success-text', '--k-success-fill', '成功小標'],
        ['--k-waiting-text', '--k-waiting-fill', '等待小標'],
        ['--k-danger-text', '--k-danger-fill', '危險小標'],
        ['--k-neutral-text', '--k-neutral-fill', '中性小標'],
        ['--k-info-text', '--k-info-fill', '資訊小標'],
        ['--k-danger-text', '--k-panel', '錯誤訊息'],
      ]
      for (const [fg, bg, label] of pairs) {
        expect(t[fg], `${name}缺 ${fg}`).toBeTruthy()
        expect(t[bg], `${name}缺 ${bg}`).toBeTruthy()
        // 選中的導覽在側欄（視窗底上）；小標的半透明底在面板上。
        const back = rgba(t[bg], bg === '--k-accent-fill' ? t['--k-bg'] : panel)
        const r = ratio(rgba(t[fg], back), back)
        expect(r, `${name} ${label} ${t[fg]} on ${back} = ${r.toFixed(2)}`).toBeGreaterThanOrEqual(4.5)
      }
    })
  }
})

describe('版面與圓角（AC-AS-02）', () => {
  it('側欄 232 px、頂端 44 px 拖曳區；面板離上右下 8 px、圓角 16 px、細邊框、極淡陰影、自己捲動', () => {
    expect(decl('.k-app', 'grid-template-columns')).toMatch(/^232px\s/)
    expect(decl('.k-sidebar__drag', 'height')).toBe('44px')
    expect(decl('.k-panel', 'margin')).toBe('8px 8px 8px 0')
    expect(decl('.k-panel', 'border-radius')).toBe('16px')
    expect(decl('.k-panel', 'border')).toBe('1px solid var(--k-border)')
    expect(decl('.k-panel', 'box-shadow')).toBe('0 1px 2px rgb(0 0 0 / .04)')
    expect(decl('.k-panel__scroll', 'overflow-y')).toBe('auto')
    expect(decl('.k-panel__scroll', 'overflow-x')).toBe('hidden')
    expect(decl('.k-app', 'height')).toBe('100vh')
  })

  it.each([
    ['.k-btn', '8px'],
    ['.k-input', '8px'],
    ['.k-segmented', '8px'],
    ['.k-nav-item', '8px'],
    ['.k-card', '12px'],
    ['.k-table', '12px'],
    ['.k-stats', '12px'],
    ['.k-code', '12px'],
    ['.k-popover', '12px'],
    ['.k-toast', '12px'],
    ['.k-empty__tile', '12px'],
    ['.k-pill', '999px'],
    ['.k-badge', '999px'],
    ['.k-switch', '999px'],
    ['.k-filter', '999px'],
  ])('%s 圓角 %s（沒有任何直角）', (selector, radius) => {
    expect(decl(selector, 'border-radius')).toBe(radius)
  })

  it('尺寸：按鈕 32 px（頁面主按鈕 36 px）、輸入 32 px、導覽 32 px、表格列 56 px、開關 36×20、篩選 28 px、頁首 56 px', () => {
    expect(decl('.k-btn', 'height')).toBe('32px')
    expect(decl('.k-btn[data-size="lg"]', 'height')).toBe('36px')
    expect(decl('.k-input', 'height')).toBe('32px')
    expect(decl('.k-nav-item', 'height')).toBe('32px')
    // 2026-10-09 QA 第 1 輪：列固定 56 px（有沒有副標都一樣），頁首固定 56 px（說明移到內容區）。
    expect(decl('.k-row', 'height')).toBe('56px')
    expect(decl('.k-switch', 'width')).toBe('36px')
    expect(decl('.k-switch', 'height')).toBe('20px')
    expect(decl('.k-filter', 'height')).toBe('28px')
    expect(decl('.k-page-header', 'height')).toBe('56px')
  })

  it('焦點環：藍色 2 px、偏移 2 px（AC-AS-09）', () => {
    expect(decl('.k-app :focus-visible', 'outline')).toBe('2px solid var(--k-accent)')
    expect(decl('.k-app :focus-visible', 'outline-offset')).toBe('2px')
  })

  it('hover 與按下的轉場 120 ms ease-out；主題切換當下停掉轉場', () => {
    expect(decl('.k-btn', 'transition')).toMatch(/120ms ease-out/)
    expect(rules.some((r) => r.selector.startsWith(`${LIGHT}[data-theme-switching]`) && /transition:\s*none\s*!important/.test(r.body))).toBe(true)
  })
})

describe('窄面板（AC-AS-07）', () => {
  const narrow = '@container k-panel (width < 760px)'
  it('面板是 container', () => {
    expect(decl('.k-panel', 'container')).toBe('k-panel / inline-size')
  })
  it('< 760 px：表頭隱藏、表格列上下堆疊；設定導覽變成橫向可捲動的小標列', () => {
    expect(rules.some((r) => r.at === narrow)).toBe(true)
    expect(decl('.k-table__head', 'display', narrow)).toBe('none')
    // 列改成可換行的 flex：第一格（題目）獨佔一行，其餘欄位排在第二行。
    expect(decl('.k-row', 'display', narrow)).toBe('flex')
    expect(decl('.k-row', 'flex-wrap', narrow)).toBe('wrap')
    expect(decl('.k-row > .k-cell:first-child', 'flex-basis', narrow)).toBe('100%')
    expect(decl('.k-settings', 'grid-template-columns', narrow)).toBe('minmax(0, 1fr)')
    expect(decl('.k-settings__nav', 'flex-direction', narrow)).toBe('row')
    expect(decl('.k-settings__nav', 'overflow-x', narrow)).toBe('auto')
  })
})
