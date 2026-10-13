// 主視窗各頁（BDD application-shell AC-AS-03～06；settings AC-ST-04／08／10～13）：用靜態渲染檢查結構、文字與語意。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { AppStatus, Question, SettingsState } from '../types'
import { INACTIVE_DND } from '../dnd'
import { Sidebar } from './Sidebar'
import { InboxPage } from './pages/InboxPage'
import { HistoryPage } from './pages/HistoryPage'
import { AgentsPage } from './pages/AgentsPage'
import { SettingsPage } from './pages/SettingsPage'
import { SETTINGS_SECTIONS } from './settings/sections'
import { Onboarding } from './Onboarding'
import { POLICY_DEFAULTS } from './settings/schema'

const ipc = { isMock: true } as unknown as KunsuIpc
const NOW = new Date('2026-10-09T04:00:00Z')

function q(id: string, partial: Partial<Question> = {}): Question {
  return {
    id,
    title: id,
    question_line: `題目 ${id}`,
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    product: 'Demo 課',
    evidence: [],
    options: [{ id: 'a', label: '照做' }],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'none' },
    source: { agent: 'claude', cwd: '/w' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-06',
    last_seen: '2026-10-08T10:00:00Z',
    created_at: '2026-10-08T09:00:00Z',
    next_transition_at: '2026-10-08T10:00:00Z',
    ...partial,
  } as Question
}

const STATUS: AppStatus = {
  version: '0.1.0',
  uptime_s: 3 * 3600 + 12 * 60,
  server_addr: '127.0.0.1:4777',
  vault_root: '/Users/you/Kunsu',
  db_path: '/Users/you/.kunsu/kunsu.db',
  ledger: { backlog: 0, open_json_dirty: false, stuck_s: 0 },
  integrations: [
    { id: 'claude_mcp', status: 'ok', detail: '~/.claude.json 已註冊 kunsu' },
    { id: 'codex_mcp', status: 'missing', detail: '~/.codex/config.toml 沒有 [mcp_servers.kunsu]', fix: 'codex mcp add kunsu -- ~/.kunsu/bin/kunsu mcp' },
    { id: 'cli', status: 'ok', detail: '~/.kunsu/bin/kunsu 可執行' },
    { id: 'skill', status: 'unknown', detail: '讀不到 ~/.claude/skills' },
  ],
}

const SETTINGS: SettingsState = {
  effective: { ...POLICY_DEFAULTS, force: { cycle_min: 20, warn_before_min: 3 } },
  defaults: POLICY_DEFAULTS,
  overridden: ['force.cycle_min'],
  local_path: '/Users/you/.kunsu/policy.local.toml',
}

describe('側欄（AC-AS-03）', () => {
  const out = renderToStaticMarkup(<Sidebar section="inbox" onSection={() => {}} pending={3} />)
  it('頂端 44 px 拖曳區、Kunsu 圖示與名稱', () => {
    expect(out).toMatch(/class="k-sidebar__drag"[^>]*data-tauri-drag-region/)
    expect(out).toMatch(/<img[^>]*class="k-brand__icon"/)
    expect(out).toContain('Kunsu')
  })
  it('分組：收件匣（待決事項＋計數、歷史紀錄）、管理（督導、Agents）；選中 aria-current', () => {
    const inbox = out.indexOf('收件匣')
    const manage = out.indexOf('管理')
    expect(inbox).toBeGreaterThan(-1)
    expect(manage).toBeGreaterThan(inbox)
    for (const label of ['待決事項', '歷史紀錄']) expect(out.indexOf(label)).toBeGreaterThan(inbox)
    for (const label of ['督導', 'Agents']) expect(out.indexOf(label)).toBeGreaterThan(manage)
    expect(out).not.toContain('董事會') // AC-AS-16：2026-10-12 拿掉
    expect(out).toMatch(/aria-current="page"[^>]*>.*待決事項.*<span class="k-badge"[^>]*>3<\/span>/)
  })
  it('底部狀態列與設定', () => {
    expect(out).toContain('3 題正在等你')
    expect(out.indexOf('3 題正在等你')).toBeLessThan(out.lastIndexOf('設定'))
    expect(renderToStaticMarkup(<Sidebar section="settings" onSection={() => {}} pending={0} />)).toContain('目前沒有待決')
  })
})

