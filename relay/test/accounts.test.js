// 帳號服務驗收（BDD docs/bdd/accounts.md AC-AC-01～08；INVARIANTS 33～35 Target）。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import WebSocket from 'ws'
import { generateKeyPair, exportJWK, SignJWT, createLocalJWKSet } from 'jose'
import { startCloud } from '../cloud.js'
import { memoryStore, ALLOWED_COLUMNS } from '../accounts/store.js'
import { pgStore } from '../accounts/pg.js'

// KUNSU_TEST_PG=postgres://… 時同一套驗收跑在真的 Postgres 上（正式環境用的實作）。
const PG = process.env.KUNSU_TEST_PG
let shared
async function makeStore() {
  if (!PG) return memoryStore()
  shared ??= await pgStore(PG)
  await shared.reset()
  return shared
}

const T0 = Date.parse('2026-10-13T00:00:00Z')

async function idp(iss) {
  const { publicKey, privateKey } = await generateKeyPair('RS256')
  const jwk = { ...(await exportJWK(publicKey)), kid: 'k1', alg: 'RS256' }
  const sign = (claims, { aud = 'client-id', exp = '10m', key = privateKey } = {}) =>
    new SignJWT(claims).setProtectedHeader({ alg: 'RS256', kid: 'k1' }).setIssuer(iss).setAudience(aud)
      .setIssuedAt().setExpirationTime(exp).sign(key)
  return { jwks: createLocalJWKSet({ keys: [jwk] }), sign }
}

const opened = []
test.afterEach(async () => { while (opened.length) await opened.pop().close() })

async function cloud({ noMailer = false } = {}) {
  let now = T0
  const mail = []
  const google = await idp('https://accounts.google.com')
  const apple = await idp('https://appleid.apple.com')
  const c = await startCloud({
    port: 0,
    store: await makeStore(),
    clock: () => now,
    mailer: noMailer ? null : { send: async (m) => mail.push(m) },
    baseUrl: 'https://kunsu.app',
    google: { clientIds: ['client-id'], jwks: google.jwks },
    apple: { clientIds: ['client-id'], jwks: apple.jwks },
    macToken: 'legacy',
  })
  opened.push(c)
  const base = `http://127.0.0.1:${c.port}`
  const call = async (method, path, body, { token, ip = '1.1.1.1' } = {}) => {
    const res = await fetch(base + path, {
      method,
      headers: { 'content-type': 'application/json', 'x-forwarded-for': ip, ...(token ? { authorization: `Bearer ${token}` } : {}) },
      body: body ? JSON.stringify(body) : undefined,
      redirect: 'manual',
    })
    const text = await res.text()
    let json = null
    try { json = JSON.parse(text) } catch {}
    return { status: res.status, json, headers: res.headers }
  }
  const linkToken = (m) => new URL(m.text.match(/https:\/\/kunsu\.app\/auth\/verify\?token=\S+/)[0]).searchParams.get('token')
  return { c, call, mail, google, apple, linkToken, advance: (ms) => { now += ms }, base }
}

/** Email 登入到拿到 bearer session。 */
async function emailLogin(env, email = 'alex@example.com', client = 'mac') {
  await env.call('POST', '/auth/email/start', { email, client })
  const token = env.linkToken(env.mail.at(-1))
  const v = await env.call('POST', '/auth/email/verify', { token })
  assert.equal(v.status, 200, JSON.stringify(v.json))
  const ex = await env.call('POST', '/auth/exchange', { code: v.json.code })
  assert.equal(ex.status, 200)
  return ex.json.session
}

test('AC-AC-01 Email 連結：寄出、單次、15 分鐘', async () => {
  const env = await cloud()
  const r = await env.call('POST', '/auth/email/start', { email: 'Alex@Example.com', client: 'web' })
  assert.equal(r.status, 202)
  assert.equal(env.mail.length, 1)
  assert.equal(env.mail[0].to, 'alex@example.com')
  assert.match(env.mail[0].text, /https:\/\/kunsu\.app\/auth\/verify\?token=/)
  const token = env.linkToken(env.mail[0])
  const ok = await env.call('POST', '/auth/email/verify', { token })
  assert.equal(ok.status, 200)
  assert.ok(ok.json.code)
  const again = await env.call('POST', '/auth/email/verify', { token })
  assert.equal(again.status, 410, '只能用一次')

  await env.call('POST', '/auth/email/start', { email: 'late@example.com', client: 'web' })
  const late = env.linkToken(env.mail.at(-1))
  env.advance(15 * 60 * 1000 + 1)
  assert.equal((await env.call('POST', '/auth/email/verify', { token: late })).status, 410, '過期')
  await env.c.close()
})

