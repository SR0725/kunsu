// 草稿題的快照資產（AC-EC-08）：提供給這一題裡所有的 Markdown；快照還沒載入或載入失敗時給空物件，外部圖片一律不載入。
import type { ReactNode } from 'react'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { DraftAssetsContext } from '../components/RichContent'
import { useEvidence } from '../canvas/useEvidence'

const EMPTY: Record<string, string> = {}

export function DraftAssets({ ipc, question, children }: { ipc: KunsuIpc; question: Question; children: ReactNode }) {
  const evidence = useEvidence(ipc, question)
  const value = question.draft ? (evidence.status === 'ready' ? evidence.bundle.assets : EMPTY) : undefined
  return <DraftAssetsContext.Provider value={value}>{children}</DraftAssetsContext.Provider>
}
