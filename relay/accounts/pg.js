// 帳號資料的 Postgres 實作（正式環境，Zeabur Postgres）。介面與 memoryStore 相同。
// 欄位名稱只來自 ALLOWED_COLUMNS（INV-33），值一律參數化。
import pg from 'pg'
import { ALLOWED_COLUMNS } from './store.js'

const TYPES = {
  id: 'text', email: 'text', display_name: 'text', user_id: 'text', provider: 'text', subject: 'text',
  token_hash: 'text', pair_hash: 'text', kind: 'text', client: 'text', name: 'text', kx_pub: 'text', sig_pub: 'text', relay_token_hash: 'text',
}
const KEYS = {
  users: 'PRIMARY KEY (id)', identities: 'PRIMARY KEY (provider, subject)', login_tokens: 'PRIMARY KEY (token_hash)',
  sessions: 'PRIMARY KEY (token_hash)', macs: 'PRIMARY KEY (id)', devices: 'PRIMARY KEY (id)',
}
const type = (col) => TYPES[col] ?? 'bigint' // 時間一律毫秒

const pick = (table, row) => {
  const cols = ALLOWED_COLUMNS[table]
  if (!cols) throw new Error(`沒有這張表：${table}`)
  return cols.filter((c) => row[c] !== undefined)
}

const toRow = (table, r) => {
  const out = {}
  for (const c of ALLOWED_COLUMNS[table]) {
    if (r[c] === null || r[c] === undefined) continue
    out[c] = type(c) === 'bigint' ? Number(r[c]) : r[c]
  }
  return out
}

export async function pgStore(connectionString) {
  const pool = new pg.Pool({ connectionString, max: 5 })
  for (const [table, cols] of Object.entries(ALLOWED_COLUMNS)) {
    await pool.query(`CREATE TABLE IF NOT EXISTS ${table} (${cols.map((c) => `${c} ${type(c)}`).join(', ')}, ${KEYS[table]})`)
  }
  // 新增欄位的遷移：舊表補上允許清單裡的新欄位
  for (const [table, cols] of Object.entries(ALLOWED_COLUMNS)) {
    for (const c of cols) await pool.query(`ALTER TABLE ${table} ADD COLUMN IF NOT EXISTS ${c} ${type(c)}`)
  }
  await pool.query('CREATE UNIQUE INDEX IF NOT EXISTS users_email ON users (email) WHERE deleted_at IS NULL')

  const where = (table, w, start = 1) => {
    const cols = pick(table, w)
    return {
      sql: cols.length ? ` WHERE ${cols.map((c, i) => `${c} = $${start + i}`).join(' AND ')}` : '',
      values: cols.map((c) => w[c]),
    }
  }

  return {
    async insert(table, row) {
      const cols = pick(table, row)
      await pool.query(`INSERT INTO ${table} (${cols.join(', ')}) VALUES (${cols.map((_, i) => `$${i + 1}`).join(', ')})`, cols.map((c) => row[c]))
      return Object.fromEntries(cols.map((c) => [c, row[c]]))
    },
    async find(table, w) {
      const q = where(table, w)
      const { rows } = await pool.query(`SELECT * FROM ${table}${q.sql} LIMIT 1`, q.values)
      return rows[0] ? toRow(table, rows[0]) : null
    },
    async all(table, w) {
      const q = where(table, w)
      const { rows } = await pool.query(`SELECT * FROM ${table}${q.sql}`, q.values)
      return rows.map((r) => toRow(table, r))
    },
    async update(table, w, patch) {
      const set = pick(table, patch)
      if (!set.length) return 0
      const q = where(table, w, set.length + 1)
      const res = await pool.query(`UPDATE ${table} SET ${set.map((c, i) => `${c} = $${i + 1}`).join(', ')}${q.sql}`, [...set.map((c) => patch[c]), ...q.values])
      return res.rowCount
    },
    async remove(table, w) {
      const q = where(table, w)
      const res = await pool.query(`DELETE FROM ${table}${q.sql}`, q.values)
      return res.rowCount
    },
    async dump() {
      const out = {}
      for (const table of Object.keys(ALLOWED_COLUMNS)) out[table] = (await pool.query(`SELECT * FROM ${table}`)).rows.map((r) => toRow(table, r))
      return out
    },
    async reset() {
      for (const table of Object.keys(ALLOWED_COLUMNS)) await pool.query(`TRUNCATE ${table}`)
    },
    async columns() {
      const { rows } = await pool.query("SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'public'")
      const out = {}
      for (const r of rows) (out[r.table_name] ??= []).push(r.column_name)
      return out
    },
    close: () => pool.end(),
  }
}
