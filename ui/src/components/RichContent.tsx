import { createContext, useContext, useMemo, useRef, useState } from 'react'
import type { ComponentProps, ReactNode } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeRaw from 'rehype-raw'
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize'
import remarkCjkFriendly from 'remark-cjk-friendly'
import remarkGfm from 'remark-gfm'
import { resolveAsset } from '../canvas/assets'

/**
 * 草稿題的資產快照（AC-EC-08）：面板在草稿題外層提供（快照還沒載入時是空物件）。有值時，這一題裡所有 Markdown
 * （決策句、選項、證據、why、內文）都只顯示快照圖片，外部圖片一律不載入。非草稿題是 undefined。
 */
export const DraftAssetsContext = createContext<Record<string, string> | undefined>(undefined)

// `picture`／`source`（srcset）會繞過「只顯示快照」載入外部圖片：一律不允許（AC-EC-08）。
const NO_SRCSET = new Set(['srcSet', 'srcset'])
const safeSchema = {
  ...defaultSchema,
  // DESIGN：內文不得有 input（GFM 待辦清單只留文字）。
  tagNames: [
    ...(defaultSchema.tagNames ?? []).filter((tag) => !['input', 'picture', 'source'].includes(tag)),
    'details',
    'summary',
    'kbd',
    'mark',
    'small',
  ],
  attributes: {
    ...Object.fromEntries(
      Object.entries(defaultSchema.attributes ?? {}).map(([tag, attrs]) => [
        tag,
        attrs.filter((attr) => !NO_SRCSET.has(Array.isArray(attr) ? String(attr[0]) : String(attr))),
      ]),
    ),
    a: [...(defaultSchema.attributes?.a ?? []), 'target', 'rel'],
  },
  protocols: {
    ...defaultSchema.protocols,
    href: ['http', 'https', 'mailto'],
    src: ['http', 'https'],
  },
}

export interface PendingLink {
  href: string
  at: number
}

const LINK_CONFIRM_MS = 4000

/**
 * 連結（BDD evidence-canvas 場景 11）：第一次點只提示「再按一次用瀏覽器開啟」，4 秒內再點同一個連結才開；
 * 只開 http(s)。
 */
export function linkClick(
  pending: PendingLink | null,
  href: string,
  now: number,
): { action: 'confirm' | 'open' | 'ignore'; pending: PendingLink | null } {
  if (!/^https?:\/\//i.test(href)) return { action: 'ignore', pending: null }
  if (pending && pending.href === href && now - pending.at <= LINK_CONFIRM_MS) return { action: 'open', pending: null }
  return { action: 'confirm', pending: { href, at: now } }
}

function hostOf(href: string): string {
  try {
    return new URL(href).host
  } catch {
    return href
  }
}

/** 這一段 Markdown 的連結狀態（哪一個連結在等第二次點擊）與點擊處理；由 `RichContent` 提供給 `RichLink`。 */
const LinkStateContext = createContext<{ pending: PendingLink | null; click: (href: string | undefined) => void } | null>(null)

/**
 * 連結（模組層級的固定元件，不在每次渲染重新建立）：第一次點出現提示時只更新、不重新掛載，鍵盤焦點留在連結上，再按一次
 * Enter 就能打開（AC-DS-13、AC-DS-07）。點擊不傳到外層可點擊的卡片（例如選項、題組項目），不會順便選取或取消。
 */
function RichLink({ children, href }: ComponentProps<'a'> & { node?: unknown }) {
  const state = useContext(LinkStateContext)
  // 沒有 href（sanitizer 拿掉 javascript: 等）的連結點了無作用，也永遠不顯示提示（AC-EC-11）。
  const waiting = Boolean(href) && state?.pending?.href === href
  return (
    <a
      href={href}
      rel="noreferrer noopener"
      data-confirm={waiting ? 'true' : undefined}
      title={waiting ? `再按一次用瀏覽器開啟 ${hostOf(href ?? '')}` : undefined}
      onClick={(event) => {
        event.preventDefault()
        event.stopPropagation()
        state?.click(href)
      }}
    >
      {children}
      {waiting && <span className="rich-content__link-confirm">（再按一次用瀏覽器開啟 {hostOf(href ?? '')}）</span>}
    </a>
  )
}

/** 行內模式的段落：只留內容（固定元件，包著的連結不會跟著重新掛載）。 */
function InlineParagraph({ children }: { children?: ReactNode; node?: unknown }) {
  return <>{children}</>
}

export function RichContent({
  children,
  className = '',
  inline = false,
  assets: assetsProp,
  onOpenLink,
}: {
  children: string
  className?: string
  inline?: boolean
  /** 草稿題的資產快照（`./assets/x.png` → data URI，AC-EC-08）。有值時只顯示快照圖片，外部圖片一律不載入。 */
  assets?: Record<string, string>
  /** 確認後用系統瀏覽器開連結（沒給就只顯示，不導覽）。 */
  onOpenLink?: (href: string) => void
}) {
  const [pending, setPending] = useState<PendingLink | null>(null)
  const pendingRef = useRef<PendingLink | null>(null)
  const openRef = useRef(onOpenLink)
  openRef.current = onOpenLink
  const draftAssets = useContext(DraftAssetsContext)
  const assets = assetsProp ?? draftAssets
  const linkState = useMemo(
    () => ({
      pending,
      click: (href: string | undefined) => {
        if (!href) return
        const next = linkClick(pendingRef.current, href, Date.now())
        pendingRef.current = next.pending
        setPending(next.pending)
        if (next.action === 'open') openRef.current?.(href)
      },
    }),
    [pending],
  )
  // 交給 react-markdown 的元件要跨渲染保持同一個（重新渲染只更新、不重新掛載，焦點才留得住）。
  const components = useMemo(
    () => ({
      ...(inline ? { p: InlineParagraph } : {}),
      ...(assets
        ? {
            img: ({ src, alt, title }: { src?: string; alt?: string; title?: string }) => {
              const resolved = resolveAsset(src, assets)
              // 草稿題：只放行同版本快照（data URI）；外部或不存在的圖片只留替代文字，不發任何請求。
              if (!resolved?.startsWith('data:')) return alt ? <span className="rich-content__missing-img">［圖片：{alt}］</span> : null
              return <img src={resolved} alt={alt ?? ''} title={title} loading="lazy" />
            },
          }
        : {}),
      a: RichLink,
    }),
    [inline, assets],
  )
  return (
    <div className={`rich-content ${inline ? 'rich-content--inline' : ''} ${className}`}>
      <LinkStateContext.Provider value={linkState}>
        <ReactMarkdown
          // CJK 全形標點旁的粗體／斜體照常生效（CommonMark 的 flanking 規則不認中日韓標點）。
          remarkPlugins={[remarkGfm, remarkCjkFriendly]}
          rehypePlugins={[rehypeRaw, [rehypeSanitize, safeSchema]]}
          components={components}
        >
          {children}
        </ReactMarkdown>
      </LinkStateContext.Provider>
    </div>
  )
}
