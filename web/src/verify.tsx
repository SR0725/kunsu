// 信裡的登入連結（AC-WEB-06）：App 在等就叫使用者回 App；網站註冊的帶進歡迎頁；失效就說再寄一次。
import { StrictMode, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { CircleCheck, CircleAlert, Loader } from 'lucide-react'
import './styles.css'
import { Footer, Nav } from './components/Chrome'
import { verifyLink, type VerifyResult } from './lib/api'

function Verify() {
  const [r, setR] = useState<VerifyResult | 'loading' | 'error'>('loading')
  useEffect(() => {
    const token = new URLSearchParams(location.search).get('token') ?? ''
    verifyLink(token).then(
      (res) => {
        setR(res)
        if (res.kind === 'web') location.replace(`/welcome/?code=${encodeURIComponent(res.code)}`)
        if (res.kind === 'deeplink') location.href = res.url
      },
      () => setR('error'),
    )
  }, [])
  const [Icon, title, body] =
    r === 'loading' || (typeof r === 'object' && (r.kind === 'web' || r.kind === 'deeplink'))
      ? [Loader, '正在登入…', '']
      : r === 'error'
        ? [CircleAlert, '登入失敗', '連不上帳號服務，請稍後再試一次。']
        : r.kind === 'app'
          ? [CircleCheck, '登入成功', '可以回到 Kunsu 了，它會自動登入。這個分頁可以關掉。']
          : [CircleAlert, '連結已失效', '登入連結 15 分鐘內有效、只能用一次。回到 Kunsu 或首頁再寄一次。']
  return (
    <>
      <Nav />
      <main className="wrap center-card">
        <div className="card" role="status">
          <Icon size={28} aria-hidden="true" />
          <h1 style={{ fontSize: 24, margin: '12px 0 6px' }}>{title}</h1>
          {body && <p>{body}</p>}
          {typeof r === 'object' && r.kind === 'expired' && <a className="btn btn--primary" href="/#signup" style={{ marginTop: 16 }}>再寄一次</a>}
        </div>
      </main>
      <Footer />
    </>
  )
}

createRoot(document.getElementById('root')!).render(<StrictMode><Verify /></StrictMode>)
