// 正式啟動：帳號服務＋中轉站（Zeabur）。環境變數：
//   PORT、DATABASE_URL 或 POSTGRES_CONNECTION_STRING（Postgres；沒有就用記憶體，只適合開發）、RESEND_API_KEY、
//   GOOGLE_CLIENT_IDS（逗號分隔：網頁、iOS）、APPLE_CLIENT_IDS（逗號分隔：Services ID、App bundle id）、
//   RELAY_MAC_TOKEN（相容期，公開發表前移除）、PUBLIC_BASE_URL（預設 https://kunsu.app）
import { appendFileSync } from 'node:fs'
import { startCloud, resendMailer } from './cloud.js'
import { memoryStore } from './accounts/store.js'
import { pgStore } from './accounts/pg.js'

const list = (v) => (v ?? '').split(',').map((s) => s.trim()).filter(Boolean)
// Zeabur 的 PostgreSQL 會把連線字串分享給同專案的服務
const dbUrl = process.env.DATABASE_URL ?? process.env.POSTGRES_CONNECTION_STRING ?? process.env.POSTGRES_URI
const store = dbUrl ? await pgStore(dbUrl) : memoryStore()
if (!dbUrl) console.warn('沒有 DATABASE_URL：帳號只存在記憶體，重啟就沒了')
// 開發與測試：KUNSU_DEV_MAIL_LOG 把信寫進檔案（一行一封 JSON），不寄出
const mailer = process.env.RESEND_API_KEY
  ? resendMailer(process.env.RESEND_API_KEY)
  : process.env.KUNSU_DEV_MAIL_LOG
    ? { send: async (m) => appendFileSync(process.env.KUNSU_DEV_MAIL_LOG, JSON.stringify(m) + '\n') }
    : null
if (!mailer) console.warn('沒有 RESEND_API_KEY：登入連結一律回 503（AC-AC-12）')

const cloud = await startCloud({
  port: process.env.PORT === undefined ? 8080 : Number(process.env.PORT),
  store,
  mailer,
  baseUrl: process.env.PUBLIC_BASE_URL ?? 'https://kunsu.app',
  google: { clientIds: list(process.env.GOOGLE_CLIENT_IDS) },
  apple: { clientIds: list(process.env.APPLE_CLIENT_IDS) },
})
console.log(`kunsu-cloud listening on ${cloud.port}`)
setInterval(() => cloud.purge().catch((e) => console.error('清除刪除的帳號失敗', e)), 6 * 3600 * 1000)
