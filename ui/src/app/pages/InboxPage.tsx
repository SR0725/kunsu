// 待決事項（BDD application-shell 場景 4；PRD §27.4）：頁首＋黑色主按鈕、三格統計、表格列、空狀態。
import type { Question } from '../../types'
import type { DndStatus } from '../../dnd'
import { dndLabel } from '../../dnd'
import { daysPending } from '../../format'
import { AgentMarkIcon } from '../../components/AgentMarkIcon'
import { inboxStats, priorityPill, sourceName } from '../status'
import { Button, EmptyState, Icon, PageBody, PageHeader, Pill, StatGroup, Table, TableRow, type Column } from '../ui'

const COLUMNS: Column[] = [
  { id: 'title', label: '題目', width: 'minmax(0, 1fr)' },
  { id: 'source', label: '來源', width: '120px' },
  { id: 'age', label: '積壓', width: '88px' },
  { id: 'priority', label: '優先', width: '128px' },
]

export function InboxPage({
  questions,
  onOpenAt,
  onStartFlow,
  dnd,
  now = new Date(),
}: {
  questions: Question[]
  onOpenAt: (question: Question) => void
  onStartFlow: () => void
  dnd: DndStatus
  now?: Date
}) {
  const stats = inboxStats(questions)
  return (
    <>
      <PageHeader
        title="待決事項"
        description="這些問題擋住了 Agent 的下一步；回答後，工作從原本的對話繼續。"
        actions={
          questions.length > 0 && (
            <Button variant="primary" size="lg" onClick={onStartFlow} trailingIcon="arrow-right">
              {`開始處理 ${questions.length} 題`}
            </Button>
          )
        }
      />
      <PageBody>
        {dnd.active && (
          <div className="k-banner" role="status">
            <Icon name="bell-off" size={16} />
            <span>
              <strong>{dndLabel(dnd, now)}</strong>。待決事項仍安全保留，期間不顯示通知、瀏海或強制詢問。
            </span>
          </div>
        )}
        {questions.length === 0 ? (
          <div className="k-table k-table--empty">
            <EmptyState icon="inbox" title="Agent 都能繼續工作" description="沒有待決事項時，Kunsu 安靜待在選單列，不佔用你的螢幕。" />
          </div>
        ) : (
          <>
            <StatGroup
              items={[
                { label: '待決', value: stats.total },
                { label: '現在需要你', value: stats.urgent, tone: 'danger' },
                { label: '可批次回答', value: stats.batchable, tone: 'waiting' },
              ]}
            />
            <Table label="待決事項" columns={COLUMNS}>
              {questions.map((question) => (
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
                    age: (
                      <span className="k-cell__muted tabular">
                        <span className="k-cell__prefix">積壓 </span>
                        {`${daysPending(question.first_proposed, now)} 天`}
                      </span>
                    ),
                    priority: <Pill spec={priorityPill(question)} />,
                  }}
                />
              ))}
            </Table>
          </>
        )}
      </PageBody>
    </>
  )
}
