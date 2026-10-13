// 使用者之間完全隔離（BDD multi-user AC-MU-05；INV-30）。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import WebSocket from 'ws'
import { startCloud } from '../cloud.js'
import { memoryStore } from '../accounts/store.js'

async function setup() {
  const mail = []
  const c = await startCloud({ port: 0, store: memoryStore(), mailer: { send: async (m) => mail.push(m) }, baseUrl: 'https://kunsu.app', macToken: undefined })
  const base = `http://127.0.0.1:${c.port}`
  const call = async (method, path, body, token) => {
    const res = await fetch(base + path, { method, headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: body ? JSON.stringify(body) : undefined })
    const t = await res.text()
    return { status: res.status, json: t ? JSON.parse(t) : null }
  }
  const login = async (email) => {
    const { json } = await call('POST', '/auth/email/start', { email, client: 'mac' })
    const token = new URL(mail.at(-1).text.match(/https:\/\/kunsu\.app\/auth\/verify\?token=\S+/)[0]).searchParams.get('token')
    await call('POST', '/auth/email/verify', { token })
    return (await call('POST', '/auth/poll', { poll: json.poll })).json.session
  }
  const ws = (q, token) => new Promise((resolve, reject) => {
    const s = new WebSocket(`${base.replace('http', 'ws')}/v1/connect?${q}`, { headers: token ? { authorization: `Bearer ${token}` } : {} })
    const inbox = []
    s.on('message', (d) => inbox.push(JSON.parse(d.toString())))
    s.inbox = inbox
    s.once('open', () => resolve(s))
    s.once('unexpected-response', (_q, r) => resolve(r.statusCode))
    s.once('error', reject)
  })
  return { c, call, login, ws }
}

const MAC_A = 'aaaaaaaaaaaaaaaa'
const MAC_B = 'bbbbbbbbbbbbbbbb'

test('AC-MU-05 兩個使用者完全隔離', async () => {
  const e = await setup()
  const a = await e.login('a@example.com')
  const b = await e.login('b@example.com')
  const regA = await e.call('POST', '/account/macs', { mac_id: MAC_A, name: 'A 的 Mac', kx_pub: 'k', sig_pub: 's' }, a)
  const regB = await e.call('POST', '/account/macs', { mac_id: MAC_B, name: 'B 的 Mac', kx_pub: 'k', sig_pub: 's' }, b)

  // 憑證只對自己的 Mac 有效
  assert.equal(await e.ws(`role=mac&mac=${MAC_B}`, regA.json.relay_token), 401)
  assert.equal(await e.ws(`role=mac&mac=${MAC_A}`, regB.json.relay_token), 401)
  assert.equal(await e.ws(`role=mac&mac=${MAC_A}`), 401, '沒有共用 token 時不帶憑證上不了線')

  // 帳號只看得到自己的裝置，也動不了別人的
  assert.deepEqual((await e.call('GET', '/account', null, a)).json.macs.map((m) => m.id), [MAC_A])
  assert.deepEqual((await e.call('GET', '/account', null, b)).json.macs.map((m) => m.id), [MAC_B])
  assert.equal((await e.call('DELETE', `/account/macs/${MAC_B}`, null, a)).status, 404)
  assert.equal((await e.call('POST', '/account/macs', { mac_id: MAC_B, name: '搶', kx_pub: 'k', sig_pub: 's' }, a)).status, 409, '不能把別人的 Mac 綁到自己帳號')

  // 中轉：B 的 Mac 送給 B 的手機，A 的手機收不到
  const macB = await e.ws(`role=mac&mac=${MAC_B}`, regB.json.relay_token)
  const phoneB = await e.ws(`role=device&mac=${MAC_B}&dev=phoneb`)
  const phoneA = await e.ws(`role=device&mac=${MAC_A}&dev=phonea`)
  macB.send(JSON.stringify({ to: 'phoneb', body: { ct: 'ciphertext-for-b' } }))
  macB.send(JSON.stringify({ to: 'phonea', body: { ct: 'should-not-reach-a' } }))
  await new Promise((r) => setTimeout(r, 300))
  assert.deepEqual(phoneB.inbox, [{ ct: 'ciphertext-for-b' }])
  assert.deepEqual(phoneA.inbox, [], 'A 的手機綁的是 A 的 Mac，收不到 B 的 Mac 送的東西')
  for (const s of [macB, phoneB, phoneA]) s.close()
  await e.c.close()
})
