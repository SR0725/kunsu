// 設定欄位規則（BDD settings AC-ST-02／04／06、CONTRACT §15 範圍表、INV-26）：前端先擋、mock 與 server 同一套規則。
import { describe, expect, it } from 'vitest'
import type { SettingsView } from '../../types'
import {
  CARD_KEYS,
  POLICY_DEFAULTS,
  SETTING_KEYS,
  cardDiffersFromDefault,
  cardOverridden,
  draftFromView,
  evaluateCard,
  fieldsForKey,
  normalizeHm,
  parseSettingsResult,
  sleepSentence,
  validateChanges,
} from './schema'

const DEFAULTS: SettingsView = {
  force: { cycle_min: 15, warn_before_min: 3 },
  windows: { quiet_enabled: true, quiet_start: '23:00', quiet_end: '09:00' },
  expiry: { t1_default_after_h: 48, t2_supersede_after_d: 14 },
  quota: { per_source_daily_create: 12 },
  fatigue: { skip_rate_window_d: 7, skip_rate_downgrade: 0.5 },
}

it('預設值與 CONTRACT §10／§15 一致', () => {
  expect(POLICY_DEFAULTS).toEqual(DEFAULTS)
})

describe('可設定的鍵與範圍（CONTRACT §15）', () => {
  it('只有九個可設定鍵；錢、紅燈、deferred_retry_s、t1_batch、webhook 不在其中', () => {
    expect([...SETTING_KEYS].sort()).toEqual(
      [
        'expiry.t1_default_after_h',
        'expiry.t2_supersede_after_d',
        'fatigue.skip_rate_downgrade',
        'fatigue.skip_rate_window_d',
        'force.cycle_min',
        'force.warn_before_min',
        'quota.per_source_daily_create',
        'windows.quiet_enabled',
        'windows.quiet_hours',
      ].sort(),
    )
    expect(validateChanges({ 'force.deferred_retry_s': 5 } as never, DEFAULTS)).toEqual([
      { key: 'force.deferred_retry_s', message: expect.stringContaining('不能從設定頁修改') },
    ])
    expect(validateChanges({ 'discord.webhook': 'x' } as never, DEFAULTS)).toHaveLength(1)
  })

  it.each([
    ['force.cycle_min', 5, 120],
    ['expiry.t1_default_after_h', 1, 336],
    ['expiry.t2_supersede_after_d', 1, 14],
    ['quota.per_source_daily_create', 1, 100],
    ['fatigue.skip_rate_window_d', 1, 30],
  ] as const)('%s 含端點 %i–%i，外面一律拒絕', (key, min, max) => {
    expect(validateChanges({ [key]: min }, DEFAULTS)).toEqual([])
    expect(validateChanges({ [key]: max }, DEFAULTS)).toEqual([])
    expect(validateChanges({ [key]: min - 1 }, DEFAULTS)).toEqual([{ key, message: expect.stringContaining(`${min}–${max}`) }])
    expect(validateChanges({ [key]: max + 1 }, DEFAULTS)).toHaveLength(1)
    expect(validateChanges({ [key]: min + 0.5 }, DEFAULTS)).toHaveLength(1)
  })

  it('策略題取代上限 14 天（INV-14），說明題目最長只等 14 天', () => {
    expect(validateChanges({ 'expiry.t2_supersede_after_d': 15 }, DEFAULTS)[0].message).toContain('14 天')
  })

  it('略過率門檻 0.1–1.0', () => {
    expect(validateChanges({ 'fatigue.skip_rate_downgrade': 0.1 }, DEFAULTS)).toEqual([])
    expect(validateChanges({ 'fatigue.skip_rate_downgrade': 1 }, DEFAULTS)).toEqual([])
    expect(validateChanges({ 'fatigue.skip_rate_downgrade': 0.05 }, DEFAULTS)).toHaveLength(1)
    expect(validateChanges({ 'fatigue.skip_rate_downgrade': 1.2 }, DEFAULTS)).toHaveLength(1)
  })

  it('提前警告要小於週期（用同一筆變更裡的新週期判斷），訊息照 BDD', () => {
    expect(validateChanges({ 'force.warn_before_min': 15 }, DEFAULTS)).toEqual([
      { key: 'force.warn_before_min', message: '提前警告要小於週期長度（1–14 分鐘）' },
    ])
    expect(validateChanges({ 'force.warn_before_min': 0 }, DEFAULTS)).toHaveLength(1)
    expect(validateChanges({ 'force.warn_before_min': 14 }, DEFAULTS)).toEqual([])
    expect(validateChanges({ 'force.cycle_min': 30, 'force.warn_before_min': 20 }, DEFAULTS)).toEqual([])
    // 只縮短週期也要檢查既有的提前警告
    expect(validateChanges({ 'force.cycle_min': 5 }, { ...DEFAULTS, force: { cycle_min: 15, warn_before_min: 10 } })).toEqual([
      { key: 'force.warn_before_min', message: '提前警告要小於週期長度（1–4 分鐘）' },
    ])
  })

  it('睡眠時間：24 小時制 HH:MM、開始與結束不同、可跨午夜；開關只收 true／false', () => {
    expect(validateChanges({ 'windows.quiet_hours': ['23:00', '09:00'] }, DEFAULTS)).toEqual([])
    expect(validateChanges({ 'windows.quiet_hours': ['13:00', '15:30'] }, DEFAULTS)).toEqual([])
    expect(validateChanges({ 'windows.quiet_hours': ['23:00', '23:00'] }, DEFAULTS)[0].message).toContain('不能相同')
    expect(validateChanges({ 'windows.quiet_hours': ['24:00', '09:00'] }, DEFAULTS)).toHaveLength(1)
    expect(validateChanges({ 'windows.quiet_hours': ['9:00', '10:00'] }, DEFAULTS)).toHaveLength(1)
    expect(validateChanges({ 'windows.quiet_hours': ['09:00'] } as never, DEFAULTS)).toHaveLength(1)
    expect(validateChanges({ 'windows.quiet_enabled': false }, DEFAULTS)).toEqual([])
    expect(validateChanges({ 'windows.quiet_enabled': 'no' } as never, DEFAULTS)).toHaveLength(1)
  })
})

