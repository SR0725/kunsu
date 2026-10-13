// 主視窗的狀態對照（BDD application-shell AC-AS-04／05／06、settings AC-ST-10／11／12）：小標一律有文字與圖示、不只靠
// 顏色。純函數。
import type { AgentKind, IntegrationId, IntegrationStatus, LedgerStatus, Question } from '../types'

export type Tone = 'success' | 'waiting' | 'danger' | 'neutral' | 'info'
/** 圖示名稱（ui/Icon.tsx 對照到實際圖示）。 */
export type IconKey =
  | 'alert'
  | 'clock'
  | 'landmark'
  | 'scroll'
  | 'check'
  | 'hourglass'
  | 'undo'
  | 'ban'
  | 'replace'
  | 'help'
  | 'inbox'
  | 'archive'
  | 'routine'
  | 'agents'
  | 'settings'
  | 'chevron'
  | 'copy'
  | 'refresh'
  | 'database'
  | 'terminal'
  | 'book'
  | 'plug'
  | 'moon'
  | 'sun'
  | 'monitor'
  | 'bell'
  | 'bell-off'
  | 'map-pin'
  | 'info'
  | 'arrow-left'
  | 'arrow-right'
  | 'external'
  | 'sparkles'
  | 'compass'
  | 'gauge'
  | 'timer'
  | 'x'
  | 'smartphone'
  | 'user'

export interface PillSpec {
  label: string
  tone: Tone
  icon: IconKey
}

// ---------------------------------------------------------------- 待決事項

export function priorityPill(q: Question): PillSpec {
  switch (q.tier) {
    case 't0':
      return { label: '現在需要你', tone: 'danger', icon: 'alert' }
    case 't1':
      return { label: '今天', tone: 'waiting', icon: 'clock' }
    case 't2':
      return { label: '策略', tone: 'info', icon: 'landmark' }
    case 't3':
      return { label: '政策', tone: 'neutral', icon: 'scroll' }
  }
}

export function inboxStats(questions: Question[]): { total: number; urgent: number; batchable: number } {
  const urgent = questions.filter((q) => q.tier === 't0').length
  return { total: questions.length, urgent, batchable: questions.length - urgent }
}

export function sourceName(agent: AgentKind): string {
  return agent === 'claude' ? 'Claude' : agent === 'codex' ? 'Codex' : '其他 AI'
}

// ---------------------------------------------------------------- 歷史紀錄

export type Outcome = 'answered' | 'missed' | 'rejected' | 'cancelled' | 'superseded'
export type HistoryFilter = 'all' | Outcome

/** 依實際終態分類（與督導頁 checkInStatus 同一套判斷，另把「被取代」與「取消」分開）。 */
export function historyOutcome(q: Question): Outcome {
  if (q.state === 'expired' || q.answer?.channel === 'auto_policy' || (q.state === 'routed' && !q.answer)) return 'missed'
  if (q.state === 'cancelled' && q.rejected) return 'rejected'
  if (q.state === 'cancelled') return 'cancelled'
  if (q.state === 'superseded') return 'superseded'
  return 'answered'
}

export const OUTCOME_PILL: Record<Outcome, PillSpec> = {
  answered: { label: '已回答', tone: 'success', icon: 'check' },
  missed: { label: '未回應', tone: 'waiting', icon: 'hourglass' },
  rejected: { label: '退回', tone: 'danger', icon: 'undo' },
  cancelled: { label: '取消', tone: 'neutral', icon: 'ban' },
  superseded: { label: '被取代', tone: 'info', icon: 'replace' },
}

export const HISTORY_FILTERS: { id: HistoryFilter; label: string }[] = [
  { id: 'all', label: '全部' },
  { id: 'answered', label: '已回答' },
  { id: 'missed', label: '未回應' },
  { id: 'rejected', label: '退回' },
  { id: 'cancelled', label: '取消' },
  { id: 'superseded', label: '被取代' },
]

export function historyCounts(questions: Question[]): Record<HistoryFilter, number> {
  const counts: Record<HistoryFilter, number> = { all: questions.length, answered: 0, missed: 0, rejected: 0, cancelled: 0, superseded: 0 }
  for (const q of questions) counts[historyOutcome(q)]++
  return counts
}

