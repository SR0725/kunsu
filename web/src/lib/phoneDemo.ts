// 手機示範（AC-WEB-04）：一問一答、5 秒撤回、下拉滿才出現歷史。全部是假資料。
export interface DemoQuestion {
  id: string
  agent: 'claude' | 'codex'
  project: string
  question: string
  why: string
  options: string[]
  recommended: number
}

export interface PhoneState {
  current: DemoQuestion | null
  answered: { id: string; answer: string }[]
  undoable: boolean
  historyShown: boolean
}

const questions: DemoQuestion[] = [
  { id: 'q1', agent: 'codex', project: 'kunsu', question: '邊緣瀏海拖到角落時，要吸成 L 形嗎？', why: '決定後 Codex 會照這個改測試，大約 20 分鐘。', options: ['吸成 L 形', '滑回直邊'], recommended: 0 },
  { id: 'q2', agent: 'claude', project: 'landing-page', question: '首屏標題要用哪一版？', why: 'A 版點擊率高 12%，但比較長。', options: ['A：AI 卡住時叫醒你', 'B：別讓 AI 等你'], recommended: 0 },
  { id: 'q3', agent: 'claude', project: 'api', question: '測試全綠了，要我直接 commit 嗎？', why: '改了 3 個檔，跑過 214 個測試。', options: ['commit', '先給我看 diff'], recommended: 0 },
]

const PULL = 120

function initial(): PhoneState {
  return { current: questions[0], answered: [], undoable: false, historyShown: false }
}

function next(answered: number): DemoQuestion | null {
  return questions[answered] ?? null
}

export const phoneDemo = {
  questions,
  PULL,
  initial,
  answer(s: PhoneState, option: number): PhoneState {
    if (!s.current) return s
    const answered = [...s.answered, { id: s.current.id, answer: s.current.options[option] }]
    return { ...s, answered, current: next(answered.length), undoable: true }
  },
  undo(s: PhoneState): PhoneState {
    if (!s.undoable || s.answered.length === 0) return s
    const answered = s.answered.slice(0, -1)
    return { ...s, answered, current: next(answered.length), undoable: false }
  },
  commit(s: PhoneState): PhoneState {
    return { ...s, undoable: false }
  },
  progress(distance: number): number {
    return Math.min(Math.max(distance / PULL, 0), 1)
  },
  release(s: PhoneState, distance: number): PhoneState {
    return distance >= PULL ? { ...s, historyShown: true } : s
  },
}
