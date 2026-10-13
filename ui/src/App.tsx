import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { KunsuIpc, OverlayTransition } from './ipc'
import { getIpc } from './ipc'
import type { Question } from './types'
import { isTerminal } from './types'
import { EdgeNotch } from './edge/EdgeNotch'
import { notchQueue } from './edge/content'
import { DecisionSurface } from './surface/DecisionSurface'
import { compareForceQueue } from './forceCycle'
import { OVERLAY_CLOSE_GRACE_MS, overlayShellAction, overlayShellMounted, shouldAckMount } from './overlayShell'
import { flowFeed } from './components/overlaySession'
import { ApplicationLaunch, ApplicationShell } from './app/ApplicationShell'
import { closeFlowView, openFlowView, viewSection, type View } from './app/flowView'
import { bindMotionAttribute } from './motion'
import { bindTheme } from './app/theme'
import { withNotchAnchorReport } from './notchAnchor'

const TIER_ORDER = { t0: 0, t1: 1, t2: 2, t3: 3 } as const
const ONBOARDING_KEY = 'kunsu:onboarding-complete:v1'

function shellView(): 'capsule' | 'overlay' | null {
  if (typeof window === 'undefined') return null
  const value = new URLSearchParams(window.location.search).get('view')
  return value === 'capsule' || value === 'overlay' ? value : null
}

export default function App() {
  const [ipc, setIpc] = useState<KunsuIpc | null>(null)
  const [questions, setQuestions] = useState<Question[]>([])
  const [loaded, setLoaded] = useState(false)
  const [view, setView] = useState<View>({ kind: 'app', section: 'inbox' })
  const [onboarding, setOnboarding] = useState(
    () => typeof localStorage !== 'undefined' && localStorage.getItem(ONBOARDING_KEY) !== '1',
  )
  const shell = useMemo(shellView, [])

  const refresh = useCallback(async (adapter: KunsuIpc) => {
    const list = await adapter.listQuestions()
    list.sort((a, b) => {
      const terminal = Number(isTerminal(a.state)) - Number(isTerminal(b.state))
      if (terminal !== 0) return terminal
      return compareForceQueue(a, b) || TIER_ORDER[a.tier] - TIER_ORDER[b.tier]
    })
    setQuestions(list)
    setLoaded(true)
  }, [])

  useEffect(() => {
    let unsub: (() => void) | undefined
    void getIpc().then((adapter) => {
      setIpc(adapter)
      void refresh(adapter)
      unsub = adapter.subscribeQuestions(() => void refresh(adapter))
    })
    return () => unsub?.()
  }, [refresh])

  useEffect(() => {
    if (!ipc || shell) return
    return ipc.subscribeOpenSettings(() => setView({ kind: 'app', section: 'settings' }))
  }, [ipc, shell])

  // 蓋板就緒逾時的後備：主視窗直接打開作答流程，題目仍保持強制資格（AC-OP-05）。
  const pendingFallback = useRef(false)
  useEffect(() => {
    if (!ipc || shell) return
    return ipc.subscribeOpenFlow(() => {
      pendingFallback.current = true
      void refresh(ipc)
    })
  }, [ipc, shell, refresh])

  useEffect(() => {
    if (!shell) return
    document.documentElement.style.background = 'transparent'
    document.body.style.background = 'transparent'
  }, [shell])

  // `<html data-motion>` 跟著 app 內設定即時走（蓋板視窗以前只在載入時讀一次）。
  useEffect(() => bindMotionAttribute(), [])
  // 瀏海視窗：每次命中區更新後回報來源圖示中心，沒有點擊座標時蓋板從這裡展開（AC-OP-02）。
  const notchIpc = useMemo(() => (ipc && shell === 'capsule' ? withNotchAnchorReport(ipc) : ipc), [ipc, shell])

  const actionable = useMemo(() => questions.filter((question) => !isTerminal(question.state)), [questions])
  const notch = useMemo(() => notchQueue(actionable), [actionable])
  // `review`：從歷史紀錄回看已結案的題（AC-DS-17），面板只跟著那幾題的最新資料、不接強制題。
  // 作答流程記住開啟它的頁面（AC-AS-05）：面板開著時底下仍是那一頁，收起後回到那一頁，焦點回到開啟它的列。
  const opener = useRef<HTMLElement | null>(null)
  const openFlow = useCallback((flowQuestions: Question[], startIndex = 0, review = false) => {
    if (flowQuestions.length === 0) return
    opener.current = typeof document !== 'undefined' && document.activeElement instanceof HTMLElement ? document.activeElement : null
    setView((current) => openFlowView(current, flowQuestions, startIndex, review) ?? current)
  }, [])
  // 主視窗的作答流程也跟著最新清單（AC-DS-16、AC-BR-09）：被其他通道結案的就地唯讀、題組重新註冊時 rebase、送出帶新雜湊。
  const flowLatest = useMemo(
    () => (view.kind === 'overlay' ? flowFeed(questions, view.questions, view.review) : questions),
    [questions, view],
  )
  useEffect(() => {
    if (!pendingFallback.current || actionable.length === 0) return
    pendingFallback.current = false
    openFlow(actionable)
  }, [actionable, openFlow])
  const exitOverlay = useCallback(() => {
    setView((current) => closeFlowView(current))
    if (ipc) void refresh(ipc)
    // 面板卸載後把焦點還給開啟它的列（還在畫面上的話）。
    const back = opener.current
    opener.current = null
    if (back) window.requestAnimationFrame(() => back.isConnected && back.focus({ preventScroll: true }))
  }, [ipc, refresh])

  if (!ipc) return shell ? null : <LaunchScreen />

  // 邊緣瀏海（PRD §19）取代浮動膠囊：沿用膠囊視窗（現在是貼滿螢幕的穿透舞台）。internal 題不算題數也不進跑馬燈。
  if (shell === 'capsule') return <EdgeNotch ipc={notchIpc ?? ipc} questions={notch} />
  if (shell === 'overlay') return <OverlayShell ipc={ipc} questions={actionable} latest={questions} loaded={loaded} />

  return (
    <ApplicationShell
      ipc={ipc}
      questions={questions}
      actionable={actionable}
      section={viewSection(view)}
      onSection={(section) => setView({ kind: 'app', section })}
      onOpenAt={(question) => {
        if (isTerminal(question.state)) return openFlow([question], 0, true)
        openFlow(actionable, Math.max(0, actionable.findIndex((item) => item.id === question.id)))
      }}
      onStartFlow={() => openFlow(actionable)}
      onboarding={onboarding}
      onOnboarding={() => setOnboarding(true)}
      onOnboardingComplete={() => {
        localStorage.setItem(ONBOARDING_KEY, '1')
        setOnboarding(false)
      }}
    >
      {view.kind === 'overlay' && (
        <DecisionSurface
          key={view.questions.map((question) => question.id).join('|')}
          ipc={ipc}
          questions={view.questions}
          latest={flowLatest}
          startIndex={view.startIndex}
          review={view.review}
          onExit={exitOverlay}
        />
      )}
    </ApplicationShell>
  )
}

