// 設定欄位規則（CONTRACT §15 範圍表；BDD settings AC-ST-02／04／06；INV-26）。純函數：前端先擋，mock 用同一套規則模擬
// server（真正的關卡在 kunsu-core `Policy::layered`）。錢的護欄、紅燈規則、deferred_retry_s、t1_batch、webhook 不在這裡。
import type { SettingKey, SettingsChanges, SettingsFieldError, SettingsState, SettingsView } from '../../types'

export const SETTING_KEYS: readonly SettingKey[] = [
  'force.cycle_min',
  'force.warn_before_min',
  'windows.quiet_enabled',
  'windows.quiet_hours',
  'expiry.t1_default_after_h',
  'expiry.t2_supersede_after_d',
  'quota.per_source_daily_create',
  'fatigue.skip_rate_window_d',
  'fatigue.skip_rate_downgrade',
]

/** repo `config/policy.toml` 的預設值（CONTRACT §10）。 */
export const POLICY_DEFAULTS: SettingsView = {
  force: { cycle_min: 15, warn_before_min: 3 },
  windows: { quiet_enabled: true, quiet_start: '23:00', quiet_end: '09:00' },
  expiry: { t1_default_after_h: 48, t2_supersede_after_d: 14 },
  quota: { per_source_daily_create: 12 },
  fatigue: { skip_rate_window_d: 7, skip_rate_downgrade: 0.5 },
}

interface IntRange {
  label: string
  min: number
  max: number
  unit: string
  /** 範圍後面補一句（例如策略題最長只等 14 天）。 */
  note?: string
}

/** 整數欄位的範圍（含端點）。提前警告的上限跟著週期走，另外處理。 */
export const INT_RANGES: Partial<Record<SettingKey, IntRange>> = {
  'force.cycle_min': { label: '週期長度', min: 5, max: 120, unit: '分鐘' },
  'expiry.t1_default_after_h': { label: '一般題到期時間', min: 1, max: 336, unit: '小時', note: '最多 14 天' },
  // 所有題目的壽命上限（INV-14）：策略題到期由新題取代、其他題到期結案（2026-10-09 取代「策略題取代天數」）。
  'expiry.t2_supersede_after_d': { label: '題目最長等待天數', min: 1, max: 14, unit: '天', note: '所有題目，最多 14 天' },
  'quota.per_source_daily_create': { label: '每日建題上限', min: 1, max: 100, unit: '題' },
  'fatigue.skip_rate_window_d': { label: '觀察天數', min: 1, max: 30, unit: '天' },
}

export const SKIP_RATE = { min: 0.1, max: 1 } as const

/** 設定頁上每個鍵的名稱（「區段 › 欄位」）：個人設定檔的錯誤用這個指出哪一項，不露檔案裡的鍵名（AC-ST-13）。 */
export const SETTING_NAMES: Record<SettingKey, string> = {
  'force.cycle_min': '強制詢問 › 週期長度',
  'force.warn_before_min': '強制詢問 › 提前警告',
  'windows.quiet_enabled': '睡眠時間 › 啟用睡眠時間',
  'windows.quiet_hours': '睡眠時間 › 開始與結束時間',
  'expiry.t1_default_after_h': '到期與配額 › 一般題到期時間',
  'expiry.t2_supersede_after_d': '到期與配額 › 題目最長等待天數',
  'quota.per_source_daily_create': '到期與配額 › 每個來源每天',
  'fatigue.skip_rate_window_d': '到期與配額 › 觀察天數',
  'fatigue.skip_rate_downgrade': '到期與配額 › 略過率超過',
}

/** server 的錯誤字串（local_error、settings_set 的訊息）裡的鍵名換成設定頁上的名稱；原文另外留給除錯。 */
export function humanizeSettingsError(text: string): string {
  let out = text
  // 長的先換，避免一個鍵是另一個鍵的前綴時換到一半。
  for (const key of [...SETTING_KEYS].sort((a, b) => b.length - a.length)) out = out.split(key).join(SETTING_NAMES[key])
  return out
}

export function rangeText(key: SettingKey): string {
  const r = INT_RANGES[key]
  return r ? `${r.min}–${r.max} ${r.unit}` : ''
}

export function warnRange(cycleMin: number): string {
  return `1–${Math.max(1, cycleMin - 1)} 分鐘`
}

export const warnMessage = (cycleMin: number) => `提前警告要小於週期長度（${warnRange(cycleMin)}）`

function intMessage(key: SettingKey): string {
  const r = INT_RANGES[key]!
  return `${r.label}要在 ${r.min}–${r.max} ${r.unit}之間${r.note ? `（${r.note}）` : ''}`
}

