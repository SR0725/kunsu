// CONTRACT §2 資料模型的 TS 鏡像（serde snake_case；型別以 crates/kunsu-core 為準）。
// 含 2026-07-31 P1 收貨後補欄位：irreversible / standing_policy_ref / tier_downgrade_reason。

export type Kind = 'choice' | 'text' | 'confirm' | 'kongko_review' | 'batch'
export type Tier = 't0' | 't1' | 't2' | 't3'
export type Gate = 'yellow' | 'red'
export type QState =
  | 'created'
  | 'pending'
  | 'warned'
  | 'forced'
  | 'snoozed'
  | 'answered'
  | 'expired'
  | 'cancelled'
  | 'superseded'
  | 'routed'
export type Channel = 'ui' | 'kongko' | 'discord' | 'cli' | 'auto_policy' | 'mobile'
export type OnExpire = 'noop_log' | 'escalate_discord' | 'pick_option'
export type WakeMode = 'none' | 'resume' | 'fresh' | 'continue'
export type AgentKind = 'claude' | 'codex' | 'other'

export interface EvidenceLine {
  text: string
  /** 身世標籤（來源＋期間），必填。 */
  source: string
}

export interface Opt {
  id: string
  label: string
  /** 後果預覽（hover / 選中時顯示）。 */
  detail?: string
  recommended?: boolean
}

/** 題組項目（PRD §25、CONTRACT §14）。content_hash 由 server 算。 */
export interface BatchItem {
  id: string
  /** 寄件人＋主旨、留言者、申請人（純文字）。 */
  title: string
  subtitle?: string
  /** 原文脈絡（來信全文、留言串、申請內容），Markdown。 */
  context_md: string
  /** AI 的草稿或建議（純文字）。 */
  proposal: string
  /** true＝可改草稿；false＝只核可（照建議或不處理）。 */
  editable: boolean
  default: 'approve' | 'skip'
  default_reason?: string
  content_hash?: string
}

export type ItemAction = 'approve' | 'skip' | 'revise'

/** 使用者推掉一題的時間與原因（原因選填）。 */
export interface Rejection {
  at: string
  reason?: string | null
}

/** 題組答案的一項：採用的可編輯項目帶最後文字；請 AI 改帶指示。 */
export interface ItemDecision {
  item_id: string
  action: ItemAction
  text?: string
  note?: string
  content_hash?: string
}

export interface Money {
  is_money_op: boolean
  /** 新台幣元（非 cents）。 */
  amount_twd?: number
  entity_id?: string
  executor_prepare_cmd?: string
}

export interface DefaultAction {
  on_expire: OnExpire
  option_id?: string
}

export interface Wake {
  mode: WakeMode
  session_id?: string
  playbook?: string
}

export type HostKind = 'claude_desktop' | 'claude_cli' | 'codex_app' | 'codex_cli' | 'other'

export interface Source {
  agent: AgentKind
  session_id?: string
  cwd: string
  routine_id?: string
  /** 回答延續原對話（PRD §20）：發問的主機與對話身分。 */
  host?: HostKind
  conversation_id?: string
  host_session_id?: string
  attended?: boolean
  cli_path?: string
  /** 排程督導（PRD §21）。 */
  trigger?: 'scheduled' | 'interactive'
  routine_name?: string
  routine_kind?: 'claude_scheduled_task' | 'codex_cron' | 'codex_heartbeat' | 'other'
  expected_every_hours?: number
}

export type DeliveryState =
  | 'pending'
  | 'held'
  | 'resuming'
  | 'live_waiter'
  | 'resumed'
  | 'picked_up'
  | 'fresh'
  | 'no_continuation'
  | 'dead'

/** 答案交付到哪裡（server 寫入）。 */
export interface Delivery {
  state: DeliveryState
  reason?: string
  conversation_link?: string
  updated_at: string
}

/** agent 以 ask_report 回報的處理結果與 AI 回饋。 */
export interface AgentReport {
  status: 'done' | 'failed' | 'blocked'
  summary_md: string
  reported_at: string
}

export interface Answer {
  answer_id?: string
  choice_id?: string
  free_text?: string
  edited_params?: string
  channel: Channel
  answered_at: string
  signature?: string
  acked_at?: string
  executed_at?: string
  execution_result?: string
  delivery?: Delivery
  report?: AgentReport
  /** 作答當下顯示的草稿版本與內容雜湊（PRD §24）。 */
  revision?: number
  content_hash?: string
  /** 題組逐項決定；題組的 free_text 是整體指示。 */
  items?: ItemDecision[]
}

/** 證據畫布種類（Claude Artifact 模式）。 */
export type CanvasKind = 'tsx' | 'jsx' | 'html'

/** 草稿註冊的版本參照（PRD §24）。 */
export interface DraftRef {
  revision: number
  content_hash: string
  canvas?: CanvasKind
  registered_at: string
}

/** 某題某版本的證據：畫布原始碼與資產（`./assets/x.png` → data URI）。 */
export interface EvidenceBundle {
  canvas?: { kind: CanvasKind; source: string } | null
  assets: Record<string, string>
  /** 看門狗停用這一版畫布的原因（畫布卡住主執行緒，AC-EC-09b）。 */
  canvas_disabled?: string
}

