// 證據畫布的註冊檢查頁（PRD §24、AC-EC-01～03）：在隱藏的 canvas-check 視窗裡，用同一個決策面板渲染候選題，
// 畫布以檢查模式執行（回報編譯、執行、版面問題），定案後交回 Rust 截圖。
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { KunsuIpc } from '../ipc'
import type { EvidenceBundle, Question } from '../types'
import { DecisionSurface } from '../surface/DecisionSurface'
import { CanvasCheckContext, type CanvasCheckMode } from './CanvasFrame'
import { CHECK_TIMEOUT_MS, type FrameState } from './frameState'
import type { CanvasIssue } from './messages'

interface CheckJob {
  id: string
  question: Question
  evidence: EvidenceBundle
}


function afterPaint(fn: () => void) {
  requestAnimationFrame(() => requestAnimationFrame(() => window.setTimeout(fn, 120)))
}

/** 檢查頁不需要真的 IPC：只提供這一題與它的證據，其餘都是無作用。 */
function checkIpc(job: CheckJob): KunsuIpc {
  const target: Record<string, unknown> = {
    isMock: true,
    getQuestion: async () => job.question,
    listQuestions: async () => [job.question],
    getEvidence: async () => job.evidence,
  }
  return new Proxy(target, {
    get(t, key: string) {
      if (key in t) return t[key]
      if (key.startsWith('subscribe')) return () => () => {}
      return async () => null
    },
  }) as unknown as KunsuIpc
}

function CheckRun({ job, report }: { job: CheckJob; report: (issues: CanvasIssue[], height?: number) => void }) {
  const ipc = useMemo(() => checkIpc(job), [job])
  const onSettled = useCallback(
    (state: FrameState) => {
      // 版面套用後再交出去截圖（等兩個繪製週期）。
      if (state.phase === 'ready') afterPaint(() => report([], state.height))
      else if (state.phase === 'failed') {
        report(
          state.issues.length > 0
            ? state.issues
            : [{ file: `canvas.${job.evidence.canvas?.kind ?? 'tsx'}`, code: 'canvas_timeout', message: state.reason, hint: '畫布太重或卡在無窮迴圈：減少資料點、拿掉不必要的計算' }],
        )
      }
    },
    [job, report],
  )
  const mode: CanvasCheckMode = useMemo(() => ({ timeoutMs: CHECK_TIMEOUT_MS, onSettled }), [onSettled])

  // 沒有畫布：等字型與版面穩定後直接回報。
  useEffect(() => {
    if (job.evidence.canvas) return
    let alive = true
    void document.fonts.ready.then(() => window.setTimeout(() => alive && report([]), 400))
    return () => {
      alive = false
    }
  }, [job, report])

  return (
    <CanvasCheckContext.Provider value={mode}>
      <DecisionSurface ipc={ipc} questions={[job.question]} transition={null} onExit={() => {}} />
    </CanvasCheckContext.Provider>
  )
}

export function CanvasCheckPage() {
  const [job, setJob] = useState<CheckJob | null>(null)
  const sent = useRef(new Set<string>())

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let alive = true
    void (async () => {
      const { invoke } = await import('@tauri-apps/api/core')
      const { listen } = await import('@tauri-apps/api/event')
      const take = async () => {
        const next = await invoke<CheckJob | null>('canvas_check_take')
        if (alive && next && !sent.current.has(next.id)) setJob(next)
      }
      unlisten = await listen('kunsu://canvas_check', () => void take())
      await take()
    })()
    return () => {
      alive = false
      unlisten?.()
    }
  }, [])

  const report = useCallback(
    (issues: CanvasIssue[], height?: number) => {
      if (!job || sent.current.has(job.id)) return
      sent.current.add(job.id)
      void import('@tauri-apps/api/core').then(({ invoke }) =>
        invoke('canvas_check_done', { id: job.id, report: { issues, ...(height ? { height } : {}) } }),
      )
    },
    [job],
  )

  return job ? <CheckRun key={job.id} job={job} report={report} /> : null
}
