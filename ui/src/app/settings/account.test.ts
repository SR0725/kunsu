// 主視窗「設定 › 帳號」（BDD multi-user AC-MU-03；PRD §31.2）。
import { describe, expect, it } from 'vitest'
import { settingsTauriIpc } from './settingsIpc'
import { makeSettingsMock } from './settingsMock'
import { SETTINGS_SECTIONS } from './sections'
import { accountFlow, LOGIN_WAIT_MS, type AccountFlow } from './account'

describe('AC-MU-03 帳號 IPC', () => {
  it('命令名稱與參數形狀', async () => {
    const calls: { cmd: string; args?: Record<string, unknown> }[] = []
    const ipc = settingsTauriIpc(async <T,>(cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args })
      return {} as T
    })
    await ipc.accountStatus()
    await ipc.accountStartLogin('alex@example.com')
    await ipc.accountPoll('p1')
    await ipc.accountLogout()
    expect(calls).toEqual([
      { cmd: 'account_status', args: undefined },
      { cmd: 'account_start_login', args: { email: 'alex@example.com' } },
      { cmd: 'account_poll', args: { poll: 'p1' } },
      { cmd: 'account_logout', args: undefined },
    ])
  })

  it('mock：寄信 → 等 → 登入 → 登出', async () => {
    const mock = makeSettingsMock({})
    expect(await mock.accountStatus()).toBeNull()
    const poll = await mock.accountStartLogin('alex@example.com')
    expect(await mock.accountPoll(poll)).toBe(true)
    expect((await mock.accountStatus())?.email).toBe('alex@example.com')
    await mock.accountLogout()
    expect(await mock.accountStatus()).toBeNull()
  })
})

describe('登入流程（純函數）', () => {
  const t0 = 1_000_000
  it('email 格式不對不寄', () => {
    expect(accountFlow({ kind: 'idle' }, { type: 'submit', email: 'nope', now: t0 })).toEqual({ kind: 'idle', error: '請輸入正確的 email' })
  })
  it('寄出後等待，點了連結就登入', () => {
    let s: AccountFlow = accountFlow({ kind: 'idle' }, { type: 'submit', email: ' Alex@Example.com ', now: t0 })
    expect(s).toEqual({ kind: 'sending', email: 'alex@example.com', since: t0 })
    s = accountFlow(s, { type: 'sent', poll: 'p1' })
    expect(s).toEqual({ kind: 'waiting', email: 'alex@example.com', poll: 'p1', since: t0 })
    s = accountFlow(s, { type: 'polled', done: false, now: t0 + 2000 })
    expect(s.kind).toBe('waiting')
    s = accountFlow(s, { type: 'polled', done: true, now: t0 + 4000 })
    expect(s).toEqual({ kind: 'done' })
  })
  it('等超過 15 分鐘就請使用者重寄；錯誤寫出原因', () => {
    const waiting: AccountFlow = { kind: 'waiting', email: 'a@b.co', poll: 'p', since: t0 }
    expect(accountFlow(waiting, { type: 'polled', done: false, now: t0 + LOGIN_WAIT_MS + 1 })).toEqual({ kind: 'idle', error: '連結已過期，請重新寄一次' })
    expect(accountFlow(waiting, { type: 'failed', error: '連不上帳號服務' })).toEqual({ kind: 'idle', error: '連不上帳號服務' })
  })
})

describe('設定頁有「帳號」分頁', () => {
  it('排在一般之後、手機之前', () => {
    expect(SETTINGS_SECTIONS.map((s) => s.id).slice(0, 3)).toEqual(['general', 'account', 'mobile'])
  })
})
