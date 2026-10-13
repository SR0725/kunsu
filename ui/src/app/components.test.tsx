// 主視窗元件（DESIGN「Application (dub.co language)」；BDD application-shell AC-AS-02／09、settings AC-ST-15）：
// 語意、鍵盤、狀態不只靠顏色、錯誤以 role="alert" 讀出、設定卡片的儲存列。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  Button,
  CodeBlock,
  EmptyState,
  FilterPills,
  NumberField,
  PageHeader,
  SegmentedControl,
  SettingsCard,
  StatGroup,
  StatusPill,
  Switch,
  Table,
  TableRow,
  TimeField,
  Toast,
  segmentKey,
} from './ui'

const html = (node: React.ReactElement) => renderToStaticMarkup(node)

describe('Button', () => {
  it('四種樣式、兩種尺寸；預設 type=button', () => {
    for (const variant of ['primary', 'secondary', 'ghost', 'danger'] as const) {
      expect(html(<Button variant={variant}>好</Button>)).toMatch(new RegExp(`<button type="button" class="k-btn"[^>]*data-variant="${variant}"`))
    }
    expect(html(<Button size="lg">開始</Button>)).toContain('data-size="lg"')
    expect(html(<Button disabled>不行</Button>)).toContain('disabled=""')
  })
})

describe('StatusPill', () => {
  it('有圖示（裝飾、不讀出）也有文字', () => {
    const out = html(<StatusPill tone="danger" icon="alert">現在需要你</StatusPill>)
    expect(out).toMatch(/class="k-pill"[^>]*data-tone="danger"/)
    expect(out).toMatch(/<svg[^>]*aria-hidden="true"/)
    expect(out).toContain('現在需要你')
  })
})

describe('Switch', () => {
  it('role=switch、aria-checked、由可見標籤命名', () => {
    const out = html(<Switch id="motion" label="連續轉場動畫" checked onChange={() => {}} />)
    expect(out).toMatch(/<label[^>]*for="motion"[^>]*>連續轉場動畫<\/label>/)
    expect(out).toMatch(/<button[^>]*id="motion"[^>]*role="switch"[^>]*aria-checked="true"/)
  })
})

describe('SegmentedControl', () => {
  const out = html(
    <SegmentedControl
      label="外觀"
      value="light"
      onChange={() => {}}
      options={[
        { value: 'system', label: '跟隨系統' },
        { value: 'light', label: '淺色' },
        { value: 'dark', label: '深色' },
      ]}
    />,
  )
  it('radiogroup；選中的是唯一的 Tab 停點', () => {
    expect(out).toMatch(/role="radiogroup"[^>]*aria-label="外觀"/)
    expect(out.match(/role="radio"/g)).toHaveLength(3)
    expect(out).toMatch(/aria-checked="true"[^>]*tabindex="0"[^>]*>(<svg[^>]*>.*?<\/svg>)?淺色/)
    expect(out.match(/tabindex="-1"/g)).toHaveLength(2)
  })
  it('方向鍵循環、Home／End 到兩端，其他鍵不處理', () => {
    expect(segmentKey(0, 3, 'ArrowRight')).toBe(1)
    expect(segmentKey(2, 3, 'ArrowRight')).toBe(0)
    expect(segmentKey(0, 3, 'ArrowLeft')).toBe(2)
    expect(segmentKey(1, 3, 'ArrowDown')).toBe(2)
    expect(segmentKey(1, 3, 'ArrowUp')).toBe(0)
    expect(segmentKey(1, 3, 'Home')).toBe(0)
    expect(segmentKey(1, 3, 'End')).toBe(2)
    expect(segmentKey(1, 3, 'a')).toBeNull()
  })
})

describe('欄位', () => {
  it('數字欄位：可見標籤、單位後綴、範圍說明；錯誤是紅框＋role=alert', () => {
    const ok = html(<NumberField id="cycle" label="週期長度" unit="分鐘" hint="預設 15 · 範圍 5–120" value="15" onChange={() => {}} />)
    expect(ok).toMatch(/<label[^>]*for="cycle"[^>]*>週期長度<\/label>/)
    expect(ok).toContain('分鐘')
    expect(ok).toContain('預設 15 · 範圍 5–120')
    expect(ok).toMatch(/aria-describedby="cycle-hint"/)
    expect(ok).not.toContain('role="alert"')
    const bad = html(<NumberField id="warn" label="提前警告" unit="分鐘" value="15" error="提前警告要小於週期長度（1–14 分鐘）" onChange={() => {}} />)
    expect(bad).toMatch(/aria-invalid="true"/)
    expect(bad).toMatch(/role="alert"[^>]*>提前警告要小於週期長度（1–14 分鐘）/)
    expect(bad).toMatch(/aria-describedby="[^"]*warn-error/)
  })

  it('時間欄位：24 小時制 HH:MM', () => {
    const out = html(<TimeField id="start" label="開始" value="23:00" onChange={() => {}} />)
    expect(out).toMatch(/placeholder="HH:MM"/)
    expect(out).toMatch(/value="23:00"/)
    expect(out).toMatch(/inputMode="numeric"|inputmode="numeric"/)
  })
})

