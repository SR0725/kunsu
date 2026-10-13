// 手機配對的文字與狀態（BDD mobile-pairing AC-MP-08）：純函數，主視窗外框與設定頁共用。
import type { MobileStatus } from '../../types'

/** 有待允許的配對時，任何頁面都跳的提醒（INV-29：使用者在電腦上按允許才成立）。 */
export function pairingBanner(status: MobileStatus | null): { id: string; text: string } | null {
  const p = status?.pending[0]
  return p ? { id: p.id, text: `「${p.device_name}」要配對這台 Mac` } : null
}

/** 和中轉站的連線：文字寫出原因，不只靠顏色。 */
export function describeConnection(status: MobileStatus): { tone: 'success' | 'waiting' | 'danger'; text: string } {
  if (status.connected) return { tone: 'success', text: '已連上中轉站' }
  if (status.last_error) return { tone: 'danger', text: status.last_error }
  return { tone: 'waiting', text: '正在連線中轉站…' }
}