export function filterHistory(questions: Question[], filter: HistoryFilter): Question[] {
  return filter === 'all' ? questions : questions.filter((q) => historyOutcome(q) === filter)
}

/** 結案時間：回答時間 → 退回時間 → 最後更新。 */
export function closedAt(q: Question): string {
  return q.answer?.answered_at ?? q.rejected?.at ?? q.last_seen
}

// ---------------------------------------------------------------- Agent 連線

const INTEGRATION_NAME: Record<IntegrationId, string> = {
  claude_mcp: 'Claude MCP',
  codex_mcp: 'Codex MCP',
  cli: 'kunsu 指令列工具',
  skill: 'kunsu skill',
}

const INTEGRATION_ROLE: Record<IntegrationId, string> = {
  // 寫出實際檢查的地方（~/.claude.json 是 Claude Code 的設定；AC-ST-10，2026-10-09）。
  claude_mcp: 'Claude Code（~/.claude.json）透過它建題、等答案',
  codex_mcp: 'Codex（~/.codex/config.toml 的 kunsu 外掛或 MCP）透過它建題、等答案',
  cli: '排程任務與終端機用 kunsu ask 建題',
  skill: '教 Claude 什麼時候該問你、怎麼寫一張好題',
}

export function integrationName(id: IntegrationId): string {
  return INTEGRATION_NAME[id] ?? id
}

export function integrationRole(id: IntegrationId): string {
  return INTEGRATION_ROLE[id] ?? ''
}

export function integrationPill(status: IntegrationStatus['status']): PillSpec {
  switch (status) {
    case 'ok':
      return { label: '已設定', tone: 'success', icon: 'check' }
    case 'missing':
      return { label: '未設定', tone: 'waiting', icon: 'alert' }
    default:
      return { label: '無法確認', tone: 'neutral', icon: 'help' }
  }
}

// ---------------------------------------------------------------- 資料與帳本

export interface LedgerHealth extends PillSpec {
  problem: boolean
  explain?: string
  restart?: string
}

const STUCK_RESTART_S = 120

export function ledgerHealth(ledger: LedgerStatus): LedgerHealth {
  const problem = ledger.backlog > 0 || ledger.open_json_dirty || ledger.stuck_s > 0
  if (!problem) return { label: '正常', tone: 'success', icon: 'check', problem: false }
  const label =
    ledger.stuck_s > 0
      ? `寫入卡住 ${ledger.stuck_s} 秒`
      : ledger.backlog > 0
        ? `有 ${ledger.backlog} 行待補寫`
        : 'OPEN.json 待重寫'
  return {
    label,
    tone: ledger.stuck_s > 0 ? 'danger' : 'waiting',
    icon: ledger.stuck_s > 0 ? 'alert' : 'hourglass',
    problem: true,
    explain:
      'macOS 可能還沒允許 Kunsu 讀寫桌面，帳本寫不進去；答案都安全在資料庫，權限打開後會自動補寫。' +
      (ledger.backlog > 0 ? `目前有 ${ledger.backlog} 行排隊等著補寫。` : '') +
      (ledger.open_json_dirty ? 'OPEN.json 也會在寫得進去後重寫。' : ''),
    restart:
      ledger.stuck_s >= STUCK_RESTART_S
        ? `已經卡住超過 ${Math.floor(ledger.stuck_s / 60)} 分鐘。打開權限後還是沒恢復的話，請重新啟動 Kunsu。`
        : undefined,
  }
}

/** 運作時間用人話。 */
export function formatUptime(seconds: number): string {
  if (seconds < 60) return '不到 1 分鐘'
  const days = Math.floor(seconds / 86_400)
  const hours = Math.floor((seconds % 86_400) / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (days > 0) return hours > 0 ? `${days} 天 ${hours} 小時` : `${days} 天`
  if (hours > 0) return minutes > 0 ? `${hours} 小時 ${minutes} 分` : `${hours} 小時`
  return `${minutes} 分`
}
