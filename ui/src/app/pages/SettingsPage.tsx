// 設定（BDD settings；PRD §27.3；DESIGN「Settings layout」「Cards」）：左側區段導覽＋dub 式設定卡片。每張 policy 卡片
// 顯示目前值、單位、範圍、「已改過」與「回復預設」，有合法變更才能「儲存」；server 的逐欄錯誤顯示在欄位下。
// 設定只寫 ~/.kunsu/policy.local.toml 與這台 Mac 的呈現偏好，永不碰任何題目（INV-26）。
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import type { KunsuIpc } from '../../ipc'
import type { AppStatus, SettingsState } from '../../types'
import type { DndStatus } from '../../dnd'
import { MOTION_KEY } from '../../motion'
import { DndOptions } from '../DndControl'
import { SETTINGS_SECTIONS, type SettingsSectionId } from '../settings/sections'
import { useAppStatus } from '../appStatus'
import { IntegrationList } from '../Integrations'
import { resetCard, saveCard } from '../settings/actions'
import {
  CARD_KEYS,
  INT_RANGES,
  cardDiffersFromDefault,
  cardOverridden,
  draftFromView,
  evaluateCard,
  humanizeSettingsError,
  percentFromRate,
  sleepSentence,
  valueOf,
  warnRange,
  normalizeHm,
  type CardDraft,
  type CardId,
} from '../settings/schema'
import { formatUptime, ledgerHealth } from '../status'
import { readThemePref, setThemePref, type ThemePref } from '../theme'
import { BRAND_ICON } from '../brand'
import { MobileSection } from './MobileSection'
import { AccountSection } from './AccountSection'
import { Button, Card, Icon, NumberField, PageBody, PageHeader, Pill, SegmentedControl, SettingsCard, Switch, TimeField, useToast } from '../ui'

const describeDefault = (text: string, range: string) => `預設 ${text} · 範圍 ${range}`

// ---------------------------------------------------------------- policy 卡片

function PolicyCard({
  card,
  ipc,
  state,
  onState,
  title,
  description,
  help,
  children,
}: {
  card: CardId
  ipc: KunsuIpc
  state: SettingsState
  onState: (state: SettingsState) => void
  title: string
  description: ReactNode
  help: ReactNode
  children: (fields: { draft: CardDraft; set: (field: string, value: string | boolean) => void; error: (field: string) => string | undefined }) => ReactNode
}) {
  const toast = useToast()
  const slice = JSON.stringify(CARD_KEYS[card].map((key) => valueOf(state.effective, key)))
  const effective = useRef(state.effective)
  effective.current = state.effective
  const [draft, setDraft] = useState<CardDraft>(() => draftFromView(card, state.effective))
  const [serverErrors, setServerErrors] = useState<Record<string, string>>({})
  const [failure, setFailure] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  // 存好或回復預設後「儲存」變成停用、「回復預設」消失：焦點移到卡片標題，不掉到頁面（AC-ST-15）。
  const titleRef = useRef<HTMLHeadingElement>(null)
  const focusTitle = useRef(false)
  useEffect(() => {
    if (!focusTitle.current) return
    focusTitle.current = false
    titleRef.current?.focus()
  })
  // 生效值改了（儲存、回復預設、重新讀取）就以新值為準；別張卡片儲存不會清掉這張的草稿。
  useEffect(() => {
    setDraft(draftFromView(card, effective.current))
    setServerErrors({})
  }, [card, slice])

  const evaluation = evaluateCard(card, draft, state.effective)
  const set = (field: string, value: string | boolean) => {
    setDraft((current) => ({ ...current, [field]: value }))
    setServerErrors((current) => {
      if (!(field in current)) return current
      const next = { ...current }
      delete next[field]
      return next
    })
    setFailure(null)
  }
  const error = (field: string) => serverErrors[field] ?? (evaluation.dirty ? evaluation.errors[field] : undefined)

  const save = async () => {
    setSaving(true)
    setFailure(null)
    const outcome = await saveCard(ipc, card, draft, state)
    setSaving(false)
    if (outcome.kind === 'saved') {
      focusTitle.current = true
      onState(outcome.state)
      toast('已儲存，立即生效')
    } else if (outcome.kind === 'invalid') {
      setServerErrors(outcome.errors)
      if (outcome.message) setFailure(outcome.message)
    } else if (outcome.kind === 'failed') setFailure(outcome.message)
  }
  const reset = async () => {
    setSaving(true)
    setFailure(null)
    const outcome = await resetCard(ipc, card)
    setSaving(false)
    if (outcome.kind === 'saved') {
      focusTitle.current = true
      onState(outcome.state)
      toast('已回復預設，立即生效')
    } else if (outcome.kind === 'failed') setFailure(outcome.message)
  }

  const overridden = cardOverridden(card, state.overridden)
  return (
    <SettingsCard
      title={title}
      titleRef={titleRef}
      description={description}
      modified={overridden}
      help={
        failure ? (
          <span role="alert" className="k-card__help-error" title={failure}>
            {`沒有存成功：${humanizeSettingsError(failure)}`}
          </span>
        ) : (
          help
        )
      }
      canSave={evaluation.dirty && evaluation.valid && Object.keys(evaluation.changes).length > 0 && Object.keys(serverErrors).length === 0}
      saving={saving}
      onSave={() => void save()}
      showReset={overridden || cardDiffersFromDefault(card, state.effective, state.defaults)}
      onReset={() => void reset()}
    >
      {children({ draft, set, error })}
    </SettingsCard>
  )
}

