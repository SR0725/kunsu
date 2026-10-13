// 首次導覽（PRD §8.3；BDD application-shell 場景 6、settings 場景 10）：顯示真實的連線狀態（app_status.integrations）與
// 可複製的設定指令，app 不替你執行；沒有假的「一鍵安裝／已連接」。
import { useEffect, useRef, type KeyboardEvent } from 'react'
import type { AppStatus } from '../types'
import type { SettingsIpc } from './settings/settingsIpc'
import { GENERIC_AGENT_PROMPT, useAppStatus } from './appStatus'
import { IntegrationList } from './Integrations'
import { Button, CodeBlock } from './ui'
import { BRAND_ICON } from './brand'

export function Onboarding({
  ipc,
  onComplete,
  initialStatus = null,
}: {
  ipc: Pick<SettingsIpc, 'appStatus'>
  onComplete: () => void
  initialStatus?: AppStatus | null
}) {
  const app = useAppStatus(ipc, { initial: initialStatus })
  const dialogRef = useRef<HTMLDivElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)

  useEffect(() => {
    headingRef.current?.focus()
    const previous = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    return () => {
      document.body.style.overflow = previous
    }
  }, [])

  // 對話框：Esc 等於「稍後設定」；Tab 在對話框裡循環。
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      onComplete()
      return
    }
    if (event.key !== 'Tab' || !dialogRef.current) return
    const focusable = [...dialogRef.current.querySelectorAll<HTMLElement>('button:not([disabled]), [href], input, [tabindex]:not([tabindex="-1"])')]
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first.focus()
    }
  }

  return (
    <div className="k-modal-backdrop">
      <div ref={dialogRef} className="k-modal k-onboarding" role="dialog" aria-modal="true" aria-labelledby="onboarding-title" onKeyDown={onKeyDown}>
        <div className="k-onboarding__head">
          <img className="k-onboarding__icon" src={BRAND_ICON} alt="" width={40} height={40} draggable={false} />
          <div>
            <h1 id="onboarding-title" ref={headingRef} tabIndex={-1} className="k-onboarding__title">
              讓 Agent 知道何時該來找你
            </h1>
            <p className="k-onboarding__desc">
              Kunsu 只收會擋住工作的決定，不收進度與 FYI。下面是這台 Mac 上 Claude 與 Codex 的真實連線狀態。
            </p>
          </div>
        </div>
        <div className="k-onboarding__body">
          <IntegrationList status={app.status} error={app.error} loading={app.loading} checkedAt={app.checkedAt} onRefresh={() => void app.refresh()} />
          <div className="k-onboarding__other">
            <p className="k-label">Cursor 或其他 Agent：把這段提示詞貼進它的系統指示</p>
            <CodeBlock code={GENERIC_AGENT_PROMPT} label="標準提示詞" />
          </div>
        </div>
        <div className="k-onboarding__foot">
          <Button variant="ghost" onClick={onComplete}>
            稍後設定
          </Button>
          <Button variant="primary" trailingIcon="arrow-right" onClick={onComplete}>
            進入 Kunsu
          </Button>
        </div>
      </div>
    </div>
  )
}
