// 帳號服務（docs/design/accounts.md）。官網只用 Email 連結註冊、讀帳號、刪除帳號。
export const API_BASE: string = (import.meta.env?.VITE_API_BASE as string | undefined) ?? 'https://api.kunsu.app'

const json = (body: unknown): RequestInit => ({ method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })

export async function startSignup(email: string, f: typeof fetch = fetch): Promise<{ ok: boolean }> {
  const res = await f(`${API_BASE}/auth/email/start`, json({ email: email.trim().toLowerCase(), client: 'web' }))
  return { ok: res.status === 202 }
}

export type VerifyResult = { kind: 'app' } | { kind: 'web'; code: string } | { kind: 'deeplink'; url: string } | { kind: 'expired' }

/** 信裡的連結：App 在等就叫使用者回 App；網站註冊的帶 code 進歡迎頁；舊版 App 用 kunsu:// 帶回。 */
export async function verifyLink(token: string, f: typeof fetch = fetch): Promise<VerifyResult> {
  const res = await f(`${API_BASE}/auth/email/verify`, json({ token }))
  if (!res.ok) return { kind: 'expired' }
  const j = (await res.json()) as { code: string; redirect: string; waiting_app?: boolean }
  if (j.waiting_app) return { kind: 'app' }
  if (j.redirect.startsWith('kunsu://')) return { kind: 'deeplink', url: j.redirect }
  return { kind: 'web', code: j.code }
}

export async function exchangeCode(code: string, f: typeof fetch = fetch): Promise<string | null> {
  const res = await f(`${API_BASE}/auth/exchange`, json({ code }))
  if (!res.ok) return null
  return ((await res.json()) as { session: string }).session
}

export interface AccountInfo {
  email: string
  display_name?: string
  macs: { id?: string; name: string }[]
  devices: { id?: string; name: string }[]
}

export async function accountInfo(session: string, f: typeof fetch = fetch): Promise<AccountInfo | null> {
  const res = await f(`${API_BASE}/account`, { headers: { authorization: `Bearer ${session}` } })
  return res.ok ? ((await res.json()) as AccountInfo) : null
}

export async function deleteAccount(session: string, f: typeof fetch = fetch): Promise<boolean> {
  const res = await f(`${API_BASE}/account`, { method: 'DELETE', headers: { authorization: `Bearer ${session}` } })
  return res.status === 204 || res.status === 401
}
