import { useEffect, useState } from 'react'
import { LINKS } from '../config'

export function Nav() {
  const [scrolled, setScrolled] = useState(false)
  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 8)
    on()
    window.addEventListener('scroll', on, { passive: true })
    return () => window.removeEventListener('scroll', on)
  }, [])
  return (
    <header className="nav" data-scrolled={scrolled}>
      <div className="wrap nav__in">
        <a className="brand" href="/">
          <img src="/kunsu-mark.png" alt="" />
          Kunsu
        </a>
        <nav className="nav__links" aria-label="主要">
          <a href="/#how">怎麼運作</a>
          <a href="/#phone">手機</a>
          <a href="/#privacy">隱私</a>
          <a href="/#faq">常見問題</a>
        </nav>
        <div className="nav__right">
          {LINKS.github ? (
            <a className="btn" href={LINKS.github}>GitHub</a>
          ) : null}
          <a className="btn btn--primary" href="/#signup">免費註冊</a>
        </div>
      </div>
    </header>
  )
}

export function Footer() {
  return (
    <footer className="footer">
      <div className="wrap footer__in">
        <a className="brand" href="/">
          <img src="/kunsu-mark.png" alt="" />
          Kunsu
        </a>
        <span>內容只在你的 Mac。</span>
        <nav className="footer__links" aria-label="頁尾">
          <a href="/privacy/">隱私權政策</a>
          <a href="/terms/">服務條款</a>
          <a href="/support/">支援</a>
          {LINKS.github && <a href={LINKS.github}>GitHub</a>}
        </nav>
      </div>
    </footer>
  )
}
