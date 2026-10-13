import { describe, expect, it } from 'vitest'
import type { Question } from '../types'
import { mergeQuestionSession } from './overlaySession'

const question = (id: string, state: Question['state'] = 'pending') =>
  ({ id, state } as Question)

describe('overlay question session', () => {
  it('keeps an answered question when the pending feed removes it', () => {
    const current = [question('a'), question('b')]
    expect(mergeQuestionSession(current, [question('b')]).map((item) => item.id)).toEqual([
      'a',
      'b',
    ])
  })

  it('refreshes matching questions without inserting mid-flow arrivals', () => {
    const current = [question('a'), question('b')]
    const merged = mergeQuestionSession(current, [question('b', 'warned'), question('c')])
    expect(merged.map((item) => [item.id, item.state])).toEqual([
      ['a', 'pending'],
      ['b', 'warned'],
    ])
  })
})
