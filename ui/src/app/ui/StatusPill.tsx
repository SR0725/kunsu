import type { ReactNode } from 'react'
import type { IconKey, PillSpec, Tone } from '../status'
import { Icon } from './Icon'

/**
 * DESIGN「Status pills」：全圓角、淺色底＋深色字，一律帶圖示與文字（狀態不只靠顏色）。
 * `status` 會標在文字上（data-status），給督導頁等既有的狀態辨識與測試用。
 */
export function StatusPill({ tone, icon, status, children }: { tone: Tone; icon: IconKey; status?: string; children: ReactNode }) {
  return (
    <span className="k-pill" data-tone={tone}>
      <Icon name={icon} size={12} />
      <span data-status={status}>{children}</span>
    </span>
  )
}

export function Pill({ spec, status }: { spec: PillSpec; status?: string }) {
  return (
    <StatusPill tone={spec.tone} icon={spec.icon} status={status}>
      {spec.label}
    </StatusPill>
  )
}
