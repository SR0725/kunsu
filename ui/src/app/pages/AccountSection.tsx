// 設定 › 帳號（PRD §31.2；BDD multi-user AC-MU-03）：寄登入連結 → 在任何裝置點 → 這台 Mac 自動登入並註冊。
import { useEffect, useReducer, useState } from 'react'
import type { KunsuIpc } from '../../ipc'
import { accountFlow, type AccountInfo } from '../settings/account'
import { Button, SettingsCard, useToast } from '../ui'

const POLL_MS = 2000

export function AccountSection({ ipc }: { ipc: KunsuIpc }) {
  const toast = useToast()
  const [account, setAccount] = useState<AccountInfo | null | undefined>(undefined)
  const [email, setEmail] = useState('')
  const [flow, dispatch] = useReducer(accountFlow, { kind: 'idle' })
  const [confirmLogout, setConfirmLogout] = useState(false)

  useEffect(() => {
    ipc.accountStatus().then(setAccount, () => setAccount(null))
  }, [ipc])

  // 寄信
  useEffect(() => {
    if (flow.kind !== 'sending') return
    ipc.accountStartLogin(flow.email).then(
      (poll) => dispatch({ type: 'sent', poll }),
      (e) => dispatch({ type: 'failed', error: e instanceof Error ? e.message : String(e) }),
    )
  }, [flow, ipc])

  // 等使用者點連結
  useEffect(() => {
    if (flow.kind !== 'waiting') return
    const t = window.setTimeout(() => {
      ipc.accountPoll(flow.poll).then(
        (done) => dispatch({ type: 'polled', done, now: Date.now() }),
        (e) => dispatch({ type: 'failed', error: e instanceof Error ? e.message : String(e) }),
      )
    }, POLL_MS)
    return () => window.clearTimeout(t)
  }, [flow, ipc])

  useEffect(() => {
    if (flow.kind !== 'done') return
    ipc.accountStatus().then((a) => {
      setAccount(a)
      toast('已登入，這台 Mac 已加入你的帳號')
    })
    dispatch({ type: 'cancel' })
  }, [flow, ipc, toast])

  if (account === undefined) return null

  if (account) {
    return (
      <SettingsCard
        title="帳號"
        description="這台 Mac 已經加入你的 Kunsu 帳號，手機登入同一個帳號就找得到它。題目、答案與對話只留在這台 Mac。"
        footer={<p className="k-card__help">登出會把這台 Mac 從帳號移除，手機會連不到它，直到再次登入。</p>}
      >
        <div className="k-stack">
          <p className="k-sentence">
            <span>
              已登入：<strong>{account.email}</strong>
            </span>
          </p>
          {confirmLogout ? (
            <div className="k-mobile__actions">
              <Button onClick={() => setConfirmLogout(false)}>取消</Button>
              <Button
                variant="danger"
                onClick={async () => {
                  await ipc.accountLogout()
                  setAccount(null)
                  setConfirmLogout(false)
                  toast('已登出')
                }}
              >
                確定登出
              </Button>
            </div>
          ) : (
            <div>
              <Button onClick={() => setConfirmLogout(true)}>登出</Button>
            </div>
          )}
        </div>
      </SettingsCard>
    )
  }

  const busy = flow.kind === 'sending' || flow.kind === 'waiting'
  return (
    <SettingsCard
      title="登入 Kunsu"
      description="用 email 登入，把這台 Mac 和你的手機串起來。雲端只存你的 email 和裝置清單；題目、答案與對話只留在這台 Mac。"
      footer={<p className="k-card__help">登入連結會寄到你的信箱，15 分鐘內有效。</p>}
    >
      {flow.kind === 'waiting' ? (
        <div className="k-stack" aria-live="polite">
          <p className="k-sentence">
            <span>
              已寄登入連結到 <strong>{flow.email}</strong>。在任何裝置點信裡的連結，這裡就會自動登入（15 分鐘內有效）。
            </span>
          </p>
          <div>
            <Button onClick={() => dispatch({ type: 'cancel' })}>改用別的 email</Button>
          </div>
        </div>
      ) : (
        <form
          className="k-stack"
          onSubmit={(e) => {
            e.preventDefault()
            dispatch({ type: 'submit', email, now: Date.now() })
          }}
        >
          <div className="k-field">
            <label className="k-field__label" htmlFor="account-email">
              Email
            </label>
            <div className="k-input" data-invalid={flow.kind === 'idle' && flow.error ? 'true' : undefined}>
              <input
                id="account-email"
                className="k-input__control"
                type="email"
                autoComplete="email"
                spellCheck={false}
                placeholder="you@example.com"
                value={email}
                disabled={busy}
                aria-invalid={flow.kind === 'idle' && flow.error ? true : undefined}
                aria-describedby={flow.kind === 'idle' && flow.error ? 'account-email-error' : undefined}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>
            {flow.kind === 'idle' && flow.error && (
              <p id="account-email-error" role="alert" className="k-field__error">
                {flow.error}
              </p>
            )}
          </div>
          <div>
            <Button variant="primary" type="submit" disabled={busy || !email.trim()}>
              {flow.kind === 'sending' ? '寄送中…' : '寄登入連結'}
            </Button>
          </div>
        </form>
      )}
    </SettingsCard>
  )
}
