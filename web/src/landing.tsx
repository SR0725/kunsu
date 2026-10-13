import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { Apple, BellOff, Check, Clock4, Download, EyeOff, GitBranch, KeyRound, Laptop, Layers, Lock, MessageSquareReply, ServerCog, ShieldCheck, Smartphone } from 'lucide-react'
import './styles.css'
import { Footer, Nav } from './components/Chrome'
import { SignupForm } from './components/SignupForm'
import { NotchDemo } from './components/NotchDemo'
import { StationDemo } from './components/StationDemo'
import { PhoneDemo } from './components/PhoneDemo'
import { LINKS } from './config'

function Downloads() {
  return (
    <div className="hero__downloads">
      {LINKS.macDownload ? (
        <a className="btn btn--lg" href={LINKS.macDownload}><Download size={16} />下載 Mac 版</a>
      ) : (
        <span className="btn btn--lg" aria-disabled="true"><Laptop size={16} />Mac 版<span className="soon">即將推出</span></span>
      )}
      {LINKS.appStore ? (
        <a className="btn btn--lg" href={LINKS.appStore}><Apple size={16} />App Store</a>
      ) : (
        <span className="btn btn--lg" aria-disabled="true"><Apple size={16} />iPhone<span className="soon">即將推出</span></span>
      )}
    </div>
  )
}

function Check2({ children }: { children: React.ReactNode }) {
  return <li><Check size={18} aria-hidden="true" /><span>{children}</span></li>
}

