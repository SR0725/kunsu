# Kunsu 證據畫布（Claude Artifact 模式）— 需要畫圖時才讀

畫布是 `~/.kunsu/drafts/<id>/canvas.tsx`：default export 一個 React 元件，在 Kunsu 的沙盒 iframe 裡執行，
顯示在決策面板左欄「AI 提供的資料」框內（寬約 560–760 px、深色背景、高度跟著內容，上限 2400 px）。

## 規則（違反就會在 issues 裡被點名）

- 只能 import：`react`、`recharts`、`lucide-react`、`kunsu`。沒有 npm、沒有動態 import。
- 沒有網路：不能 `fetch`、WebSocket、外部圖片／字型／腳本。**數據直接寫進畫布**；圖片放 `assets/`，
  在畫布裡用字串 `'./assets/<檔名>'` 引用（會換成快照）。
- 不能碰上層視窗、`localStorage`、cookie。畫布不能作答：選項與送出由 Kunsu 原生介面處理。
- 寬度用 100%／`Grid`／`ChartFrame`，不要寫死超過 560 px 的寬度（會被判橫向溢出）。
- 2 秒內要畫完（註冊檢查與使用者的面板同一個時限）；資料只放決策需要的彙總（原始明細留在 vault，連結寫在 Markdown）。
- 不能換頁（`location`、`<meta http-equiv>`、`<a>` 導覽都會被擋）。
- 先 `kunsu ask <dir> --dry-run`，用 Read 看 `preview_png`，確認使用者看得懂再正式註冊。

## `kunsu` 模組

| 匯出 | 用途 |
|---|---|
| `Stat {label, value, unit?, delta?, good?='up'\|'down', spark?, note?}` | 指標卡。`delta` 是比例（0.12＝12%），會顯示 ▲／▼＋「好／差」文字；`good='down'` 表示下降是好事（例如 CPA、頻次） |
| `Panel {title?, note?, children}` | 卡片；`note` 放資料出處與期間 |
| `Grid {min?=150, gap?=10, children}` | 自動換行格線，每格至少 `min` px |
| `ChartFrame {height?=220, children}` | 包住一張 Recharts 圖表（寬度跟容器） |
| `Delta {value, good?}`、`Sparkline {data, color?}` | 單獨用的變化量與走勢線 |
| `chart` | Recharts 主題：`chart.margin`、`chart.grid`、`chart.axis`、`chart.yAxis`、`chart.tooltip`、`chart.line`、`chart.area`、`chart.bar`、`chart.legend`，直接展開 `{...chart.grid}` |
| `series` | 類別色：`series[0]` 藍、`[1]` 橘、`[2]` 綠、`[3]` 黃 |
| `colors` | 面板色票：`text`、`text2`、`muted`、`faint`、`line`、`card`、`accent`、`good`、`bad`、`warn` |
| `fmt` | `fmt.twd(1234)`→`NT$1,234`、`fmt.pct(0.123)`→`12.3%`、`fmt.num(n, 位數)`、`fmt.compact(12000)`→`1.2萬` |

Tailwind 也可用，Kunsu 色票是 `text-k-text`、`text-k-muted`、`bg-k-card`、`border-k-line`、`text-k-good`、`text-k-bad`。

## 寫法原則

- 最上面放 1–3 張 `Stat` 講結論（使用者 10 秒內看懂），下面才是圖。
- 一張圖只回答一個問題；圖例用 `name` 寫人話（「素材 B」不是「cr_b_v2」）。
- 好壞不要只靠顏色：用 `Stat`／`Delta` 的「好／差」文字。
- 沒有圖也能說清楚時，不要硬做畫布——Markdown 表格就好。

## 範例

### 1. 素材比較（指標卡＋折線）

