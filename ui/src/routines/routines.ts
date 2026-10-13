// 排程督導（PRD §21、BDD routine-check-in）：從題目上的排程身分整理每個 routine 的狀態與歷史。純函數，時鐘可注入。
// 目標正本留在 vault 或排程 prompt；這裡只看每次 check-in 的題目、回答、AI 回饋與交付狀態。
import type { AgentKind, Question } from '../types'
import { handoffFromDelivery } from '../delivery'
import { batchSummary, type BatchCounts } from '../surface/batch'

/** rejected＝使用者按「這題不該問我」推掉（2026-10-09 作者核准：不和 agent 取消混在一起）。 */
export type CheckInStatus = 'pending' | 'answered' | 'missed' | 'rejected' | 'cancelled'

export interface RoutineRow {
  routineId: string
  name: string
  agent: AgentKind
  lastCheckInMs: number
  everyHours: number | null
  latestStatus: CheckInStatus
  answered30d: number
  missed30d: number
  rejected30d: number
  /** 超過宣告頻率兩倍沒報到時，距上次報到的小時數；否則 null。 */
  staleHours: number | null
}

export interface CheckIn {
  id: string
  atMs: number
  questionLine: string
  status: CheckInStatus
  answer: string | null
  /** 逾時由 Kunsu 依預設自動選定的選項文字（不算使用者的回答）。 */
  autoDefault: string | null
  feedback: string | null
  handoff: string | null
  question: Question
}

const DAY_MS = 86_400_000

export function checkInStatus(q: Question): CheckInStatus {
  // routed 但沒有答案：修正前就被收割的過期題（現在收割只動已回答的題）。
  if (q.state === 'expired' || q.answer?.channel === 'auto_policy' || (q.state === 'routed' && !q.answer)) return 'missed'
  if (q.state === 'cancelled' && q.rejected) return 'rejected'
  if (q.state === 'cancelled' || q.state === 'superseded') return 'cancelled'
  if (q.answer && (q.state === 'answered' || q.state === 'routed')) return 'answered'
  return 'pending'
}

export const STATUS_LABEL: Record<CheckInStatus, string> = {
  pending: '待回應',
  answered: '已回應',
  missed: '未回應',
  rejected: '退回',
  cancelled: '已取消',
}

const created = (q: Question) => Date.parse(q.created_at) || 0

/**
 * 宣告為 check-in 的排程題才進督導頁（AC-RC-05）：有 routine_id、不是 internal、不是 `kunsu-` 保留前綴（Kunsu 自己的題）、
 * `trigger = scheduled`，而且宣告了名稱或頻率。只帶 routine_id 的營運排程決策題留在待決與歷史。
 */
function routineOf(q: Question): string | null {
  const id = q.source.routine_id?.trim()
  if (!id || q.internal || id.startsWith('kunsu-') || q.source.trigger !== 'scheduled') return null
  const declared = Boolean(q.source.routine_name?.trim()) || Boolean(q.source.expected_every_hours)
  return declared ? id : null
}

export function summarizeRoutines(questions: Question[], nowMs: number): RoutineRow[] {
  const groups = new Map<string, Question[]>()
  for (const q of questions) {
    const id = routineOf(q)
    if (!id) continue
    groups.set(id, [...(groups.get(id) ?? []), q])
  }
  const rows: RoutineRow[] = []
  for (const [routineId, items] of groups) {
    const sorted = [...items].sort((a, b) => created(b) - created(a))
    const latest = sorted[0]
    const named = sorted.find((q) => q.source.routine_name?.trim())
    const every = sorted.find((q) => q.source.expected_every_hours)?.source.expected_every_hours ?? null
    const recent = sorted.filter((q) => nowMs - created(q) <= 30 * DAY_MS)
    const lastMs = created(latest)
    const sinceHours = Math.floor((nowMs - lastMs) / 3_600_000)
    rows.push({
      routineId,
      name: named?.source.routine_name?.trim() ?? routineId,
      agent: latest.source.agent,
      lastCheckInMs: lastMs,
      everyHours: every,
      latestStatus: checkInStatus(latest),
      answered30d: recent.filter((q) => checkInStatus(q) === 'answered').length,
      missed30d: recent.filter((q) => checkInStatus(q) === 'missed').length,
      rejected30d: recent.filter((q) => checkInStatus(q) === 'rejected').length,
      staleHours: every && nowMs - lastMs > every * 2 * 3_600_000 ? sinceHours : null,
    })
  }
  return rows.sort((a, b) => b.lastCheckInMs - a.lastCheckInMs)
}

export function staleText(hours: number): string {
  return `排程可能沒在跑：已 ${hours} 小時沒報到，請到 Claude／Codex 檢查排程設定`
}

// 純確認題（沒有選項）的答案是 choice_id「confirm」，顯示「確認」（同決策面板）。
const optionLabel = (q: Question, id: string) =>
  q.options.find((o) => o.id === id)?.label ?? (id === 'confirm' ? '確認' : id)

/** 題組的回答：逐項決定的總結（同面板總結列）＋整體指示。 */
function batchAnswerText(q: Question): string | null {
  const decisions = q.answer?.items ?? []
  const items = q.items ?? []
  if (decisions.length === 0) return null
  const counts: BatchCounts = { approve: 0, edited: 0, skip: 0, revise: 0 }
  for (const d of decisions) {
    if (d.action === 'skip') counts.skip++
    else if (d.action === 'revise') counts.revise++
    else {
      counts.approve++
      const item = items.find((i) => i.id === d.item_id)
      if (item?.editable && d.text != null && d.text.trim() !== item.proposal.trim()) counts.edited++
    }
  }
  const summary = batchSummary(items, counts)
  const instruction = q.answer?.free_text?.trim()
  return instruction ? `${summary}（整體指示：${instruction}）` : summary
}

/** 使用者的完整回答：選項文字（＋補充）、自己寫的回答，或題組逐項決定的總結。逾時依預設自動選定的不算。 */
export function answerText(q: Question): string | null {
  const a = q.answer
  if (!a || a.channel === 'auto_policy') return null
  if (q.kind === 'batch') return batchAnswerText(q)
  const option = a.choice_id ? optionLabel(q, a.choice_id) : null
  const note = a.free_text?.trim() || null
  if (option && note) return `${option}（${note}）`
  return option ?? note
}

export function routineHistory(questions: Question[], routineId: string): CheckIn[] {
  return questions
    .filter((q) => routineOf(q) === routineId)
    .sort((a, b) => created(b) - created(a))
    .map((q) => ({
      id: q.id,
      atMs: created(q),
      questionLine: q.question_line,
      status: checkInStatus(q),
      answer: answerText(q),
      autoDefault: q.answer?.channel === 'auto_policy' && q.answer.choice_id ? optionLabel(q, q.answer.choice_id) : null,
      feedback: q.answer?.report?.summary_md ?? null,
      handoff: handoffFromDelivery(q)?.text ?? null,
      question: q,
    }))
}

/** 歷史裡「你的回答」那一欄的文字（AC-RC-06）。 */
export function answerLine(item: CheckIn): string {
  if (item.status === 'rejected') {
    const reason = item.question.rejected?.reason?.trim()
    return reason ? `你退回了這題：${reason}` : '你退回了這題'
  }
  if (item.answer) return item.answer
  if (item.autoDefault) return `沒有回答（逾時依預設：${item.autoDefault}）`
  return item.status === 'pending' ? '還沒回答' : '沒有回答'
}
