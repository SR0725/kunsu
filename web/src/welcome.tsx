// 歡迎頁（AC-WEB-07）：網站註冊後的下一步，可以登出與刪除帳號。session 只存在這個瀏覽器。
import { StrictMode, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import { Footer, Nav } from './components/Chrome'
import { accountInfo, deleteAccount, exchangeCode, type AccountInfo } from './lib/api'
import { LINKS } from './config'

const KEY = 'kunsu.session'
const read = () => { try { return localStorage.getItem(KEY) } catch { return null } }
const write = (v: string | null) => { try { v ? localStorage.setItem(KEY, v) : localStorage.removeItem(KEY) } catch { /* 無痕模式 */ } }

function Welcome() {
  const [session, setSession] = useState<string | null>(read)
  const [acc, setAcc] = useState<AccountInfo | null>(null)
  const [state, setState] = useState<'loading' | 'ready' | 'signed-out' | 'deleted'>('loading')
  const [confirm, setConfirm] = useState(false)

  useEffect(() => {
    const code = new URLSearchParams(location.search).get('code')
    ;(async () => {
      let s = session
      if (code) {
        s = await exchangeCode(code).catch(() => null)
        history.replaceState(null, '', '/welcome/')
        if (s) { write(s); setSession(s) }
      }
      if (!s) return setState('signed-out')
      const a = await accountInfo(s).catch(() => null)
      if (!a) { write(null); return setState('signed-out') }
      setAcc(a)
      setState('ready')
    })()
  }, [])

  if (state === 'loading') return <Shell><p>載入中…</p></Shell>
  if (state === 'deleted') return <Shell><h1 style={{ fontSize: 24 }}>帳號已刪除</h1><p>雲端的帳號與裝置清單已刪除，你 Mac 上的資料不受影響。</p></Shell>
  if (state === 'signed-out' || !acc) {
    return <Shell><h1 style={{ fontSize: 24 }}>還沒登入</h1><p>回到首頁用 email 註冊或登入。</p><a className="btn btn--primary" href="/#signup">回首頁</a></Shell>
  }
  return (
    <Shell>
      <h1 style={{ fontSize: 26, margin: '0 0 4px' }}>歡迎使用 Kunsu</h1>
      <p style={{ color: 'var(--muted)', margin: 0 }}>已登入：{acc.email}</p>
      <div className="next">
        <div><span className="n">1</span><div><b>下載 Mac 版</b><span>{LINKS.macDownload ? <a href={LINKS.macDownload}>下載 Kunsu for Mac</a> : 'Mac 版即將推出，準備好會寄信通知你。'}</span></div></div>
        <div><span className="n">2</span><div><b>在 Mac 上登入同一個帳號</b><span>打開 Kunsu「設定 › 帳號」，用 {acc.email} 登入。</span></div></div>
        <div><span className="n">3</span><div><b>裝 Claude Code 或 Codex 外掛</b><span>Kunsu 第一次打開會帶你一步步裝好。</span></div></div>
        <div><span className="n">4</span><div><b>配對手機（選用）</b><span>在 Mac 的「設定 › 手機」按「配對新手機」，用 iPhone 掃 QR code。</span></div></div>
      </div>
      {acc.macs.length > 0 && <p style={{ marginTop: 16, color: 'var(--muted)' }}>你的 Mac：{acc.macs.map((m) => m.name).join('、')}</p>}
      <div style={{ display: 'flex', gap: 16, marginTop: 24, alignItems: 'center', flexWrap: 'wrap' }}>
        <button className="btn" onClick={() => { write(null); setState('signed-out') }}>登出</button>
        {confirm ? (
          <>
            <span style={{ color: 'var(--muted)', fontSize: 14 }}>確定刪除？雲端的帳號與裝置清單會刪除，不能復原。</span>
            <button className="btn" onClick={() => setConfirm(false)}>取消</button>
            <button className="btn" style={{ color: 'var(--bad)' }} onClick={async () => { if (session && (await deleteAccount(session))) { write(null); setState('deleted') } }}>確定刪除</button>
          </>
        ) : (
          <button className="danger" onClick={() => setConfirm(true)}>刪除帳號</button>
        )}
      </div>
    </Shell>
  )
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <>
      <Nav />
      <main className="wrap center-card"><div className="card">{children}</div></main>
      <Footer />
    </>
  )
}

createRoot(document.getElementById('root')!).render(<StrictMode><Welcome /></StrictMode>)
