// 版面元件（DESIGN「Page header」「Cards」「Stats」「Tables and rows」「Tabs and filters」「Empty states」「Code and
// commands」「Toasts」）。
import { createContext, useContext, useEffect, useId, useRef, useState, type CSSProperties, type ReactNode, type Ref } from 'react'
import { ShellToolbarContext } from './context'
import type { IconKey, Tone } from '../status'
import { Button } from './Button'
import { CopiedIcon, Icon } from './Icon'

// ---------------------------------------------------------------- 頁首

export type PageWidth = 'wide' | 'narrow' | 'settings'

/**
 * 頁首：固定 56 px 一行（返回、標題、右側動作）；內層與內容同一個最大寬度置中（DESIGN「Page header」，2026-10-09）。
 * 說明不放在頁首：畫在頁首下方、內容區的第一行（`.k-page-intro`），頁首不會因為說明長高，窄版也不會擠出孤字。
 */
export function PageHeader({
  title,
  description,
  actions,
  back,
  titleRef,
  width = 'wide',
}: {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  /** 標題前面的返回（例如「‹ 所有排程」），和標題同一行。 */
  back?: ReactNode
  /** 進入子頁時把焦點移到標題（tabIndex -1）。 */
  titleRef?: Ref<HTMLHeadingElement>
  /** 與這一頁 PageBody 相同的寬度，標題與按鈕才會對齊內容的左右緣。 */
  width?: PageWidth
}) {
  const toolbar = useContext(ShellToolbarContext)
  return (
    <>
      <header className="k-page-header" data-tauri-drag-region="">
        <div className="k-page-header__inner" data-width={width} data-tauri-drag-region="">
          <div className="k-page-header__main" data-tauri-drag-region="">
            {back}
            <h1 ref={titleRef} tabIndex={titleRef ? -1 : undefined} className="k-page-header__title" data-tauri-drag-region="">
              {title}
            </h1>
          </div>
          {(actions || toolbar) && (
            <div className="k-page-header__actions">
              {actions}
              {toolbar}
            </div>
          )}
        </div>
      </header>
      {description && (
        <div className="k-page-intro" data-width={width}>
          <p className="k-page-intro__text">{description}</p>
        </div>
      )}
    </>
  )
}

/** 頁面內容（24 px 留白、置中；內容最寬 1080 px；設定 180＋32＋760 px；督導歷史 760 px）。 */
export function PageBody({ children, width = 'wide' }: { children: ReactNode; width?: PageWidth }) {
  return (
    <div className="k-page-body" data-width={width}>
      {children}
    </div>
  )
}

// ---------------------------------------------------------------- 卡片

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={className ? `k-card ${className}` : 'k-card'}>{children}</div>
}

/**
 * dub 式設定卡片：20 px 內容（標題、一行說明、控制）＋底部儲存列（淡底、細線、左邊說明、右邊儲存）。
 * 儲存在有合法變更前停用；值和預設不同時多一顆「回復預設」。
 * 焦點不掉到頁面（AC-ST-15）：儲存中的按鈕是 aria-disabled（仍可聚焦）；標題可以接焦點（`titleRef`），存好或回復預設後
 * 由呼叫端把焦點移過來。
 */
export function SettingsCard({
  title,
  description,
  modified = false,
  children,
  help,
  canSave = false,
  saving = false,
  onSave,
  showReset = false,
  onReset,
  footer,
  id,
  titleRef,
}: {
  title: string
  description?: ReactNode
  modified?: boolean
  children: ReactNode
  help?: ReactNode
  canSave?: boolean
  saving?: boolean
  onSave?: () => void
  showReset?: boolean
  onReset?: () => void
  /** 自訂底部（例如沒有儲存的即時設定）；false＝沒有底部。 */
  footer?: ReactNode | false
  id?: string
  titleRef?: Ref<HTMLHeadingElement>
}) {
  const titleId = useId()
  const hasDefaultFooter = onSave !== undefined || showReset || help !== undefined
  return (
    <section className="k-card k-setting-card" aria-labelledby={titleId} id={id}>
      <div className="k-card__body">
        <div className="k-card__head">
          <h3 id={titleId} ref={titleRef} tabIndex={-1} className="k-card__title">
            {title}
          </h3>
          {modified && (
            // 中性小標＋小圓點：只是「這張卡片的值在個人設定檔裡」，不用 ✨ 或 AI 意象。
            <span className="k-pill k-pill--dot" data-tone="neutral">
              <span className="k-pill__dot" aria-hidden="true" />
              已改過
            </span>
          )}
        </div>
        {description && <p className="k-card__desc">{description}</p>}
        <div className="k-card__content">{children}</div>
      </div>
      {footer !== false && (footer !== undefined || hasDefaultFooter) && (
        <div className="k-card__footer">
          {footer ?? (
            <>
              <p className="k-card__help">{help}</p>
              <div className="k-card__actions">
                {showReset && (
                  <Button variant="ghost" aria-disabled={saving || undefined} onClick={saving ? undefined : onReset}>
                    回復預設
                  </Button>
                )}
                {onSave && (
                  // 儲存中用 aria-disabled（不是 disabled）：按鈕仍可聚焦，焦點不會掉到頁面。沒有合法變更時才真的停用。
                  <Button
                    variant="primary"
                    disabled={!canSave && !saving}
                    aria-disabled={saving || undefined}
                    onClick={saving ? undefined : onSave}
                  >
                    {saving ? '儲存中…' : '儲存'}
                  </Button>
                )}
              </div>
            </>
          )}
        </div>
      )}
    </section>
  )
}