const HM = /^([01]\d|2[0-3]):([0-5]\d)$/

export function isHm(value: unknown): value is string {
  return typeof value === 'string' && HM.test(value)
}

/** 使用者打的時間 → `HH:MM`（接受 `9:05`、`0930`）；看不懂回 null。 */
export function normalizeHm(raw: string): string | null {
  const text = raw.trim()
  let m = text.match(/^(\d{1,2}):(\d{2})$/)
  if (!m) m = text.match(/^(\d{1,2})(\d{2})$/)
  if (!m) return null
  const h = Number(m[1])
  const min = Number(m[2])
  if (h > 23 || min > 59) return null
  return `${String(h).padStart(2, '0')}:${String(min).padStart(2, '0')}`
}

const minutesOf = (hm: string) => Number(hm.slice(0, 2)) * 60 + Number(hm.slice(3, 5))

export function crossesMidnight(start: string, end: string): boolean {
  return minutesOf(end) <= minutesOf(start)
}

export const HM_MESSAGE = '時間要是 24 小時制 HH:MM，例如 23:00'
export const SAME_MESSAGE = '開始與結束不能相同'

/** 睡眠時間卡片的說明（AC-ST-04）：跨午夜寫「隔天」。 */
export function sleepSentence(input: { enabled: boolean; start: string; end: string }): string {
  if (!input.enabled) return '睡眠時間已關閉：睡眠時間不再擋強制詢問，任何時間到期的題都會照常跳出來。'
  const next = crossesMidnight(input.start, input.end) ? '隔天 ' : ' '
  return `每天 ${input.start} 到${next}${input.end} 不會強制詢問，也不會跳出提醒；題目照常保存，醒來後從原進度繼續`
}

/** 依 CONTRACT §15 驗證一筆變更（整筆一起看：提前警告用同一筆變更裡的新週期）。 */
export function validateChanges(changes: SettingsChanges, effective: SettingsView): SettingsFieldError[] {
  const errors: SettingsFieldError[] = []
  const entries = Object.entries(changes) as [string, unknown][]
  for (const [key, value] of entries) {
    if (!(SETTING_KEYS as readonly string[]).includes(key)) {
      errors.push({ key, message: '這個設定不能從設定頁修改' })
      continue
    }
    const k = key as SettingKey
    if (INT_RANGES[k]) {
      const r = INT_RANGES[k]!
      if (typeof value !== 'number' || !Number.isInteger(value) || value < r.min || value > r.max) errors.push({ key, message: intMessage(k) })
    } else if (k === 'fatigue.skip_rate_downgrade') {
      if (typeof value !== 'number' || !Number.isFinite(value) || value < SKIP_RATE.min - 1e-9 || value > SKIP_RATE.max + 1e-9) {
        errors.push({ key, message: '略過率門檻要在 10%–100% 之間' })
      }
    } else if (k === 'windows.quiet_enabled') {
      if (typeof value !== 'boolean') errors.push({ key, message: '睡眠時間開關只能是開或關' })
    } else if (k === 'windows.quiet_hours') {
      if (!Array.isArray(value) || value.length !== 2 || !isHm(value[0]) || !isHm(value[1])) errors.push({ key, message: HM_MESSAGE })
      else if (value[0] === value[1]) errors.push({ key, message: SAME_MESSAGE })
    }
  }
  // 提前警告：1 到（新的或既有的）週期減 1。只改週期也要檢查既有的提前警告。
  const cycleRaw = changes['force.cycle_min']
  const cycle = typeof cycleRaw === 'number' ? cycleRaw : effective.force.cycle_min
  const cycleOk = !errors.some((e) => e.key === 'force.cycle_min')
  if ('force.warn_before_min' in changes || 'force.cycle_min' in changes) {
    const warnRaw = changes['force.warn_before_min']
    const warn = 'force.warn_before_min' in changes ? warnRaw : effective.force.warn_before_min
    const ok = typeof warn === 'number' && Number.isInteger(warn) && warn >= 1 && warn <= cycle - 1
    if (!ok && cycleOk) errors.push({ key: 'force.warn_before_min', message: warnMessage(cycle) })
  }
  return errors
}

