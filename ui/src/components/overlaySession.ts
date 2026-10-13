import type { Question } from '../types'
import { isTerminal } from '../types'

/**
 * 作答 flow 的題目 session（PRD §5.8、BDD overlay-presentation AC-OP-07）。`latest` 是最新的完整清單（含終態）：
 * - session 既有題目以最新資料更新——被其他通道結案的題變成終態（面板就地唯讀），已在面板答完的題不會因為不在待決清單裡
 *   而消失；
 * - session 開著時才變成強制（`forced`）、不是 internal 的題接到尾端，答完目前的題可以直接前往它，不必關閉再彈出；
 * - 其他半途出現的題（還沒到期、internal）不插進來，避免 flow 一直變長。
 * 全新的 session（目前是空的）只帶進還沒結案的題。
 */
export function mergeQuestionSession(current: Question[], latest: Question[]): Question[] {
  if (current.length === 0) return latest.filter((question) => !isTerminal(question.state))
  const updates = new Map(latest.map((question) => [question.id, question]))
  const known = new Set(current.map((question) => question.id))
  const joined = latest.filter((question) => question.state === 'forced' && !question.internal && !known.has(question.id))
  return [...current.map((question) => updates.get(question.id) ?? question), ...joined]
}

/**
 * 主視窗作答 flow 交給面板的最新清單（AC-DS-16、AC-BR-09）：一般 flow 用完整清單（和蓋板一樣就地更新、接上新強制題）；
 * 從歷史紀錄回看已結案的題只給那幾題的最新資料（例如 AI 回饋），不把強制題接進來——回看不是作答 session。
 */
export function flowFeed(latest: Question[], session: Question[], review: boolean): Question[] {
  if (!review) return latest
  const ids = new Set(session.map((question) => question.id))
  return latest.filter((question) => ids.has(question.id))
}
