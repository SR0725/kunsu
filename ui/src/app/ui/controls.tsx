// 表單控制（DESIGN「Inputs」「Switches and segmented controls」）：每個控制都有可見標籤；錯誤是紅框＋12 px 訊息
// （role="alert"）；開關 role="switch"；分段控制是 radiogroup，方向鍵移動。
import { useRef, type KeyboardEvent, type ReactNode } from 'react'
import type { IconKey } from '../status'
import { normalizeHm } from '../settings/schema'
import { Icon } from './Icon'
import { segmentKey } from './keys'

const describedBy = (...ids: (string | false | undefined)[]) => ids.filter(Boolean).join(' ') || undefined

interface FieldProps {
  id: string
  label: string
  value: string
  onChange: (value: string) => void
  /** 預設值與範圍說明（muted 12 px）。 */
  hint?: ReactNode
  error?: string
  disabled?: boolean
}

function FieldShell({ id, label, hint, error, children }: { id: string; label: string; hint?: ReactNode; error?: string; children: ReactNode }) {
  return (
    <div className="k-field">
      <label htmlFor={id} className="k-label">
        {label}
      </label>
      {children}
      {hint && (
        <p id={`${id}-hint`} className="k-field__hint">
          {hint}
        </p>
      )}
      {error && (
        <p id={`${id}-error`} role="alert" className="k-field__error">
          {error}
        </p>
      )}
    </div>
  )
}

/** 數字欄位：單位放在框內右側。只收數字，範圍檢查交給 schema（不合法時儲存停用）。 */
export function NumberField({ id, label, value, onChange, hint, error, disabled, unit }: FieldProps & { unit?: string }) {
  return (
    <FieldShell id={id} label={label} hint={hint} error={error}>
      <div className="k-input" data-invalid={error ? 'true' : undefined} data-disabled={disabled ? 'true' : undefined}>
        <input
          id={id}
          className="k-input__control tabular"
          type="text"
          inputMode="numeric"
          autoComplete="off"
          spellCheck={false}
          value={value}
          disabled={disabled}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy(hint !== undefined && `${id}-hint`, Boolean(error) && `${id}-error`)}
          onChange={(event) => onChange(event.target.value)}
        />
        {unit && (
          <span className="k-input__suffix" aria-hidden="true">
            {unit}
          </span>
        )}
      </div>
    </FieldShell>
  )
}

/** 24 小時制 HH:MM（不用 type=time：WebKit 依系統語系可能顯示上午／下午）。離開欄位時把 0930、9:30 整理成 09:30。 */
export function TimeField({ id, label, value, onChange, hint, error, disabled }: FieldProps) {
  return (
    <FieldShell id={id} label={label} hint={hint} error={error}>
      <div className="k-input k-input--time" data-invalid={error ? 'true' : undefined} data-disabled={disabled ? 'true' : undefined}>
        <Icon name="clock" size={14} className="k-input__prefix" />
        <input
          id={id}
          className="k-input__control tabular"
          type="text"
          inputMode="numeric"
          autoComplete="off"
          spellCheck={false}
          placeholder="HH:MM"
          maxLength={5}
          value={value}
          disabled={disabled}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy(hint !== undefined && `${id}-hint`, Boolean(error) && `${id}-error`)}
          onChange={(event) => onChange(event.target.value)}
          onBlur={(event) => {
            const normalized = normalizeHm(event.target.value)
            if (normalized && normalized !== event.target.value) onChange(normalized)
          }}
        />
      </div>
    </FieldShell>
  )
}

/** 開關：可見標籤（label for）＋可選說明；Space／Enter 由 button 原生處理。 */
export function Switch({
  id,
  label,
  checked,
  onChange,
  description,
  disabled,
}: {
  id: string
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
  description?: ReactNode
  disabled?: boolean
}) {
  return (
    <div className="k-switch-field">
      <div className="k-switch-field__text">
        <label htmlFor={id} className="k-label">
          {label}
        </label>
        {description && (
          <p id={`${id}-desc`} className="k-field__hint">
            {description}
          </p>
        )}
      </div>
      <button
        type="button"
        id={id}
        role="switch"
        aria-checked={checked}
        aria-describedby={description ? `${id}-desc` : undefined}
        className="k-switch"
        disabled={disabled}
        onClick={() => onChange(!checked)}
      >
        <span className="k-switch__thumb" />
      </button>
    </div>
  )
}

export interface SegmentOption<T extends string> {
  value: T
  label: string
  icon?: IconKey
}

/** 分段控制（radiogroup）：選中的是唯一的 Tab 停點，方向鍵直接換選。 */
export function SegmentedControl<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: SegmentOption<T>[]
  onChange: (value: T) => void
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([])
  const current = Math.max(0, options.findIndex((o) => o.value === value))
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const next = segmentKey(current, options.length, event.key)
    if (next === null) return
    event.preventDefault()
    onChange(options[next].value)
    refs.current[next]?.focus()
  }
  return (
    <div role="radiogroup" aria-label={label} className="k-segmented" onKeyDown={onKeyDown}>
      {options.map((option, i) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          tabIndex={option.value === value ? 0 : -1}
          className="k-segmented__item"
          ref={(el) => {
            refs.current[i] = el
          }}
          onClick={() => onChange(option.value)}
        >
          {option.icon && <Icon name={option.icon} size={14} />}
          {option.label}
        </button>
      ))}
    </div>
  )
}