```tsx
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, Legend } from 'recharts'
import { Grid, Stat, Panel, ChartFrame, chart, series } from 'kunsu'

const days = ['10/02', '10/03', '10/04', '10/05', '10/06', '10/07', '10/08']
const roas = { A: [2.6, 2.4, 2.3, 2.1, 1.9, 1.7, 1.6], B: [1.9, 2.1, 2.2, 2.3, 2.4, 2.4, 2.4] }
const data = days.map((day, i) => ({ day, A: roas.A[i], B: roas.B[i] }))

export default function Canvas() {
  return (
    <div className="flex flex-col gap-3">
      <Grid min={140}>
        <Stat label="素材 B ROAS" value="2.4" delta={0.26} spark={roas.B} />
        <Stat label="素材 A ROAS" value="1.6" delta={-0.38} spark={roas.A} />
      </Grid>
      <Panel title="7 天 ROAS" note="Meta API，10/02–10/08">
        <ChartFrame height={200}>
          <LineChart data={data} margin={chart.margin}>
            <CartesianGrid {...chart.grid} />
            <XAxis dataKey="day" {...chart.axis} />
            <YAxis {...chart.yAxis} domain={[0, 3]} />
            <Tooltip {...chart.tooltip} />
            <Legend {...chart.legend} />
            <Line dataKey="A" name="素材 A" stroke={series[0]} {...chart.line} />
            <Line dataKey="B" name="素材 B" stroke={series[1]} {...chart.line} />
          </LineChart>
        </ChartFrame>
      </Panel>
    </div>
  )
}
```

### 2. 預算分配（長條＋差額）

```tsx
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip } from 'recharts'
import { Grid, Stat, Panel, ChartFrame, chart, series, fmt } from 'kunsu'

const rows = [
  { name: 'Demo 課', now: 900, next: 1200 },
  { name: 'Sample Pro', now: 1900, next: 1200 },
  { name: '品牌', now: 300, next: 700 },
]

export default function Canvas() {
  return (
    <div className="flex flex-col gap-3">
      <Grid>
        <Stat label="總日預算" value={fmt.twd(3100)} note="調整前後相同" />
        <Stat label="Sample Pro" value={fmt.twd(1200)} delta={-0.37} good="down" note="停掉零成交的餵餌組" />
      </Grid>
      <Panel title="日預算：現在 vs 提案" note="NT$／天">
        <ChartFrame height={200}>
          <BarChart data={rows} margin={chart.margin}>
            <CartesianGrid {...chart.grid} />
            <XAxis dataKey="name" {...chart.axis} />
            <YAxis {...chart.yAxis} />
            <Tooltip {...chart.tooltip} formatter={(v: number) => fmt.twd(v)} />
            <Bar dataKey="now" name="現在" fill={series[0]} {...chart.bar} />
            <Bar dataKey="next" name="提案" fill={series[2]} {...chart.bar} />
          </BarChart>
        </ChartFrame>
      </Panel>
    </div>
  )
}
```

### 3. 素材審圖（圖片格線，用 assets）

```tsx
import { Grid, Panel } from 'kunsu'

const shots = [
  { src: './assets/a.png', name: '素材 A', note: '主打價格' },
  { src: './assets/b.png', name: '素材 B', note: '主打學員成果' },
]

export default function Canvas() {
  return (
    <Grid min={220}>
      {shots.map((s) => (
        <Panel key={s.name} title={s.name} note={s.note}>
          <img src={s.src} alt={s.name} className="w-full rounded-lg border border-k-line" />
        </Panel>
      ))}
    </Grid>
  )
}
```

（只有圖片、沒有數據時，直接在 `question.md` 內文寫 `![素材 A](./assets/a.png)` 也可以，不一定要畫布。）

### 4. 排程督導（承諾達成率）

```tsx
import { Grid, Stat, Panel } from 'kunsu'

const week = [1, 1, 0, 1, 1, 0, 0] // 一到日：有沒有寫日記

export default function Canvas() {
  return (
    <div className="flex flex-col gap-3">
      <Grid>
        <Stat label="本週完成" value="4 / 7" delta={-0.2} note="上週 5 / 7" />
        <Stat label="連續中斷" value="2 天" good="down" delta={1} />
      </Grid>
      <Panel title="本週">
        <div className="grid grid-cols-7 gap-2">
          {week.map((done, i) => (
            <div key={i} className={`rounded-md py-2 text-center text-xs ${done ? 'bg-k-good/15 text-k-good' : 'bg-k-card-2 text-k-faint'}`}>
              {'一二三四五六日'[i]}<br />{done ? '✓ 有寫' : '— 沒寫'}
            </div>
          ))}
        </div>
      </Panel>
    </div>
  )
}
```
