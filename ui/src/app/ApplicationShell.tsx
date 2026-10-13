// 主視窗外框（PRD §27、DESIGN「Application (dub.co language)」、BDD application-shell）：232 px 側欄直接放在視窗底色上，
// 主內容是一塊離上右下 8 px、16 px 圓角的面板，自己捲動。決策面板（children）畫在外框之外，維持黑色系統、不繼承這裡的樣式。
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import './app.css'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { isTerminal } from '../types'
import type { DndStatus } from '../dnd'
import { INACTIVE_DND } from '../dnd'
import { RoutinesView } from '../routines/RoutinesView'
import { DndToolbarButton } from './DndControl'
import { Onboarding } from './Onboarding'
import { Sidebar, type AppSection } from './Sidebar'
import { bindTheme } from './theme'
import { AgentsPage } from './pages/AgentsPage'
import { HistoryPage } from './pages/HistoryPage'
import { InboxPage } from './pages/InboxPage'
import { SettingsPage } from './pages/SettingsPage'
import type { HistoryFilter } from './status'
import { Button } from './ui'
import type { MobileStatus } from '../types'
import { pairingBanner } from './settings/mobile'
import { ShellToolbarContext, Toast, ToastContext } from './ui'

export type { AppSection } from './Sidebar'

const TOAST_MS = 2500

export function ApplicationShell({
  ipc,
  questions,
  actionable,
  section,
  onSection,
  onOpenAt,
  onStartFlow,
  onboarding,
  onOnboarding,
  onOnboardingComplete,
  children,
}: {
  ipc: KunsuIpc
  questions: Question[]
  actionable: Question[]
  section: AppSection
  onSection: (section: AppSection) => void
  onOpenAt: (question: Question) => void
  onStartFlow: () => void
  onboarding: boolean
  onOnboarding: () => void
  onOnboardingComplete: () => void
  children?: ReactNode
}) {
  // 外觀：跟隨系統時 macOS 換深淺、其他主視窗改了偏好都即時同步（首次繪製前那一次在 index.html）。
  useEffect(() => bindTheme(), [])

  const [dnd, setDnd] = useState<DndStatus>(INACTIVE_DND)
  const refreshDnd = useCallback(() => void ipc.dndGet().then(setDnd).catch(console.error), [ipc])
  useEffect(() => {
    refreshDnd()
    const timer = window.setInterval(refreshDnd, 30_000)
    return () => window.clearInterval(timer)
  }, [refreshDnd])

  // 手機配對（INV-29）：有手機在等配對時，任何頁面都跳允許／拒絕。
  const [mobile, setMobile] = useState<MobileStatus | null>(null)
  useEffect(() => {
    const load = () => void ipc.mobileStatus().then(setMobile).catch(() => setMobile(null))
    load()
    const timer = window.setInterval(load, 3000)
    return () => window.clearInterval(timer)
  }, [ipc])
  const banner = pairingBanner(mobile)
  const decide = (allow: boolean) => {
    if (!banner) return
    void ipc.mobileDecidePairing(banner.id, allow).then(setMobile).catch(console.error)
  }

  const [toast, setToast] = useState<string | null>(null)
  const toastTimer = useRef<number | undefined>(undefined)
  const showToast = useCallback((message: string) => {
    setToast(message)
    window.clearTimeout(toastTimer.current)
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS)
  }, [])
  useEffect(() => () => window.clearTimeout(toastTimer.current), [])

  // 切頁時面板捲回頂端。
  const scrollRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 })
  }, [section])

  const toolbar = useMemo(
    () => (
      <>
        {ipc.isMock && <span className="k-mock-label">Mock 資料</span>}
        <DndToolbarButton ipc={ipc} status={dnd} onStatus={setDnd} />
      </>
    ),
    [ipc, dnd],
  )
  const history = useMemo(() => questions.filter((question) => isTerminal(question.state)), [questions])
  // 歷史紀錄的篩選由外框保存：回看收起、切到別頁再回來都還在（AC-AS-05）。
  const [historyFilter, setHistoryFilter] = useState<HistoryFilter>('all')

  return (
    <>
      <div className="k-app">
        <Sidebar section={section} onSection={onSection} pending={actionable.length} />
        <main className="k-panel" aria-label="主要內容">
          <ToastContext.Provider value={showToast}>
            <ShellToolbarContext.Provider value={toolbar}>
              <div className="k-panel__scroll" ref={scrollRef}>
                {banner && (
                  <div className="k-pairing-banner" role="alert">
                    <span>{banner.text}</span>
                    <span className="k-mobile__actions">
                      <Button onClick={() => decide(false)}>拒絕</Button>
                      <Button variant="primary" onClick={() => decide(true)}>允許</Button>
                    </span>
                  </div>
                )}
                {section === 'inbox' && <InboxPage questions={actionable} onOpenAt={onOpenAt} onStartFlow={onStartFlow} dnd={dnd} />}
                {section === 'history' && <HistoryPage questions={history} onOpenAt={onOpenAt} filter={historyFilter} onFilter={setHistoryFilter} />}
                {section === 'routines' && <RoutinesView ipc={ipc} questions={questions} />}
                {section === 'agents' && <AgentsPage ipc={ipc} onOnboarding={onOnboarding} />}
                {section === 'settings' && <SettingsPage ipc={ipc} dnd={dnd} onDnd={setDnd} onOnboarding={onOnboarding} />}
              </div>
              <Toast message={toast} />
            </ShellToolbarContext.Provider>
          </ToastContext.Provider>
        </main>
        {onboarding && <Onboarding ipc={ipc} onComplete={onOnboardingComplete} />}
      </div>
      {children}
    </>
  )
}

/** ipc 還沒好之前的主視窗（同一套底色，不閃深色）。 */
export function ApplicationLaunch() {
  return (
    <div className="k-launch" role="status">
      <span className="k-launch__dot" aria-hidden="true" />
      正在喚醒軍師…
    </div>
  )
}