// ---------------------------------------------------------------- 區段

function GeneralSection() {
  const [theme, setTheme] = useState<ThemePref>(() => readThemePref())
  const [motion, setMotion] = useState(() => {
    try {
      return localStorage.getItem(MOTION_KEY) !== 'reduced'
    } catch {
      return true
    }
  })
  const changeMotion = (on: boolean) => {
    setMotion(on)
    try {
      localStorage.setItem(MOTION_KEY, on ? 'full' : 'reduced')
    } catch {
      // 存不了也照樣套到這個視窗
    }
    document.documentElement.dataset.motion = on ? 'full' : 'reduced'
  }
  return (
    <>
      <SettingsCard
        title="外觀"
        description="主視窗的顏色。跟隨系統時，macOS 換深淺會立即跟著換。"
        footer={<p className="k-card__help">只存在這台 Mac；決策面板與強制詢問跟著這裡的設定，只有貼在螢幕邊緣的瀏海永遠是黑色。</p>}
      >
        <SegmentedControl<ThemePref>
          label="外觀"
          value={theme}
          onChange={(next) => {
            setTheme(next)
            setThemePref(next)
          }}
          options={[
            { value: 'system', label: '跟隨系統', icon: 'monitor' },
            { value: 'light', label: '淺色', icon: 'sun' },
            { value: 'dark', label: '深色', icon: 'moon' },
          ]}
        />
      </SettingsCard>
      <SettingsCard title="動態" footer={<p className="k-card__help">macOS 的「減少動態效果」打開時，不論這裡怎麼設都不播。</p>}>
        <Switch
          id="settings-motion"
          label="連續轉場動畫"
          description="瀏海展開成決策面板、來源圖示飛到頁首的動畫。關掉後改成 160 ms 淡入淡出。"
          checked={motion}
          onChange={changeMotion}
        />
      </SettingsCard>
    </>
  )
}

function SleepCard({ ipc, state, onState }: { ipc: KunsuIpc; state: SettingsState; onState: (s: SettingsState) => void }) {
  const d = state.defaults.windows
  return (
    <PolicyCard
      card="sleep"
      ipc={ipc}
      state={state}
      onState={onState}
      title="時段"
      description="這段時間不強制詢問、也不跳提醒；題目照常保存，時間一到依原進度恢復，不重設週期。"
      help={`台北時間，可跨午夜。預設 ${d.quiet_start}–${d.quiet_end}。`}
    >
      {({ draft, set, error }) => {
        const start = normalizeHm(String(draft.quiet_start)) ?? state.effective.windows.quiet_start
        const end = normalizeHm(String(draft.quiet_end)) ?? state.effective.windows.quiet_end
        const enabled = Boolean(draft.quiet_enabled)
        return (
          <div className="k-stack">
            <Switch id="settings-quiet-enabled" label="啟用睡眠時間" checked={enabled} onChange={(v) => set('quiet_enabled', v)} />
            <div className="k-field-row">
              <TimeField id="settings-quiet-start" label="開始" value={String(draft.quiet_start)} onChange={(v) => set('quiet_start', v)} error={error('quiet_start')} disabled={!enabled} />
              <span className="k-field-row__sep" aria-hidden="true">
                到
              </span>
              <TimeField id="settings-quiet-end" label="結束" value={String(draft.quiet_end)} onChange={(v) => set('quiet_end', v)} error={error('quiet_end')} disabled={!enabled} />
            </div>
            <p className="k-sentence" data-off={enabled ? undefined : 'true'}>
              <Icon name={enabled ? 'moon' : 'info'} size={16} />
              <span>{sleepSentence({ enabled, start, end: start === end ? state.effective.windows.quiet_end : end })}</span>
            </p>
          </div>
        )
      }}
    </PolicyCard>
  )
}

