// 中轉站驗收（BDD mobile-pairing AC-MP-06；INVARIANTS 30 inv_relay_sees_only_ciphertext）。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import WebSocket from 'ws'
import { startRelay } from '../server.js'

const TOKEN = 'test-token'

async function relay(opts = {}) {
  const r = await startRelay({ port: 0, macToken: TOKEN, ...opts })
  return { ...r, url: `ws://127.0.0.1:${r.port}/v1/connect` }
}

function open(url, headers = {}) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url, { headers })
    const inbox = []
    const waiters = []
    ws.on('message', (data) => {
      const msg = JSON.parse(data.toString())
      const w = waiters.shift()
      if (w) w(msg)
      else inbox.push(msg)
    })
    ws.next = () => new Promise((res) => (inbox.length ? res(inbox.shift()) : waiters.push(res)))
    ws.once('open', () => resolve(ws))
    ws.once('unexpected-response', (_req, res) => reject(new Error(`HTTP ${res.statusCode}`)))
    ws.once('error', reject)
  })
}

const mac = (r, token = TOKEN) => open(`${r.url}?role=mac&mac=m1`, { authorization: `Bearer ${token}` })
const device = (r, dev = 'd1') => open(`${r.url}?role=device&mac=m1&dev=${dev}`)

test('Mac 要 token 才能上線', async () => {
  const r = await relay()
  await assert.rejects(mac(r, 'wrong'), /401/)
  await assert.rejects(open(`${r.url}?role=mac&mac=m1`), /401/)
  await r.close()
})

test('inv_relay_sees_only_ciphertext：封包原樣轉送，中轉站不解析內容', async () => {
  const r = await relay()
  const m = await mac(r)
  const d = await device(r)
  const envelope = { v: 1, dev: 'd1', seq: 1, ts: 1, nonce: 'bm9uY2U', ct: 'Y2lwaGVydGV4dA', sig: 'c2ln' }
  d.send(JSON.stringify({ body: envelope }))
  assert.deepEqual(await m.next(), { from: 'd1', body: envelope })
  m.send(JSON.stringify({ to: 'd1', body: { v: 1, dev: 'd1', ct: 'eA' } }))
  assert.deepEqual(await d.next(), { v: 1, dev: 'd1', ct: 'eA' })
  // 原始碼不碰檔案系統：不落地
  const src = readFileSync(new URL('../server.js', import.meta.url), 'utf8')
  assert.doesNotMatch(src, /from 'node:fs'|require\('fs'\)|from 'fs'/)
  m.close(); d.close(); await r.close()
})

test('裝置只能找指定的 Mac；Mac 只能送給自己的裝置', async () => {
  const r = await relay()
  const m1 = await mac(r)
  const other = await open(`${r.url}?role=mac&mac=m2`, { authorization: `Bearer ${TOKEN}` })
  const d = await device(r)
  d.send(JSON.stringify({ body: { x: 1 } }))
  assert.deepEqual(await m1.next(), { from: 'd1', body: { x: 1 } })
  // m2 送給 d1（d1 綁 m1）→ 不轉送
  other.send(JSON.stringify({ to: 'd1', body: { evil: true } }))
  m1.send(JSON.stringify({ to: 'd1', body: { ok: true } }))
  assert.deepEqual(await d.next(), { ok: true })
  m1.close(); other.close(); d.close(); await r.close()
})

test('Mac 離線時，裝置封包在記憶體暫存，上線後送達', async () => {
  const r = await relay()
  const d = await device(r)
  d.send(JSON.stringify({ body: { n: 1 } }))
  d.send(JSON.stringify({ body: { n: 2 } }))
  await new Promise((res) => setTimeout(res, 50))
  const m = await mac(r)
  assert.deepEqual(await m.next(), { from: 'd1', body: { n: 1 } })
  assert.deepEqual(await m.next(), { from: 'd1', body: { n: 2 } })
  m.close(); d.close(); await r.close()
})

test('暫存最多 50 個，過期的丟掉', async () => {
  let now = 1_000_000
  const r = await relay({ clock: () => now })
  const d = await device(r)
  for (let i = 0; i < 60; i++) d.send(JSON.stringify({ body: { i } }))
  await new Promise((res) => setTimeout(res, 80))
  now += 11 * 60 * 1000
  d.send(JSON.stringify({ body: { fresh: true } }))
  await new Promise((res) => setTimeout(res, 30))
  const m = await mac(r)
  assert.deepEqual(await m.next(), { from: 'd1', body: { fresh: true } })
  m.close(); d.close(); await r.close()
})

test('超過 256 KB 的封包被拒', async () => {
  const r = await relay()
  const d = await device(r)
  d.on('error', () => {})
  const closed = new Promise((res) => d.once('close', (code) => res(code)))
  d.send(JSON.stringify({ body: { big: 'x'.repeat(300 * 1024) } }))
  assert.equal(await closed, 1009)
  await r.close()
})

test('GET /health', async () => {
  const r = await relay()
  const res = await fetch(`http://127.0.0.1:${r.port}/health`)
  assert.equal(res.status, 200)
  assert.equal((await res.json()).ok, true)
  await r.close()
})
