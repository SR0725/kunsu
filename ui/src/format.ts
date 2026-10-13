// 顯示層小工具：積壓天數、倒數、tier 徽章、預設宣告。全繁中、禁縮寫黑話。

import type { Question, Tier } from './types'

export function daysPending(firstProposed: string, now: Date): number {
  const fp = new Date(`${firstProposed}T00:00:00+08:00`)
  const ms = now.getTime() - fp.getTime()
  return Math.max(0, Math.floor(ms / 86_400_000))
}

/** T0 倒數字串；過期回「已到期」。 */
export function countdown(deadlineAt: string, now: Date): string {
  const ms = new Date(deadlineAt).getTime() - now.getTime()
  if (ms <= 0) return '已到期'
  const totalS = Math.floor(ms / 1000)
  const h = Math.floor(totalS / 3600)
  const m = Math.floor((totalS % 3600) / 60)
  const s = totalS % 60
  if (h > 0) return `${h} 時 ${m} 分`
  if (m > 0) return `${m} 分 ${String(s).padStart(2, '0')} 秒`
  return `${s} 秒`
}

export interface TierBadge {
  label: string
  /** chip 外框＋文字色。 */
  className: string
  /** 前置小圓點色（取代 emoji）。 */
  dotClassName: string
}

export function tierBadge(tier: Tier): TierBadge {
  switch (tier) {
    case 't0':
      return { label: '強制', className: 'text-danger border-danger/40', dotClassName: 'bg-danger' }
    case 't1':
      return { label: '今日', className: 'text-amber border-amber/40', dotClassName: 'bg-amber' }
    case 't2':
      return {
        label: '策略',
        className: 'text-primary border-primary/30',
        dotClassName: 'bg-primary',
      }
    case 't3':
      return {
        label: '政策',
        className: 'text-ink-dim border-outline',
        dotClassName: 'bg-outline-bright',
      }
  }
}

/** 「HH:MM」（台北時區）——作答回饋「已收到（HH:MM）」用。 */
export function fmtHm(ts: string): string {
  return new Date(ts).toLocaleTimeString('zh-TW', {
    timeZone: 'Asia/Taipei',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}

/** 尾部預設宣告一行（「不答 → 48h 後：維持現狀並歸檔」）。 */
export function defaultDeclaration(q: Question): string {
  const isMoney = q.money?.is_money_op === true
  let when: string
  switch (q.tier) {
    case 't0':
      when = q.deadline_at ? '時限到' : '今日未答'
      break
    case 't1':
      when = '48 小時後'
      break
    case 't2':
      when = '14 天後'
      break
    case 't3':
      when = '條件成立時'
      break
  }
  if (isMoney) {
    // 錢永不因超時自動動（INV-2）
    return `不答 → ${when}：歸檔＋Discord 留痕（錢不會自動動）`
  }
  switch (q.default_action.on_expire) {
    case 'noop_log':
      return `不答 → ${when}：維持現狀並歸檔`
    case 'escalate_discord':
      return `不答 → ${when}：升級 Discord 留痕`
    case 'pick_option': {
      const opt = q.options.find((o) => o.id === q.default_action.option_id)
      return `不答 → ${when}：自動採「${opt?.label ?? q.default_action.option_id}」`
    }
  }
}

export function fmtTwd(n: number): string {
  return `NT$${n.toLocaleString('en-US')}`
}

/** 時間戳顯示（台北時區、到分）。 */
export function fmtTs(ts: string): string {
  const d = new Date(ts)
  return d.toLocaleString('zh-TW', {
    timeZone: 'Asia/Taipei',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}