function ForceCard({ ipc, state, onState }: { ipc: KunsuIpc; state: SettingsState; onState: (s: SettingsState) => void }) {
  const d = state.defaults.force
  return (
    <PolicyCard
      card="force"
      ipc={ipc}
      state={state}
      onState={onState}
      title="週期"
      description="每個週期結束時，沒回答的題會強制跳到你面前；週期最後幾分鐘，瀏海開始倒數。"
      help="只影響之後新開始的週期；正在跑的週期照原本時間。"
    >
      {({ draft, set, error }) => {
        const cycle = Number(draft.cycle_min)
        const cycleForRange = Number.isInteger(cycle) && cycle >= 5 && cycle <= 120 ? cycle : state.effective.force.cycle_min
        return (
          <div className="k-field-grid">
            <NumberField
              id="settings-cycle"
              label="週期長度"
              unit="分鐘"
              value={String(draft.cycle_min)}
              onChange={(v) => set('cycle_min', v)}
              hint={describeDefault(`${d.cycle_min} 分鐘`, '5–120 分鐘')}
              error={error('cycle_min')}
            />
            <NumberField
              id="settings-warn"
              label="提前警告"
              unit="分鐘"
              value={String(draft.warn_before_min)}
              onChange={(v) => set('warn_before_min', v)}
              hint={describeDefault(`${d.warn_before_min} 分鐘`, warnRange(cycleForRange))}
              error={error('warn_before_min')}
            />
          </div>
        )
      }}
    </PolicyCard>
  )
}

function ExpirySection({ ipc, state, onState }: { ipc: KunsuIpc; state: SettingsState; onState: (s: SettingsState) => void }) {
  const d = state.defaults
  const r = (key: keyof typeof INT_RANGES) => INT_RANGES[key]!
  return (
    <>
      <PolicyCard
        card="expiry"
        ipc={ipc}
        state={state}
        onState={onState}
        title="到期"
        description="一般題沒回答多久後依預設處理。題目最長等待天數管所有題目，最多 14 天；策略題到期由新題取代，其他題到期結案。"
        help="只影響之後新建的題目：已在等的題照建題時的規則到期。錢永遠不會因為逾時自動動。"
      >
        {({ draft, set, error }) => (
          <div className="k-field-grid">
            <NumberField
              id="settings-t1-expiry"
              label="一般題到期時間"
              unit="小時"
              value={String(draft.t1_default_after_h)}
              onChange={(v) => set('t1_default_after_h', v)}
              hint={describeDefault(`${d.expiry.t1_default_after_h} 小時`, `${r('expiry.t1_default_after_h').min}–${r('expiry.t1_default_after_h').max} 小時`)}
              error={error('t1_default_after_h')}
            />
            <NumberField
              id="settings-t2-supersede"
              label="題目最長等待天數"
              unit="天"
              value={String(draft.t2_supersede_after_d)}
              onChange={(v) => set('t2_supersede_after_d', v)}
              hint={describeDefault(`${d.expiry.t2_supersede_after_d} 天`, '1–14 天')}
              error={error('t2_supersede_after_d')}
            />
          </div>
        )}
      </PolicyCard>
      <PolicyCard
        card="quota"
        ipc={ipc}
        state={state}
        onState={onState}
        title="每日建題上限"
        description="每個來源（每支排程、每個 Agent）一天最多能送幾題進來，避免失控的 Agent 洗版。"
        help="超過上限的建題會被拒絕並告訴 Agent 原因。"
      >
        {({ draft, set, error }) => (
          <div className="k-field-grid">
            <NumberField
              id="settings-quota"
              label="每個來源每天"
              unit="題"
              value={String(draft.per_source_daily_create)}
              onChange={(v) => set('per_source_daily_create', v)}
              hint={describeDefault(`${d.quota.per_source_daily_create} 題`, '1–100 題')}
              error={error('per_source_daily_create')}
            />
          </div>
        )}
      </PolicyCard>
      <PolicyCard
        card="fatigue"
        ipc={ipc}
        state={state}
        onState={onState}
        title="疲勞降級"
        description="某個來源的題你一直略過時，降低它的優先級，把注意力留給真正重要的題。"
        help="只調整之後的判斷；不會取消或回答任何題。"
      >
        {({ draft, set, error }) => (
          <div className="k-field-grid">
            <NumberField
              id="settings-fatigue-window"
              label="觀察天數"
              unit="天"
              value={String(draft.skip_rate_window_d)}
              onChange={(v) => set('skip_rate_window_d', v)}
              hint={describeDefault(`${d.fatigue.skip_rate_window_d} 天`, '1–30 天')}
              error={error('skip_rate_window_d')}
            />
            <NumberField
              id="settings-fatigue-rate"
              label="略過率超過"
              unit="%"
              value={String(draft.skip_rate_percent)}
              onChange={(v) => set('skip_rate_percent', v)}
              hint={describeDefault(`${percentFromRate(d.fatigue.skip_rate_downgrade)}%`, '10%–100%')}
              error={error('skip_rate_percent')}
            />
          </div>
        )}
      </PolicyCard>
    </>
  )
}