function Landing() {
  return (
    <>
      <Nav />
      <main>
        <section className="hero" id="signup">
          <div className="wrap">
            <a className="pill-link" href="#privacy"><b>新</b>給 Claude Code 與 Codex · 內容只在你的 Mac</a>
            <h1 className="h1"><span className="nb">AI 卡住要你決定時，</span><em className="nb">它會叫醒你。</em></h1>
            <p className="lede">
              Kunsu 是 AI Agent 的決策收件匣。Claude Code、Codex 需要你拍板，問題就出現在螢幕邊緣，15 分鐘內一定帶到你面前；
              躺在床上也能用手機回答，AI 接著做。
            </p>
            <div className="hero__cta">
              <SignupForm id="hero-email" />
              <Downloads />
            </div>
            <div className="hero__demo"><NotchDemo /></div>
          </div>
        </section>

        <section className="section section--line">
          <div className="wrap">
            <div className="eyebrow">問題</div>
            <h2 className="h2">AI 在等你，你卻不知道。</h2>
            <p className="sub">你讓三個 AI 跑長任務，其中一個問了一句「要用哪一版？」——然後整條工作停了一整晚。</p>
            <div className="cards3">
              <div className="card"><div className="card__icon"><BellOff size={18} /></div><h3>通知一下就被掃掉</h3><p>一般通知沒有「一定要處理」的意思，關掉就沒了。</p></div>
              <div className="card"><div className="card__icon"><Layers size={18} /></div><h3>問題散在各個對話</h3><p>Claude Code、Codex、終端機，每個視窗都可能有一句在等你。</p></div>
              <div className="card"><div className="card__icon"><Clock4 size={18} /></div><h3>回來時脈絡早就沒了</h3><p>原本的選項、證據與後果，要重新翻對話才找得回來。</p></div>
            </div>
          </div>
        </section>

        <section className="section section--line" id="how">
          <div className="wrap">
            <div className="eyebrow">怎麼運作</div>
            <h2 className="h2">一個地方收齊，答完回到原本的對話。</h2>
            <div className="steps">
              <div><div className="n">01</div><h3>AI 透過外掛發問</h3><p>裝好 Claude Code 或 Codex 外掛，AI 需要你拍板時呼叫 <code>ask_create</code>，問題連同選項與證據送進 Kunsu。</p></div>
              <div><div className="n">02</div><h3>15 分鐘內帶到你面前</h3><p>先在螢幕邊緣出現，剩 3 分鐘提醒，時間到就打開。分享螢幕、會議中、睡眠時間自動延後。</p></div>
              <div><div className="n">03</div><h3>答案回到原本的對話</h3><p>你回答後，發問的那段 Claude／Codex 對話接著做，並把結果回報給你。</p></div>
            </div>
            <div className="split" style={{ marginTop: 72 }}>
              <div>
                <h3 className="h2" style={{ fontSize: 30 }}>所有待決事項，一個視窗看完。</h3>
                <p className="sub">Station 主視窗依急迫程度排好，歷史紀錄、排程督導與 Agent 連線狀態都在同一個地方。</p>
                <ul className="checks">
                  <Check2><b>不替你作答。</b>關掉視窗不等於回答，沒答的題目會一直回來。</Check2>
                  <Check2><b>動錢一定先核對。</b>金額與參數所見即所簽，批准只對這一次有效。</Check2>
                  <Check2><b>勿擾是有時限的。</b>最長 24 小時，題目與倒數照常保存。</Check2>
                </ul>
              </div>
              <StationDemo />
            </div>
          </div>
        </section>

        <section className="section section--line" id="phone">
          <div className="wrap split split--rev">
            <div>
              <div className="eyebrow">iPhone</div>
              <h2 className="h2">躺在床上，也能讓 AI 繼續工作。</h2>
              <p className="sub">打開就只有這次要處理的事。軍師一次問一題，點一下就回答；想多知道一點，先問軍師。</p>
              <ul className="checks">
                <Check2><b>一次一題。</b>短選項兩欄，長的一排一個，拇指點得到。</Check2>
                <Check2><b>為什麼？前因後果？</b>軍師讀得到題目的脈絡，答不了就幫你問發問的 AI 本人。</Check2>
                <Check2><b>5 秒內可以撤回。</b>撤回前不會送到 Mac；動錢的題目一定先核對。</Check2>
                <Check2><b>答完可以聊天。</b>問大家在做什麼、從手機開新任務。</Check2>
              </ul>
            </div>
            <PhoneDemo />
          </div>
        </section>

        <section className="section section--line" id="privacy">
          <div className="wrap">
            <div className="eyebrow">隱私</div>
            <h2 className="h2">內容只在你的 Mac。我們看不到。</h2>
            <p className="sub">題目、答案、證據與對話，全部只存在你自己的電腦。手機和 Mac 之間端對端加密，中轉站只轉送看不懂的密文。</p>
            <div className="flow" aria-label="資料流向示意">
              <div className="flow__node"><Smartphone size={20} /><b>你的 iPhone</b><span>私鑰在 Secure Enclave</span></div>
              <div className="flow__wire"><span className="flow__packet">密文</span></div>
              <div className="flow__node" data-dim="true"><ServerCog size={20} /><b>中轉站</b><span>只轉送、不落地、沒有金鑰</span></div>
              <div className="flow__wire"><span className="flow__packet">密文</span></div>
              <div className="flow__node"><Laptop size={20} /><b>你的 Mac</b><span>所有內容都在這裡</span></div>
            </div>
            <div className="cards3">
              <div className="card"><div className="card__icon"><Lock size={18} /></div><h3>端對端加密</h3><p>X25519、ChaCha20-Poly1305；每個請求都用裝置私鑰簽章，Mac 拒絕重放與偽造。</p></div>
              <div className="card"><div className="card__icon"><EyeOff size={18} /></div><h3>雲端只有帳號</h3><p>只存 email 和裝置清單，用來登入與找到你的 Mac。沒有任何欄位放得下你的題目。</p></div>
              <div className="card"><div className="card__icon"><KeyRound size={18} /></div><h3>AI 不能替你回答</h3><p>AI 用的工具沒有作答或批准的能力。只有你本人在電腦或配對的手機上才能回答。</p></div>
            </div>
          </div>
        </section>

        <section className="section section--line">
          <div className="wrap split">
            <div>
              <div className="eyebrow">開源</div>
              <h2 className="h2">程式碼全部公開，你可以自己查。</h2>
              <p className="sub">
                Mac App、手機 App、中轉站與帳號服務都以 MIT 授權開源，產品規格與設計文件也一起公開。
                {LINKS.github ? '' : '原始碼即將公開。'}
              </p>
              <div style={{ display: 'flex', gap: 10, marginTop: 24, flexWrap: 'wrap' }}>
                {LINKS.github ? (
                  <a className="btn btn--lg" href={LINKS.github}><GitBranch size={16} />在 GitHub 查看</a>
                ) : (
                  <span className="btn btn--lg" aria-disabled="true"><GitBranch size={16} />GitHub<span className="soon">即將公開</span></span>
                )}
              </div>
            </div>
            <div className="card" style={{ fontFamily: 'var(--mono)', fontSize: 13, lineHeight: 1.8 }}>
              <div style={{ color: 'var(--muted)' }}># Claude Code 裡</div>
              <div>/plugin install kunsu</div>
              <div style={{ color: 'var(--muted)', marginTop: 10 }}># AI 需要你拍板時</div>
              <div>ask_create {'{'} question_line: "首屏標題要用哪一版？" {'}'}</div>
              <div style={{ color: 'var(--ok)' }}>→ 已送進 Kunsu，等你回答</div>
              <div style={{ color: 'var(--muted)', marginTop: 10 }}># 你在手機上點了「A 版」</div>
              <div style={{ color: 'var(--ok)' }}>✓ 答案回到原本的對話，Claude 接著做</div>
            </div>
          </div>
        </section>

        <section className="section section--line" id="faq">
          <div className="wrap" style={{ maxWidth: 760 }}>
            <div className="eyebrow">常見問題</div>
            <h2 className="h2">常見問題</h2>
            <div className="faq">
              <details><summary>要錢嗎？</summary><p>免費。帳號服務與中轉站由我們負擔。</p></details>
              <details><summary>需要什麼才能用？</summary><p>一台 Mac，以及 Claude Code 或 Codex。手機 App 是選用的，用 iPhone。</p></details>
              <details><summary>我的題目和答案存在哪裡？</summary><p>只在你的 Mac。雲端只有你的 email 與裝置清單；手機和 Mac 之間端對端加密，中轉站看不到內容。</p></details>
              <details><summary>AI 會不會替我回答或自己批准？</summary><p>不會。AI 用的工具沒有作答或批准的能力，動錢的題目一定要你在電腦或配對的手機上核對後才會執行。</p></details>
              <details><summary>開會或分享螢幕時會跳出來嗎？</summary><p>不會。分享螢幕、錄影、全螢幕簡報、睡眠時間與勿擾期間都會延後，題目與倒數照常保存。</p></details>
              <details><summary>支援 Windows 或 Android 嗎？</summary><p>目前只有 Mac 與 iPhone。</p></details>
            </div>
          </div>
        </section>

        <section className="final section--line">
          <div className="wrap">
            <ShieldCheck size={28} aria-hidden="true" />
            <h2 className="h2" style={{ marginTop: 12 }}>別再讓 AI 等你一整晚。</h2>
            <p className="sub" style={{ margin: '14px auto 0' }}>免費註冊，內容只在你的 Mac。</p>
            <SignupForm id="final-email" />
            <p style={{ marginTop: 18, color: 'var(--muted)', fontSize: 14, display: 'inline-flex', gap: 6, alignItems: 'center' }}>
              <MessageSquareReply size={15} aria-hidden="true" />有問題？寫信到 <a href={`mailto:${LINKS.supportEmail}`}>{LINKS.supportEmail}</a>
            </p>
          </div>
        </section>
      </main>
      <Footer />
    </>
  )
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Landing />
  </StrictMode>,
)
