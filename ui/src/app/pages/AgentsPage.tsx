// Agents（BDD application-shell 場景 6、settings 場景 10）：真實連線狀態（app_status.integrations，與 kunsu doctor 同源），
// 未設定附可複製指令；刪掉假的連接／中斷。另提供給其他 Agent 的標準提示詞（純文字）。
import type { AppStatus } from '../../types'
import type { SettingsIpc } from '../settings/settingsIpc'
import { GENERIC_AGENT_PROMPT, useAppStatus } from '../appStatus'
import { IntegrationList } from '../Integrations'
import { Button, CodeBlock, PageBody, PageHeader, SettingsCard } from '../ui'

export function AgentsPage({
  ipc,
  onOnboarding,
  initialStatus = null,
}: {
  ipc: Pick<SettingsIpc, 'appStatus'>
  onOnboarding: () => void
  initialStatus?: AppStatus | null
}) {
  const app = useAppStatus(ipc, { initial: initialStatus })
  return (
    <>
      <PageHeader
        title="Agents"
        description="Agent 需要你決定時，透過這些連線把題目送進來。狀態與 kunsu doctor 同一份檢查。"
        actions={
          <Button variant="secondary" icon="compass" onClick={onOnboarding}>
            重新執行導覽
          </Button>
        }
      />
      <PageBody>
        <IntegrationList status={app.status} error={app.error} loading={app.loading} checkedAt={app.checkedAt} onRefresh={() => void app.refresh()} />
        <SettingsCard
          title="其他 Agent"
          description="Cursor 或其他支援 MCP 的工具：把這段提示詞貼進它的系統指示，並把 MCP 指到本機服務。"
          footer={<p className="k-card__help">只有會擋住工作的決定才該送進來；進度、FYI 與聊天留在原本的地方。</p>}
        >
          <CodeBlock code={GENERIC_AGENT_PROMPT} label="標準提示詞" />
        </SettingsCard>
      </PageBody>
    </>
  )
}