describe('時間欄位', () => {
  it('接受 HH:MM、H:MM、HHMM，統一成兩位數；其他回 null', () => {
    expect(normalizeHm('23:00')).toBe('23:00')
    expect(normalizeHm('9:05')).toBe('09:05')
    expect(normalizeHm('0930')).toBe('09:30')
    expect(normalizeHm(' 7:00 ')).toBe('07:00')
    expect(normalizeHm('24:00')).toBeNull()
    expect(normalizeHm('12:60')).toBeNull()
    expect(normalizeHm('下午三點')).toBeNull()
    expect(normalizeHm('')).toBeNull()
  })
})

describe('睡眠時間說明（AC-ST-04）', () => {
  it('跨午夜寫「隔天」', () => {
    expect(sleepSentence({ enabled: true, start: '23:00', end: '09:00' })).toBe(
      '每天 23:00 到隔天 09:00 不會強制詢問，也不會跳出提醒；題目照常保存，醒來後從原進度繼續',
    )
  })
  it('同一天就不寫隔天', () => {
    expect(sleepSentence({ enabled: true, start: '13:00', end: '15:00' })).toBe(
      '每天 13:00 到 15:00 不會強制詢問，也不會跳出提醒；題目照常保存，醒來後從原進度繼續',
    )
  })
  it('關閉時說明睡眠時間不再擋強制詢問', () => {
    expect(sleepSentence({ enabled: false, start: '23:00', end: '09:00' })).toContain('不再擋強制詢問')
  })
})

