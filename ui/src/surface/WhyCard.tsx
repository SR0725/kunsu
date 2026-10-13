// 為什麼問你（BDD decision-surface AC-DS-08）：脈絡摘要、自動時間軸、打開原對話、原文與字數。
import { useState } from 'react'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { conversationLink } from '../delivery'
import { RichContent } from '../components/RichContent'
import { rawSource, whyContent } from './content'
import { useOpenLink } from './openLink'

export function WhyCard({ ipc, question }: { ipc: KunsuIpc; question: Question }) {
  const [showRaw, setShowRaw] = useState(false)
  const why = whyContent(question)
  const link = conversationLink(question)
  const raw = rawSource(question)
  const discuss = question.tier === 't2' || question.gate === 'red'
  const openLink = useOpenLink(ipc)
  return (
    <section className="surface-card surface-why" aria-label="為什麼問你">
      <div className="surface-sec-label">為什麼問你</div>
      {why.blocks.map((block, index) =>
        block.kind === 'timeline' ? (
          <ol key={index} className="surface-steps">
            {block.items.map((item, i) => (
              <li key={i} data-now={item.now ? 'true' : 'false'}>
                <time className="tabular">{item.time}</time>
                <span className="surface-steps__node" aria-hidden="true" />
                <RichContent inline onOpenLink={openLink}>
                  {item.text}
                </RichContent>
              </li>
            ))}
          </ol>
        ) : (
          <RichContent
            key={index}
            className={index === 0 ? 'surface-why__sum' : 'surface-why__more'}
            onOpenLink={openLink}
          >
            {block.markdown}
          </RichContent>
        ),
      )}
      <div className="surface-ctx-actions">
        {link && (
          <button type="button" className="surface-link" onClick={() => openLink(link)}>
            打開原對話（{link.startsWith('codex://') ? 'Codex App' : 'Claude App'}）↗
          </button>
        )}
        {discuss && (
          <button type="button" className="surface-link" onClick={() => openLink(`kunsu://discuss/${question.id}`)}>
            轉討論給 Claude
          </button>
        )}
        {question.deep_link && (
          <button type="button" className="surface-link" onClick={() => openLink(question.deep_link!)}>
            開 Kongko ↗
          </button>
        )}
        {raw.chars > 0 && (
          <button type="button" className="surface-link" aria-expanded={showRaw} onClick={() => setShowRaw((value) => !value)}>
            {showRaw ? '收起原文' : '看 AI 送來的原文'}
          </button>
        )}
      </div>
      {showRaw && (
        <>
          <pre className="surface-raw">{raw.text}</pre>
          <div className="surface-raw-meta tabular">
            共 {raw.lines} 行、{raw.chars.toLocaleString('en-US')} 字
          </div>
        </>
      )}
    </section>
  )
}