/** 把一筆（已驗證的）變更套到生效值上。 */
export function applyChanges(view: SettingsView, changes: SettingsChanges): SettingsView {
  const next: SettingsView = structuredClone(view)
  for (const [key, value] of Object.entries(changes)) {
    switch (key as SettingKey) {
      case 'force.cycle_min':
        next.force.cycle_min = value as number
        break
      case 'force.warn_before_min':
        next.force.warn_before_min = value as number
        break
      case 'windows.quiet_enabled':
        next.windows.quiet_enabled = value as boolean
        break
      case 'windows.quiet_hours':
        next.windows.quiet_start = (value as [string, string])[0]
        next.windows.quiet_end = (value as [string, string])[1]
        break
      case 'expiry.t1_default_after_h':
        next.expiry.t1_default_after_h = value as number
        break
      case 'expiry.t2_supersede_after_d':
        next.expiry.t2_supersede_after_d = value as number
        break
      case 'quota.per_source_daily_create':
        next.quota.per_source_daily_create = value as number
        break
      case 'fatigue.skip_rate_window_d':
        next.fatigue.skip_rate_window_d = value as number
        break
      case 'fatigue.skip_rate_downgrade':
        next.fatigue.skip_rate_downgrade = value as number
        break
    }
  }
  return next
}

/** 某個鍵在生效值裡的值（給「回復預設」比較用）。 */
export function valueOf(view: SettingsView, key: SettingKey): number | boolean | [string, string] {
  switch (key) {
    case 'force.cycle_min':
      return view.force.cycle_min
    case 'force.warn_before_min':
      return view.force.warn_before_min
    case 'windows.quiet_enabled':
      return view.windows.quiet_enabled
    case 'windows.quiet_hours':
      return [view.windows.quiet_start, view.windows.quiet_end]
    case 'expiry.t1_default_after_h':
      return view.expiry.t1_default_after_h
    case 'expiry.t2_supersede_after_d':
      return view.expiry.t2_supersede_after_d
    case 'quota.per_source_daily_create':
      return view.quota.per_source_daily_create
    case 'fatigue.skip_rate_window_d':
      return view.fatigue.skip_rate_window_d
    case 'fatigue.skip_rate_downgrade':
      return view.fatigue.skip_rate_downgrade
  }
}

// ---------------------------------------------------------------- 卡片

export type CardId = 'sleep' | 'force' | 'expiry' | 'quota' | 'fatigue'

export const CARD_KEYS: Record<CardId, SettingKey[]> = {
  sleep: ['windows.quiet_enabled', 'windows.quiet_hours'],
  force: ['force.cycle_min', 'force.warn_before_min'],
  expiry: ['expiry.t1_default_after_h', 'expiry.t2_supersede_after_d'],
  quota: ['quota.per_source_daily_create'],
  fatigue: ['fatigue.skip_rate_window_d', 'fatigue.skip_rate_downgrade'],
}

/** 畫面上的欄位（字串草稿；開關是布林）。 */
export type CardDraft = Record<string, string | boolean>

/** 鍵 → 畫面欄位。睡眠時間一個鍵兩個欄位；略過率畫面用百分比。 */
export function fieldsForKey(key: string): string[] {
  switch (key) {
    case 'windows.quiet_hours':
      return ['quiet_start', 'quiet_end']
    case 'fatigue.skip_rate_downgrade':
      return ['skip_rate_percent']
    default:
      return [key.split('.')[1] ?? key]
  }
}

export const percentFromRate = (rate: number) => String(Math.round(rate * 100))

export function draftFromView(card: CardId, view: SettingsView): CardDraft {
  switch (card) {
    case 'sleep':
      return { quiet_enabled: view.windows.quiet_enabled, quiet_start: view.windows.quiet_start, quiet_end: view.windows.quiet_end }
    case 'force':
      return { cycle_min: String(view.force.cycle_min), warn_before_min: String(view.force.warn_before_min) }
    case 'expiry':
      return { t1_default_after_h: String(view.expiry.t1_default_after_h), t2_supersede_after_d: String(view.expiry.t2_supersede_after_d) }
    case 'quota':
      return { per_source_daily_create: String(view.quota.per_source_daily_create) }
    case 'fatigue':
      return { skip_rate_window_d: String(view.fatigue.skip_rate_window_d), skip_rate_percent: percentFromRate(view.fatigue.skip_rate_downgrade) }
  }
}

export interface CardEvaluation {
  /** 只含真的改了的鍵。 */
  changes: SettingsChanges
  /** 畫面欄位 → 錯誤訊息。 */
  errors: Record<string, string>
  dirty: boolean
  valid: boolean
}

function parseIntField(text: string): number | null {
  const t = String(text).trim()
  if (!/^\d+$/.test(t)) return null
  return Number(t)
}