describe('待決事項（AC-AS-04）', () => {
  const questions = [q('urgent', { tier: 't0', source: { agent: 'codex', cwd: '/w' } }), q('today'), q('board', { tier: 't2' })]
  const out = renderToStaticMarkup(<InboxPage questions={questions} onOpenAt={() => {}} onStartFlow={() => {}} dnd={INACTIVE_DND} now={NOW} />)
  it('頁首與黑色主按鈕', () => {
    expect(out).toMatch(/<h1[^>]*>待決事項<\/h1>/)
    expect(out).toMatch(/data-variant="primary"[^>]*data-size="lg"[^>]*>.*開始處理 3 題/)
  })
  it('三格統計', () => {
    expect(out).toMatch(/<dt[^>]*>待決<\/dt><dd[^>]*>3<\/dd>/)
    expect(out).toMatch(/<dt[^>]*>現在需要你<\/dt><dd[^>]*>1<\/dd>/)
    expect(out).toMatch(/<dt[^>]*>可批次回答<\/dt><dd[^>]*>2<\/dd>/)
  })
  it('表格列：題目＋產品、來源圖示與名稱、積壓天數、優先小標', () => {
    expect(out.match(/class="k-row"/g)).toHaveLength(3)
    expect(out).toContain('Demo 課')
    expect(out).toMatch(/aria-label="Codex"/)
    expect(out).toMatch(/<span class="k-cell__prefix">積壓 <\/span>3 天/)
    expect(out).toMatch(/data-tone="danger"[^>]*>.*現在需要你/)
    expect(out).toMatch(/data-tone="waiting"[^>]*>.*今天/)
  })
  it('沒有待決：置中空狀態，沒有主按鈕', () => {
    const empty = renderToStaticMarkup(<InboxPage questions={[]} onOpenAt={() => {}} onStartFlow={() => {}} dnd={INACTIVE_DND} now={NOW} />)
    expect(empty).toContain('Agent 都能繼續工作')
    expect(empty).toContain('k-empty__tile')
    expect(empty).not.toContain('開始處理')
  })
  it('勿擾中：說明提醒暫停、題目保留', () => {
    const dnd = { active: true, starts_at: NOW.toISOString(), ends_at: new Date(NOW.getTime() + 3_600_000).toISOString(), remaining_seconds: 3600, actor: 'ui' }
    const out2 = renderToStaticMarkup(<InboxPage questions={questions} onOpenAt={() => {}} onStartFlow={() => {}} dnd={dnd} now={NOW} />)
    expect(out2).toMatch(/role="status"[^>]*>.*勿擾至/)
    expect(out2).toContain('仍安全保留')
  })
})

describe('歷史紀錄（AC-AS-05）', () => {
  const done = [
    q('ans', { state: 'answered', answer: { choice_id: 'a', channel: 'ui', answered_at: '2026-10-08T11:00:00Z' } }),
    q('exp', { state: 'expired' }),
    q('rej', { state: 'cancelled', rejected: { at: '2026-10-08T12:00:00Z', reason: '該問同事' } }),
  ]
  it('篩選小標與數量、結果小標、結案時間', () => {
    const out = renderToStaticMarkup(<HistoryPage questions={done} onOpenAt={() => {}} />)
    expect(out).toMatch(/aria-pressed="true"[^>]*>全部<span[^>]*>3<\/span>/)
    expect(out).toMatch(/>未回應<span[^>]*>1<\/span>/)
    expect(out).toMatch(/data-tone="success"[^>]*>.*已回答/)
    expect(out).toMatch(/data-tone="danger"[^>]*>.*退回/)
    expect(out).toMatch(/<time[^>]*dateTime="2026-10-08T11:00:00Z"/i)
  })
  it('列表本身沒有任何作答控制；點開交給決策面板唯讀回看（AC-DS-17，由 surface 測試涵蓋）', () => {
    const out = renderToStaticMarkup(<HistoryPage questions={done} onOpenAt={() => {}} />)
    expect(out.match(/<li[^>]*><button type="button" class="k-row"/g)).toHaveLength(3)
    expect(out).not.toMatch(/<textarea|<input|role="radio"|送出|這題不該問我/)
  })
  it('沒有歷史：空狀態', () => {
    expect(renderToStaticMarkup(<HistoryPage questions={[]} onOpenAt={() => {}} />)).toContain('k-empty__tile')
  })
})

describe('Agents（AC-AS-06、AC-ST-10）', () => {
  const out = renderToStaticMarkup(<AgentsPage ipc={ipc} onOnboarding={() => {}} initialStatus={STATUS} />)
  it('逐項真實狀態，未設定附可複製指令', () => {
    expect(out).toContain('Claude MCP')
    expect(out).toContain('Codex MCP')
    expect(out).toMatch(/data-tone="success"[^>]*>.*已設定/)
    expect(out).toMatch(/data-tone="waiting"[^>]*>.*未設定/)
    expect(out).toMatch(/data-tone="neutral"[^>]*>.*無法確認/)
    expect(out).toContain('codex mcp add kunsu -- ~/.kunsu/bin/kunsu mcp')
    expect(out).toMatch(/aria-label="複製[^"]*"/)
  })
  it('沒有假的連接／中斷／一鍵安裝', () => {
    expect(out).not.toMatch(/>連接<|>中斷<|一鍵安裝|已連接/)
  })
})

describe('Onboarding：誠實的真實狀態＋複製指令', () => {
  const out = renderToStaticMarkup(<Onboarding ipc={ipc} onComplete={() => {}} initialStatus={STATUS} />)
  it('對話框、真實狀態、指令', () => {
    expect(out).toMatch(/role="dialog"[^>]*aria-modal="true"/)
    expect(out).toContain('codex mcp add kunsu')
    expect(out).toContain('重新檢查')
    expect(out).not.toMatch(/一鍵安裝|>連接<|已連接/)
  })
})

