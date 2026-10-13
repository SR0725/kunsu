// 督導頁（PRD §21、DESIGN「Routines view」、BDD routine-check-in AC-RC-05～08）：每個排程一列；選一個看歷史。
// 不是聊天介面：沒有回覆框，只有「打開原對話」。2026-10-09 改用主視窗的 dub.co 語言（BDD application-shell 場景 6）：
// 列表是表格列、歷史每次 check-in 是一張卡片；行為不變。
import { useEffect, useMemo, useRef, useState } from 'react'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { AgentMarkIcon } from '../components/AgentMarkIcon'
import { RichContent } from '../components/RichContent'
import { conversationLink } from '../delivery'
import type { PillSpec } from '../app/status'
import { Button, EmptyState, PageBody, PageHeader, Pill, Table, TableRow, type Column } from '../app/ui'
import { STATUS_LABEL, answerLine, routineHistory, staleText, summarizeRoutines, type CheckInStatus } from './routines'

const fmt = new Intl.DateTimeFormat('zh-TW', {
  month: 'numeric',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
})
const when = (ms: number) => (ms ? fmt.format(new Date(ms)) : '—')
const AGENT_NAME = {
  claude: 'Claude',
  codex: 'Codex',
  other: '其他 AI',
} as const
const cadence = (h: number | null) => (h === null ? '未宣告頻率' : h % 24 === 0 ? (h === 24 ? '每天' : h === 168 ? '每週' : `每 ${h / 24} 天`) : `每 ${h} 小時`)

/** 狀態小標：每個狀態都有文字與圖示（AC-RC-08）。 */
const STATUS_PILL: Record<CheckInStatus, PillSpec> = {
  pending: { label: STATUS_LABEL.pending, tone: 'info', icon: 'clock' },
  answered: { label: STATUS_LABEL.answered, tone: 'success', icon: 'check' },
  missed: { label: STATUS_LABEL.missed, tone: 'waiting', icon: 'hourglass' },
  rejected: { label: STATUS_LABEL.rejected, tone: 'danger', icon: 'undo' },
  cancelled: { label: STATUS_LABEL.cancelled, tone: 'neutral', icon: 'ban' },
}

const COLUMNS: Column[] = [
  { id: 'name', label: '排程', width: 'minmax(0, 1fr)' },
  { id: 'last', label: '最近報到', width: '112px' },
  { id: 'counts', label: '近 30 天', width: '216px' },
  { id: 'status', label: '狀態', width: '104px' },
]