export interface Question {
  id: string
  title: string
  /** ≤24 字決策句（蓋板大字）。 */
  question_line: string
  body_md: string
  /** 為什麼問你（PRD §23）；舊題沒有。 */
  why_md?: string
  /** Agent 可選的左側粒子輪廓；server 限制最多四個 Unicode 字元。 */
  visual_symbol?: string
  kind: Kind
  tier: Tier
  requested_tier: Tier
  product?: string
  gate?: Gate
  evidence: EvidenceLine[]
  /** 動錢題必填：精確變更一行（所見即所簽）。 */
  params_line?: string
  options: Opt[]
  allow_free_text: boolean
  money?: Money
  decision_ref?: string
  prediction_ref?: string
  kongko_ref?: string
  deep_link?: string
  internal?: boolean
  default_action: DefaultAction
  deadline_at?: string
  wake_on_answer: Wake
  source: Source
  state: QState
  snooze_count: number
  forced_today_used: boolean
  /** YYYY-MM-DD（upsert 不動，積壓天數誠實原則）。 */
  first_proposed: string
  last_seen: string
  created_at: string
  /** 持久化牆鐘週期；internal 題沒有這些欄位。 */
  cycle_started_at?: string
  warn_at?: string
  force_at?: string
  cycle_number?: number
  next_transition_at: string
  answer?: Answer
  /** 以草稿註冊的題目：目前版本與畫布種類。 */
  draft?: DraftRef
  /** 題組項目（kind=batch）。 */
  items?: BatchItem[]
  /** 使用者按「這題不該問我」推掉這題（督導頁標「退回」）。 */
  rejected?: Rejection
  // ---- P1 收貨後補欄位 ----
  irreversible?: boolean
  standing_policy_ref?: string
  tier_downgrade_reason?: string
  // ---- UI 可見的執行阻斷標記（答後執行失敗回列表用；server 回寫） ----
  execution_blocked?: string
  /**
   * 已作答錢題的錢的狀態（shell 附上伺服器算的值，CONTRACT §7、§9）：回執一律依它給按鈕。沒有這欄位（舊資料、讀取失敗）才退回
   * 看執行紀錄開頭。
   */
  money_state?: MoneyState
}

/**
 * 錢的狀態（CONTRACT §7）：`running` 錢鏈列還沒跑完（含之後同一列的交付）；`unknown` 結果不明、可人工對帳；`reauthorizable` 可重新授權；
 * `settled` 已定案、沒有後續動作。
 */
export type MoneyState = 'running' | 'unknown' | 'reauthorizable' | 'settled'

export const TERMINAL_STATES: QState[] = [
  'answered',
  'expired',
  'cancelled',
  'superseded',
  'routed',
]

export function isTerminal(s: QState): boolean {
  return TERMINAL_STATES.includes(s)
}

// ---- 主視窗設定與狀態（CONTRACT §15；BDD settings） ----

/** 設定頁看得到的生效值（policy.toml 疊上 policy.local.toml）。 */
export interface SettingsView {
  force: { cycle_min: number; warn_before_min: number }
  windows: { quiet_enabled: boolean; quiet_start: string; quiet_end: string }
  expiry: { t1_default_after_h: number; t2_supersede_after_d: number }
  quota: { per_source_daily_create: number }
  fatigue: { skip_rate_window_d: number; skip_rate_downgrade: number }
}

/** 可從設定頁修改的鍵（CONTRACT §15 範圍表）。 */
export type SettingKey =
  | 'force.cycle_min'
  | 'force.warn_before_min'
  | 'windows.quiet_enabled'
  | 'windows.quiet_hours'
  | 'expiry.t1_default_after_h'
  | 'expiry.t2_supersede_after_d'
  | 'quota.per_source_daily_create'
  | 'fatigue.skip_rate_window_d'
  | 'fatigue.skip_rate_downgrade'

/** `settings_set { changes }`：`windows.quiet_hours` 是 `["HH:MM","HH:MM"]`，`windows.quiet_enabled` 是布林，其餘是數字。 */
export type SettingsChanges = Partial<Record<SettingKey, number | boolean | [string, string]>>

/** `settings_get`（以及 `settings_set`／`settings_reset` 成功時）的回應。 */
export interface SettingsState {
  effective: SettingsView
  defaults: SettingsView
  /** local 檔裡有的鍵（「已改過」）。 */
  overridden: string[]
  local_path: string
  /** local 檔解析失敗或超出範圍：生效值只用 base。 */
  local_error?: string | null
}

export interface SettingsFieldError {
  key: string
  message: string
}

/** `settings_set` 的回應：成功是新狀態；不合法整筆拒絕、回逐欄錯誤。 */
export type SettingsSetResult = SettingsState | { errors: SettingsFieldError[] }

export type IntegrationId = 'claude_mcp' | 'codex_mcp' | 'cli' | 'skill'

/** 與 `kunsu doctor` 同源的整合檢查；`fix` 是給使用者複製到終端機的指令（app 不執行）。 */
export interface IntegrationStatus {
  id: IntegrationId
  status: 'ok' | 'missing' | 'unknown'
  detail: string
  fix?: string | null
}

export interface LedgerStatus {
  /** 帳本寫不進去、排隊待補寫的行數。 */
  backlog: number
  open_json_dirty: boolean
  /** 寫入卡住多久（秒）；0＝沒卡住。 */
  stuck_s: number
}

/** `app_status`（CONTRACT §15）。 */
export interface AppStatus {
  version: string
  uptime_s: number
  server_addr: string
  vault_root: string
  db_path: string
  ledger: LedgerStatus
  integrations: IntegrationStatus[]
}

/** 手機配對與中轉 AI（PRD §30.7、§30.8；Rust kunsu_server::mobile::MobileStatus 的鏡像）。 */
export interface MobileDevice {
  id: string
  name: string
  paired_at: number
  last_seen: number | null
  revoked: boolean
}
export interface MobilePending {
  id: string
  device_name: string
  received_at: number
}
export interface MobileStatus {
  relay_url: string
  connected: boolean
  last_error: string | null
  mac_id: string
  relay_agent: 'claude' | 'codex'
  devices: MobileDevice[]
  pending: MobilePending[]
}
export interface PairingQr {
  payload: string
  svg: string
  expires_at: number
}

