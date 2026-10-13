// 隱私權政策、服務條款、支援（AC-WEB-08）。依網址決定顯示哪一頁。
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import { Footer, Nav } from './components/Chrome'
import { LINKS } from './config'

const UPDATED = '2026 年 10 月 13 日'

function Privacy() {
  return (
    <article className="doc">
      <h1>隱私權政策</h1>
      <p className="updated">最後更新：{UPDATED}</p>
      <p>Kunsu 的設計原則是：你的工作內容只留在你自己的電腦上。這份政策說明我們收什麼、不收什麼。</p>
      <h2>我們不收的資料</h2>
      <p>AI 問你的題目、你的答案、證據、對話內容、草稿、檔案路徑與帳本，全部只存在你的 Mac。我們的伺服器沒有任何地方存放這些內容。</p>
      <h2>我們收的資料</h2>
      <ul>
        <li><b>帳號：</b>email、顯示名稱、登入方式（Email、Google 或 Apple）與建立時間。</li>
        <li><b>裝置清單：</b>你的 Mac 與手機的識別碼、名稱、公開金鑰、配對與最後上線時間，用來讓你的裝置找到彼此。</li>
        <li><b>登入憑證：</b>只存雜湊值，用來驗證你的登入與 Mac 連上中轉站。</li>
      </ul>
      <h2>中轉站</h2>
      <p>手機與 Mac 之間的訊息端對端加密。中轉站只依裝置識別碼轉送密文，不持有任何解密金鑰、不把訊息寫進硬碟；Mac 離線時最多在記憶體暫存 10 分鐘。</p>
      <h2>寄信</h2>
      <p>登入連結透過 Resend 寄出，信裡只有登入連結，沒有追蹤像素。</p>
      <h2>我們不做的事</h2>
      <p>不賣資料、不放廣告追蹤、不用你的內容訓練任何模型。</p>
      <h2>刪除</h2>
      <p>你可以在 Mac、iPhone 或網站上刪除帳號。刪除後所有登入與裝置憑證立刻失效，帳號資料在 30 天內完全清除。你 Mac 上的資料由你自己保管，不受影響。</p>
      <h2>聯絡</h2>
      <p>有任何問題請寫信到 <a href={`mailto:${LINKS.supportEmail}`}>{LINKS.supportEmail}</a>。</p>
    </article>
  )
}

function Terms() {
  return (
    <article className="doc">
      <h1>服務條款</h1>
      <p className="updated">最後更新：{UPDATED}</p>
      <h2>服務內容</h2>
      <p>Kunsu 提供 Mac App、iPhone App、帳號服務與手機中轉站，讓 AI Agent 需要你決定時能把問題帶到你面前。目前免費提供。</p>
      <h2>你的責任</h2>
      <p>你對自己的回答與批准負責。Kunsu 不替你作答、不替你批准；動錢的操作只在你核對並批准後由你自己的工具執行。</p>
      <h2>開源</h2>
      <p>Kunsu 的原始碼以 MIT 授權公開；依 MIT 授權，軟體按「現狀」提供，不附任何明示或默示的保證。</p>
      <h2>服務變更</h2>
      <p>我們可能調整或停止帳號服務與中轉站，會提前以 email 通知。你 Mac 上的資料不受影響。</p>
      <h2>聯絡</h2>
      <p><a href={`mailto:${LINKS.supportEmail}`}>{LINKS.supportEmail}</a></p>
    </article>
  )
}

function Support() {
  return (
    <article className="doc">
      <h1>支援</h1>
      <p className="updated">寫信到 <a href={`mailto:${LINKS.supportEmail}`}>{LINKS.supportEmail}</a>，我們會盡快回覆。</p>
      <h2>收不到登入信</h2>
      <p>看一下垃圾郵件匣。連結 15 分鐘內有效、只能用一次；過期就回到 App 或首頁再寄一次。</p>
      <h2>手機連不到 Mac</h2>
      <p>確認 Mac 上的 Kunsu 有打開，並在「設定 › 帳號」登入同一個帳號。手機右上角會顯示連線狀態。</p>
      <h2>配對手機</h2>
      <p>在 Mac 的「設定 › 手機」按「配對新手機」，用 iPhone 掃 QR code，再回到 Mac 按「允許」。</p>
      <h2>換手機或手機遺失</h2>
      <p>在 Mac 的「設定 › 手機」撤銷舊裝置，撤銷後它立刻連不上你的 Mac。</p>
      <h2>刪除帳號</h2>
      <p>在 iPhone 的「設定 › 帳號」、Mac 的「設定 › 帳號」或網站的歡迎頁都能刪除。</p>
    </article>
  )
}

const page = location.pathname.startsWith('/privacy') ? <Privacy /> : location.pathname.startsWith('/terms') ? <Terms /> : <Support />

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Nav />
    <main className="wrap">{page}</main>
    <Footer />
  </StrictMode>,
)