export function RoutinesView({
  ipc,
  questions,
  nowMs = Date.now(),
  initialRoutine = null,
}: {
  ipc: KunsuIpc
  questions: Question[]
  nowMs?: number
  initialRoutine?: string | null
}) {
  const rows = useMemo(() => summarizeRoutines(questions, nowMs), [questions, nowMs])
  const [selected, setSelected] = useState<string | null>(initialRoutine)
  const current = rows.find((r) => r.routineId === selected) ?? null
  // 進入歷史時焦點移到標題；返回列表時焦點回到原來那一列（AC-RC-06）。
  const headingRef = useRef<HTMLHeadingElement>(null)
  const rowRefs = useRef(new Map<string, HTMLButtonElement>())
  const lastOpened = useRef<string | null>(null)
  useEffect(() => {
    if (selected) headingRef.current?.focus()
    else if (lastOpened.current) rowRefs.current.get(lastOpened.current)?.focus()
  }, [selected])
  const open = (id: string) => {
    lastOpened.current = id
    setSelected(id)
  }

  if (current) {
    const history = routineHistory(questions, current.routineId)
    return (
      <>
        <PageHeader
          titleRef={headingRef}
          width="narrow"
          back={
            <button type="button" className="k-back" onClick={() => setSelected(null)}>
              ‹ 所有排程
            </button>
          }
          title={current.name}
          description={
            <>
              {`${cadence(current.everyHours)} · 近 30 天已回應 ${current.answered30d} 次、未回應 ${current.missed30d} 次、退回 ${current.rejected30d} 次`}
              {current.staleHours !== null && <span className="k-stale">{staleText(current.staleHours)}</span>}
            </>
          }
        />
        <PageBody width="narrow">
          <ol className="k-history" aria-label={`${current.name} 的歷史`}>
            {history.map((item) => {
              const link = conversationLink(item.question)
              return (
                <li key={item.id} className="k-card routine-entry" data-status={item.status}>
                  <div className="routine-entry__head">
                    <Pill spec={STATUS_PILL[item.status]} status={item.status} />
                    <time className="tabular">{when(item.atMs)}</time>
                  </div>
                  <div className="routine-entry__q">{item.questionLine}</div>
                  <details className="routine-entry__more" open={item.status === 'answered'}>
                    <summary>你的回答與 AI 回饋</summary>
                    <div className="routine-entry__label">你的回答</div>
                    <div className="routine-entry__answer" data-empty={item.answer ? undefined : 'true'}>
                      {answerLine(item)}
                    </div>
                    <div className="routine-entry__label">AI 回饋</div>
                    {item.feedback ? (
                      <RichContent className="routine-entry__feedback" onOpenLink={(href) => void ipc.openDeepLink(href)}>
                        {item.feedback}
                      </RichContent>
                    ) : (
                      <div className="routine-entry__muted">{item.status === 'answered' ? 'AI 尚未回報' : '—'}</div>
                    )}
                    {item.handoff && <div className="routine-entry__handoff">{`交付：${item.handoff}`}</div>}
                    {link && (
                      <Button variant="secondary" size="sm" icon="external" className="routine-entry__link" onClick={() => void ipc.openDeepLink(link)}>
                        {`打開原對話（${link.startsWith('codex://') ? 'Codex App' : 'Claude App'}）`}
                      </Button>
                    )}
                  </details>
                </li>
              )
            })}
          </ol>
        </PageBody>
      </>
    )
  }

  return (
    <>
      <PageHeader title="督導" description="排程任務（Claude Routines、Codex automations）來要你回應的紀錄：你答了什麼、AI 怎麼回饋。" />
      <PageBody>
        {rows.length === 0 ? (
          <div className="k-table k-table--empty">
            <EmptyState
              icon="routine"
              title="還沒有排程 check-in"
              description={
                <>
                  排程任務用{' '}
                  {/* 只在旗標之間換行：每個旗標與它的佔位字不拆開（不會在 <名稱> 中間斷行）。 */}
                  <code className="k-inline-code">
                    <span className="k-nowrap">kunsu ask</span> <span className="k-nowrap">--routine &lt;id&gt;</span>{' '}
                    <span className="k-nowrap">--routine-name &lt;名稱&gt;</span> <span className="k-nowrap">--every &lt;小時&gt;</span>
                  </code>{' '}
                  建題後會出現在這裡。
                </>
              }
            />
          </div>
        ) : (
          <Table label="排程" columns={COLUMNS}>
            {rows.map((row) => (
              <TableRow
                key={row.routineId}
                className="routine-row"
                rowRef={(el) => {
                  if (el) rowRefs.current.set(row.routineId, el)
                  else rowRefs.current.delete(row.routineId)
                }}
                onClick={() => open(row.routineId)}
                cells={{
                  name: (
                    <span className="k-cell__with-mark">
                      <AgentMarkIcon agent={row.agent} size={18} label={AGENT_NAME[row.agent] ?? 'AI'} />
                      <span className="k-cell__stack">
                        <span className="k-cell__title">{row.name}</span>
                        <span className="k-cell__sub">{cadence(row.everyHours)}</span>
                        {row.staleHours !== null && <span className="k-stale">{staleText(row.staleHours)}</span>}
                      </span>
                    </span>
                  ),
                  last: (
                    <span className="k-cell__muted tabular">
                      <span className="k-cell__prefix">最近 </span>
                      {when(row.lastCheckInMs)}
                    </span>
                  ),
                  counts: (
                    <span className="k-cell__muted tabular">
                      <span className="k-cell__prefix">30 天：</span>
                      {`已回應 ${row.answered30d} · 未回應 ${row.missed30d} · 退回 ${row.rejected30d}`}
                    </span>
                  ),
                  status: <Pill spec={STATUS_PILL[row.latestStatus]} status={row.latestStatus} />,
                }}
              />
            ))}
          </Table>
        )}
      </PageBody>
    </>
  )
}
