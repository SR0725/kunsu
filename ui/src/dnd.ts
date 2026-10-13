export interface DndStatus {
  active: boolean
  starts_at: string | null
  ends_at: string | null
  remaining_seconds: number
  actor: string | null
}

export const INACTIVE_DND: DndStatus = {
  active: false,
  starts_at: null,
  ends_at: null,
  remaining_seconds: 0,
  actor: null,
}

export function normalizeDndMinutes(raw: string): number | null {
  if (!/^\d+$/.test(raw.trim())) return null
  const value = Number(raw)
  return Number.isSafeInteger(value) && value >= 1 && value <= 1440 ? value : null
}

export function dndLabel(status: DndStatus, now = new Date()): string {
  if (!status.active || !status.ends_at) return '勿擾模式'
  const end = new Date(status.ends_at)
  const seconds = Math.max(0, Math.ceil((end.getTime() - now.getTime()) / 1000))
  // 先把總秒數進位到分鐘再拆小時與分鐘：按「4 小時」一秒後是「剩 4 小時」，不會是「剩 3 小時 60 分」（AC-ST-08）。
  const totalMinutes = Math.ceil(seconds / 60)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  const remaining = hours > 0 ? `${hours} 小時${minutes > 0 ? ` ${minutes} 分` : ''}` : `${minutes} 分`
  const endLabel = new Intl.DateTimeFormat('zh-TW', { hour: '2-digit', minute: '2-digit', hour12: false }).format(end)
  return `勿擾至 ${endLabel} · 剩 ${remaining}`
}