test('AC-AC-02 要求連結不透露帳號是否存在', async () => {
  const env = await cloud()
  await emailLogin(env, 'exists@example.com')
  const a = await env.call('POST', '/auth/email/start', { email: 'exists@example.com', client: 'web' })
  const b = await env.call('POST', '/auth/email/start', { email: 'nobody@example.com', client: 'web' })
  assert.equal(a.status, b.status)
  assert.deepEqual(Object.keys(a.json).sort(), Object.keys(b.json).sort(), '回應長得一樣（poll 是隨機值）')
  assert.equal(typeof a.json.poll, typeof b.json.poll)
  assert.equal((await env.call('POST', '/auth/email/start', { email: 'not-an-email', client: 'web' })).status, 400)
  await env.c.close()
})

test('AC-AC-03 寄信上限', async () => {
  const env = await cloud()
  for (let i = 0; i < 6; i++) await env.call('POST', '/auth/email/start', { email: 'a@example.com', client: 'web' })
  assert.equal(env.mail.length, 5, '同一個 email 10 分鐘最多 5 封')
  env.advance(10 * 60 * 1000 + 1)
  await env.call('POST', '/auth/email/start', { email: 'a@example.com', client: 'web' })
  assert.equal(env.mail.length, 6)
  const before = env.mail.length
  for (let i = 0; i < 25; i++) await env.call('POST', '/auth/email/start', { email: `p${i}@example.com`, client: 'web' }, { ip: '9.9.9.9' })
  assert.equal(env.mail.length - before, 20, '同一個 IP 一小時最多 20 封')
  await env.c.close()
})

test('AC-AC-04 Google／Apple id_token 驗簽並以 email 合併帳號', async () => {
  const env = await cloud()
  const g = await env.google.sign({ sub: 'g-1', email: 'alex@example.com', email_verified: true, name: 'Alex' })
  const r1 = await env.call('POST', '/auth/google/native', { id_token: g, client: 'ios' })
  assert.equal(r1.status, 200, JSON.stringify(r1.json))
  const s1 = (await env.call('POST', '/auth/exchange', { code: r1.json.code })).json.session
  const a = await env.apple.sign({ sub: 'a-1', email: 'alex@example.com', email_verified: 'true' })
  const r2 = await env.call('POST', '/auth/apple/native', { id_token: a, client: 'ios' })
  assert.equal(r2.status, 200)
  const s2 = (await env.call('POST', '/auth/exchange', { code: r2.json.code })).json.session
  const acc1 = (await env.call('GET', '/account', null, { token: s1 })).json
  const acc2 = (await env.call('GET', '/account', null, { token: s2 })).json
  assert.equal(acc1.id, acc2.id, '同 email 是同一個帳號')
  assert.deepEqual([...acc2.providers].sort(), ['apple', 'google'])
  assert.equal(acc1.display_name, 'Alex')

  const wrongAud = await env.google.sign({ sub: 'g-2', email: 'x@example.com', email_verified: true }, { aud: 'someone-else' })
  assert.equal((await env.call('POST', '/auth/google/native', { id_token: wrongAud, client: 'ios' })).status, 401)
  const expired = await env.google.sign({ sub: 'g-3', email: 'y@example.com', email_verified: true }, { exp: Math.floor(Date.now() / 1000) - 60 })
  assert.equal((await env.call('POST', '/auth/google/native', { id_token: expired, client: 'ios' })).status, 401)
  const forged = (await env.apple.sign({ sub: 'a-2', email: 'z@example.com' })).replace(/.$/, (c) => (c === 'A' ? 'B' : 'A'))
  assert.equal((await env.call('POST', '/auth/apple/native', { id_token: forged, client: 'ios' })).status, 401)
  const unverified = await env.google.sign({ sub: 'g-4', email: 'u@example.com', email_verified: false })
  assert.equal((await env.call('POST', '/auth/google/native', { id_token: unverified, client: 'ios' })).status, 401, '未驗證的 email 不合併、不登入')
  await env.c.close()
})

