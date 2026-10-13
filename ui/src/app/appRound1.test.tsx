// 主視窗與設定 QA 第 1 輪的修正（2026-10-09 主線代作者決定；BDD application-shell AC-AS-02／05／08／10／11／12、settings
// AC-ST-06／08／10／13／15；DESIGN「Application (dub.co language)」）。焦點移動、實際捲動與截圖由瀏覽器 QA 驗證；這裡驗證結構、
// token、純函數與靜態渲染。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import css from './app.css?raw'
import indexCss from '../index.css?raw'
import type { KunsuIpc } from '../ipc'
import type { Question, SettingsState } from '../types'
import { dndLabel, INACTIVE_DND, type DndStatus } from '../dnd'
import { PageHeader, PageBody, SettingsCard } from './ui'
import { SettingsPage } from './pages/SettingsPage'
import { HistoryPage } from './pages/HistoryPage'
import { RoutinesView } from '../routines/RoutinesView'
import { POLICY_DEFAULTS, humanizeSettingsError, SETTING_NAMES, SETTING_KEYS } from './settings/schema'
import { integrationRole } from './status'
import { closeFlowView, openFlowView, viewSection, type View } from './flowView'

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
      } else if (head.startsWith('@font-face')) out.push({ selector: '@font-face', body: inner, at })
      else if (head.startsWith('@')) walk(inner, head)
      else out.push({ selector: head, body: inner, at })
      i = j
    }
  }
  walk(text, null)
  return out
}

const appRules = parse(css)
const indexRules = parse(indexCss)
const selectorsOf = (r: Rule) => r.selector.split(',').map((s) => s.trim()).filter(Boolean)
function declIn(rules: Rule[], selector: string, prop: string, at: string | null = null): string | undefined {
  for (const r of [...rules].reverse()) {
    if (r.at !== at || !selectorsOf(r).includes(selector)) continue
    const m = r.body.match(new RegExp(`(?:^|;|\\s)${prop.replace(/[-]/g, '\\-')}\\s*:\\s*([^;]+)`))
    if (m) return m[1].trim()
  }
  return undefined
}
const decl = (selector: string, prop: string, at: string | null = null) => declIn(appRules, selector, prop, at)
function tokens(selector: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const r of appRules.filter((r) => r.at === null && r.selector === selector)) {
    for (const m of r.body.matchAll(/(--k-[\w-]+)\s*:\s*([^;]+)/g)) out[m[1]] = m[2].trim()
  }
  return out
}
const LIGHT_SEL = 'html[data-surface="app"]'
const DARK_SEL = 'html[data-surface="app"][data-theme="dark"]'
const LIGHT = tokens(LIGHT_SEL)
const DARK = { ...LIGHT, ...tokens(DARK_SEL) }
const NARROW = '@container k-panel (width < 760px)'

const hex = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
const lum = (h: string) => {
  const [r, g, b] = hex(h).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}
const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)]
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
}
const resolve = (value: string, t: Record<string, string>) => value.replace(/var\((--k-[\w-]+)\)/g, (_, name: string) => t[name] ?? name)

const html = (node: React.ReactNode) => renderToStaticMarkup(<>{node}</>)

// ---------------------------------------------------------------- 1. 勿擾剩餘時間（AC-ST-08）

describe('勿擾剩餘時間先進位到分鐘再拆小時（AC-ST-08）', () => {
  const start = new Date('2026-10-09T06:00:00Z')
  const at = (ms: number): DndStatus => ({
    active: true,
    starts_at: start.toISOString(),
    ends_at: new Date(start.getTime() + ms).toISOString(),
    remaining_seconds: Math.round(ms / 1000),
    actor: 'ui',
  })
  it.each([
    [3600_000, '剩 1 小時'],
    [3600_000 - 1000, '剩 1 小時'],
    [3600_000 + 1000, '剩 1 小時 1 分'],
    [4 * 3600_000 - 30_000, '剩 4 小時'],
    [4 * 3600_000 - 61_000, '剩 3 小時 59 分'],
    [59_000, '剩 1 分'],
    [61 * 60_000, '剩 1 小時 1 分'],
    [24 * 3600_000 - 1000, '剩 24 小時'],
  ])('還剩 %i ms → %s', (ms, expected) => {
    expect(dndLabel(at(ms), start).endsWith(expected)).toBe(true)
  })
})

