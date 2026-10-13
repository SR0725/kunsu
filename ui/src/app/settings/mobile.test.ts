// 主視窗的手機配對與中轉 AI（BDD mobile-pairing AC-MP-08；PRD §30.7、§30.8）。
import { describe, expect, it } from 'vitest'
import indexSettingsPage from '../pages/SettingsPage.tsx?raw'
import { settingsTauriIpc } from './settingsIpc'
import { makeSettingsMock } from './settingsMock'
import { SETTINGS_SECTIONS } from './sections'
import { pairingBanner, describeConnection } from './mobile'
import type { MobileStatus } from '../../types'

function recorder() {
  const calls: { cmd: string; args?: Record<string, unknown> }[] = []
  const invoke = async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ cmd, args })
    return {} as T
  }
  return { calls, invoke }
}

const base: MobileStatus = {
  relay_url: 'wss://kunsu-relay.zeabur.app/v1',
  connected: true,
  last_error: null,
  mac_id: 'abcd1234abcd1234',
  relay_agent: 'claude',
  devices: [],
  pending: [],
}

describe('AC-MP-08 手機 IPC', () => {
  it('命令名稱與參數形狀', async () => {
    const r = recorder()
    const ipc = settingsTauriIpc(r.invoke)
    await ipc.mobileStatus()
    await ipc.mobileStartPairing()
    await ipc.mobileDecidePairing('p1', true)
    await ipc.mobileRevoke('d1')
    await ipc.mobileSetRelayAgent('codex')
    expect(r.calls).toEqual([
      { cmd: 'mobile_status', args: undefined },
      { cmd: 'mobile_start_pairing', args: undefined },
      { cmd: 'mobile_decide_pairing', args: { id: 'p1', allow: true } },
      { cmd: 'mobile_revoke', args: { device_id: 'd1' } },
      { cmd: 'mobile_set_relay_agent', args: { agent: 'codex' } },
    ])
  })

  it('mock：配對要按允許才出現在裝置清單；撤銷後標成已撤銷', async () => {
    const mock = makeSettingsMock({})
    const qr = await mock.mobileStartPairing()
    expect(qr.svg).toContain('<svg')
    mock.mobileSimulatePairRequest?.('Alex 的 iPhone')
    let s = await mock.mobileStatus()
    expect(s.pending.map((p) => p.device_name)).toEqual(['Alex 的 iPhone'])
    expect(s.devices).toEqual([])
    s = await mock.mobileDecidePairing(s.pending[0].id, true)
    expect(s.pending).toEqual([])
    expect(s.devices.map((d) => d.name)).toEqual(['Alex 的 iPhone'])
    s = await mock.mobileRevoke(s.devices[0].id)
    expect(s.devices[0].revoked).toBe(true)
    s = await mock.mobileSetRelayAgent('codex')
    expect(s.relay_agent).toBe('codex')
  })
})

describe('設定頁有「手機」分頁', () => {
  it('分頁排在一般與帳號之後', () => {
    expect(SETTINGS_SECTIONS.map((s) => s.id).slice(0, 3)).toEqual(['general', 'account', 'mobile'])
  })
  it('外觀說明不再說決策面板永遠是黑色（統一設計語言）', () => {
    expect(indexSettingsPage).not.toContain('瀏海、強制詢問與決策面板永遠是黑色')
  })
})

describe('配對提醒與連線狀態的文字', () => {
  it('有待允許的配對：任何頁面都跳提醒', () => {
    expect(pairingBanner(base)).toBeNull()
    const s = { ...base, pending: [{ id: 'p', device_name: 'Alex 的 iPhone', received_at: 1 }] }
    expect(pairingBanner(s)).toEqual({ id: 'p', text: '「Alex 的 iPhone」要配對這台 Mac' })
  })
  it('連線狀態寫出原因，不只靠顏色', () => {
    expect(describeConnection(base)).toEqual({ tone: 'success', text: '已連上中轉站' })
    expect(describeConnection({ ...base, connected: false, last_error: '連不上中轉站：timeout' })).toEqual({
      tone: 'danger',
      text: '連不上中轉站：timeout',
    })
    expect(describeConnection({ ...base, connected: false })).toEqual({ tone: 'waiting', text: '正在連線中轉站…' })
  })
})
