import type { Question, Tier } from './types'

const CYCLE_MS = 15 * 60_000
const WARNING_MS = 3 * 60_000

export interface ForceCycleSnapshot {
  progress: number
  remainingMs: number
  warning: boolean
  overdue: boolean
  label: string
}

function time(value: string | undefined): number | null {
  if (!value) return null
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : null
}

/**
 * 週期的三個時間點（毫秒）。以 server 持久化的牆鐘欄位為正本；fallback 只服務尚未 migration 的 mock/舊資料，
 * 不會寫回或在畫面重 mount 時重啟週期。internal 題沒有週期。
 */
export function cycleTimes(question: Question): { startedAt: number; warnAt: number; forceAt: number } | null {
  if (question.internal) return null
  const startedAt = time(question.cycle_started_at) ?? time(question.created_at)
  const forceAt = time(question.force_at) ?? (startedAt === null ? null : startedAt + CYCLE_MS)
  if (startedAt === null || forceAt === null || forceAt <= startedAt) return null
  const warnAt = time(question.warn_at) ?? forceAt - WARNING_MS
  return { startedAt, warnAt, forceAt }
}

export function forceCycleSnapshot(
  question: Question,
  nowMs = Date.now(),
): ForceCycleSnapshot | null {
  const times = cycleTimes(question)
  if (!times) return null
  const { startedAt, warnAt, forceAt } = times
  const remainingMs = Math.max(0, forceAt - nowMs)
  const progress = Math.min(1, Math.max(0, (forceAt - nowMs) / (forceAt - startedAt)))
  const overdue = nowMs >= forceAt
  const warning = !overdue && nowMs >= warnAt
  const minutes = Math.max(1, Math.ceil(remainingMs / 60_000))
  return {
    progress,
    remainingMs,
    warning,
    overdue,
    label: overdue
      ? '現在需要你的回答'
      : warning
        ? `${minutes} 分鐘後強制詢問`
        : `${minutes} 分鐘後提醒`,
  }
}

const RISK: Record<Tier, number> = { t0: 0, t1: 1, t2: 2, t3: 3 }

/** 全域單視窗佇列：最早 force_at → 風險 → 建立時間。 */
export function compareForceQueue(a: Question, b: Question, _nowMs = Date.now()): number {
  const aForce = time(a.force_at) ?? (time(a.created_at) ?? Number.MAX_SAFE_INTEGER) + CYCLE_MS
  const bForce = time(b.force_at) ?? (time(b.created_at) ?? Number.MAX_SAFE_INTEGER) + CYCLE_MS
  return (
    aForce - bForce ||
    RISK[a.tier] - RISK[b.tier] ||
    (time(a.created_at) ?? Number.MAX_SAFE_INTEGER) -
      (time(b.created_at) ?? Number.MAX_SAFE_INTEGER)
  )
}
