// 草稿資產（AC-EC-08）：Markdown 的 ./assets/x.png 只換成該版本快照的 data URI。
import { describe, expect, it } from 'vitest'
import { resolveAsset } from './assets'

describe('resolveAsset', () => {
  const assets = { './assets/a.png': 'data:image/png;base64,AAAA' }
  it('相對路徑換成快照 data URI', () => {
    expect(resolveAsset('./assets/a.png', assets)).toBe('data:image/png;base64,AAAA')
    expect(resolveAsset('assets/a.png', assets)).toBe('data:image/png;base64,AAAA')
  })
  it('其他網址原樣交給 sanitizer；不存在的資產不亂補', () => {
    expect(resolveAsset('https://x.test/a.png', assets)).toBe('https://x.test/a.png')
    expect(resolveAsset('./assets/none.png', assets)).toBe('./assets/none.png')
    expect(resolveAsset(undefined, assets)).toBeUndefined()
  })
})

describe('evidenceKey', () => {
  it('同一題同版本但內容不同（dry-run 重檢）不能共用快取', async () => {
    const { evidenceKey } = await import('./useEvidence')
    const base = { id: 'q', draft: { revision: 1, content_hash: 'sha256:a', registered_at: 'x' } }
    expect(evidenceKey(base as never)).not.toBe(evidenceKey({ ...base, draft: { ...base.draft, content_hash: 'sha256:b' } } as never))
    expect(evidenceKey({ id: 'q' } as never)).toBeNull()
  })
})
