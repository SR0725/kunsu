// 主視窗「設定 › 帳號」的登入流程（BDD multi-user AC-MU-03；PRD §31.2）：寄連結 → 等使用者在任何裝置點 → 登入。

export interface AccountInfo {
  email: string
}

/** 跟登入連結一樣 15 分鐘。 */
export const LOGIN_WAIT_MS = 15 * 60 * 1000
const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

export type AccountFlow =
  | { kind: 'idle'; error?: string }
  | { kind: 'sending'; email: string; since: number }
  | { kind: 'waiting'; email: string; poll: string; since: number }
  | { kind: 'done' }

export type AccountEvent =
  | { type: 'submit'; email: string; now: number }
  | { type: 'sent'; poll: string }
  | { type: 'polled'; done: boolean; now: number }
  | { type: 'failed'; error: string }
  | { type: 'cancel' }

export function accountFlow(s: AccountFlow, e: AccountEvent): AccountFlow {
  switch (e.type) {
    case 'submit': {
      const email = e.email.trim().toLowerCase()
      if (!EMAIL.test(email)) return { kind: 'idle', error: '請輸入正確的 email' }
      return { kind: 'sending', email, since: e.now }
    }
    case 'sent':
      return s.kind === 'sending' ? { kind: 'waiting', email: s.email, poll: e.poll, since: s.since } : s
    case 'polled':
      if (s.kind !== 'waiting') return s
      if (e.done) return { kind: 'done' }
      if (e.now - s.since > LOGIN_WAIT_MS) return { kind: 'idle', error: '連結已過期，請重新寄一次' }
      return s
    case 'failed':
      return { kind: 'idle', error: e.error }
    case 'cancel':
      return { kind: 'idle' }
  }
}