// ---------------------------------------------------------------- 統計

export function StatGroup({ items }: { items: { label: string; value: ReactNode; tone?: Tone }[] }) {
  return (
    <dl className="k-stats">
      {items.map((item) => (
        <div key={item.label} className="k-stat" data-tone={item.tone ?? 'neutral'}>
          <dt className="k-stat__label">{item.label}</dt>
          <dd className="k-stat__value tabular">{item.value}</dd>
        </div>
      ))}
    </dl>
  )
}

// ---------------------------------------------------------------- 表格

export interface Column {
  id: string
  label: string
  /** grid 欄寬，預設 minmax(0, 1fr)。 */
  width?: string
  align?: 'end'
}

const ColumnsContext = createContext<Column[]>([])

/** 有框的 12 px 表格；表頭只給眼睛看，每列是清單裡的按鈕（整列可點、最後有箭頭）。窄面板改成上下堆疊。 */
export function Table({ label, columns, children }: { label: string; columns: Column[]; children: ReactNode }) {
  const template = columns.map((c) => c.width ?? 'minmax(0, 1fr)').join(' ')
  return (
    <ColumnsContext.Provider value={columns}>
      <div className="k-table" style={{ '--k-cols': template } as CSSProperties}>
        <div className="k-table__head" aria-hidden="true">
          {columns.map((c) => (
            <span key={c.id} className="k-cell" data-col={c.id} data-align={c.align}>
              {c.label}
            </span>
          ))}
          <span className="k-row__chevron-space" />
        </div>
        <ul className="k-table__body" aria-label={label}>
          {children}
        </ul>
      </div>
    </ColumnsContext.Provider>
  )
}

export function TableRow({
  cells,
  onClick,
  title,
  rowRef,
  className,
}: {
  cells: Record<string, ReactNode>
  onClick: () => void
  /** 完整標題（截斷時滑鼠停留看得到）。 */
  title?: string
  rowRef?: Ref<HTMLButtonElement>
  className?: string
}) {
  const columns = useContext(ColumnsContext)
  return (
    <li className="k-table__item">
      <button type="button" className={className ? `k-row ${className}` : 'k-row'} onClick={onClick} title={title} ref={rowRef}>
        {columns.map((c) => (
          <span key={c.id} className="k-cell" data-col={c.id} data-align={c.align}>
            {cells[c.id]}
          </span>
        ))}
        <Icon name="chevron" size={16} className="k-row__chevron" />
      </button>
    </li>
  )
}

// ---------------------------------------------------------------- 篩選

export function FilterPills<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: { id: T; label: string; count: number }[]
  onChange: (value: T) => void
}) {
  return (
    <div role="group" aria-label={label} className="k-filters">
      {options.map((o) => (
        <button key={o.id} type="button" className="k-filter" aria-pressed={o.id === value} onClick={() => onChange(o.id)}>
          {o.label}
          <span className="k-filter__count tabular">{o.count}</span>
        </button>
      ))}
    </div>
  )
}

// ---------------------------------------------------------------- 空狀態

export function EmptyState({ icon, title, description, action }: { icon: IconKey; title: string; description?: ReactNode; action?: ReactNode }) {
  return (
    <div className="k-empty">
      <div className="k-empty__tile">
        <Icon name={icon} size={20} />
      </div>
      <h2 className="k-empty__title">{title}</h2>
      {description && <p className="k-empty__desc">{description}</p>}
      {action && <div className="k-empty__action">{action}</div>}
    </div>
  )
}

// ---------------------------------------------------------------- 指令

async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    // 退回 execCommand
  }
  try {
    const area = document.createElement('textarea')
    area.value = text
    area.setAttribute('readonly', '')
    area.style.position = 'fixed'
    area.style.opacity = '0'
    document.body.appendChild(area)
    area.select()
    const ok = document.execCommand('copy')
    area.remove()
    return ok
  } catch {
    return false
  }
}

/** 等寬 12 px、淡底 12 px 圓角；複製後 1.5 秒顯示「已複製」。app 不執行這些指令。 */
export function CodeBlock({ code, label = '指令' }: { code: string; label?: string }) {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(timer.current), [])
  const copy = async () => {
    const ok = await copyText(code)
    setState(ok ? 'copied' : 'failed')
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setState('idle'), 1500)
  }
  return (
    <div className="k-code">
      <pre className="k-code__pre">
        <code>{code}</code>
      </pre>
      <button type="button" className="k-btn k-code__copy" data-variant="ghost" data-size="sm" aria-label={`複製${label}`} onClick={() => void copy()}>
        {state === 'copied' ? <CopiedIcon size={14} aria-hidden="true" className="k-icon" /> : <Icon name="copy" size={14} />}
        {state === 'copied' ? '已複製' : state === 'failed' ? '複製失敗' : '複製'}
      </button>
    </div>
  )
}

// ---------------------------------------------------------------- Toast

/** 面板右下角，2.5 秒自動消失（由外框計時），role="status"。 */
export function Toast({ message }: { message: string | null }) {
  return (
    <div className="k-toast-region" role="status" aria-live="polite">
      {message && (
        <div className="k-toast">
          <Icon name="check" size={16} />
          {message}
        </div>
      )}
    </div>
  )
}