describe('設定（BDD settings）', () => {
  it('區段導覽：一般、帳號、手機、睡眠時間、強制詢問、到期與配額、勿擾、瀏海、Agent 連線、資料與帳本、關於', () => {
    expect(SETTINGS_SECTIONS.map((s) => s.label)).toEqual(['一般', '帳號', '手機', '睡眠時間', '強制詢問', '到期與配額', '勿擾', '瀏海', 'Agent 連線', '資料與帳本', '關於'])
    const out = renderToStaticMarkup(
      <SettingsPage ipc={ipc} dnd={INACTIVE_DND} onDnd={() => {}} onOnboarding={() => {}} initialSettings={SETTINGS} initialStatus={STATUS} />,
    )
    expect(out).toMatch(/<nav[^>]*aria-label="設定區段"/)
    for (const s of SETTINGS_SECTIONS) expect(out).toContain(s.label)
  })

  const render = (section: string, settings: SettingsState = SETTINGS, status: AppStatus = STATUS) =>
    renderToStaticMarkup(
      <SettingsPage ipc={ipc} dnd={INACTIVE_DND} onDnd={() => {}} onOnboarding={() => {}} initialSection={section as never} initialSettings={settings} initialStatus={status} />,
    )

  it('一般：外觀三選一（分段控制）與連續轉場動畫開關', () => {
    const out = render('general')
    expect(out).toMatch(/role="radiogroup"[^>]*aria-label="外觀"/)
    for (const label of ['跟隨系統', '淺色', '深色']) expect(out).toContain(label)
    expect(out).toMatch(/role="switch"/)
    expect(out).toContain('連續轉場動畫')
  })

  it('睡眠時間：開關、24 小時制時間、跨午夜說明', () => {
    const out = render('sleep')
    expect(out).toContain('每天 23:00 到隔天 09:00 不會強制詢問')
    expect(out).toMatch(/role="switch"[^>]*aria-checked="true"/)
    expect(out.match(/placeholder="HH:MM"/g)).toHaveLength(2)
  })

  it('強制詢問：目前值、單位、範圍、已改過、回復預設；沒改就不能儲存', () => {
    const out = render('force')
    expect(out).toMatch(/value="20"/)
    expect(out).toContain('分鐘')
    expect(out).toContain('5–120')
    expect(out).toContain('已改過')
    expect(out).toMatch(/>回復預設<\/button>/)
    expect(out).toMatch(/disabled=""[^>]*>儲存<\/button>/)
  })

  it('到期與配額：一般題到期、題目最長等待天數（所有題目，最多 14 天）、每日上限、疲勞降級', () => {
    const out = render('expiry')
    for (const text of ['小時', '天', '題', '%', '1–336', '1–14', '1–100', '1–30', '10%–100%', '題目最長等待天數', '所有題目，最多 14 天']) expect(out).toContain(text)
  })

  it('勿擾：快捷時長與自訂時長', () => {
    const out = render('dnd')
    for (const text of ['1 小時', '4 小時', '8 小時', '24 小時']) expect(out).toContain(text)
    expect(out).toContain('自訂')
  })

  it('瀏海：回到預設位置', () => {
    expect(render('notch')).toContain('回到預設位置')
  })

  it('Agent 連線：與 Agents 頁同一份真實狀態', () => {
    const out = render('agents')
    expect(out).toContain('codex mcp add kunsu')
    expect(out).not.toMatch(/>連接<|>中斷</)
  })

  it('資料與帳本：位置、正常狀態；有問題時人話說明與打開完全取用磁碟設定', () => {
    const ok = render('data')
    expect(ok).toContain('/Users/you/Kunsu')
    expect(ok).toContain('/Users/you/.kunsu/kunsu.db')
    expect(ok).toMatch(/data-tone="success"[^>]*>.*正常/)
    expect(ok).not.toContain('打開完全取用磁碟設定')
    const stuck = render('data', SETTINGS, { ...STATUS, ledger: { backlog: 4, open_json_dirty: true, stuck_s: 180 } })
    expect(stuck).toContain('答案都安全在資料庫')
    expect(stuck).toContain('打開完全取用磁碟設定')
    expect(stuck).toContain('重新啟動 Kunsu')
  })

  it('關於：版本、運作時間、服務位址、重新執行導覽', () => {
    const out = render('about')
    for (const text of ['0.1.0', '3 小時 12 分', '127.0.0.1:4777', '重新執行導覽']) expect(out).toContain(text)
  })

  it('local 檔壞掉：頂端錯誤、檔案位置、全部回復預設', () => {
    const out = render('force', { ...SETTINGS, overridden: [], effective: POLICY_DEFAULTS, local_error: '第 3 行：force.cycle_min 要在 5–120 之間' })
    expect(out).toMatch(/role="alert"[^>]*>.*第 3 行/)
    expect(out).toContain('/Users/you/.kunsu/policy.local.toml')
    expect(out).toContain('全部回復預設')
  })
})