describe('卡片草稿（AC-ST-01／02／03）', () => {
  it('每張卡片管的鍵', () => {
    expect(CARD_KEYS.sleep).toEqual(['windows.quiet_enabled', 'windows.quiet_hours'])
    expect(CARD_KEYS.force).toEqual(['force.cycle_min', 'force.warn_before_min'])
    expect(CARD_KEYS.expiry).toEqual(['expiry.t1_default_after_h', 'expiry.t2_supersede_after_d'])
    expect(CARD_KEYS.quota).toEqual(['quota.per_source_daily_create'])
    expect(CARD_KEYS.fatigue).toEqual(['fatigue.skip_rate_window_d', 'fatigue.skip_rate_downgrade'])
  })

  it('沒改就不能儲存；合法改動只送改到的鍵', () => {
    const draft = draftFromView('force', DEFAULTS)
    expect(evaluateCard('force', draft, DEFAULTS)).toMatchObject({ dirty: false, valid: true, changes: {} })
    const next = evaluateCard('force', { ...draft, cycle_min: '20' }, DEFAULTS)
    expect(next).toMatchObject({ dirty: true, valid: true, changes: { 'force.cycle_min': 20 } })
  })

  it('不合法：逐欄錯誤、不能儲存', () => {
    const draft = draftFromView('force', DEFAULTS)
    const bad = evaluateCard('force', { ...draft, warn_before_min: '15' }, DEFAULTS)
    expect(bad.valid).toBe(false)
    expect(bad.errors.warn_before_min).toBe('提前警告要小於週期長度（1–14 分鐘）')
    expect(evaluateCard('force', { ...draft, cycle_min: '' }, DEFAULTS).errors.cycle_min).toContain('5–120')
    expect(evaluateCard('force', { ...draft, cycle_min: 'abc' }, DEFAULTS).valid).toBe(false)
  })

  it('睡眠卡片：時間以陣列送出，接受 0930 這種寫法', () => {
    const draft = draftFromView('sleep', DEFAULTS)
    expect(draft).toEqual({ quiet_enabled: true, quiet_start: '23:00', quiet_end: '09:00' })
    expect(evaluateCard('sleep', { ...draft, quiet_end: '0930' }, DEFAULTS).changes).toEqual({ 'windows.quiet_hours': ['23:00', '09:30'] })
    expect(evaluateCard('sleep', { ...draft, quiet_enabled: false }, DEFAULTS).changes).toEqual({ 'windows.quiet_enabled': false })
    expect(evaluateCard('sleep', { ...draft, quiet_end: '23:00' }, DEFAULTS).errors.quiet_end).toContain('不能相同')
    expect(evaluateCard('sleep', { ...draft, quiet_start: '25:00' }, DEFAULTS).errors.quiet_start).toContain('HH:MM')
  })

  it('疲勞卡片：畫面用百分比（10%–100%），送出 0.1–1.0', () => {
    const draft = draftFromView('fatigue', DEFAULTS)
    expect(draft.skip_rate_percent).toBe('50')
    expect(evaluateCard('fatigue', { ...draft, skip_rate_percent: '70' }, DEFAULTS).changes).toEqual({ 'fatigue.skip_rate_downgrade': 0.7 })
    expect(evaluateCard('fatigue', { ...draft, skip_rate_percent: '5' }, DEFAULTS).errors.skip_rate_percent).toContain('10%–100%')
  })

  it('「已改過」看 local 檔有沒有這張卡片的鍵；回復預設看值或 local 檔', () => {
    expect(cardOverridden('force', ['force.cycle_min'])).toBe(true)
    expect(cardOverridden('force', ['quota.per_source_daily_create'])).toBe(false)
    expect(cardDiffersFromDefault('force', { ...DEFAULTS, force: { cycle_min: 20, warn_before_min: 3 } }, DEFAULTS)).toBe(true)
    expect(cardDiffersFromDefault('force', DEFAULTS, DEFAULTS)).toBe(false)
  })

  it('server 的逐欄錯誤對到畫面欄位', () => {
    expect(fieldsForKey('force.warn_before_min')).toEqual(['warn_before_min'])
    expect(fieldsForKey('windows.quiet_hours')).toEqual(['quiet_start', 'quiet_end'])
    expect(fieldsForKey('fatigue.skip_rate_downgrade')).toEqual(['skip_rate_percent'])
  })
})

describe('settings_set 回應', () => {
  const state = { effective: DEFAULTS, defaults: DEFAULTS, overridden: [], local_path: '~/.kunsu/policy.local.toml' }
  it('正常回應是 settings_get 的形狀', () => {
    expect(parseSettingsResult(state)).toEqual({ state })
  })
  it('{errors} 是逐欄錯誤（不論是回傳值還是被拒絕的錯誤物件）', () => {
    const errors = [{ key: 'force.cycle_min', message: '週期長度要在 5–120 分鐘之間' }]
    expect(parseSettingsResult({ errors })).toEqual({ errors })
    expect(parseSettingsResult(JSON.stringify({ errors }))).toEqual({ errors })
  })
  it('其他錯誤變成一則可讀訊息', () => {
    expect(parseSettingsResult('寫入失敗：磁碟已滿')).toEqual({ message: '寫入失敗：磁碟已滿' })
    expect(parseSettingsResult(new Error('timeout'))).toEqual({ message: 'timeout' })
  })
})
