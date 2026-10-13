// 首屏與頁尾的註冊表單（AC-WEB-01）。
import { useReducer, useState } from 'react'
import { MailCheck } from 'lucide-react'
import { startSignup } from '../lib/api'
import { signupFlow } from '../lib/signup'

export function SignupForm({ id }: { id: string }) {
  const [email, setEmail] = useState('')
  const [state, dispatch] = useReducer(signupFlow, { kind: 'idle' })

  if (state.kind === 'sent') {
    return (
      <div className="sent" role="status">
        <MailCheck size={22} aria-hidden="true" />
        <div>
          <strong>去收信吧</strong>
          <span>
            登入連結已寄到 {state.email}，15 分鐘內有效。
            <button className="danger" style={{ color: 'var(--muted)', marginLeft: 6 }} onClick={() => dispatch({ type: 'reset' })}>
              換個 email
            </button>
          </span>
        </div>
      </div>
    )
  }

  const error = state.kind === 'idle' ? state.error : undefined
  return (
    <form
      onSubmit={async (e) => {
        e.preventDefault()
        const next = signupFlow(state, { type: 'submit', email })
        dispatch({ type: 'submit', email })
        if (next.kind !== 'sending') return
        try {
          const r = await startSignup(next.email)
          dispatch({ type: r.ok ? 'sent' : 'failed' })
        } catch {
          dispatch({ type: 'failed' })
        }
      }}
    >
      <div className="signup" data-invalid={error ? 'true' : undefined}>
        <label className="sr-only" htmlFor={id}>Email</label>
        <input
          id={id}
          type="email"
          autoComplete="email"
          placeholder="you@example.com"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          aria-invalid={error ? true : undefined}
          aria-describedby={`${id}-note`}
        />
        <button className="btn btn--primary" type="submit" disabled={state.kind === 'sending'}>
          {state.kind === 'sending' ? '寄送中…' : '免費註冊'}
        </button>
      </div>
      <p id={`${id}-note`} className="signup__note" data-error={error ? 'true' : undefined} role={error ? 'alert' : undefined}>
        {error ?? '用 email 註冊，不用密碼。'}
      </p>
    </form>
  )
}