test('AC-AC-05 App 登入 code 單次、2 分鐘', async () => {
  const env = await cloud()
  await env.call('POST', '/auth/email/start', { email: 'c@example.com', client: 'mac' })
  const v = await env.call('POST', '/auth/email/verify', { token: env.linkToken(env.mail.at(-1)) })
  assert.equal(v.json.redirect, `kunsu://auth?code=${v.json.code}`, 'App 登入回到 kunsu://')
  assert.equal((await env.call('POST', '/auth/exchange', { code: v.json.code })).status, 200)
  assert.equal((await env.call('POST', '/auth/exchange', { code: v.json.code })).status, 410)

  await env.call('POST', '/auth/email/start', { email: 'd@example.com', client: 'mac' })
  const v2 = await env.call('POST', '/auth/email/verify', { token: env.linkToken(env.mail.at(-1)) })
  env.advance(2 * 60 * 1000 + 1)
  assert.equal((await env.call('POST', '/auth/exchange', { code: v2.json.code })).status, 410)
  await env.c.close()
})

function connectMac(env, mac, token) {
  const url = env.base.replace('http', 'ws') + `/v1/connect?role=mac&mac=${mac}`
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url, { headers: token ? { authorization: `Bearer ${token}` } : {} })
    ws.once('open', () => { ws.close(); resolve(true) })
    ws.once('unexpected-response', (_q, res) => resolve(res.statusCode))
    ws.once('error', reject)
  })
}

test('AC-AC-06 Mac 註冊拿到中轉站憑證；撤銷後立刻失效', async () => {
  const env = await cloud()
  const s = await emailLogin(env)
  assert.equal((await env.call('POST', '/account/macs', { mac_id: 'abcdefghijklmnop', name: 'MacBook', kx_pub: 'k', sig_pub: 's' })).status, 401, '要登入')
  const reg = await env.call('POST', '/account/macs', { mac_id: 'abcdefghijklmnop', name: 'MacBook', kx_pub: 'k', sig_pub: 's' }, { token: s })
  assert.equal(reg.status, 201)
  assert.ok(reg.json.relay_token)
  assert.equal(await connectMac(env, 'abcdefghijklmnop', reg.json.relay_token), true)
  assert.equal(await connectMac(env, 'abcdefghijklmnop', 'wrong'), 401)
  assert.equal(await connectMac(env, 'zzzzzzzzzzzzzzzz', reg.json.relay_token), 401, '憑證只對自己的 Mac 有效')
  const acc = (await env.call('GET', '/account', null, { token: s })).json
  assert.equal(acc.macs.length, 1)
  assert.equal(acc.macs[0].name, 'MacBook')
  assert.equal(JSON.stringify(acc).includes(reg.json.relay_token), false, '憑證不再出現')
  assert.equal((await env.call('DELETE', '/account/macs/abcdefghijklmnop', null, { token: s })).status, 204)
  assert.equal(await connectMac(env, 'abcdefghijklmnop', reg.json.relay_token), 401)
  assert.equal(await connectMac(env, 'legacy-mac-id-00', 'legacy'), true, '相容期：舊的 RELAY_MAC_TOKEN 仍可用')
  await env.c.close()
})

test('AC-AC-07 刪除帳號：立刻撤銷，30 天後清除', async () => {
  const env = await cloud()
  const s = await emailLogin(env)
  const reg = await env.call('POST', '/account/macs', { mac_id: 'abcdefghijklmnop', name: 'M', kx_pub: 'k', sig_pub: 's' }, { token: s })
  assert.equal((await env.call('DELETE', '/account', null, { token: s })).status, 204)
  assert.equal((await env.call('GET', '/account', null, { token: s })).status, 401)
  assert.equal(await connectMac(env, 'abcdefghijklmnop', reg.json.relay_token), 401)
  assert.equal((await env.c.store.dump()).users.length, 1, '30 天內還在（標記刪除）')
  env.advance(30 * 24 * 3600 * 1000 + 1)
  await env.c.purge()
  const d = (await env.c.store.dump())
  assert.equal(d.users.length, 0)
  assert.equal(d.macs.length, 0)
  assert.equal(d.identities.length, 0)
  assert.equal(d.sessions.length, 0)
  await env.c.close()
})

test('AC-AC-08／INV-33 雲端只存允許的欄位（inv_cloud_stores_no_content）', async () => {
  const env = await cloud()
  const s = await emailLogin(env)
  await env.call('POST', '/account/macs', { mac_id: 'abcdefghijklmnop', name: 'M', kx_pub: 'k', sig_pub: 's', question: '不該存', answer: 'x' }, { token: s })
  await env.call('PATCH', '/account', { display_name: 'R', evidence: '不該存' }, { token: s })
  const d = (await env.c.store.dump())
  for (const [table, rows] of Object.entries(d)) {
    for (const row of rows) {
      for (const col of Object.keys(row)) assert.ok(ALLOWED_COLUMNS[table]?.includes(col), `${table}.${col} 不在允許清單`)
    }
  }
  assert.deepEqual(Object.keys(ALLOWED_COLUMNS).sort(), ['devices', 'identities', 'login_tokens', 'macs', 'sessions', 'users'])
  const banned = /question|answer|evidence|body|content|path|conversation|draft/
  for (const cols of Object.values(ALLOWED_COLUMNS)) for (const c of cols) assert.doesNotMatch(c, banned)
  await env.c.close()
})

