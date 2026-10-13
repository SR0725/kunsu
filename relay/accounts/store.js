// 帳號資料（docs/design/accounts.md；INV-33 inv_cloud_stores_no_content）。
// 只准存這些欄位：寫入時多出來的欄位一律丟掉，不論呼叫端傳了什麼。

export const ALLOWED_COLUMNS = {
  users: ['id', 'email', 'display_name', 'created_at', 'deleted_at'],
  identities: ['user_id', 'provider', 'subject', 'created_at'],
  login_tokens: ['token_hash', 'kind', 'email', 'user_id', 'client', 'pair_hash', 'expires_at', 'used_at'],
  sessions: ['token_hash', 'user_id', 'client', 'created_at', 'last_used_at', 'revoked_at'],
  macs: ['id', 'user_id', 'name', 'kx_pub', 'sig_pub', 'relay_token_hash', 'created_at', 'last_seen_at', 'revoked_at'],
  devices: ['id', 'user_id', 'name', 'created_at', 'last_seen_at', 'revoked_at'],
}

const pick = (table, row) => {
  const cols = ALLOWED_COLUMNS[table]
  if (!cols) throw new Error(`沒有這張表：${table}`)
  return Object.fromEntries(cols.filter((c) => row[c] !== undefined).map((c) => [c, row[c]]))
}

/** 記憶體實作（測試與本機開發）。正式環境用同介面的 Postgres 實作（accounts/pg.js）。 */
export function memoryStore() {
  const t = Object.fromEntries(Object.keys(ALLOWED_COLUMNS).map((k) => [k, []]))
  return {
    async insert(table, row) {
      const r = pick(table, row)
      t[table].push(r)
      return { ...r }
    },
    async find(table, where) {
      const r = t[table].find((x) => Object.entries(where).every(([k, v]) => x[k] === v))
      return r ? { ...r } : null
    },
    async all(table, where) {
      return t[table].filter((x) => Object.entries(where).every(([k, v]) => x[k] === v)).map((x) => ({ ...x }))
    },
    async update(table, where, patch) {
      const p = pick(table, patch)
      let n = 0
      for (const x of t[table]) {
        if (Object.entries(where).every(([k, v]) => x[k] === v)) { Object.assign(x, p); n++ }
      }
      return n
    },
    async remove(table, where) {
      const before = t[table].length
      t[table] = t[table].filter((x) => !Object.entries(where).every(([k, v]) => x[k] === v))
      return before - t[table].length
    },
    dump: async () => structuredClone(t),
  }
}
