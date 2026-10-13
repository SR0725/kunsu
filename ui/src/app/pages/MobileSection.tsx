// 設定 › 手機（PRD §30.7、§30.8；BDD mobile-pairing AC-MP-08）：中轉 AI、配對 QR、待允許、已配對裝置。
import { useCallback, useEffect, useState } from 'react'
import type { KunsuIpc } from '../../ipc'
import type { MobileStatus, PairingQr } from '../../types'
import { describeConnection } from '../settings/mobile'
import { Button, SegmentedControl, SettingsCard, StatusPill, useToast } from '../ui'

const TONE_ICON = { success: 'check', waiting: 'clock', danger: 'alert' } as const

function fmtTime(unix: number | null): string {
  if (!unix) return '還沒上線'
  const d = new Date(unix * 1000)
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

export function MobileSection({ ipc }: { ipc: KunsuIpc }) {
  const toast = useToast()
  const [status, setStatus] = useState<MobileStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [qr, setQr] = useState<PairingQr | null>(null)
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000))
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null)

  const refresh = useCallback(() => {
    ipc.mobileStatus().then(setStatus, (e) => setError(e instanceof Error ? e.message : String(e)))
  }, [ipc])
  useEffect(() => {
    refresh()
    const t = window.setInterval(() => {
      refresh()
      setNow(Math.floor(Date.now() / 1000))
    }, 2000)
    return () => window.clearInterval(t)
  }, [refresh])

  const run = async (work: () => Promise<MobileStatus>, done?: string) => {
    setError(null)
    try {
      setStatus(await work())
      if (done) toast(done)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  const startPairing = async () => {
    setError(null)
    try {
      setQr(await ipc.mobileStartPairing())
      setNow(Math.floor(Date.now() / 1000))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  if (!status) {
    return error ? <p role="alert" className="k-card__help-error">{error}</p> : null
  }
  const conn = describeConnection(status)
  const qrLeft = qr ? qr.expires_at - now : 0
  const devices = status.devices.filter((d) => !d.revoked)
  const revoked = status.devices.filter((d) => d.revoked)

  return (
    <>
      <SettingsCard
        title="中轉 AI"
        description="軍師在手機上用這個 AI 回答你的追問、陪你聊天和開任務。它不能替你回答決定題或批准動錢。"
        footer={<p className="k-card__help">跟手機 App 的設定是同一個值。</p>}
      >
        <SegmentedControl<'claude' | 'codex'>
          label="中轉 AI"
          value={status.relay_agent}
          onChange={(agent) => void run(() => ipc.mobileSetRelayAgent(agent), `中轉 AI 改成 ${agent === 'claude' ? 'Claude' : 'Codex'}`)}
          options={[
            { value: 'claude', label: 'Claude' },
            { value: 'codex', label: 'Codex' },
          ]}
        />
      </SettingsCard>

      <SettingsCard
        title="手機"
        description="用 iPhone 掃 QR code 配對，之後就能在手機上回答。內容經你的 Zeabur 中轉站端對端加密傳輸，中轉站看不到。"
        footer={
          <p className="k-card__help">
            {error ? (
              <span role="alert" className="k-card__help-error">{error}</span>
            ) : (
              <>中轉站：<code>{status.relay_url}</code></>
            )}
          </p>
        }
      >
        <div className="k-mobile">
          <div className="k-mobile__row">
            <StatusPill tone={conn.tone} icon={TONE_ICON[conn.tone]}>{conn.text}</StatusPill>
            <Button variant="primary" icon="smartphone" onClick={() => void startPairing()}>
              {qr && qrLeft > 0 ? '重新產生 QR code' : '配對新手機'}
            </Button>
          </div>

          {qr && qrLeft > 0 && (
            <div className="k-mobile__qr">
              {/* QR 是 Kunsu 自己（Rust qrcode）產生的 SVG，不是 AI 內容 */}
              <div className="k-mobile__qr-code" role="img" aria-label="配對用的 QR code" dangerouslySetInnerHTML={{ __html: qr.svg }} />
              <div className="k-mobile__qr-text">
                <p className="k-mobile__qr-title">用 iPhone 的軍師 App 掃這個 QR code</p>
                <p>掃完之後，這裡會跳出「要配對」，你按允許才算配對成功。QR 被別人拍到也配不上。</p>
                <p className="k-mobile__qr-left">{`${Math.floor(qrLeft / 60)}:${String(qrLeft % 60).padStart(2, '0')} 後失效，只能用一次`}</p>
              </div>
            </div>
          )}

          {status.pending.map((p) => (
            <div key={p.id} className="k-mobile__pending" role="alert">
              <span>{`「${p.device_name}」要配對這台 Mac`}</span>
              <span className="k-mobile__actions">
                <Button onClick={() => void run(() => ipc.mobileDecidePairing(p.id, false), '已拒絕配對')}>拒絕</Button>
                <Button
                  variant="primary"
                  onClick={() =>
                    void run(async () => {
                      const s = await ipc.mobileDecidePairing(p.id, true)
                      setQr(null)
                      return s
                    }, `已配對 ${p.device_name}`)
                  }
                >
                  允許
                </Button>
              </span>
            </div>
          ))}

          <div className="k-mobile__devices">
            <p className="k-mobile__label">{devices.length ? `已配對 ${devices.length} 台` : '還沒有配對的手機'}</p>
            {devices.map((d) => (
              <div key={d.id} className="k-mobile__device">
                <span className="k-mobile__device-name">{d.name}</span>
                <span className="k-mobile__device-meta">{`配對於 ${fmtTime(d.paired_at)} · 最後上線 ${fmtTime(d.last_seen)}`}</span>
                {confirmRevoke === d.id ? (
                  <span className="k-mobile__actions">
                    <Button onClick={() => setConfirmRevoke(null)}>取消</Button>
                    <Button
                      variant="danger"
                      onClick={() => void run(() => ipc.mobileRevoke(d.id), `已撤銷 ${d.name}`).then(() => setConfirmRevoke(null))}
                    >
                      確定撤銷
                    </Button>
                  </span>
                ) : (
                  <Button variant="ghost" onClick={() => setConfirmRevoke(d.id)}>撤銷</Button>
                )}
              </div>
            ))}
            {revoked.length > 0 && <p className="k-mobile__label">{`已撤銷：${revoked.map((d) => d.name).join('、')}`}</p>}
          </div>
        </div>
      </SettingsCard>
    </>
  )
}
