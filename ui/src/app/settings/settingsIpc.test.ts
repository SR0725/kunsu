// 設定與狀態 IPC（CONTRACT §15）：invoke 名稱與 JSON 形狀嚴格照契約；mock 用同一套範圍規則、記住變更、可模擬帳本卡住與
// local 檔錯誤（BDD settings AC-ST-01／02／03／09／11／13；INV-26 的 UI 這一半：儲存只碰設定）。
import { describe, expect, it } from 'vitest'
import { makeSettingsMock } from './settingsMock'
import { settingsTauriIpc } from './settingsIpc'
import { resetCard, saveCard } from './actions'
import { draftFromView } from './schema'
import type { SettingsIpc } from './settingsIpc'

function recorder(reply: (cmd: string, args?: Record<string, unknown>) => unknown) {
  const calls: { cmd: string; args?: Record<string, unknown> }[] = []
  const invoke = async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ cmd, args })
    return reply(cmd, args) as T
  }
  return { calls, invoke }
}

function memoryStorage() {
  const map = new Map<string, string>()
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
    map,
  }
}

describe('Tauri invoke（CONTRACT §15）', () => {
  it('命令名稱與參數形狀', async () => {
    const r = recorder(() => ({ effective: {}, defaults: {}, overridden: [], local_path: 'p' }))
    const ipc = settingsTauriIpc(r.invoke)
    await ipc.settingsGet()
    await ipc.settingsSet({ 'force.cycle_min': 20, 'windows.quiet_hours': ['23:00', '08:00'] })
    await ipc.settingsReset(['force.cycle_min'])
    await ipc.appStatus()
    await ipc.edgeResetPosition()
    await ipc.openPrivacySettings()
    expect(r.calls).toEqual([
      { cmd: 'settings_get', args: undefined },
      { cmd: 'settings_set', args: { changes: { 'force.cycle_min': 20, 'windows.quiet_hours': ['23:00', '08:00'] } } },
      { cmd: 'settings_reset', args: { keys: ['force.cycle_min'] } },
      { cmd: 'app_status', args: undefined },
      { cmd: 'edge_reset_position', args: undefined },
      { cmd: 'open_privacy_settings', args: undefined },
    ])
  })

  it('settings_reset 被拒絕（{errors}）時丟出可讀訊息', async () => {
    const rejecting = async () => {
      throw { errors: [{ key: 'force.warn_before_min', message: '提前警告要小於週期長度（1–14 分鐘）' }] }
    }
    await expect(settingsTauriIpc(rejecting).settingsReset(['force.cycle_min'])).rejects.toThrow('提前警告要小於週期長度（1–14 分鐘）')
  })

  it('settings_set 的 {errors} 不論是回傳值還是被拒絕的錯誤都變成逐欄錯誤', async () => {
    const errors = [{ key: 'force.cycle_min', message: '週期長度要在 5–120 分鐘之間' }]
    expect(await settingsTauriIpc(recorder(() => ({ errors })).invoke).settingsSet({ 'force.cycle_min': 1 })).toEqual({ errors })
    const rejecting = async () => {
      throw JSON.stringify({ errors })
    }
    expect(await settingsTauriIpc(rejecting).settingsSet({ 'force.cycle_min': 1 })).toEqual({ errors })
    const failing = async () => {
      throw '寫入失敗'
    }
    await expect(settingsTauriIpc(failing).settingsSet({ 'force.cycle_min': 20 })).rejects.toThrow('寫入失敗')
  })
})

