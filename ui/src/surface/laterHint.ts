// 「稍後再問」寫出的後果（BDD decision-surface 場景 4、AC-DS-04；2026-10-09 取代寫死的 15）：N 是目前生效的強制週期長度，
// 面板掛載時讀設定（settings_get 不限視窗）；讀不到、還沒讀到或值不合理時是 15。
import type { SettingsIpc } from '../app/settings/settingsIpc'

export const DEFAULT_CYCLE_MIN = 15

/** CONTRACT §15 的週期範圍（5–120 分鐘）；範圍外視為讀不到。 */
function validCycle(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 5 && value <= 120
}

export function laterHint(cycleMin?: number | null): string {
  return `${validCycle(cycleMin) ? cycleMin : DEFAULT_CYCLE_MIN} 分鐘後再問`
}

/** 讀目前生效的週期長度；任何失敗都退回 15（面板永遠不因為讀設定而壞掉）。 */
export async function readCycleMinutes(ipc: Partial<Pick<SettingsIpc, 'settingsGet'>>): Promise<number> {
  try {
    const state = await ipc.settingsGet?.()
    const cycle = state?.effective?.force?.cycle_min
    return validCycle(cycle) ? cycle : DEFAULT_CYCLE_MIN
  } catch {
    return DEFAULT_CYCLE_MIN
  }
}
