// Kunsu 帳號服務＋中轉站（PRD §31.2；docs/design/accounts.md；BDD docs/bdd/accounts.md）。
// 雲端只存帳號基本資料與裝置清單（INV-33）；中轉站只轉密文（INV-30）。
import { createServer } from 'node:http'
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { jwtVerify, createRemoteJWKSet } from 'jose'
import { attachRelay } from './server.js'
import { memoryStore } from './accounts/store.js'

const MIN = 60 * 1000
const LINK_TTL = 15 * MIN
const CODE_TTL = 2 * MIN
const PURGE_AFTER = 30 * 24 * 60 * MIN
const EMAIL_LIMIT = { n: 5, window: 10 * MIN }
const IP_LIMIT = { n: 20, window: 60 * MIN }
const SESSION_TTL = 30 * 24 * 60 * MIN
const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
const MAC_ID = /^[a-z2-7]{16}$/
const CLIENTS = new Set(['web', 'mac', 'ios'])

const hash = (s) => createHash('sha256').update(s).digest('hex')
const secret = () => randomBytes(32).toString('base64url')

const ISSUERS = {
  google: ['https://accounts.google.com', 'accounts.google.com'],
  apple: ['https://appleid.apple.com'],
}
const DEFAULT_JWKS = {
  google: 'https://www.googleapis.com/oauth2/v3/certs',
  apple: 'https://appleid.apple.com/auth/keys',
}

/** Resend 寄信（正式環境）。測試注入假的 mailer。 */
export function resendMailer(apiKey, from = 'Kunsu <login@kunsu.app>') {
  return {
    async send({ to, subject, text, html }) {
      const res = await fetch('https://api.resend.com/emails', {
        method: 'POST',
        headers: { authorization: `Bearer ${apiKey}`, 'content-type': 'application/json' },
        body: JSON.stringify({ from, to, subject, text, html }),
      })
      if (!res.ok) throw new Error(`Resend ${res.status}`)
    },
  }
}

/** 信裡的連結（AC-AC-10）：自動完成登入，再告訴使用者回到 App 或帶回網站。官網上線後由官網的同一頁取代。 */
const VERIFY_PAGE = `<!doctype html><html lang="zh-Hant"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>登入 Kunsu</title><style>body{font:16px -apple-system,system-ui,sans-serif;background:#f5f5f5;color:#171717;display:grid;place-items:center;min-height:100vh;margin:0}
main{background:#fff;border:1px solid #e5e5e5;border-radius:16px;padding:32px;max-width:360px;text-align:center}h1{font-size:20px;margin:0 0 8px}p{color:#6b6b6b;margin:0}
@media (prefers-color-scheme:dark){body{background:#0a0a0a;color:#fafafa}main{background:#111;border-color:#262626}p{color:#a3a3a3}}</style></head>
<body><main><h1 id="t">正在登入…</h1><p id="m"></p></main><script>
(async()=>{const t=document.getElementById('t'),m=document.getElementById('m');
const token=new URLSearchParams(location.search).get('token')||'';
try{const r=await fetch('/auth/email/verify',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({token})});
const j=await r.json();if(!r.ok){t.textContent='連結已失效';m.textContent='回到 Kunsu 再寄一次登入連結。';return}
if(j.waiting_app){t.textContent='登入成功';m.textContent='可以回到 Kunsu 了，它會自動登入。';return}
location.href=j.redirect}catch(e){t.textContent='登入失敗';m.textContent='請稍後再試一次。'}})()
</script></body></html>`

