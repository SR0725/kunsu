import { describe, expect, it } from 'vitest'
import { CLAUDE_SPARK, CODEX_MARK, agentMarkSource, agentDisplayName } from './agentMarks'

describe('來源圖示（AC-OP-11）', () => {
  it('Claude 與 Codex 預設使用官方向量輪廓，不再是文字符號', () => {
    expect(agentMarkSource('claude')).toEqual({ kind: 'vector', agent: 'claude' })
    expect(agentMarkSource('codex')).toEqual({ kind: 'vector', agent: 'codex' })
    expect(agentMarkSource('other')).toEqual({ kind: 'vector', agent: 'other' })
  })

  it('visual_symbol 只覆寫粒子輪廓，最多 4 個字', () => {
    expect(agentMarkSource('claude', '  山  ')).toEqual({ kind: 'glyph', text: '山' })
    expect(agentMarkSource('codex', 'ABCDE')).toEqual({ kind: 'glyph', text: 'ABCD' })
    expect(agentMarkSource('other', '   ')).toEqual({ kind: 'vector', agent: 'other' })
  })

  it('Claude 星芒取自 Claude App 內建 AppLogo 向量', () => {
    expect(CLAUDE_SPARK.viewBox).toBe(248)
    expect(CLAUDE_SPARK.path.startsWith('M52.4285 162.873')).toBe(true)
    expect(CLAUDE_SPARK.color).toBe('#D97757')
  })

  it('Codex 標記是八瓣雲加 >_ 提示字元', () => {
    expect(CODEX_MARK.lobes).toHaveLength(8)
    expect(CODEX_MARK.prompt).toContain('M8.4 9.2')
  })

  it('顯示名稱', () => {
    expect(agentDisplayName('claude')).toBe('Claude')
    expect(agentDisplayName('codex')).toBe('Codex')
    expect(agentDisplayName('other')).toBe('AI Agent')
  })
})
