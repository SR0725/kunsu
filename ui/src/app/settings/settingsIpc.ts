// 主視窗設定與狀態的 Tauri invoke（CONTRACT §15）：命令名稱與 JSON 形狀嚴格照契約。後端另行實作；這裡只負責呼叫與
// 把 `settings_set` 的 {errors}（不論是回傳值還是被拒絕的錯誤）整理成逐欄錯誤。
import type { AccountInfo } from './account'
import type { AppStatus, MobileStatus, PairingQr, SettingKey, SettingsChanges, SettingsSetResult, SettingsState } from '../../types'
import { parseSettingsResult } from './schema'

export type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>

export interface SettingsIpc {
  /** 生效值、預設值、已改過的鍵、local 檔位置與錯誤。 */
  settingsGet(): Promise<SettingsState>
  /** 寫 `~/.kunsu/policy.local.toml`；不合法整筆拒絕、回逐欄錯誤。 */
  settingsSet(changes: SettingsChanges): Promise<SettingsSetResult>
  /** 從 local 檔移除這些鍵（回到預設）；`[]`＝全部回復預設（local 檔壞掉時清空整個檔）。被拒絕時丟出可讀訊息。 */
  settingsReset(keys: SettingKey[]): Promise<SettingsState>
  /** 版本、運作時間、服務位址、資料位置、帳本健康與整合狀態。 */
  appStatus(): Promise<AppStatus>
  /** 刪掉保存的瀏海位置，瀏海回到預設位置。 */
  edgeResetPosition(): Promise<void>
  /** 只打開「隱私權與安全性 › 完全取用磁碟」。 */
  openPrivacySettings(): Promise<void>
  /** 手機配對與中轉 AI（PRD §30.7、§30.8；只收主視窗）。 */
  mobileStatus(): Promise<MobileStatus>
  mobileStartPairing(): Promise<PairingQr>
  mobileDecidePairing(id: string, allow: boolean): Promise<MobileStatus>
  mobileRevoke(deviceId: string): Promise<MobileStatus>
  mobileSetRelayAgent(agent: 'claude' | 'codex'): Promise<MobileStatus>
  /** 帳號（PRD §31.2；BDD multi-user AC-MU-03）：寄登入連結、等使用者點、登出。只收主視窗。 */
  accountStatus(): Promise<AccountInfo | null>
  accountStartLogin(email: string): Promise<string>
  accountPoll(poll: string): Promise<boolean>
  accountLogout(): Promise<void>
  /** 只有 mock：模擬一支手機掃了 QR、送來配對請求（瀏覽器 QA 用）。 */
  mobileSimulatePairRequest?(deviceName: string): void
}

export function settingsTauriIpc(invoke: Invoke): SettingsIpc {
  return {
    settingsGet: () => invoke<SettingsState>('settings_get'),
    async settingsSet(changes) {
      let raw: unknown
      try {
        raw = await invoke<unknown>('settings_set', { changes })
      } catch (error) {
        const parsed = parseSettingsResult(error)
        if ('errors' in parsed) return { errors: parsed.errors }
        throw new Error('message' in parsed ? parsed.message : String(error))
      }
      const parsed = parseSettingsResult(raw)
      if ('errors' in parsed) return { errors: parsed.errors }
      if ('state' in parsed) return parsed.state
      throw new Error(parsed.message)
    },
    async settingsReset(keys) {
      try {
        return await invoke<SettingsState>('settings_reset', { keys })
      } catch (error) {
        const parsed = parseSettingsResult(error)
        if ('errors' in parsed) throw new Error(parsed.errors.map((e) => e.message).join('；'))
        throw new Error('message' in parsed ? parsed.message : String(error))
      }
    },
    appStatus: () => invoke<AppStatus>('app_status'),
    edgeResetPosition: () => invoke<void>('edge_reset_position'),
    openPrivacySettings: () => invoke<void>('open_privacy_settings'),
    mobileStatus: () => invoke<MobileStatus>('mobile_status'),
    mobileStartPairing: () => invoke<PairingQr>('mobile_start_pairing'),
    mobileDecidePairing: (id, allow) => invoke<MobileStatus>('mobile_decide_pairing', { id, allow }),
    mobileRevoke: (deviceId) => invoke<MobileStatus>('mobile_revoke', { device_id: deviceId }),
    mobileSetRelayAgent: (agent) => invoke<MobileStatus>('mobile_set_relay_agent', { agent }),
    accountStatus: () => invoke<AccountInfo | null>('account_status'),
    accountStartLogin: (email) => invoke<string>('account_start_login', { email }),
    accountPoll: (poll) => invoke<boolean>('account_poll', { poll }),
    accountLogout: () => invoke<void>('account_logout'),
  }
}
