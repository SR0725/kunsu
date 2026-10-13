// 官網的純邏輯（BDD docs/bdd/website.md）。
import { describe, expect, it, vi } from 'vitest'
import { startSignup, verifyLink, exchangeCode, accountInfo, deleteAccount, API_BASE } from './api'
import { snapToEdge, type Edge } from './notch'
import { phoneDemo, type PhoneState } from './phoneDemo'
import { signupFlow } from './signup'

function fakeFetch(status: number, body: unknown) {
  const calls: { url: string; init?: RequestInit }[] = []
  const f = vi.fn(async (url: string, init?: RequestInit) => {
    calls.push({ url, init })
    return new Response(body == null ? null : JSON.stringify(body), { status })
  })
  return { f: f as unknown as typeof fetch, calls }
}

describe('AC-WEB-01／06／07 帳號 API', () => {
  it('註冊寄連結用 client=web', async () => {
    const { f, calls } = fakeFetch(202, { ok: true, poll: 'p' })
    expect(await startSignup('Alex@Example.com', f)).toEqual({ ok: true })
    expect(calls[0].url).toBe(`${API_BASE}/auth/email/start`)
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({ email: 'alex@example.com', client: 'web' })
  })
  it('登入連結：App 在等、網站註冊、失效三種結果', async () => {
    expect(await verifyLink('t', fakeFetch(200, { code: 'c', redirect: 'kunsu://auth?code=c', waiting_app: true }).f)).toEqual({ kind: 'app' })
    expect(await verifyLink('t', fakeFetch(200, { code: 'c', redirect: 'https://kunsu.app/welcome?code=c', waiting_app: false }).f)).toEqual({ kind: 'web', code: 'c' })
    expect(await verifyLink('t', fakeFetch(200, { code: 'c', redirect: 'kunsu://auth?code=c', waiting_app: false }).f)).toEqual({ kind: 'deeplink', url: 'kunsu://auth?code=c' })
    expect(await verifyLink('t', fakeFetch(410, { error: 'x' }).f)).toEqual({ kind: 'expired' })
  })
  it('換 session、讀帳號、刪除帳號', async () => {
    expect(await exchangeCode('c', fakeFetch(200, { session: 's' }).f)).toBe('s')
    const acc = fakeFetch(200, { email: 'a@b.co', macs: [{ name: 'M' }], devices: [] })
    expect(await accountInfo('s', acc.f)).toMatchObject({ email: 'a@b.co' })
    expect((acc.calls[0].init?.headers as Record<string, string>).authorization).toBe('Bearer s')
    const del = fakeFetch(204, null)
    expect(await deleteAccount('s', del.f)).toBe(true)
    expect(del.calls[0].init?.method).toBe('DELETE')
  })
})

describe('AC-WEB-01 註冊表單流程', () => {
  it('格式錯、寄送中、已寄出、失敗', () => {
    expect(signupFlow({ kind: 'idle' }, { type: 'submit', email: 'x' })).toEqual({ kind: 'idle', error: '請輸入正確的 email' })
    const s = signupFlow({ kind: 'idle' }, { type: 'submit', email: ' A@B.co ' })
    expect(s).toEqual({ kind: 'sending', email: 'a@b.co' })
    expect(signupFlow(s, { type: 'sent' })).toEqual({ kind: 'sent', email: 'a@b.co' })
    expect(signupFlow(s, { type: 'failed' })).toEqual({ kind: 'idle', error: '現在寄不出去，請稍後再試' })
  })
})

describe('AC-WEB-02 邊緣瀏海：放開吸到最近的邊', () => {
  const W = 800, H = 500
  const cases: [number, number, Edge][] = [[400, 10, 'top'], [790, 250, 'right'], [400, 495, 'bottom'], [5, 250, 'left'], [780, 30, 'right']]
  for (const [x, y, edge] of cases) {
    it(`(${x},${y}) → ${edge}`, () => expect(snapToEdge(x, y, W, H).edge).toBe(edge))
  }
  it('沿邊的位置保留在 0～1', () => {
    expect(snapToEdge(400, 10, W, H).along).toBeCloseTo(0.5)
    expect(snapToEdge(-50, 600, W, H).along).toBeGreaterThanOrEqual(0)
    expect(snapToEdge(9999, -9, W, H).along).toBeLessThanOrEqual(1)
  })
})

describe('AC-WEB-04 手機示範', () => {
  const start = phoneDemo.initial()
  it('一開始只有問候與第一題，看不到歷史', () => {
    expect(start.current?.id).toBe('q1')
    expect(start.historyShown).toBe(false)
    expect(start.answered).toEqual([])
  })
  it('點答案後 5 秒內可撤回，撤回回到原狀', () => {
    let s: PhoneState = phoneDemo.answer(start, 0)
    expect(s.answered).toHaveLength(1)
    expect(s.undoable).toBe(true)
    expect(s.current?.id).toBe('q2')
    s = phoneDemo.undo(s)
    expect(s).toEqual({ ...start })
  })
  it('全部答完：沒有事在等你', () => {
    let s = start
    while (s.current) s = phoneDemo.commit(phoneDemo.answer(s, 0))
    expect(s.answered).toHaveLength(phoneDemo.questions.length)
    expect(s.undoable).toBe(false)
  })
  it('下拉沒滿不出現歷史；滿了放開才出現', () => {
    expect(phoneDemo.release(start, phoneDemo.PULL - 1).historyShown).toBe(false)
    expect(phoneDemo.release(start, phoneDemo.PULL).historyShown).toBe(true)
    expect(phoneDemo.progress(phoneDemo.PULL / 2)).toBeCloseTo(0.5)
    expect(phoneDemo.progress(phoneDemo.PULL * 3)).toBe(1)
  })
})