test('INV-33 Postgres 的資料表欄位與允許清單一致', { skip: !PG }, async () => {
  const s = await makeStore()
  const cols = await s.columns()
  for (const [table, allowed] of Object.entries(ALLOWED_COLUMNS)) assert.deepEqual([...cols[table]].sort(), [...allowed].sort(), table)
  assert.deepEqual(Object.keys(cols).sort(), Object.keys(ALLOWED_COLUMNS).sort(), '沒有別的表')
})

test.after(async () => { await shared?.close() })

test('AC-AC-09 App 等待登入：在任何裝置點信裡的連結，等著的 App 自動登入', async () => {
  const env = await cloud()
  const start = await env.call('POST', '/auth/email/start', { email: 'mac@example.com', client: 'mac' })
  assert.equal(start.status, 202)
  assert.ok(start.json.poll, '回一個等待用的 id')
  const fake = await env.call('POST', '/auth/email/start', { email: 'nobody-else@example.com', client: 'mac' })
  assert.deepEqual(Object.keys(fake.json).sort(), Object.keys(start.json).sort(), '回應長得一樣，不透露帳號存在與否')
  assert.equal((await env.call('POST', '/auth/poll', { poll: start.json.poll })).status, 202, '還沒點連結')
  const v = await env.call('POST', '/auth/email/verify', { token: env.linkToken(env.mail.at(-2)) })
  assert.equal(v.status, 200)
  assert.equal(v.json.waiting_app, true, '網頁告訴使用者回到 App')
  const got = await env.call('POST', '/auth/poll', { poll: start.json.poll })
  assert.equal(got.status, 200)
  assert.ok(got.json.session)
  assert.equal((await env.call('GET', '/account', null, { token: got.json.session })).json.email, 'mac@example.com')
  assert.equal((await env.call('POST', '/auth/poll', { poll: start.json.poll })).status, 410, '只能領一次')
  assert.equal((await env.call('POST', '/auth/poll', { poll: 'made-up' })).status, 410)
  env.advance(16 * 60 * 1000)
  await env.c.close()
})

test('AC-AC-10 信裡的連結打開一個會自己完成登入的頁面', async () => {
  const env = await cloud()
  const res = await fetch(`${env.base}/auth/verify?token=abc`)
  assert.equal(res.status, 200)
  assert.match(res.headers.get('content-type'), /text\/html/)
  const html = await res.text()
  assert.match(html, /\/auth\/email\/verify/)
  assert.doesNotMatch(html, /<script[^>]+src=/, '不載入外部程式')
  await env.c.close()
})

test('AC-AC-11 官網 kunsu.app 可以跨網域呼叫帳號服務；其他網域不行', async () => {
  const env = await cloud()
  const pre = await fetch(`${env.base}/auth/email/start`, { method: 'OPTIONS', headers: { origin: 'https://kunsu.app', 'access-control-request-method': 'POST', 'access-control-request-headers': 'content-type' } })
  assert.equal(pre.status, 204)
  assert.equal(pre.headers.get('access-control-allow-origin'), 'https://kunsu.app')
  assert.match(pre.headers.get('access-control-allow-headers') ?? '', /authorization/i)
  const res = await fetch(`${env.base}/auth/email/start`, { method: 'POST', headers: { origin: 'https://kunsu.app', 'content-type': 'application/json' }, body: JSON.stringify({ email: 'w@example.com', client: 'web' }) })
  assert.equal(res.headers.get('access-control-allow-origin'), 'https://kunsu.app')
  const evil = await fetch(`${env.base}/auth/email/start`, { method: 'OPTIONS', headers: { origin: 'https://evil.example', 'access-control-request-method': 'POST' } })
  assert.equal(evil.headers.get('access-control-allow-origin'), null)
})

test('AC-AC-12 沒有寄信服務時要求登入連結回 503，不建立權杖', async () => {
  const env = await cloud({ noMailer: true })
  const r = await env.call('POST', '/auth/email/start', { email: 'w@example.com', client: 'web' })
  assert.equal(r.status, 503)
  assert.match(r.json.error, /寄不出去/)
  assert.equal(r.json.poll, undefined, '不給 poll')
  assert.equal(env.mail.length, 0)
})
