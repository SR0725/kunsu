// 主視窗的畫面狀態（BDD application-shell 場景 5、AC-AS-05；DESIGN「History rows」）：作答流程（決策面板）記住開啟它的頁面。
// 面板開著時主視窗底下仍是那一頁（從歷史紀錄回看不會先閃一下待決事項），收起後回到那一頁——篩選與捲動位置跟著頁面留著。
import type { Question } from '../types'
import type { AppSection } from './Sidebar'

export type View =
  | { kind: 'app'; section: AppSection }
  | { kind: 'overlay'; questions: Question[]; startIndex: number; review: boolean; from: AppSection }

/** 主視窗外框此刻顯示的頁面：作答流程開著時是開啟它的那一頁。 */
export function viewSection(view: View): AppSection {
  return view.kind === 'app' ? view.section : view.from
}

/** 打開作答流程；沒有題目就不開（回 null）。`review`＝從歷史紀錄回看已結案的題（AC-DS-17）。 */
export function openFlowView(current: View, questions: Question[], startIndex = 0, review = false): View | null {
  if (questions.length === 0) return null
  return { kind: 'overlay', questions, startIndex, review, from: viewSection(current) }
}

/** 收起作答流程：回到開啟它的那一頁。 */
export function closeFlowView(view: View): View {
  return { kind: 'app', section: viewSection(view) }
}