function LaunchScreen() {
  return <ApplicationLaunch />
}

function OverlayShell({ ipc, questions, latest, loaded }: { ipc: KunsuIpc; questions: Question[]; latest: Question[]; loaded: boolean }) {
  // 蓋板跟著外觀設定（AC-DS-19）：跟隨系統時系統深淺一換、主視窗改了設定，都即時同步。
  useEffect(() => bindTheme(), [])
  const [transition, setTransition] = useState<OverlayTransition | null>(null)
  // 面板正顯示這次 session 在面板裡送出的回執：留著，直到使用者按完成或 Esc（AC-OP-07、AC-DS-09）；切去別題就不算。
  const [holdingReceipt, setHoldingReceipt] = useState(false)
  // 面板有作答正在送出：server 先發 questions_changed 再回傳結果，結果回來前不收起（AC-OP-07(c)）。
  const [answering, setAnswering] = useState(false)
  const ackedId = useRef<number | null>(null)
  // 收起：通知原生隱藏蓋板（原生隨後再跑一次強制題對帳）；回傳 Promise，往外開要等它完成才打開目標（AC-DS-13）。
  const exit = useCallback(() => {
    setTransition(null)
    setHoldingReceipt(false)
    setAnswering(false)
    ackedId.current = null
    return ipc.openDeepLink('kunsu://overlay/close')
  }, [ipc])

  // 先訂閱事件，再主動拉取待播轉場：不論原生殼何時送出都不會遺失（AC-OP-04）。
  useEffect(() => {
    const unsub = ipc.subscribeOverlayTransition((next) => setTransition((current) => (current?.id === next.id ? current : next)))
    void ipc.overlayReady().then((pending) => {
      if (pending) setTransition((current) => (current?.id === pending.id ? current : pending))
    }).catch((error) => console.error('蓋板就緒回報失敗', error))
    return unsub
  }, [ipc])

  // Esc 的「網頁已接手」由決策面板在真的處理了之後才通知原生（AC-OP-06）；面板沒掛載時不通知，原生 2 秒後完成。

  const action = overlayShellAction({ loaded, questionCount: questions.length, hasTransition: transition !== null, holdingReceipt, answering })
  // 收起有緩衝：期間面板照樣掛著，面板回報回執或作答中，判斷變回 render，計時就取消。
  useEffect(() => {
    if (action !== 'close') return
    const timeout = window.setTimeout(() => void exit(), OVERLAY_CLOSE_GRACE_MS)
    return () => window.clearTimeout(timeout)
  }, [action, exit])

  // 內容渲染後（effect 在 paint 後執行）才回報掛載，原生殼才會把視窗帶到前景。
  useEffect(() => {
    const id = transition?.id ?? null
    if (!shouldAckMount({ transitionId: id, ackedId: ackedId.current, questionCount: questions.length })) return
    ackedId.current = id
    void ipc.overlayMounted(id as number).catch((error) => console.error('蓋板掛載回報失敗', error))
  }, [ipc, transition, questions.length])

  if (!overlayShellMounted(action) || !transition) return null
  return (
    <DecisionSurface
      key={transition.id}
      ipc={ipc}
      questions={questions}
      latest={latest}
      transition={transition}
      onExit={exit}
      onHoldingReceipt={setHoldingReceipt}
      onAnswering={setAnswering}
    />
  )
}
