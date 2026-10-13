// 某題目前版本的證據（畫布＋資產）與沙盒 runtime 的載入（PRD §24）。
import { useEffect, useState } from 'react'
import type { KunsuIpc } from '../ipc'
import type { EvidenceBundle, Question } from '../types'

export type EvidenceState =
  | { status: 'none' }
  | { status: 'loading' }
  | { status: 'ready'; bundle: EvidenceBundle }
  | { status: 'error'; message: string }

const cache = new Map<string, EvidenceBundle>()

/** 快取鍵含內容雜湊：同一題同版本號但內容不同（dry-run 重檢）不會拿到舊畫布。 */
export function evidenceKey(question: Pick<Question, 'id' | 'draft'>): string | null {
  const d = question.draft
  return d ? `${question.id}@${d.revision}@${d.content_hash}` : null
}

export function useEvidence(ipc: KunsuIpc, question: Question): EvidenceState {
  const revision = question.draft?.revision
  const key = evidenceKey(question)
  const [state, setState] = useState<EvidenceState>(() =>
    key === null ? { status: 'none' } : cache.has(key) ? { status: 'ready', bundle: cache.get(key)! } : { status: 'loading' },
  )
  useEffect(() => {
    if (key === null || revision === undefined) {
      setState({ status: 'none' })
      return
    }
    const hit = cache.get(key)
    if (hit) {
      setState({ status: 'ready', bundle: hit })
      return
    }
    let alive = true
    setState({ status: 'loading' })
    ipc
      .getEvidence(question.id, revision)
      .then((bundle) => {
        if (!alive) return
        if (!bundle) {
          setState({ status: 'error', message: '找不到這個版本的證據快照' })
          return
        }
        cache.set(key, bundle)
        setState({ status: 'ready', bundle })
      })
      .catch((e: unknown) => alive && setState({ status: 'error', message: String(e) }))
    return () => {
      alive = false
    }
  }, [ipc, key, question.id, revision])
  return state
}

let runtime: Promise<string> | null = null

/** 沙盒 runtime 原始碼（app 內建資產；沙盒沒有網路，所以整份內聯進 srcdoc）。 */
export function loadCanvasRuntime(): Promise<string> {
  runtime ??= fetch('/canvas-runtime.js')
    .then((r) => {
      if (!r.ok) throw new Error(`畫布 runtime 讀取失敗（${r.status}）`)
      return r.text()
    })
    .catch((e: unknown) => {
      runtime = null
      throw e
    })
  return runtime
}