describe('mock：與 server 同一套規則', () => {
  it('初始是預設值、沒有改過；local 檔路徑', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    const s = await mock.settingsGet()
    expect(s.effective.force.cycle_min).toBe(15)
    expect(s.overridden).toEqual([])
    expect(s.local_path).toContain('policy.local.toml')
    expect(s.local_error ?? null).toBeNull()
  })

  it('合法變更：立即生效、標成已改過、重開（同一個儲存）還在', async () => {
    const storage = memoryStorage()
    const mock = makeSettingsMock({ storage })
    const result = await mock.settingsSet({ 'force.cycle_min': 20 })
    expect('errors' in result).toBe(false)
    const again = await makeSettingsMock({ storage }).settingsGet()
    expect(again.effective.force.cycle_min).toBe(20)
    expect(again.overridden).toEqual(['force.cycle_min'])
  })

  it('不合法：整筆拒絕，任何值都不變', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    const result = await mock.settingsSet({ 'force.cycle_min': 30, 'force.warn_before_min': 30 })
    expect(result).toEqual({ errors: [{ key: 'force.warn_before_min', message: '提前警告要小於週期長度（1–29 分鐘）' }] })
    const s = await mock.settingsGet()
    expect(s.effective.force).toEqual({ cycle_min: 15, warn_before_min: 3 })
    expect(s.overridden).toEqual([])
  })

  it('回復預設：鍵移除、回到預設值', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    await mock.settingsSet({ 'windows.quiet_hours': ['22:00', '07:00'], 'windows.quiet_enabled': false })
    const s = await mock.settingsReset(['windows.quiet_hours'])
    expect(s.effective.windows).toEqual({ quiet_enabled: false, quiet_start: '23:00', quiet_end: '09:00' })
    expect(s.overridden).toEqual(['windows.quiet_enabled'])
  })

  it('local 檔壞掉：生效值只用預設、帶錯誤與檔案位置；全部回復預設後恢復', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage(), scenario: { localError: true } })
    const broken = await mock.settingsGet()
    expect(broken.local_error).toMatch(/policy\.local\.toml|第 \d+ 行/)
    expect(broken.effective.force.cycle_min).toBe(15)
    // 「全部回復預設」＝settings_reset { keys: [] }（CONTRACT §15）。
    const fixed = await mock.settingsReset([])
    expect(fixed.local_error ?? null).toBeNull()
  })

  it('帳本卡住、待補寫與未設定的整合可以模擬；未設定附可複製的指令', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage(), scenario: { ledger: 'stuck', integrations: 'missing' } })
    const status = await mock.appStatus()
    expect(status.ledger.stuck_s).toBeGreaterThanOrEqual(120)
    expect(status.ledger.backlog).toBeGreaterThan(0)
    const missing = status.integrations.filter((i) => i.status === 'missing')
    expect(missing.length).toBeGreaterThan(0)
    for (const item of missing) expect(item.fix).toBeTruthy()
    expect(status.integrations.map((i) => i.id).sort()).toEqual(['claude_mcp', 'cli', 'codex_mcp', 'skill'])
    // 只引用 token 檔路徑，不含任何 token 值（長串英數）。
    expect(JSON.stringify(status)).not.toMatch(/[A-Za-z0-9_-]{32,}/)
  })

  it('瀏海回到預設位置會刪掉保存的位置', async () => {
    const storage = memoryStorage()
    storage.setItem('kunsu:mock:edge', JSON.stringify({ corner: 'tr' }))
    await makeSettingsMock({ storage }).edgeResetPosition()
    expect(storage.getItem('kunsu:mock:edge')).toBeNull()
  })
})

describe('儲存與回復預設只碰設定（INV-26 的 UI 這一半）', () => {
  function spyIpc(base: ReturnType<typeof makeSettingsMock>) {
    const called: string[] = []
    const ipc = new Proxy(base as unknown as Record<string, unknown>, {
      get(target, prop: string) {
        const value = target[prop]
        if (typeof value === 'function') {
          return (...args: unknown[]) => {
            called.push(prop)
            return (value as (...a: unknown[]) => unknown).apply(target, args)
          }
        }
        // 作答、稍後再問等不存在於設定 mock 的方法：被呼叫就會記錄並丟錯
        return (...args: unknown[]) => {
          called.push(prop)
          throw new Error(`不該呼叫 ${prop}(${args.length})`)
        }
      },
    })
    return { ipc: ipc as unknown as SettingsIpc, called }
  }

  it('儲存合法變更：只呼叫 settingsSet，成功回新狀態', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    const state = await mock.settingsGet()
    const { ipc, called } = spyIpc(mock)
    const draft = { ...draftFromView('force', state.effective), cycle_min: '20' }
    const outcome = await saveCard(ipc, 'force', draft, state)
    expect(outcome.kind).toBe('saved')
    expect(called).toEqual(['settingsSet'])
  })

  it('不合法：連 IPC 都不呼叫，直接回逐欄錯誤', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    const state = await mock.settingsGet()
    const { ipc, called } = spyIpc(mock)
    const draft = { ...draftFromView('force', state.effective), warn_before_min: '15' }
    const outcome = await saveCard(ipc, 'force', draft, state)
    expect(outcome).toEqual({ kind: 'invalid', errors: { warn_before_min: '提前警告要小於週期長度（1–14 分鐘）' } })
    expect(called).toEqual([])
  })

  it('server 回逐欄錯誤：對到畫面欄位', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    const state = await mock.settingsGet()
    const ipc = {
      settingsSet: async () => ({ errors: [{ key: 'force.cycle_min', message: 'server 說不行' }] }),
    } as unknown as Parameters<typeof saveCard>[0]
    const draft = { ...draftFromView('force', state.effective), cycle_min: '20' }
    expect(await saveCard(ipc, 'force', draft, state)).toEqual({ kind: 'invalid', errors: { cycle_min: 'server 說不行' } })
  })

  it('檔案層錯誤（key 是 policy.local.toml）不對應任何欄位：變成卡片上的一則訊息', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    const state = await mock.settingsGet()
    const ipc = {
      settingsSet: async () => ({ errors: [{ key: 'policy.local.toml', message: '寫不進 policy.local.toml：權限不足' }] }),
    } as unknown as SettingsIpc
    const draft = { ...draftFromView('force', state.effective), cycle_min: '20' }
    expect(await saveCard(ipc, 'force', draft, state)).toEqual({ kind: 'invalid', errors: {}, message: '寫不進 policy.local.toml：權限不足' })
  })

  it('回復預設：只呼叫 settingsReset，帶這張卡片的鍵', async () => {
    const mock = makeSettingsMock({ storage: memoryStorage() })
    await mock.settingsSet({ 'quota.per_source_daily_create': 30 })
    const { ipc, called } = spyIpc(mock)
    const outcome = await resetCard(ipc, 'quota')
    expect(called).toEqual(['settingsReset'])
    expect(outcome.kind === 'saved' && outcome.state.effective.quota.per_source_daily_create).toBe(12)
  })
})
