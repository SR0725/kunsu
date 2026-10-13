// 決策面板的內容整理（BDD decision-surface AC-DS-02/03/08）：為什麼問你、證據、版型。
import type { Question } from '../types'

export interface TimelineItem {
  time: string
  text: string
  now: boolean
}

export type WhyBlock =
  | { kind: 'text'; markdown: string }
  | { kind: 'timeline'; items: TimelineItem[] }

export interface WhyContent {
  blocks: WhyBlock[]
  /** 真的有 why_md；false 表示舊題以 title＋body_md 首段代替。 */
  fromWhy: boolean
}

const TIME_ITEM = /^(\d{1,2}:\d{2}|現在)\s+(.+)$/
const LIST_ITEM = /^\s*(?:[-*+]|\d+[.)])\s+(.*)$/

/** 以空行分段（保留段內換行）。 */
function chunks(md: string): string[] {
  return md
    .replace(/\r/g, '')
    .split(/\n\s*\n/)
    .map((c) => c.trim())
    .filter((c) => c !== '')
}

function blockOf(chunk: string): WhyBlock {
  const lines = chunk.split('\n')
  const items = lines.map((l) => l.match(LIST_ITEM)?.[1])
  if (items.every((i) => i !== undefined)) {
    const timed = (items as string[]).map((i) => i.match(TIME_ITEM))
    if (timed.every((m) => m !== null)) {
      const hasNow = timed.some((m) => m![1] === '現在')
      return {
        kind: 'timeline',
        items: timed.map((m, i) => ({
          time: m![1],
          text: m![2].trim(),
          now: hasNow ? m![1] === '現在' : i === timed.length - 1,
        })),
      }
    }
  }
  return { kind: 'text', markdown: chunk }
}

/** 為什麼問你：why_md 解析；以 `HH:MM` 或 `現在` 開頭的清單 → 時間軸。舊題退回 title＋body 首段。 */
export function whyContent(q: Question): WhyContent {
  const why = q.why_md?.trim()
  if (why) return { blocks: chunks(why).map(blockOf), fromWhy: true }
  const first = chunks(q.body_md ?? '')[0]
  const markdown = first ? `**${q.title}**\n\n${first}` : `**${q.title}**`
  return { blocks: [{ kind: 'text', markdown }], fromWhy: false }
}

/** 證據內文：舊題的 body_md 首段已被拿去當脈絡時，證據只留其餘部分。 */
export function evidenceMarkdown(q: Question): string {
  const body = (q.body_md ?? '').trim()
  if (q.why_md?.trim()) return body
  return chunks(body).slice(1).join('\n\n')
}

/** 有沒有證據（決定兩欄或單欄）。 */
export function hasEvidence(q: Question): boolean {
  return evidenceMarkdown(q).trim() !== '' || q.evidence.length > 0 || Boolean(q.draft?.canvas)
}

/** 「看 AI 送來的原文」：題目脈絡與證據的原始文字與統計。 */
export function rawSource(q: Question): { text: string; lines: number; chars: number } {
  const text = [q.why_md?.trim(), q.body_md?.trim()].filter((s): s is string => Boolean(s)).join('\n\n')
  return { text, lines: text === '' ? 0 : text.split('\n').length, chars: text.length }
}
