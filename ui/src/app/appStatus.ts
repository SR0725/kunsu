// app_status 的讀取（CONTRACT §15）：Agents 頁、設定頁與導覽共用。
import { useCallback, useEffect, useRef, useState } from 'react'
import type { AppStatus } from '../types'
import type { SettingsIpc } from './settings/settingsIpc'

/** 讀 `app_status`；`intervalMs` 給了就定期更新（資料與帳本每 5 秒）。 */
export function useAppStatus(ipc: Pick<SettingsIpc, 'appStatus'>, options: { initial?: AppStatus | null; intervalMs?: number } = {}) {
  const [status, setStatus] = useState<AppStatus | null>(options.initial ?? null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(options.initial == null)
  const [checkedAt, setCheckedAt] = useState<Date | null>(options.initial ? new Date() : null)
  const alive = useRef(true)
  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const next = await ipc.appStatus()
      if (!alive.current) return
      setStatus(next)
      setError(null)
      setCheckedAt(new Date())
    } catch (cause) {
      if (alive.current) setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      if (alive.current) setLoading(false)
    }
  }, [ipc])
  useEffect(() => {
    alive.current = true
    void refresh()
    const timer = options.intervalMs ? window.setInterval(() => void refresh(), options.intervalMs) : undefined
    return () => {
      alive.current = false
      window.clearInterval(timer)
    }
  }, [refresh, options.intervalMs])
  return { status, error, loading, checkedAt, refresh }
}

/** 給其他 Agent 的標準提示詞（純文字，複製後自己貼）。 */
export const GENERIC_AGENT_PROMPT =
  '使用 Kunsu MCP（http://127.0.0.1:4777/mcp）。當工作沒有我的決定就無法安全繼續時，建立一張十秒可決的題目並等我回答；不要傳 FYI、進度通知或聊天訊息。'
