// Station 主視窗示範（AC-WEB-03）：側欄切換待決事項、歷史紀錄、督導、Agents、設定。全部假資料。
import { useState } from 'react'
import { Archive, Bot, Inbox, Repeat2, Settings } from 'lucide-react'

export const SECTIONS = [
  { id: 'inbox', label: '待決事項', icon: Inbox, badge: 3 },
  { id: 'history', label: '歷史紀錄', icon: Archive },
  { id: 'routines', label: '督導', icon: Repeat2 },
  { id: 'agents', label: 'Agents', icon: Bot },
  { id: 'settings', label: '設定', icon: Settings },
] as const

type SectionId = (typeof SECTIONS)[number]['id']

function Inboxes() {
  return (
    <>
      <div className="stats">
        <div><b>3</b><span>待決</span></div>
        <div><b>1</b><span>現在需要你</span></div>
        <div><b>2</b><span>可以一次答完</span></div>
      </div>
      {[
        ['把早鳥廣告預算從 500 調到 600？', 'ads · Claude', '現在需要你', 'red', '1 天'],
        ['首屏標題要用哪一版？', 'landing-page · Claude', '今天', 'amber', '2 小時'],
        ['要把舊的 v1 端點一起拿掉嗎？', 'api · Codex', '策略', 'blue', '3 天'],
      ].map(([t, s, p, tone, age]) => (
        <div className="row" key={t}>
          <div>{t}<small>{s}</small></div>
          <span className="src">{age}</span>
          <span className={`chip chip--${tone}`}>{p}</span>
        </div>
      ))}
    </>
  )
}

function History() {
  return (
    <>
      {[
        ['測試全綠了，要我直接 commit 嗎？', '已回答：commit', '23:12'],
        ['這週決策紀錄放進 W41？', '已回答：放進 W41', '22:40'],
        ['landing page hero 換新文案？', '已回答：換', '21:05'],
        ['要不要重跑昨天失敗的部署？', '退回：這題不該問我', '18:30'],
      ].map(([t, a, time]) => (
        <div className="row" key={t}>
          <div>{t}<small>{a}</small></div>
          <span className="src">{time}</span>
          <span className="chip chip--ok">已處理</span>
        </div>
      ))}
    </>
  )
}

function Routines() {
  return (
    <>
      {[
        ['每日日記', '每天 22:30 · Claude', '今天已完成'],
        ['週復盤', '每週日 20:00 · Codex', '3 天後'],
        ['廣告健檢', '每天 09:00 · Claude', '已完成'],
      ].map(([t, s, st]) => (
        <div className="row" key={t}>
          <div>{t}<small>{s}</small></div>
          <span />
          <span className="chip chip--blue">{st}</span>
        </div>
      ))}
    </>
  )
}

function Agents() {
  return (
    <>
      {[
        ['Claude Code', '外掛已安裝 · MCP 已連線'],
        ['Codex', '外掛已安裝 · MCP 已連線'],
        ['手機', 'iPhone 已配對 · 端對端加密'],
      ].map(([t, s]) => (
        <div className="row" key={t}>
          <div>{t}<small>{s}</small></div>
          <span />
          <span className="chip chip--ok">已連線</span>
        </div>
      ))}
    </>
  )
}

function SettingsPane() {
  return (
    <>
      {[
        ['強制詢問', '15 分鐘內把問題帶到你面前；剩 3 分鐘先提醒'],
        ['勿擾', '最長 24 小時；題目照常保存，時間到就恢復'],
        ['睡眠時間', '23:00–09:00 不打擾'],
        ['分享螢幕時', '自動延後，不會讓你出糗'],
      ].map(([t, s]) => (
        <div className="row" key={t}>
          <div>{t}<small>{s}</small></div>
          <span />
          <span className="chip chip--ok">開啟</span>
        </div>
      ))}
    </>
  )
}

const BODY: Record<SectionId, () => React.ReactElement> = { inbox: Inboxes, history: History, routines: Routines, agents: Agents, settings: SettingsPane }

export function StationDemo() {
  const [section, setSection] = useState<SectionId>('inbox')
  const Body = BODY[section]
  const current = SECTIONS.find((s) => s.id === section)!
  return (
    <div className="station" style={{ position: 'relative' }}>
      <div className="station__dots" aria-hidden="true">
        <span style={{ background: '#ff5f57' }} />
        <span style={{ background: '#febc2e' }} />
        <span style={{ background: '#28c840' }} />
      </div>
      <nav className="station__side" aria-label="Station 示範的側欄">
        <div className="station__brand"><img src="/kunsu-mark.png" alt="" />Kunsu</div>
        <div className="station__group">收件匣</div>
        {SECTIONS.map((s, i) => (
          <span key={s.id} style={{ display: 'contents' }}>
            {i === 2 && <div className="station__group">管理</div>}
            <button className="station__item" aria-current={section === s.id ? 'page' : undefined} onClick={() => setSection(s.id)}>
              <s.icon size={16} aria-hidden="true" />
              {s.label}
              {'badge' in s && <span className="badge">{s.badge}</span>}
            </button>
          </span>
        ))}
        <div className="station__foot"><i />3 題正在等你</div>
      </nav>
      <section className="station__main" aria-live="polite">
        <div className="station__head">{current.label}</div>
        <div className="station__body" key={section}><Body /></div>
      </section>
    </div>
  )
}
