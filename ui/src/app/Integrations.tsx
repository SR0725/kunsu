// Agent 連線的真實狀態（BDD settings 場景 10、application-shell 場景 6）：與 `kunsu doctor` 同源；未設定的項目附上可複製的
// 設定指令（app 不執行）。沒有任何假的「連接／中斷」按鈕；不顯示 token 或設定檔內容。
import type { AppStatus, IntegrationId, IntegrationStatus } from '../types'
import { AgentMarkIcon } from '../components/AgentMarkIcon'
import { integrationName, integrationPill, integrationRole } from './status'
import { Button, CodeBlock, Icon, Pill } from './ui'

function IntegrationMark({ id }: { id: IntegrationId }) {
  if (id === 'claude_mcp') return <AgentMarkIcon agent="claude" size={18} />
  if (id === 'codex_mcp') return <AgentMarkIcon agent="codex" size={18} />
  return <Icon name={id === 'cli' ? 'terminal' : 'book'} size={18} />
}

function IntegrationItem({ item }: { item: IntegrationStatus }) {
  return (
    <li className="k-list__item k-integration" data-status={item.status}>
      <div className="k-integration__row">
        <span className="k-integration__mark">
          <IntegrationMark id={item.id} />
        </span>
        <div className="k-integration__text">
          <div className="k-integration__name">{integrationName(item.id)}</div>
          <div className="k-integration__role">{integrationRole(item.id)}</div>
        </div>
        <Pill spec={integrationPill(item.status)} />
      </div>
      <p className="k-integration__detail">{item.detail}</p>
      {item.fix && (
        <div className="k-integration__fix">
          <p className="k-integration__fix-label">複製到終端機執行（Kunsu 不會替你執行）：</p>
          <CodeBlock code={item.fix} label={`${integrationName(item.id)} 的設定指令`} />
        </div>
      )}
    </li>
  )
}

const timeFmt = new Intl.DateTimeFormat('zh-TW', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })

/** 四個檢查項目＋重新檢查。 */
export function IntegrationList({
  status,
  error,
  loading,
  checkedAt,
  onRefresh,
}: {
  status: AppStatus | null
  error: string | null
  loading: boolean
  checkedAt: Date | null
  onRefresh: () => void
}) {
  const items = status?.integrations ?? []
  const missing = items.filter((i) => i.status !== 'ok').length
  return (
    <div className="k-integrations">
      <div className="k-integrations__bar">
        <p className="k-integrations__summary" role="status">
          {error
            ? `讀不到狀態：${error}`
            : !status
              ? '正在檢查…'
              : missing === 0
                ? '四項都已設定，Agent 可以把需要你決定的事送進 Kunsu。'
                : `${missing} 項需要處理。照下面的指令設定好之後按「重新檢查」。`}
        </p>
        <div className="k-integrations__actions">
          {checkedAt && <span className="k-integrations__time tabular">{timeFmt.format(checkedAt)} 檢查</span>}
          <Button variant="secondary" size="sm" icon="refresh" disabled={loading} onClick={onRefresh}>
            重新檢查
          </Button>
        </div>
      </div>
      {items.length > 0 && (
        <ul className="k-list" aria-label="Agent 連線狀態">
          {items.map((item) => (
            <IntegrationItem key={item.id} item={item} />
          ))}
        </ul>
      )}
    </div>
  )
}