// ---------------------------------------------------------------- 2. 開關與分段控制的非文字對比（AC-AS-08、AC-ST-15）

describe('非文字對比 ≥ 3:1（WCAG 1.4.11）', () => {
  for (const [name, t] of [
    ['淺色', LIGHT],
    ['深色', DARK],
  ] as const) {
    it(`${name}：關著的開關軌道用專用 token，對卡片 ≥ 3:1`, () => {
      expect(decl('.k-switch', 'background')).toBe('var(--k-switch-off)')
      expect(ratio(t['--k-switch-off'], t['--k-panel'])).toBeGreaterThanOrEqual(3)
    })
    it(`${name}：分段控制選中的那一格對軌道 ≥ 3:1`, () => {
      const chip = resolve(decl('.k-segmented__item[aria-checked="true"]', 'background') ?? '', t)
      const track = resolve(decl('.k-segmented', 'background') ?? '', t)
      expect(ratio(chip, track), `${chip} on ${track}`).toBeGreaterThanOrEqual(3)
      expect(decl('.k-segmented__item[aria-checked="true"]', 'color')).toBe('var(--k-primary-text)')
    })
  }
  it('淺色 #8F8F8F、深色 #6B6B6B（DESIGN tokens）', () => {
    expect(LIGHT['--k-switch-off']).toBe('#8F8F8F')
    expect(tokens(DARK_SEL)['--k-switch-off']).toBe('#6B6B6B')
  })
})

// ---------------------------------------------------------------- 3. 焦點不掉到頁面（AC-ST-15）

describe('設定卡片：焦點不掉到頁面（AC-ST-15）', () => {
  it('儲存中是 aria-disabled（仍可聚焦），不是真的 disabled', () => {
    const out = html(
      <SettingsCard title="週期" canSave saving onSave={() => {}}>
        <span />
      </SettingsCard>,
    )
    expect(out).toMatch(/<button[^>]*aria-disabled="true"[^>]*>儲存中…<\/button>/)
    expect(out).not.toMatch(/<button[^>]*disabled=""[^>]*>儲存中…<\/button>/)
  })
  it('卡片標題可以接焦點（tabindex -1），存好或回復預設後焦點移到這裡', () => {
    const out = html(
      <SettingsCard title="週期" canSave onSave={() => {}}>
        <span />
      </SettingsCard>,
    )
    expect(out).toMatch(/<h3[^>]*tabindex="-1"[^>]*class="k-card__title"[^>]*>週期<\/h3>|<h3[^>]*class="k-card__title"[^>]*tabindex="-1"[^>]*>週期<\/h3>/i)
  })
  it('沒有合法變更時仍是真的 disabled（不能存）', () => {
    const out = html(
      <SettingsCard title="週期" canSave={false} onSave={() => {}}>
        <span />
      </SettingsCard>,
    )
    expect(out).toMatch(/<button[^>]*disabled=""[^>]*>儲存<\/button>/)
  })
})

// ---------------------------------------------------------------- 7. 題目最長等待天數（AC-ST-06）

const ipc = { isMock: true } as unknown as KunsuIpc
const SETTINGS: SettingsState = {
  effective: POLICY_DEFAULTS,
  defaults: POLICY_DEFAULTS,
  overridden: ['force.cycle_min'],
  local_path: '/Users/you/.kunsu/policy.local.toml',
}
const renderSettings = (section: string, settings: SettingsState = SETTINGS, dnd: DndStatus = INACTIVE_DND) =>
  renderToStaticMarkup(
    <SettingsPage ipc={ipc} dnd={dnd} onDnd={() => {}} onOnboarding={() => {}} initialSection={section as never} initialSettings={settings} initialStatus={null} />,
  )

describe('到期卡片：題目最長等待天數管所有題目、到期規則只影響新題（AC-ST-06）', () => {
  const out = renderSettings('expiry')
  it('標籤與說明', () => {
    expect(out).toContain('題目最長等待天數')
    expect(out).toContain('所有題目，最多 14 天；策略題到期由新題取代，其他題到期結案')
    expect(out).not.toContain('策略題取代天數')
  })
  it('到期卡片說明：只影響之後新建的題目', () => {
    expect(out).toContain('只影響之後新建的題目')
  })
})

// ---------------------------------------------------------------- 8／10／11／12. 版面（AC-AS-02、AC-AS-11）

