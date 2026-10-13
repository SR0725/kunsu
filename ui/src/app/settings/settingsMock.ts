// 設定與狀態的 mock（pnpm dev 免 Tauri）：範圍驗證與 server 同一套規則（schema.ts）、記住變更（localStorage 模擬
// policy.local.toml）、可模擬 local 檔壞掉、帳本卡住與未設定的整合。真相永遠在 kunsu-server；本檔只為 UI 開發。
//
// 瀏覽器 QA：網址參數 `?local=broken`、`?ledger=backlog|stuck`、`?integrations=missing`；或在 console 用
// `__kunsuMock.settings.scenario({ ledger: 'stuck' })` 即時切換。
import type { AppStatus, IntegrationStatus, SettingKey, SettingsChanges, SettingsState } from '../../types'
import type { MobileStatus } from '../../types'
import { POLICY_DEFAULTS, SETTING_KEYS, applyChanges, validateChanges } from './schema'
import type { SettingsIpc } from './settingsIpc'

export interface SettingsMockScenario {
  /** local 檔語法錯或超出範圍：生效值只用 base，設定頁頂端顯示錯誤。 */
  localError?: boolean
  ledger?: 'ok' | 'backlog' | 'stuck'
  integrations?: 'ok' | 'missing'
}

interface StorageLike {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
  removeItem?(key: string): void
}

const STORE_KEY = 'kunsu:mock:settings:v1'
const EDGE_KEY = 'kunsu:mock:edge'
const LOCAL_PATH = '/Users/you/.kunsu/policy.local.toml'

function readLocal(storage: StorageLike | undefined): SettingsChanges {
  try {
    const raw = storage?.getItem(STORE_KEY)
    const parsed = raw ? (JSON.parse(raw) as SettingsChanges) : {}
    return typeof parsed === 'object' && parsed !== null ? parsed : {}
  } catch {
    return {}
  }
}

function writeLocal(storage: StorageLike | undefined, local: SettingsChanges) {
  try {
    storage?.setItem(STORE_KEY, JSON.stringify(local))
  } catch {
    // 瀏覽器不給存就只留在記憶體（mock 用）
  }
}

function integrations(missing: boolean): IntegrationStatus[] {
  // 指令照 `kunsu doctor` 印給使用者的修法（只引用 token 檔路徑，不含 token 本身）。
  return [
    missing
      ? {
          id: 'claude_mcp',
          status: 'missing',
          detail: 'Claude Code 還沒註冊 kunsu MCP（~/.claude.json 沒有 mcpServers.kunsu）',
          fix: 'claude mcp add --scope user --transport http kunsu http://127.0.0.1:4777/mcp --header "Authorization: Bearer $(cat ~/.kunsu/token)"',
        }
      : { id: 'claude_mcp', status: 'ok', detail: 'Claude Code 已註冊 kunsu MCP（~/.claude.json）' },
    missing
      ? {
          id: 'codex_mcp',
          status: 'missing',
          detail: 'Codex 還沒接上 kunsu（~/.codex/config.toml 沒有啟用的 [mcp_servers.kunsu] 或 kunsu 外掛）',
          // 與 server 的 CODEX_FIX、kunsu doctor 同一行：安裝本機外掛（CONTRACT §15、§16）。
          fix: 'python3 ~/.kunsu/plugins/kunsu/scripts/manage.py install',
        }
      : { id: 'codex_mcp', status: 'ok', detail: 'Codex 已接上 kunsu（~/.codex/config.toml）' },
    { id: 'cli', status: 'ok', detail: '~/.kunsu/bin/kunsu 可以執行' },
    missing
      ? { id: 'skill', status: 'unknown', detail: '讀不到 ~/.claude/skills（權限不足），無法確認 kunsu skill' }
      : { id: 'skill', status: 'ok', detail: '~/.claude/skills/kunsu/SKILL.md 存在' },
  ]
}

