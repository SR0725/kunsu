// 面板裡往外開（原對話、轉討論、Kongko、內文連結）的唯一入口（AC-DS-13）：在蓋板裡先讓開再打開。
import { createContext, useContext } from 'react'
import type { KunsuIpc } from '../ipc'

export const OpenLinkContext = createContext<((href: string) => void) | null>(null)

/** 有面板提供的入口就用它（蓋板會先讓開）；否則直接打開（主視窗、督導頁）。 */
export function useOpenLink(ipc: KunsuIpc): (href: string) => void {
  const provided = useContext(OpenLinkContext)
  return provided ?? ((href: string) => void ipc.openDeepLink(href))
}