function NotchCard({ ipc }: { ipc: KunsuIpc }) {
  const toast = useToast()
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // 按鈕被換掉時焦點移到新按鈕（AC-ST-15）：「回到預設位置」→「確定移回」；確定或取消後回到「回到預設位置」。
  const startRef = useRef<HTMLButtonElement>(null)
  const confirmRef = useRef<HTMLButtonElement>(null)
  const moved = useRef(false)
  useEffect(() => {
    if (!moved.current) return
    moved.current = false
    ;(confirming ? confirmRef : startRef).current?.focus()
  }, [confirming])
  const switchTo = (next: boolean) => {
    moved.current = true
    setConfirming(next)
  }
  const reset = async () => {
    setBusy(true)
    setError(null)
    try {
      await ipc.edgeResetPosition()
      switchTo(false)
      toast('瀏海已回到預設位置')
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setBusy(false)
    }
  }
  return (
    <SettingsCard
      title="位置"
      description="把瀏海拖到別處之後，可以讓它回到主螢幕左側、選單列下方的預設位置。沒有待決題時，下次出現就在預設位置。"
      footer={
        <>
          <p className="k-card__help">
            {error ? (
              <span role="alert" className="k-card__help-error">
                {`沒有移成功：${error}`}
              </span>
            ) : confirming ? (
              '確定要把瀏海移回預設位置嗎？保存的位置會被刪除。'
            ) : (
              '位置記在這台 Mac，跟著螢幕走。'
            )}
          </p>
          <div className="k-card__actions">
            {confirming ? (
              <>
                <Button variant="ghost" onClick={busy ? undefined : () => switchTo(false)} aria-disabled={busy || undefined}>
                  取消
                </Button>
                <Button variant="primary" ref={confirmRef} onClick={busy ? undefined : () => void reset()} aria-disabled={busy || undefined}>
                  {busy ? '移動中…' : '確定移回'}
                </Button>
              </>
            ) : (
              <Button variant="secondary" icon="map-pin" ref={startRef} onClick={() => switchTo(true)}>
                回到預設位置
              </Button>
            )}
          </div>
        </>
      }
    >
      <div className="k-notch-preview" aria-hidden="true">
        <span className="k-notch-preview__screen">
          <span className="k-notch-preview__menubar" />
          <span className="k-notch-preview__band" />
        </span>
        <span className="k-notch-preview__caption">預設：主螢幕左側，選單列下方 8 px</span>
      </div>
    </SettingsCard>
  )
}