describe('SettingsCard（dub 式儲存列）', () => {
  it('標題、說明、說明列與儲存；沒有合法變更時儲存停用', () => {
    const out = html(
      <SettingsCard title="強制詢問" description="多久問一次" help="只影響之後新開始的週期" canSave={false} onSave={() => {}}>
        <span>欄位</span>
      </SettingsCard>,
    )
    expect(out).toMatch(/<section[^>]*class="k-card k-setting-card"/)
    expect(out).toContain('只影響之後新開始的週期')
    expect(out).toMatch(/<button[^>]*disabled=""[^>]*>儲存<\/button>/)
    expect(out).not.toContain('回復預設')
    expect(out).not.toContain('已改過')
  })
  it('改過：已改過標記與回復預設；可儲存時儲存可按', () => {
    const out = html(
      <SettingsCard title="強制詢問" modified showReset onReset={() => {}} canSave onSave={() => {}}>
        <span />
      </SettingsCard>,
    )
    expect(out).toContain('已改過')
    expect(out).toMatch(/>回復預設<\/button>/)
    expect(out).toMatch(/<button type="button" class="k-btn"[^>]*data-variant="primary"[^>]*>儲存<\/button>/)
  })
})

describe('其他元件', () => {
  it('統計卡：一張卡分格，每格標籤＋數字', () => {
    const out = html(<StatGroup items={[{ label: '待決', value: 3 }, { label: '現在需要你', value: 1, tone: 'danger' }, { label: '可批次回答', value: 2 }]} />)
    expect(out).toMatch(/<dl class="k-stats"/)
    expect(out.match(/class="k-stat"/g)).toHaveLength(3)
    expect(out).toMatch(/<dt[^>]*>現在需要你<\/dt><dd[^>]*>1<\/dd>/)
  })

  it('表格：表頭只給眼睛看，每列是清單裡的按鈕，最後有箭頭', () => {
    const out = html(
      <Table label="待決事項" columns={[{ id: 'title', label: '題目' }, { id: 'source', label: '來源' }]}>
        <TableRow title="要不要暫停 TW-3" onClick={() => {}} cells={{ title: <span>要不要暫停 TW-3</span>, source: <span>Claude</span> }} />
      </Table>,
    )
    expect(out).toMatch(/class="k-table__head"[^>]*aria-hidden="true"/)
    expect(out).toMatch(/<ul[^>]*aria-label="待決事項"/)
    expect(out).toMatch(/<li[^>]*><button type="button" class="k-row"/)
    expect(out).toMatch(/k-row__chevron/)
  })

  it('篩選小標：aria-pressed 與數量', () => {
    const out = html(
      <FilterPills label="依結果篩選" value="all" onChange={() => {}} options={[{ id: 'all', label: '全部', count: 6 }, { id: 'answered', label: '已回答', count: 1 }]} />,
    )
    expect(out).toMatch(/role="group"[^>]*aria-label="依結果篩選"/)
    expect(out).toMatch(/aria-pressed="true"[^>]*>全部<span[^>]*>6<\/span>/)
    expect(out).toMatch(/aria-pressed="false"[^>]*>已回答<span[^>]*>1<\/span>/)
  })

  it('空狀態：圖示方塊、標題、一行說明', () => {
    const out = html(<EmptyState icon="inbox" title="Agent 都能繼續工作" description="沒有待決事項" />)
    expect(out).toContain('k-empty__tile')
    expect(out).toMatch(/<h2[^>]*>Agent 都能繼續工作<\/h2>/)
  })

  it('指令區塊：等寬字與複製按鈕', () => {
    const out = html(<CodeBlock code="kunsu doctor --fix" label="設定指令" />)
    expect(out).toMatch(/<pre[^>]*><code>kunsu doctor --fix<\/code><\/pre>/)
    expect(out).toMatch(/<button[^>]*aria-label="複製設定指令"[^>]*>.*複製<\/button>/)
  })

  it('Toast：role=status，沒有訊息時不畫', () => {
    expect(html(<Toast message="已儲存，立即生效" />)).toMatch(/role="status"[^>]*>.*已儲存，立即生效/)
    expect(html(<Toast message={null} />)).not.toContain('已儲存')
  })

  it('頁首：h1 標題、說明、右側動作；可拖曳視窗', () => {
    const out = html(<PageHeader title="待決事項" description="這些問題擋住了 Agent" actions={<Button variant="primary">開始處理 3 題</Button>} />)
    expect(out).toMatch(/<header[^>]*class="k-page-header"[^>]*data-tauri-drag-region/)
    expect(out).toMatch(/<h1[^>]*>待決事項<\/h1>/)
    expect(out).toContain('開始處理 3 題')
  })
})