export async function startCloud({
  port = Number(process.env.PORT) || 8080,
  store = memoryStore(),
  clock = Date.now,
  mailer,
  baseUrl = 'https://kunsu.app',
  google = {},
  apple = {},
  macToken = process.env.RELAY_MAC_TOKEN,
  // 官網（AC-AC-11）：只有這些網域能跨網域呼叫
  allowedOrigins = (process.env.CORS_ORIGINS ?? 'https://kunsu.app,https://www.kunsu.app').split(',').map((s) => s.trim()).filter(Boolean),
} = {}) {
  const jwks = {
    google: google.jwks ?? createRemoteJWKSet(new URL(DEFAULT_JWKS.google)),
    apple: apple.jwks ?? createRemoteJWKSet(new URL(DEFAULT_JWKS.apple)),
  }
  const audiences = { google: google.clientIds ?? [], apple: apple.clientIds ?? [] }
  const sent = { email: new Map(), ip: new Map() } // key → [時間]

  const underLimit = (map, key, { n, window }) => {
    const now = clock()
    const recent = (map.get(key) ?? []).filter((t) => now - t < window)
    if (recent.length >= n) { map.set(key, recent); return false }
    recent.push(now)
    map.set(key, recent)
    return true
  }

  // ------------------------------------------------------------ 帳號

  async function userByEmail(email) {
    const u = await store.find('users', { email })
    return u && !u.deleted_at ? u : null
  }

  async function signIn(provider, subject, email, name) {
    let ident = await store.find('identities', { provider, subject })
    let user = ident ? await store.find('users', { id: ident.user_id }) : null
    if (user?.deleted_at) user = null
    if (!user) user = await userByEmail(email)
    if (!user) {
      user = await store.insert('users', { id: randomUUID(), email, display_name: name || email.split('@')[0], created_at: clock() })
    }
    if (!ident || ident.user_id !== user.id) {
      await store.remove('identities', { provider, subject })
      await store.insert('identities', { user_id: user.id, provider, subject, created_at: clock() })
    }
    return user
  }

  /** 登入完成：發一次性的 code，App 回到 kunsu://，網頁回到 /welcome。 */
  async function issueCode(user, client) {
    const code = secret()
    await store.insert('login_tokens', { token_hash: hash(code), kind: 'code', user_id: user.id, client, expires_at: clock() + CODE_TTL })
    const redirect = client === 'web' ? `${baseUrl}/welcome?code=${code}` : `kunsu://auth?code=${code}`
    return { code, redirect }
  }

  async function consume(token, kind) {
    const row = await store.find('login_tokens', { token_hash: hash(token), kind })
    if (!row || row.used_at || clock() > row.expires_at) return null
    await store.update('login_tokens', { token_hash: row.token_hash }, { used_at: clock() })
    return row
  }

  async function sessionUser(req) {
    const h = req.headers.authorization ?? ''
    const cookie = /(?:^|;\s*)kunsu_session=([^;]+)/.exec(req.headers.cookie ?? '')?.[1]
    const token = h.startsWith('Bearer ') ? h.slice(7) : cookie
    if (!token) return null
    const s = await store.find('sessions', { token_hash: hash(token) })
    if (!s || s.revoked_at || clock() - s.created_at > SESSION_TTL) return null
    const u = await store.find('users', { id: s.user_id })
    if (!u || u.deleted_at) return null
    await store.update('sessions', { token_hash: s.token_hash }, { last_used_at: clock() })
    return u
  }

  async function verifyIdToken(provider, idToken) {
    if (typeof idToken !== 'string' || audiences[provider].length === 0) return null
    try {
      const { payload } = await jwtVerify(idToken, jwks[provider], {
        issuer: ISSUERS[provider],
        audience: audiences[provider],
        currentDate: new Date(Date.now()),
      })
      const verified = payload.email_verified === true || payload.email_verified === 'true'
      if (!payload.sub || typeof payload.email !== 'string' || !verified) return null
      return { subject: String(payload.sub), email: payload.email.toLowerCase(), name: typeof payload.name === 'string' ? payload.name : '' }
    } catch {
      return null
    }
  }

  async function revokeAll(userId) {
    await store.update('sessions', { user_id: userId }, { revoked_at: clock() })
    for (const m of await store.all('macs', { user_id: userId })) {
      await store.update('macs', { id: m.id }, { revoked_at: clock() })
      relay.kickMac(m.id)
    }
    await store.update('devices', { user_id: userId }, { revoked_at: clock() })
  }

  async function purge() {
    const now = clock()
    for (const u of await store.all('users', {})) {
      if (!u.deleted_at || now - u.deleted_at < PURGE_AFTER) continue
      for (const t of ['identities', 'sessions', 'macs', 'devices', 'login_tokens']) await store.remove(t, { user_id: u.id })
      await store.remove('users', { id: u.id })
    }
  }

  // ------------------------------------------------------------ HTTP

  const routes = {
    'POST /auth/email/start': async (req, body) => {
      const email = String(body.email ?? '').trim().toLowerCase()
      const client = CLIENTS.has(body.client) ? body.client : 'web'
      if (!EMAIL.test(email)) return [400, { error: '請輸入正確的 email' }]
      // AC-AC-12：寄信服務還沒接上就照實說，不假裝寄出
      if (!mailer) return [503, { error: '登入信暫時寄不出去，請稍後再試' }]
      const ip = String(req.headers['x-forwarded-for'] ?? req.socket.remoteAddress).split(',')[0].trim()
      // AC-AC-02／03：不論存不存在、有沒有超過上限，回應都一樣
      // AC-AC-09：App 拿 poll 等使用者在任何裝置點連結
      const poll = secret()
      if (underLimit(sent.email, email, EMAIL_LIMIT) && underLimit(sent.ip, ip, IP_LIMIT)) {
        const token = secret()
        await store.insert('login_tokens', { token_hash: hash(poll), kind: 'poll', email, client, expires_at: clock() + LINK_TTL })
        await store.insert('login_tokens', { token_hash: hash(token), kind: 'link', email, client, pair_hash: hash(poll), expires_at: clock() + LINK_TTL })
        const link = `${baseUrl}/auth/verify?token=${token}`
        await mailer.send({
          to: email,
          subject: '登入 Kunsu',
          text: `點這個連結登入 Kunsu（15 分鐘內有效，只能用一次）：\n${link}\n\n如果不是你要求的，忽略這封信就好。`,
          html: `<p>點下面的按鈕登入 Kunsu（15 分鐘內有效，只能用一次）。</p><p><a href="${link}">登入 Kunsu</a></p><p>如果不是你要求的，忽略這封信就好。</p>`,
        }).catch(() => {})
      }
      return [202, { ok: true, poll }]
    },
    'POST /auth/poll': async (_req, body) => {
      const row = await store.find('login_tokens', { token_hash: hash(String(body.poll ?? '')), kind: 'poll' })
      if (!row || row.used_at || clock() > row.expires_at) return [410, { error: '登入已失效，請重新寄一次' }]
      if (!row.user_id) return [202, { pending: true }]
      await store.update('login_tokens', { token_hash: row.token_hash }, { used_at: clock() })
      const session = secret()
      await store.insert('sessions', { token_hash: hash(session), user_id: row.user_id, client: row.client, created_at: clock() })
      return [200, { session }]
    },
    'POST /auth/email/verify': async (_req, body) => {
      const row = await consume(String(body.token ?? ''), 'link')
      if (!row) return [410, { error: '連結已失效，再寄一次' }]
      const user = await signIn('email', row.email, row.email, '')
      // 等著的 App（AC-AC-09）：把等待單標成可以領
      const waiting = row.pair_hash ? await store.update('login_tokens', { token_hash: row.pair_hash, kind: 'poll' }, { user_id: user.id }) : 0
      return [200, { ...(await issueCode(user, row.client)), waiting_app: waiting > 0 && row.client !== 'web' }]
    },
    'POST /auth/google/native': async (_req, body) => idTokenLogin('google', body),
    'POST /auth/apple/native': async (_req, body) => idTokenLogin('apple', body),
    'POST /auth/exchange': async (_req, body) => {
      const row = await consume(String(body.code ?? ''), 'code')
      if (!row) return [410, { error: '登入已失效，請重新登入' }]
      const session = secret()
      await store.insert('sessions', { token_hash: hash(session), user_id: row.user_id, client: row.client, created_at: clock() })
      return [200, { session }]
    },
    'GET /account': async (req) => {
      const u = await sessionUser(req)
      if (!u) return [401, { error: '請先登入' }]
      const providers = [...new Set((await store.all('identities', { user_id: u.id })).map((i) => i.provider))]
      const macs = (await store.all('macs', { user_id: u.id })).filter((m) => !m.revoked_at)
        .map(({ id, name, created_at, last_seen_at }) => ({ id, name, created_at, last_seen_at }))
      const devices = (await store.all('devices', { user_id: u.id })).filter((d) => !d.revoked_at)
        .map(({ id, name, created_at, last_seen_at }) => ({ id, name, created_at, last_seen_at }))
      return [200, { id: u.id, email: u.email, display_name: u.display_name, providers, macs, devices }]
    },
    'PATCH /account': async (req, body) => {
      const u = await sessionUser(req)
      if (!u) return [401, { error: '請先登入' }]
      const name = String(body.display_name ?? '').trim().slice(0, 40)
      if (name) await store.update('users', { id: u.id }, { display_name: name })
      return [204, null]
    },
    'DELETE /account': async (req) => {
      const u = await sessionUser(req)
      if (!u) return [401, { error: '請先登入' }]
      await revokeAll(u.id)
      await store.update('users', { id: u.id }, { deleted_at: clock() })
      return [204, null]
    },
    'POST /account/macs': async (req, body) => {
      const u = await sessionUser(req)
      if (!u) return [401, { error: '請先登入' }]
      const id = String(body.mac_id ?? '')
      if (!MAC_ID.test(id)) return [400, { error: 'mac_id 格式不對' }]
      const existing = await store.find('macs', { id })
      if (existing && existing.user_id !== u.id && !existing.revoked_at) return [409, { error: '這台 Mac 已經綁在別的帳號' }]
      await store.remove('macs', { id })
      const relayToken = secret()
      await store.insert('macs', {
        id, user_id: u.id, name: String(body.name ?? 'Mac').slice(0, 60),
        kx_pub: String(body.kx_pub ?? ''), sig_pub: String(body.sig_pub ?? ''),
        relay_token_hash: hash(relayToken), created_at: clock(),
      })
      return [201, { relay_token: relayToken }]
    },
  }

  async function idTokenLogin(provider, body) {
    const claims = await verifyIdToken(provider, body.id_token)
    if (!claims) return [401, { error: '登入驗證失敗' }]
    const user = await signIn(provider, claims.subject, claims.email, claims.name)
    return [200, await issueCode(user, CLIENTS.has(body.client) ? body.client : 'ios')]
  }

  const http = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://cloud')
    const origin = req.headers.origin
    if (origin && allowedOrigins.includes(origin)) {
      res.setHeader('access-control-allow-origin', origin)
      res.setHeader('vary', 'Origin')
      res.setHeader('access-control-allow-headers', 'content-type, authorization')
      res.setHeader('access-control-allow-methods', 'GET, POST, PATCH, DELETE, OPTIONS')
      res.setHeader('access-control-max-age', '600')
    }
    if (req.method === 'OPTIONS') return res.writeHead(204).end()
    const send = (status, body) => {
      res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' })
      res.end(body == null ? '' : JSON.stringify(body))
    }
    try {
      if (url.pathname === '/health') return send(200, { ok: true, macs: relay.macsOnline() })
      if (req.method === 'GET' && url.pathname === '/auth/verify') {
        res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', 'content-security-policy': "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'" })
        return res.end(VERIFY_PAGE)
      }
      let key = `${req.method} ${url.pathname}`
      let handler = routes[key]
      const macDel = /^\/account\/macs\/([a-z2-7]{16})$/.exec(url.pathname)
      if (!handler && req.method === 'DELETE' && macDel) {
        handler = async (r) => {
          const u = await sessionUser(r)
          if (!u) return [401, { error: '請先登入' }]
          const n = await store.update('macs', { id: macDel[1], user_id: u.id }, { revoked_at: clock() })
          relay.kickMac(macDel[1])
          return n ? [204, null] : [404, { error: '找不到這台 Mac' }]
        }
      }
      if (!handler) return send(404, { error: 'not found' })
      let raw = ''
      for await (const chunk of req) {
        raw += chunk
        if (raw.length > 16 * 1024) return send(413, { error: 'too large' })
      }
      const body = raw ? JSON.parse(raw) : {}
      const [status, out] = await handler(req, body)
      send(status, out)
    } catch {
      send(400, { error: 'bad request' })
    }
  })

  const relay = attachRelay(http, {
    macToken,
    clock,
    macAuth: async (mac, header) => {
      if (!header?.startsWith('Bearer ')) return false
      const m = await store.find('macs', { id: mac })
      if (!m || m.revoked_at || m.relay_token_hash !== hash(header.slice(7))) return false
      const u = await store.find('users', { id: m.user_id })
      if (!u || u.deleted_at) return false
      await store.update('macs', { id: mac }, { last_seen_at: clock() })
      return true
    },
  })

  await new Promise((r) => http.listen(port, r))
  return {
    port: http.address().port,
    store,
    purge,
    close: () => new Promise((r) => { relay.close(); http.close(() => r()) }),
  }
}