function DataSection({ ipc, status }: { ipc: KunsuIpc; status: AppStatus | null }) {
  const [error, setError] = useState<string | null>(null)
  if (!status) return <p className="k-loading">正在讀取…</p>
  const health = ledgerHealth(status.ledger)
  const open = async () => {
    setError(null)
    try {
      await ipc.openPrivacySettings()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    }
  }
  return (
    <>
      <Card className="k-card--pad">
        <h3 className="k-card__title">資料位置</h3>
        <dl className="k-kv">
          <div>
            <dt>決策帳本（vault）</dt>
            <dd className="k-mono">{status.vault_root}</dd>
          </div>
          <div>
            <dt>資料庫</dt>
            <dd className="k-mono">{status.db_path}</dd>
          </div>
        </dl>
      </Card>
      <SettingsCard
        title="帳本健康"
        description="每個回答都先安全存進資料庫，再追加到 vault 的決策帳本與 OPEN.json。"
        footer={
          <>
            <p className="k-card__help">{health.problem ? '權限打開後不用做別的事，Kunsu 會自動補寫。' : '每 5 秒更新一次。'}</p>
            <div className="k-card__actions">
              {health.problem && (
                <Button variant="primary" icon="external" onClick={() => void open()}>
                  打開完全取用磁碟設定
                </Button>
              )}
            </div>
          </>
        }
      >
        <div className="k-health">
          <Pill spec={health} />
          {health.explain && <p className="k-health__explain">{health.explain}</p>}
          {health.restart && (
            <p className="k-health__restart" role="alert">
              {health.restart}
            </p>
          )}
          {error && (
            <p role="alert" className="k-field__error">
              {`打不開系統設定：${error}`}
            </p>
          )}
        </div>
      </SettingsCard>
    </>
  )
}

function AboutSection({ status, onOnboarding }: { status: AppStatus | null; onOnboarding: () => void }) {
  return (
    <SettingsCard
      title="版本與服務"
      footer={
        <>
          <p className="k-card__help">導覽會重新檢查 Agent 連線，不會改任何設定。</p>
          <div className="k-card__actions">
            <Button variant="secondary" icon="compass" onClick={onOnboarding}>
              重新執行導覽
            </Button>
          </div>
        </>
      }
    >
      <div className="k-about">
        <img className="k-about__icon" src={BRAND_ICON} alt="" width={48} height={48} draggable={false} />
        <div>
          <div className="k-about__name">Kunsu 軍師</div>
          <div className="k-about__tag">需要你決定時的強制收件匣</div>
        </div>
      </div>
      <dl className="k-kv">
        <div>
          <dt>版本</dt>
          <dd className="tabular">{status?.version ?? '—'}</dd>
        </div>
        <div>
          <dt>運作時間</dt>
          <dd>{status ? formatUptime(status.uptime_s) : '—'}</dd>
        </div>
        <div>
          <dt>本機服務位址</dt>
          <dd className="k-mono">{status?.server_addr ?? '—'}</dd>
        </div>
      </dl>
    </SettingsCard>
  )
}

// ---------------------------------------------------------------- 頁面