describe('寬視窗置中、頁首與內容同寬（AC-AS-11）', () => {
  it('內容置中', () => {
    expect(decl('.k-page-body', 'margin')).toBe('0 auto')
  })
  it('頁首內層與內容用同一個最大寬度（含 24 px 左右留白）', () => {
    for (const [width, max] of [
      ['wide', '1128px'],
      ['narrow', '808px'],
      ['settings', '1020px'],
    ] as const) {
      const body = width === 'wide' ? decl('.k-page-body', 'max-width') : decl(`.k-page-body[data-width="${width}"]`, 'max-width')
      const head = width === 'wide' ? decl('.k-page-header__inner', 'max-width') : decl(`.k-page-header__inner[data-width="${width}"]`, 'max-width')
      expect(body, `${width} 內容`).toBe(max)
      expect(head, `${width} 頁首`).toBe(max)
    }
    expect(decl('.k-page-header__inner', 'margin')).toBe('0 auto')
  })
  it('PageHeader 帶 width 時，頁首內層標同一個寬度', () => {
    const out = html(<PageHeader title="設定" width="settings" />)
    expect(out).toMatch(/class="k-page-header__inner"[^>]*data-width="settings"/)
  })
})

describe('捲軸槽固定、頁首固定 56 px、列固定 56 px（AC-AS-02）', () => {
  it('面板捲動區保留捲軸槽', () => {
    expect(decl('.k-panel__scroll', 'scrollbar-gutter')).toBe('stable')
  })
  it('頁首固定 56 px（寬窄都是）', () => {
    expect(decl('.k-page-header', 'height')).toBe('56px')
    expect(decl('.k-page-header', 'height', NARROW)).toBeUndefined()
  })
  it('說明不在頁首裡：是內容區的第一行（不會讓頁首長高或擠出孤字）', () => {
    const out = html(
      <>
        <PageHeader title="待決事項" description="這些問題擋住了 Agent 的下一步" />
        <PageBody>
          <span />
        </PageBody>
      </>,
    )
    const header = out.slice(out.indexOf('<header'), out.indexOf('</header>'))
    expect(header).not.toContain('這些問題擋住了')
    expect(out).toMatch(/class="k-page-intro"[^>]*>.*這些問題擋住了 Agent 的下一步/)
    expect(decl('.k-page-intro__text', 'text-wrap')).toBe('pretty')
  })
  it('表格列固定 56 px；只有兩行標題或逾期警告的列可以長高', () => {
    expect(decl('.k-row', 'height')).toBe('56px')
    expect(appRules.some((r) => /\.k-row:has\(/.test(r.selector) && /height:\s*auto/.test(r.body))).toBe(true)
    // 窄面板堆疊時不限高度。
    expect(decl('.k-row', 'height', NARROW)).toBe('auto')
  })
})

// ---------------------------------------------------------------- 13. 選中樣式一套語言（AC-AS-12）

describe('選中樣式一套語言（AC-AS-12）', () => {
  it('側欄、設定導覽、窄版小標：藍色淡底＋藍字', () => {
    for (const [sel, at] of [
      ['.k-nav-item[aria-current="page"]', null],
      ['.k-settings__nav-item[aria-current="page"]', null],
      ['.k-settings__nav-item[aria-current="page"]', NARROW],
    ] as const) {
      expect(decl(sel, 'background', at), `${sel} ${at ?? ''}`).toBe('var(--k-accent-fill)')
      expect(decl(sel, 'color', at), `${sel} ${at ?? ''}`).toBe('var(--k-accent-text)')
    }
    expect(decl('.k-settings__nav-item[aria-current="page"]', 'border-color', NARROW)).toBe('var(--k-accent)')
  })
  it('hover 是灰底，和選中不同', () => {
    expect(decl('.k-settings__nav-item:hover', 'background')).toBe('var(--k-hover)')
    expect(decl('.k-nav-item:hover', 'background')).toBe('var(--k-sidebar-hover)')
  })
  it('單選（歷史篩選、分段控制）：主色底', () => {
    expect(decl('.k-filter[aria-pressed="true"]', 'background')).toBe('var(--k-primary)')
    expect(decl('.k-segmented__item[aria-checked="true"]', 'background')).toBe('var(--k-primary)')
  })
})

// ---------------------------------------------------------------- 14. 已改過（AC-ST-03）

describe('「已改過」是中性小標＋圓點，不用 ✨（DESIGN Cards）', () => {
  const out = html(
    <SettingsCard title="週期" modified showReset onReset={() => {}} canSave onSave={() => {}}>
      <span />
    </SettingsCard>,
  )
  it('中性小標與圓點', () => {
    expect(out).toMatch(/class="k-pill k-pill--dot"[^>]*data-tone="neutral"[^>]*>.*已改過/)
    expect(out).toContain('k-pill__dot')
  })
  it('沒有 sparkles 圖示', () => {
    expect(out).not.toContain('lucide-sparkles')
  })
  it('重新執行導覽也不用 sparkles', () => {
    expect(renderSettings('about')).not.toContain('lucide-sparkles')
  })
})

// ---------------------------------------------------------------- 15. 勿擾卡片寫出目前狀態（AC-ST-08）

describe('勿擾卡片寫出目前狀態（AC-ST-08）', () => {
  it('沒開時「目前沒有開勿擾」', () => {
    expect(renderSettings('dnd')).toContain('目前沒有開勿擾')
  })
  it('開著時寫勿擾至與剩餘時間', () => {
    const now = Date.now()
    const active: DndStatus = {
      active: true,
      starts_at: new Date(now).toISOString(),
      ends_at: new Date(now + 2 * 3600_000).toISOString(),
      remaining_seconds: 7200,
      actor: 'ui',
    }
    const out = renderSettings('dnd', SETTINGS, active)
    expect(out).toContain('勿擾至')
    expect(out).not.toContain('目前沒有開勿擾')
  })
})

// ---------------------------------------------------------------- 16. local 檔錯誤用人話名稱（AC-ST-13）

describe('個人設定檔的錯誤用設定頁上的名稱（AC-ST-13）', () => {
  it('每個可設定鍵都有「區段 › 欄位」名稱', () => {
    for (const key of SETTING_KEYS) expect(SETTING_NAMES[key], key).toMatch(/ › /)
    expect(SETTING_NAMES['force.cycle_min']).toBe('強制詢問 › 週期長度')
    expect(SETTING_NAMES['expiry.t2_supersede_after_d']).toBe('到期與配額 › 題目最長等待天數')
  })
  it('server 的錯誤字串換成人話名稱、不露鍵名', () => {
    const raw = '個人設定有不合法的值（force.cycle_min：週期長度要在 5–120 分鐘之間；force.warn_before_min：提前警告要小於週期長度（1–14 分鐘））'
    const out = humanizeSettingsError(raw)
    expect(out).toContain('強制詢問 › 週期長度')
    expect(out).toContain('強制詢問 › 提前警告')
    expect(out).not.toMatch(/force\.\w+/)
  })
  it('橫幅顯示人話，原文留在 title 給除錯', () => {
    const out = renderSettings('force', { ...SETTINGS, overridden: [], local_error: '個人設定有不合法的值（force.cycle_min：週期長度要在 5–120 分鐘之間）' })
    expect(out).toContain('強制詢問 › 週期長度')
    expect(out).toMatch(/title="個人設定有不合法的值（force\.cycle_min/)
    const visible = out.replace(/title="[^"]*"/g, '')
    expect(visible).not.toContain('force.cycle_min')
  })
})

// ---------------------------------------------------------------- 17. 指令範例不在 <…> 中間斷行

describe('督導空狀態的指令只在旗標之間換行', () => {
  it('每個旗標與它的佔位字是一個不換行片段', () => {
    const out = renderToStaticMarkup(<RoutinesView ipc={ipc} questions={[]} nowMs={Date.parse('2026-10-09T04:00:00Z')} />)
    for (const part of ['kunsu ask', '--routine &lt;id&gt;', '--routine-name &lt;名稱&gt;', '--every &lt;小時&gt;']) {
      expect(out, part).toContain(`<span class="k-nowrap">${part}</span>`)
    }
    expect(decl('.k-nowrap', 'white-space')).toBe('nowrap')
  })
})

// ---------------------------------------------------------------- 18. Agent 連線的說明

describe('Agent 連線寫出實際檢查的地方（AC-ST-10）', () => {
  it('Claude 那一項是 Claude Code（~/.claude.json）', () => {
    expect(integrationRole('claude_mcp')).toContain('Claude Code（~/.claude.json）')
    expect(integrationRole('claude_mcp')).not.toContain('Claude App')
  })
})

// ---------------------------------------------------------------- 19. 深色 K 圖示與中性小標

describe('深色 K 圖示看得見、中性小標在 hover 列上仍有外形（AC-AS-08）', () => {
  it('深色主題的 K 圖示有一圈細邊（不加濾鏡、不換色）', () => {
    for (const sel of ['.k-brand__icon', '.k-about__icon', '.k-onboarding__icon']) {
      const ring = decl(`${DARK_SEL} ${sel}`, 'box-shadow')
      expect(ring, sel).toMatch(/rgb\(255 255 255 \/ \.16\)/)
      expect(decl(`${DARK_SEL} ${sel}`, 'filter'), sel).toBeUndefined()
    }
  })
  it('中性小標底色 ≠ hover 底色（淺色與深色）', () => {
    expect(LIGHT['--k-neutral-fill']).not.toBe(LIGHT['--k-hover'])
    expect(DARK['--k-neutral-fill']).not.toBe(DARK['--k-hover'])
    expect(LIGHT['--k-neutral-fill']).toBe('#EBEBEB')
    expect(tokens(DARK_SEL)['--k-neutral-fill']).toBe('#2A2A2A')
  })
})

// ---------------------------------------------------------------- 4／20. 決策面板跟著主題、字型限定（AC-AS-10、AC-DS-18）

describe('主視窗裡的決策面板（AC-AS-10、AC-DS-18）', () => {
  it('舞台跟著主題宣告 color-scheme（2026-10-12 統一設計語言，AC-DS-18）', () => {
    expect(declIn(indexRules, '.surface-stage', 'color-scheme')).toBe('light')
    expect(declIn(indexRules, 'html[data-theme="dark"] .surface-stage', 'color-scheme')).toBe('dark')
  })
  it('主視窗裡舞台是主視窗底色（蓋板不標 data-surface，維持透明）', () => {
    expect(declIn(indexRules, 'html[data-surface="app"] .surface-stage', 'background')).toBe('var(--s-stage)')
    expect(declIn(indexRules, '.surface-stage', 'background')).toBeUndefined()
  })
  it('面板字型：Geist 只排拉丁字元，中文與全形標點交給蘋方', () => {
    const faces = indexRules.filter((r) => r.selector === '@font-face' && /"Kunsu Geist"/.test(r.body))
    expect(faces.length).toBeGreaterThanOrEqual(3)
    for (const face of faces) {
      expect(face.body).toMatch(/unicode-range:/)
      expect(face.body).not.toMatch(/U\+3000|U\+FF0C|U\+FF1A/i)
    }
    const font = declIn(indexRules, '.surface-stage', '--font-sans') ?? ''
    expect(font.startsWith('"Kunsu Geist"')).toBe(true)
    expect(font.indexOf('"PingFang TC"')).toBeGreaterThan(0)
    expect(declIn(indexRules, '.surface-stage', 'font-family')).toBe('var(--font-sans)')
  })
})

// ---------------------------------------------------------------- 9. 歷史回看回到原頁（AC-AS-05）

describe('作答流程記住來源頁（AC-AS-05）', () => {
  const closed = { id: 'done', state: 'answered' } as Question
  it('從歷史紀錄回看：底下仍是歷史紀錄，收起後回到歷史紀錄', () => {
    const app: View = { kind: 'app', section: 'history' }
    const overlay = openFlowView(app, [closed], 0, true)
    expect(overlay).toMatchObject({ kind: 'overlay', review: true, from: 'history' })
    expect(viewSection(overlay!)).toBe('history')
    expect(closeFlowView(overlay!)).toEqual({ kind: 'app', section: 'history' })
  })
  it('從督導開始：收起後回到督導；沒有題目不開', () => {
    const overlay = openFlowView({ kind: 'app', section: 'routines' }, [closed], 0, false)
    expect(closeFlowView(overlay!)).toEqual({ kind: 'app', section: 'routines' })
    expect(openFlowView({ kind: 'app', section: 'routines' }, [], 0, false)).toBeNull()
  })
  it('歷史篩選可以由外框保存（受控）', () => {
    const out = renderToStaticMarkup(<HistoryPage questions={[{ ...closed, question_line: 'x', source: { agent: 'claude', cwd: '/' }, last_seen: '2026-10-08T10:00:00Z' } as Question]} onOpenAt={() => {}} filter="answered" onFilter={() => {}} />)
    expect(out).toMatch(/aria-pressed="true"[^>]*>已回答/)
  })
})
