import type { BatchItem } from '../types'
import { initDrafts, rebaseDrafts } from './batch'
import type { ItemDraft } from './batch'
const SOURCES = [
  {id:'course',label:'課程',prefix:'課程｜'},
  {id:'facebook',label:'Facebook',prefix:'Facebook｜'},
  {id:'instagram',label:'Instagram',prefix:'Instagram｜'},
  {id:'threads',label:'Threads',prefix:'Threads｜'},
  {id:'line',label:'LINE',prefix:'LINE｜'},
] as const
export type ReplySource = {id:string;label:string}
export function sourceOf(item:BatchItem): ReplySource {
  return SOURCES.find(s=>item.subtitle?.startsWith(s.prefix)) ?? {id:'other',label:'其他'}
}
export function sourceGroups(items:BatchItem[]) {
  const groups: (ReplySource & {count:number;firstIndex:number})[]=[]
  items.forEach((item,index)=>{
    const source=sourceOf(item), existing=groups.find(g=>g.id===source.id)
    if(existing) existing.count++
    else groups.push({...source,count:1,firstIndex:index})
  })
  return groups
}
/** Review is a bookmark only: never calls IPC or creates an answer. */
export function toggleReviewed(drafts:ItemDraft[],index:number):ItemDraft[] {
  return drafts.map((d,i)=>i===index?{...d,reviewed:!d.reviewed}:d)
}
export const draftStorageKey=(id:string)=>`kunsu:assistant-drafts:v1:${id}`
export function serializeDrafts(items:BatchItem[],drafts:ItemDraft[]):string {
  return JSON.stringify({items,drafts})
}
export function restoreDrafts(raw:string|null,items:BatchItem[]):ItemDraft[] {
  if(!raw) return initDrafts(items)
  try {
    const saved=JSON.parse(raw)
    if(!Array.isArray(saved.items)||!Array.isArray(saved.drafts)||saved.items.length!==saved.drafts.length) return initDrafts(items)
    if(!saved.items.every((i:BatchItem)=>i&&typeof i.id==='string'&&typeof i.proposal==='string'&&typeof i.context_md==='string') ||
       !saved.drafts.every((d:ItemDraft)=>d&&['approve','skip','revise'].includes(d.action)&&typeof d.text==='string'&&typeof d.note==='string')) return initDrafts(items)
    return rebaseDrafts(saved.items,items,saved.drafts)
  } catch { return initDrafts(items) }
}
