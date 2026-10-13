// Kunsu 中轉站（PRD §30.8；docs/design/mobile-relay.md；INVARIANTS 30）。
// 只依 mac_id／device_id 轉送 JSON 封包，不解析內容、不持有金鑰、不寫硬碟；Mac 或手機離線時只在記憶體暫存。
import { createServer } from 'node:http'
import { timingSafeEqual } from 'node:crypto'
import { WebSocketServer } from 'ws'

const MAX_BYTES = 256 * 1024
const QUEUE_MAX = 50
const QUEUE_TTL_MS = 10 * 60 * 1000
const RATE_PER_MIN = 120
const ID = /^[a-z2-7]{16}$|^[a-z0-9-]{1,32}$/

function tokenOk(header, expected) {
  if (!expected || !header?.startsWith('Bearer ')) return false
  const a = Buffer.from(header.slice(7))
  const b = Buffer.from(expected)
  return a.length === b.length && timingSafeEqual(a, b)
}

/**
 * 把中轉站接到一個 HTTP 伺服器上。`macAuth(mac, authorizationHeader)` 決定 Mac 能不能上線
 * （帳號服務查每台 Mac 自己的憑證，PRD §31.2）；沒給就只認 macToken。
 */
export function attachRelay(http, { macToken, macAuth, clock = Date.now } = {}) {
  const macs = new Map() // mac_id → socket
  const devices = new Map() // `${mac}/${dev}` → Set<socket>
  const queues = new Map() // `to-mac:${mac}` 或 `to-dev:${mac}/${dev}` → [{at, text}]
  const rates = new Map() // ip → {minute, count}

  const enqueue = (key, text) => {
    const q = (queues.get(key) ?? []).filter((m) => clock() - m.at < QUEUE_TTL_MS)
    q.push({ at: clock(), text })
    while (q.length > QUEUE_MAX) q.shift()
    queues.set(key, q)
  }
  const drain = (key, send) => {
    const q = (queues.get(key) ?? []).filter((m) => clock() - m.at < QUEUE_TTL_MS)
    queues.delete(key)
    for (const m of q) send(m.text)
  }
  const limited = (ip) => {
    const minute = Math.floor(clock() / 60000)
    const r = rates.get(ip)
    if (!r || r.minute !== minute) {
      rates.set(ip, { minute, count: 1 })
      return false
    }
    r.count += 1
    return r.count > RATE_PER_MIN
  }

  const wss = new WebSocketServer({ noServer: true, maxPayload: MAX_BYTES })

  http.on('upgrade', (req, socket, head) => {
    const url = new URL(req.url, 'http://relay')
    const role = url.searchParams.get('role')
    const mac = url.searchParams.get('mac') ?? ''
    const dev = url.searchParams.get('dev') ?? ''
    const reject = (code) => {
      socket.write(`HTTP/1.1 ${code} ${code === 401 ? 'Unauthorized' : 'Bad Request'}\r\n\r\n`)
      socket.destroy()
    }
    if (url.pathname !== '/v1/connect' || !ID.test(mac)) return reject(400)
    if (role === 'device' && !ID.test(dev)) return reject(400)
    if (role !== 'mac' && role !== 'device') return reject(400)
    const authorized = role === 'device' ? Promise.resolve(true)
      : tokenOk(req.headers.authorization, macToken) ? Promise.resolve(true)
      : macAuth ? Promise.resolve(macAuth(mac, req.headers.authorization)).catch(() => false)
      : Promise.resolve(false)
    authorized.then((ok) => {
      if (!ok) return reject(401)
      upgrade()
    })
    const upgrade = () => wss.handleUpgrade(req, socket, head, (ws) => {
      ws.meta = { role, mac, dev, ip: req.headers['x-forwarded-for']?.split(',')[0]?.trim() ?? req.socket.remoteAddress }
      wss.emit('connection', ws)
    })
  })

  wss.on('connection', (ws) => {
    const { role, mac, dev, ip } = ws.meta
    ws.isAlive = true
    ws.on('pong', () => (ws.isAlive = true))
    // 封包太大等錯誤：ws 會以 1009 關閉連線，這裡只吞掉事件、不讓程序崩潰
    ws.on('error', () => {})
    if (role === 'mac') {
      macs.get(mac)?.close(4000, 'replaced')
      macs.set(mac, ws)
      drain(`to-mac:${mac}`, (t) => ws.send(t))
    } else {
      const key = `${mac}/${dev}`
      if (!devices.has(key)) devices.set(key, new Set())
      devices.get(key).add(ws)
      drain(`to-dev:${key}`, (t) => ws.send(t))
    }

    ws.on('message', (data, isBinary) => {
      if (isBinary || limited(ip)) return
      let msg
      try {
        msg = JSON.parse(data.toString())
      } catch {
        return
      }
      if (role === 'device') {
        // 手機 → Mac：包上來源裝置，內容原樣
        const text = JSON.stringify({ from: dev, body: msg.body })
        const target = macs.get(mac)
        if (target?.readyState === 1) target.send(text)
        else enqueue(`to-mac:${mac}`, text)
      } else {
        // Mac → 手機：只能送給綁在這台 Mac 的裝置
        if (typeof msg.to !== 'string' || !ID.test(msg.to)) return
        const key = `${mac}/${msg.to}`
        const text = JSON.stringify(msg.body)
        const targets = [...(devices.get(key) ?? [])].filter((s) => s.readyState === 1)
        if (targets.length) targets.forEach((s) => s.send(text))
        else enqueue(`to-dev:${key}`, text)
      }
    })

    ws.on('close', () => {
      if (role === 'mac' && macs.get(mac) === ws) macs.delete(mac)
      if (role === 'device') devices.get(`${mac}/${dev}`)?.delete(ws)
    })
  })

  const heartbeat = setInterval(() => {
    for (const ws of wss.clients) {
      if (!ws.isAlive) {
        ws.terminate()
        continue
      }
      ws.isAlive = false
      ws.ping()
    }
  }, 25000)

  return {
    macsOnline: () => macs.size,
    /** 撤銷憑證或刪除帳號後立刻踢下線。 */
    kickMac: (mac) => macs.get(mac)?.close(4001, 'revoked'),
    close: () => {
      clearInterval(heartbeat)
      for (const ws of wss.clients) ws.terminate()
      wss.close()
    },
  }
}

export function startRelay({ port = Number(process.env.PORT) || 8080, macToken = process.env.RELAY_MAC_TOKEN, clock = Date.now } = {}) {
  let relay
  const http = createServer((req, res) => {
    if (req.url === '/health') {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ ok: true, macs: relay.macsOnline() }))
      return
    }
    res.writeHead(404).end()
  })
  relay = attachRelay(http, { macToken, clock })
  return new Promise((resolve) => {
    http.listen(port, () => {
      resolve({
        port: http.address().port,
        close: () =>
          new Promise((res) => {
            relay.close()
            http.close(() => res())
          }),
      })
    })
  })
}

if (import.meta.url === `file://${process.argv[1]}`) {
  if (!process.env.RELAY_MAC_TOKEN) {
    console.error('RELAY_MAC_TOKEN 沒有設定：Mac 會無法上線')
  }
  startRelay().then(({ port }) => console.log(`kunsu-relay listening on ${port}`))
}
