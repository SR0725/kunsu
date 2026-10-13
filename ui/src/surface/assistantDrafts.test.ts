import { describe, expect, it } from 'vitest'
import type { BatchItem, Question } from '../types'
import { buildBatchPayload, editText, initDrafts, rebaseDrafts } from './batch'
import { sourceGroups, sourceOf, toggleReviewed, serializeDrafts, restoreDrafts } from './assistantDrafts'
const item = (id='i1', subtitle='LINE｜我 → 客戶'): BatchItem => ({id,title:'客戶',subtitle,context_md:'問題',proposal:'原稿',editable:true,default:'approve',content_hash:`hash-${id}`})
describe('assistant drafts', () => {
  it('inv_review_bookmark_never_authorizes_send', () => {
    const items=[item(),item('i2','Facebook｜粉專 → 客戶')]
    const original=initDrafts(items)
    expect(original[0].reviewed).toBeUndefined()
    const marked=toggleReviewed(original,0)
    expect(marked[0].reviewed).toBe(true)
    expect(toggleReviewed(marked,0)[0].reviewed).toBe(false)
    const edited=editText(marked,0,'  最後文字\n')
    expect(edited[0].reviewed).toBe(false)
    const q={id:'qa',items} as Question
    expect((buildBatchPayload(q,edited) as Extract<ReturnType<typeof buildBatchPayload>,{type:'batch'}>).items?.[0].text).toBe('  最後文字\n')
    expect((buildBatchPayload(q,original) as Extract<ReturnType<typeof buildBatchPayload>,{type:'batch'}>).items?.[0].action).toBe('approve')
    expect(marked).not.toHaveProperty('answer')
  })
  it('counts exact source prefixes without guessing from the sender', () => {
    expect(sourceOf(item('1','Instagram｜私人')).id).toBe('instagram')
    expect(sourceOf({...item(),subtitle:undefined,title:'LINE先生'}).id).toBe('other')
    expect(sourceGroups([item(),item('2'),item('3','Facebook｜粉專')]).map(x=>[x.id,x.count,x.firstIndex])).toEqual([['line',2,0],['facebook',1,2]])
  })
  it('restores edits across closing, but invalidates review when recipient/context changes', () => {
    const base=[item()];const drafts=toggleReviewed(editText(initDrafts(base),0,'使用者改稿'),0)
    const saved=serializeDrafts(base,drafts)
    expect(restoreDrafts(saved,base)[0]).toMatchObject({text:'使用者改稿',reviewed:true})
    const changed=[{...item(),subtitle:'LINE｜另一個人',content_hash:'new-hash'}]
    expect(restoreDrafts(saved,changed)[0]).toMatchObject({text:'使用者改稿',reviewed:false,updated:true})
    expect(rebaseDrafts(base,changed,drafts)[0].reviewed).toBe(false)
    expect(restoreDrafts('bad json',base)).toEqual(initDrafts(base))
    expect(restoreDrafts('{"items":[],"drafts":[null]}',base)).toEqual(initDrafts(base))
  })
  it('rewrite and skipped entries never carry a sendable text', () => {
    const items=[item(),item('i2')];const drafts=initDrafts(items)
    drafts[0]={...drafts[0],action:'revise',note:'更簡短'};drafts[1].action='skip'
    const payload=buildBatchPayload({id:'qa',items} as Question,drafts)
    expect(payload.type).toBe('batch')
    if(payload.type==='batch') expect(payload.items.every(d=>d.text===undefined)).toBe(true)
  })
})
