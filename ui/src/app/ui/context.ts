// 外框提供給各頁的東西（工具列、toast）。放在獨立檔案，元件檔只匯出元件。
import { createContext, useContext, type ReactNode } from 'react'

/** 外框提供的工具列（勿擾控制）：每個頁首的右端都放它（DESIGN「Page header」）。 */
export const ShellToolbarContext = createContext<ReactNode>(null)

/** 外框提供的 toast（面板右下角）；沒有外框時什麼都不做。 */
export const ToastContext = createContext<(message: string) => void>(() => {})
export const useToast = () => useContext(ToastContext)
