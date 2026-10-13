// 首屏註冊表單（AC-WEB-01）。
export type SignupState = { kind: 'idle'; error?: string } | { kind: 'sending'; email: string } | { kind: 'sent'; email: string }
export type SignupEvent = { type: 'submit'; email: string } | { type: 'sent' } | { type: 'failed' } | { type: 'reset' }

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

export function signupFlow(s: SignupState, e: SignupEvent): SignupState {
  switch (e.type) {
    case 'submit': {
      const email = e.email.trim().toLowerCase()
      return EMAIL.test(email) ? { kind: 'sending', email } : { kind: 'idle', error: '請輸入正確的 email' }
    }
    case 'sent':
      return s.kind === 'sending' ? { kind: 'sent', email: s.email } : s
    case 'failed':
      return { kind: 'idle', error: '現在寄不出去，請稍後再試' }
    case 'reset':
      return { kind: 'idle' }
  }
}
