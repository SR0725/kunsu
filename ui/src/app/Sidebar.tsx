// 側欄（DESIGN「Sidebar」；BDD application-shell 場景 3）：直接放在視窗底色上；頂端 44 px 是紅綠燈的拖曳區，下面是品牌列、
// 分組導覽（收件匣、管理）、底部狀態列與設定。
import type { IconKey } from './status'
import { Icon } from './ui'
import { BRAND_ICON } from './brand'

export type AppSection = 'inbox' | 'history' | 'routines' | 'agents' | 'settings'

interface NavEntry {
  id: AppSection
  label: string
  icon: IconKey
}

const NAV_GROUPS: { label: string; items: NavEntry[] }[] = [
  {
    label: '收件匣',
    items: [
      { id: 'inbox', label: '待決事項', icon: 'inbox' },
      { id: 'history', label: '歷史紀錄', icon: 'archive' },
    ],
  },
  {
    label: '管理',
    items: [
      { id: 'routines', label: '督導', icon: 'routine' },
      { id: 'agents', label: 'Agents', icon: 'agents' },
    ],
  },
]

function NavItem({ entry, active, count, onSelect }: { entry: NavEntry; active: boolean; count?: number; onSelect: () => void }) {
  return (
    <button type="button" className="k-nav-item" aria-current={active ? 'page' : undefined} onClick={onSelect}>
      <Icon name={entry.icon} size={16} />
      <span className="k-nav-item__label">{entry.label}</span>
      {count !== undefined && count > 0 && (
        <span className="k-badge">{count}</span>
      )}
    </button>
  )
}

export function Sidebar({ section, onSection, pending }: { section: AppSection; onSection: (section: AppSection) => void; pending: number }) {
  return (
    <aside className="k-sidebar">
      <div className="k-sidebar__drag" data-tauri-drag-region="" />
      <div className="k-brand" data-tauri-drag-region="">
        <img className="k-brand__icon" src={BRAND_ICON} alt="" width={24} height={24} draggable={false} />
        <span className="k-brand__name">Kunsu</span>
      </div>
      <nav className="k-nav" aria-label="主要導覽">
        {NAV_GROUPS.map((group) => (
          <div key={group.label} className="k-nav__group" role="group" aria-labelledby={`k-nav-${group.label}`}>
            <div id={`k-nav-${group.label}`} className="k-nav__label">
              {group.label}
            </div>
            {group.items.map((entry) => (
              <NavItem
                key={entry.id}
                entry={entry}
                active={section === entry.id}
                count={entry.id === 'inbox' ? pending : undefined}
                onSelect={() => onSection(entry.id)}
              />
            ))}
          </div>
        ))}
      </nav>
      <div className="k-sidebar__foot">
        <div className="k-presence" data-waiting={pending > 0 ? 'true' : 'false'}>
          <span className="k-presence__dot" aria-hidden="true" />
          <span>{pending > 0 ? `${pending} 題正在等你` : '目前沒有待決'}</span>
        </div>
        <NavItem entry={{ id: 'settings', label: '設定', icon: 'settings' }} active={section === 'settings'} onSelect={() => onSection('settings')} />
      </div>
    </aside>
  )
}
