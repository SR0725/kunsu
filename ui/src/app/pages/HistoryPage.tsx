// 歷史紀錄（BDD application-shell 場景 5；DESIGN「Tabs and filters」）：結果篩選小標（各帶數量）、結果小標、結案時間。
// 點一列交給決策面板回看（BDD decision-surface 場景 17／AC-DS-17）：一律唯讀、狀態依實際終態，沒有作答控制。
import { useMemo, useState } from 'react'
import type { Question } from '../../types'
import { AgentMarkIcon } from '../../components/AgentMarkIcon'
import { HISTORY_FILTERS, OUTCOME_PILL, closedAt, filterHistory, historyCounts, historyOutcome, sourceName, type HistoryFilter } from '../status'
import { EmptyState, FilterPills, PageBody, PageHeader, Pill, Table, TableRow, type Column } from '../ui'

const COLUMNS: Column[] = [
  { id: 'title', label: '題目', width: 'minmax(0, 1fr)' },
  { id: 'source', label: '來源', width: '112px' },
  { id: 'outcome', label: '結果', width: '112px' },
  { id: 'closed', label: '結案時間', width: '120px' },
]

const fmt = new Intl.DateTimeFormat('zh-TW', { timeZone: 'Asia/Taipei', month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false })
const when = (iso: string) => {
  const ms = Date.parse(iso)
  return Number.isFinite(ms) ? fmt.format(new Date(ms)) : '—'
}

export function HistoryPage({
  questions,
  onOpenAt,
  filter: controlled,
  onFilter,
}: {
  questions: Question[]
  onOpenAt: (question: Question) => void
  /** 由外框保存的篩選（回看收起、切頁回來都還在，AC-AS-05）；沒給就自己記。 */
  filter?: HistoryFilter
  onFilter?: (filter: HistoryFilter) => void
}) {
  const [own, setOwn] = useState<HistoryFilter>('all')
  const filter = controlled ?? own
  const setFilter = (next: HistoryFilter) => {
    setOwn(next)
    onFilter?.(next)
  }
  const counts = useMemo(() => historyCounts(questions), [questions])
  const sorted = useMemo(() => [...questions].sort((a, b) => Date.parse(closedAt(b)) - Date.parse(closedAt(a))), [questions])
  const visible = filterHistory(sorted, filter)

  return (
    <>
      <PageHeader title="歷史紀錄" description="每一個回答、退回與過期都留在決策帳本。點開看結果；已結案的題不能再作答。" />
      <PageBody>
        {questions.length === 0 ? (
          <div className="k-table k-table--empty">
            <EmptyState icon="archive" title="還沒有結案的題" description="回答、退回或過期的題目都會出現在這裡。" />
          </div>
        ) : (
          <>
            <FilterPills label="依結果篩選" value={filter} onChange={setFilter} options={HISTORY_FILTERS.map((f) => ({ ...f, count: counts[f.id] }))} />
            {visible.length === 0 ? (
              <div className="k-table k-table--empty">
                <EmptyState icon="archive" title="這個結果沒有題目" description="換一個篩選看看。" />
              </div>
            ) : (
              <Table label="歷史紀錄" columns={COLUMNS}>
                {visible.map((question) => {
                  const closed = closedAt(question)
                  return (
                    <TableRow
                      key={question.id}
                      title={question.question_line}
                      onClick={() => onOpenAt(question)}
                      cells={{
                        title: (
                          <span className="k-cell__stack">
                            <span className="k-cell__title">{question.question_line}</span>
                            {question.product && <span className="k-cell__sub">{question.product}</span>}
                          </span>
                        ),
                        source: (
                          <span className="k-source">
                            <AgentMarkIcon agent={question.source.agent} size={16} label={sourceName(question.source.agent)} />
                            <span aria-hidden="true">{sourceName(question.source.agent)}</span>
                          </span>
                        ),
                        outcome: <Pill spec={OUTCOME_PILL[historyOutcome(question)]} />,
                        closed: (
                          <time className="k-cell__muted tabular" dateTime={closed}>
                            {when(closed)}
                          </time>
                        ),
                      }}
                    />
                  )
                })}
              </Table>
            )}
          </>
        )}
      </PageBody>
    </>
  )
}