/** 依草稿算出變更與逐欄錯誤；沒改就不能儲存、不合法就不能儲存。 */
export function evaluateCard(card: CardId, draft: CardDraft, view: SettingsView): CardEvaluation {
  const errors: Record<string, string> = {}
  const changes: SettingsChanges = {}
  const intField = (field: string, key: SettingKey, current: number) => {
    const n = parseIntField(String(draft[field] ?? ''))
    if (n === null) {
      errors[field] = intMessage(key)
      return
    }
    if (n !== current) changes[key] = n
  }

  if (card === 'sleep') {
    const enabled = Boolean(draft.quiet_enabled)
    if (enabled !== view.windows.quiet_enabled) changes['windows.quiet_enabled'] = enabled
    const start = normalizeHm(String(draft.quiet_start ?? ''))
    const end = normalizeHm(String(draft.quiet_end ?? ''))
    if (!start) errors.quiet_start = HM_MESSAGE
    if (!end) errors.quiet_end = HM_MESSAGE
    if (start && end) {
      if (start === end) errors.quiet_end = SAME_MESSAGE
      else if (start !== view.windows.quiet_start || end !== view.windows.quiet_end) changes['windows.quiet_hours'] = [start, end]
    }
  } else if (card === 'force') {
    const cycle = parseIntField(String(draft.cycle_min ?? ''))
    const warn = parseIntField(String(draft.warn_before_min ?? ''))
    if (cycle === null) errors.cycle_min = intMessage('force.cycle_min')
    else if (cycle !== view.force.cycle_min) changes['force.cycle_min'] = cycle
    if (warn === null) errors.warn_before_min = warnMessage(cycle ?? view.force.cycle_min)
    else if (warn !== view.force.warn_before_min) changes['force.warn_before_min'] = warn
  } else if (card === 'expiry') {
    intField('t1_default_after_h', 'expiry.t1_default_after_h', view.expiry.t1_default_after_h)
    intField('t2_supersede_after_d', 'expiry.t2_supersede_after_d', view.expiry.t2_supersede_after_d)
  } else if (card === 'quota') {
    intField('per_source_daily_create', 'quota.per_source_daily_create', view.quota.per_source_daily_create)
  } else if (card === 'fatigue') {
    intField('skip_rate_window_d', 'fatigue.skip_rate_window_d', view.fatigue.skip_rate_window_d)
    const pct = parseIntField(String(draft.skip_rate_percent ?? ''))
    if (pct === null || pct < 10 || pct > 100) errors.skip_rate_percent = '略過率門檻要在 10%–100% 之間'
    else if (pct !== Math.round(view.fatigue.skip_rate_downgrade * 100)) changes['fatigue.skip_rate_downgrade'] = pct / 100
  }

  // 同一套範圍規則（含提前警告 < 週期）。
  for (const e of validateChanges(changes, view)) {
    for (const field of fieldsForKey(e.key)) if (!errors[field]) errors[field] = e.message
  }
  const dirty = JSON.stringify(draft) !== JSON.stringify(draftFromView(card, view))
  const valid = Object.keys(errors).length === 0
  return { changes: valid ? changes : {}, errors, dirty, valid }
}

export function cardOverridden(card: CardId, overridden: readonly string[]): boolean {
  return CARD_KEYS[card].some((key) => overridden.includes(key))
}

export function cardDiffersFromDefault(card: CardId, effective: SettingsView, defaults: SettingsView): boolean {
  return CARD_KEYS[card].some((key) => JSON.stringify(valueOf(effective, key)) !== JSON.stringify(valueOf(defaults, key)))
}

// ---------------------------------------------------------------- settings_set 回應

export type ParsedSettingsResult = { state: SettingsState } | { errors: SettingsFieldError[] } | { message: string }

function isFieldErrors(value: unknown): value is { errors: SettingsFieldError[] } {
  return (
    typeof value === 'object' &&
    value !== null &&
    Array.isArray((value as { errors?: unknown }).errors) &&
    (value as { errors: unknown[] }).errors.every((e) => typeof e === 'object' && e !== null && 'key' in e && 'message' in e)
  )
}

/** 回傳值或被拒絕的錯誤 → 狀態／逐欄錯誤／一則可讀訊息（Tauri 的錯誤可能是 JSON 字串）。 */
export function parseSettingsResult(raw: unknown): ParsedSettingsResult {
  let value = raw
  if (typeof value === 'string') {
    try {
      value = JSON.parse(value)
    } catch {
      return { message: raw as string }
    }
  }
  if (isFieldErrors(value)) return { errors: value.errors }
  if (typeof value === 'object' && value !== null && 'effective' in value && 'defaults' in value) return { state: value as SettingsState }
  if (value instanceof Error) return { message: value.message }
  if (typeof value === 'string') return { message: value }
  return { message: typeof raw === 'string' ? raw : '設定沒有存成功，請再試一次' }
}
