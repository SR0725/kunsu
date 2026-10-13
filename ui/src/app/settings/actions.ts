// 設定卡片的儲存與回復預設（BDD settings AC-ST-01／02／03；INV-26 的 UI 這一半）：只呼叫設定 IPC，永不碰題目。
import type { SettingsState } from '../../types'
import type { SettingsIpc } from './settingsIpc'
import { CARD_KEYS, evaluateCard, fieldsForKey, type CardDraft, type CardId } from './schema'

export type CardOutcome =
  | { kind: 'saved'; state: SettingsState }
  /** errors：畫面欄位 → 訊息；message：不對應任何欄位的錯誤（例如 policy.local.toml 寫不進去）。 */
  | { kind: 'invalid'; errors: Record<string, string>; message?: string }
  | { kind: 'failed'; message: string }
  | { kind: 'unchanged' }

/** 先在前端驗證（不合法連 IPC 都不叫），合法才送 `settings_set`；server 的逐欄錯誤對回畫面欄位。 */
export async function saveCard(
  ipc: Pick<SettingsIpc, 'settingsSet'>,
  card: CardId,
  draft: CardDraft,
  state: SettingsState,
): Promise<CardOutcome> {
  const evaluation = evaluateCard(card, draft, state.effective)
  if (!evaluation.valid) return { kind: 'invalid', errors: evaluation.errors }
  if (Object.keys(evaluation.changes).length === 0) return { kind: 'unchanged' }
  try {
    const result = await ipc.settingsSet(evaluation.changes)
    if ('errors' in result) {
      const errors: Record<string, string> = {}
      const other: string[] = []
      for (const e of result.errors) {
        if (CARD_KEYS[card].includes(e.key as never)) for (const field of fieldsForKey(e.key)) errors[field] ??= e.message
        else other.push(e.message)
      }
      return other.length > 0 ? { kind: 'invalid', errors, message: other.join('；') } : { kind: 'invalid', errors }
    }
    return { kind: 'saved', state: result }
  } catch (error) {
    return { kind: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}

/** 回復預設：把這張卡片的鍵從 local 檔移除。 */
export async function resetCard(ipc: Pick<SettingsIpc, 'settingsReset'>, card: CardId): Promise<CardOutcome> {
  try {
    return { kind: 'saved', state: await ipc.settingsReset(CARD_KEYS[card]) }
  } catch (error) {
    return { kind: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}