export function makeSettingsMock(options: { storage?: StorageLike; scenario?: SettingsMockScenario } = {}): SettingsIpc & {
  scenario(next: SettingsMockScenario): void
} {
  const storage = options.storage
  let scenario: SettingsMockScenario = { ...options.scenario }
  let local = readLocal(storage)
  const startedAt = Date.now() - (3 * 3600 + 12 * 60) * 1000
  let stuckSince = Date.now() - 150_000

  const state = (): SettingsState => {
    if (scenario.localError) {
      return {
        effective: structuredClone(POLICY_DEFAULTS),
        defaults: structuredClone(POLICY_DEFAULTS),
        overridden: [],
        local_path: LOCAL_PATH,
        local_error: '第 3 行：force.cycle_min = 2 超出範圍（5–120 分鐘）；這個檔案的值都還沒套用',
      }
    }
    return {
      effective: applyChanges(POLICY_DEFAULTS, local),
      defaults: structuredClone(POLICY_DEFAULTS),
      overridden: SETTING_KEYS.filter((key) => key in local),
      local_path: LOCAL_PATH,
      local_error: null,
    }
  }

  // 手機配對（mock）：配對要按允許才進裝置清單（INV-29 的 UI 這一半）。
  let mobile: MobileStatus = {
    relay_url: 'wss://api.kunsu.app/v1',
    connected: true,
    last_error: null,
    mac_id: 'mockmac000000000',
    relay_agent: 'claude',
    devices: [],
    pending: [],
  }
  const snapshotMobile = () => structuredClone(mobile)

  let account: { email: string } | null = null
  let pendingLogin: string | null = null

  return {
    async accountStatus() {
      return account ? { ...account } : null
    },
    async accountStartLogin(email: string) {
      pendingLogin = email.trim().toLowerCase()
      return 'mock-poll'
    },
    async accountPoll(poll: string) {
      if (poll !== 'mock-poll' || !pendingLogin) throw new Error('登入連結已失效，請重新寄一次')
      account = { email: pendingLogin }
      pendingLogin = null
      return true
    },
    async accountLogout() {
      account = null
    },
    async mobileStatus() {
      return snapshotMobile()
    },
    async mobileStartPairing() {
      const payload = 'kunsu-pair:v1?relay=wss%3A%2F%2Fkunsu-relay.zeabur.app%2Fv1&mac=mockmac000000000&code=MOCK2345'
      const cells = Array.from({ length: 21 * 21 }, (_, n) => ((n * 7919) % 11 < 5 ? 1 : 0))
      const rects = cells.map((on, n) => (on ? `<rect x="${n % 21}" y="${Math.floor(n / 21)}" width="1" height="1"/>` : '')).join('')
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 25 25" width="240" height="240"><rect x="-2" y="-2" width="25" height="25" fill="#fff"/><g fill="#000">${rects}</g></svg>`
      return { payload, svg, expires_at: Math.floor(Date.now() / 1000) + 300 }
    },
    async mobileDecidePairing(id, allow) {
      const p = mobile.pending.find((x) => x.id === id)
      if (!p) throw new Error('找不到這個配對請求（可能已過期）')
      mobile.pending = mobile.pending.filter((x) => x.id !== id)
      if (allow) mobile.devices.push({ id: `dev-${id}`, name: p.device_name, paired_at: Math.floor(Date.now() / 1000), last_seen: null, revoked: false })
      return snapshotMobile()
    },
    async mobileRevoke(deviceId) {
      const d = mobile.devices.find((x) => x.id === deviceId)
      if (!d) throw new Error('找不到這台裝置')
      d.revoked = true
      return snapshotMobile()
    },
    async mobileSetRelayAgent(agent) {
      mobile.relay_agent = agent
      return snapshotMobile()
    },
    mobileSimulatePairRequest(deviceName: string) {
      mobile = { ...mobile, pending: [...mobile.pending, { id: `p${mobile.pending.length + 1}`, device_name: deviceName, received_at: Math.floor(Date.now() / 1000) }] }
    },
    scenario(next) {
      scenario = { ...scenario, ...next }
      if (next.ledger === 'stuck') stuckSince = Date.now() - 150_000
    },
    async settingsGet() {
      return state()
    },
    async settingsSet(changes) {
      const current = state()
      const errors = validateChanges(changes, current.effective)
      if (errors.length > 0) return { errors }
      local = { ...local, ...changes }
      writeLocal(storage, local)
      return state()
    },
    async settingsReset(keys: SettingKey[]) {
      if (scenario.localError || keys.length === 0) {
        // keys: [] 是全部回復預設；local 檔壞掉時任何回復都清空整個檔（CONTRACT §15）。
        scenario = { ...scenario, localError: false }
        local = {}
      } else {
        const next = Object.fromEntries(Object.entries(local).filter(([key]) => !keys.includes(key as SettingKey))) as SettingsChanges
        // 移除後剩下的組合不合法（例如留下的提前警告 ≥ 預設週期）整筆拒絕，與 server 相同。
        const errors = validateChanges(next, POLICY_DEFAULTS)
        if (errors.length > 0) throw new Error(errors.map((e) => e.message).join('；'))
        local = next
      }
      writeLocal(storage, local)
      return state()
    },
    async appStatus(): Promise<AppStatus> {
      const ledger =
        scenario.ledger === 'stuck'
          ? { backlog: 4, open_json_dirty: true, stuck_s: Math.round((Date.now() - stuckSince) / 1000) }
          : scenario.ledger === 'backlog'
            ? { backlog: 3, open_json_dirty: true, stuck_s: 0 }
            : { backlog: 0, open_json_dirty: false, stuck_s: 0 }
      return {
        version: '0.1.0',
        uptime_s: Math.round((Date.now() - startedAt) / 1000),
        server_addr: '127.0.0.1:4777',
        vault_root: '/Users/you/Kunsu',
        db_path: '/Users/you/.kunsu/kunsu.db',
        ledger,
        integrations: integrations(scenario.integrations === 'missing'),
      }
    },
    async edgeResetPosition() {
      try {
        storage?.removeItem?.(EDGE_KEY)
      } catch {
        // mock 用
      }
    },
    async openPrivacySettings() {
      console.info('[mock] open_privacy_settings → x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles')
    },
  }
}

/** 網址參數 → 情境（只在 mock）。 */
export function scenarioFromSearch(search: string): SettingsMockScenario {
  const params = new URLSearchParams(search)
  const ledger = params.get('ledger')
  return {
    localError: params.get('local') === 'broken',
    ledger: ledger === 'stuck' || ledger === 'backlog' ? ledger : 'ok',
    integrations: params.get('integrations') === 'missing' ? 'missing' : 'ok',
  }
}