export function SettingsPage({
  ipc,
  dnd,
  onDnd,
  onOnboarding,
  initialSection = 'general',
  initialSettings = null,
  initialStatus = null,
}: {
  ipc: KunsuIpc
  dnd: DndStatus
  onDnd: (status: DndStatus) => void
  onOnboarding: () => void
  initialSection?: SettingsSectionId
  initialSettings?: SettingsState | null
  initialStatus?: AppStatus | null
}) {
  const toast = useToast()
  const [section, setSection] = useState<SettingsSectionId>(initialSection)
  const [settings, setSettings] = useState<SettingsState | null>(initialSettings)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [resettingAll, setResettingAll] = useState(false)
  const app = useAppStatus(ipc, { initial: initialStatus, intervalMs: section === 'data' ? 5000 : undefined })
  // 「全部回復預設」成功後錯誤橫幅（連同按鈕）消失：焦點移到區段標題，不掉到頁面（AC-ST-15）。
  const headingRef = useRef<HTMLHeadingElement>(null)
  const focusHeading = useRef(false)
  useEffect(() => {
    if (!focusHeading.current) return
    focusHeading.current = false
    headingRef.current?.focus()
  })

  const load = useMemo(
    () => async () => {
      try {
        setSettings(await ipc.settingsGet())
        setLoadError(null)
      } catch (cause) {
        setLoadError(cause instanceof Error ? cause.message : String(cause))
      }
    },
    [ipc],
  )
  useEffect(() => {
    void load()
  }, [load])

  const resetAll = async () => {
    setResettingAll(true)
    try {
      const next = await ipc.settingsReset([])
      focusHeading.current = true
      setSettings(next)
      toast('已全部回復預設，立即生效')
    } catch (cause) {
      setLoadError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setResettingAll(false)
    }
  }

  // 窄面板的橫向小標列：選中的那顆捲進看得到的地方。
  const navRef = useRef<HTMLElement>(null)
  useEffect(() => {
    navRef.current?.querySelector('[aria-current="page"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
  }, [section])

  const meta = SETTINGS_SECTIONS.find((s) => s.id === section) ?? SETTINGS_SECTIONS[0]
  const policyReady = settings ?? null
  const policySection = section === 'sleep' || section === 'force' || section === 'expiry'

  return (
    <>
      <PageHeader title="設定" width="settings" description="改了立即生效，重新開啟或重新安裝都還在。設定永遠不會回答、取消或重設任何題目。" />
      <PageBody width="settings">
        {settings?.local_error && (
          <div className="k-banner" data-tone="danger" role="alert">
            <Icon name="alert" size={16} />
            <div className="k-banner__text">
              <strong>個人設定檔有錯，目前全部用預設值。</strong>
              {/* 用設定頁上的名稱指出哪一項（不露 force.cycle_min 這種鍵名）；原文留在 title 給除錯（AC-ST-13）。 */}
              <span title={settings.local_error}>{humanizeSettingsError(settings.local_error)}</span>
              <span className="k-mono">{settings.local_path}</span>
            </div>
            <Button variant="secondary" size="sm" disabled={resettingAll} onClick={() => void resetAll()}>
              全部回復預設
            </Button>
          </div>
        )}
        {loadError && (
          <div className="k-banner" data-tone="danger" role="alert">
            <Icon name="alert" size={16} />
            <div className="k-banner__text">
              <strong>讀不到設定</strong>
              <span>{loadError}</span>
            </div>
            <Button variant="secondary" size="sm" onClick={() => void load()}>
              重試
            </Button>
          </div>
        )}
        <div className="k-settings">
          <nav className="k-settings__nav" aria-label="設定區段" ref={navRef}>
            {SETTINGS_SECTIONS.map((s) => (
              <button
                key={s.id}
                type="button"
                className="k-settings__nav-item"
                aria-current={s.id === section ? 'page' : undefined}
                onClick={() => setSection(s.id)}
              >
                <Icon name={s.icon} size={16} />
                <span>{s.label}</span>
              </button>
            ))}
          </nav>
          <section className="k-settings__content" aria-labelledby="settings-section-title">
            <div className="k-settings__heading">
              <h2 id="settings-section-title" ref={headingRef} tabIndex={-1}>
                {meta.label}
              </h2>
              <p>{meta.description}</p>
            </div>
            {section === 'general' && <GeneralSection />}
            {section === 'account' && <AccountSection ipc={ipc} />}
            {section === 'mobile' && <MobileSection ipc={ipc} />}
            {policySection && !policyReady && !loadError && <p className="k-loading">正在讀取設定…</p>}
            {section === 'sleep' && policyReady && <SleepCard ipc={ipc} state={policyReady} onState={setSettings} />}
            {section === 'force' && policyReady && <ForceCard ipc={ipc} state={policyReady} onState={setSettings} />}
            {section === 'expiry' && policyReady && <ExpirySection ipc={ipc} state={policyReady} onState={setSettings} />}
            {section === 'dnd' && (
              <SettingsCard title="暫停提醒" description="期間不顯示系統通知、瀏海或強制詢問；題目與倒數都會保留，可以隨時提前結束。" footer={<p className="k-card__help">頁首右上角的「勿擾模式」是同一個開關。</p>}>
                <DndOptions ipc={ipc} status={dnd} onStatus={onDnd} idPrefix="settings-dnd" statusLine />
              </SettingsCard>
            )}
            {section === 'notch' && <NotchCard ipc={ipc} />}
            {section === 'agents' && (
              <IntegrationList status={app.status} error={app.error} loading={app.loading} checkedAt={app.checkedAt} onRefresh={() => void app.refresh()} />
            )}
            {section === 'data' && <DataSection ipc={ipc} status={app.status} />}
            {section === 'about' && <AboutSection status={app.status} onOnboarding={onOnboarding} />}
          </section>
        </div>
      </PageBody>
    </>
  )
}
