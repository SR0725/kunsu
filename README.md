# Kunsu（軍師）

**AI 卡住要你決定時，它會叫醒你。**

Kunsu 是給 Claude Code 與 Codex 使用者的決策收件匣。AI 需要你拍板時，問題連同選項與證據送進 Kunsu，
出現在螢幕邊緣、15 分鐘內一定帶到你面前；你回答後，答案回到發問的那段對話，AI 接著做。
不在電腦前，也能用 iPhone 回答。

官網：<https://kunsu.app>

## 特色

- **一個地方收齊**：所有 AI 在等你的事，依急迫程度排好。
- **答案回到原本的對話**：發問的 Claude／Codex 對話直接接續。
- **不替你作答**：AI 用的工具沒有作答或批准的能力；關掉視窗不等於回答。
- **動錢一定先核對**：金額與參數所見即所簽，批准只對這一次有效。
- **內容只在你的 Mac**：手機與 Mac 之間端對端加密（X25519、ChaCha20-Poly1305），中轉站只轉送密文；
  雲端只有 email 與裝置清單。

## 專案結構

| 路徑 | 內容 |
|---|---|
| `src-tauri/` | Mac App（Tauri） |
| `crates/kunsu-core` | 題目狀態機、路徑與核心型別 |
| `crates/kunsu-server` | 本機 HTTP／MCP 服務、帳本、手機橋接 |
| `crates/kunsu-cli` | `kunsu` 指令列工具與 doctor |
| `crates/kunsu-mobile` | 手機配對與加密協定 |
| `ui/` | Mac App 介面（React） |
| `mobile/` | iPhone App（SwiftUI）與 `KunsuCore` Swift 套件 |
| `relay/` | 中轉站與帳號服務（Node） |
| `web/` | 官網 kunsu.app |
| `plugins/kunsu/` | Claude Code／Codex 外掛 |
| `docs/` | 產品規格（PRD）、工程契約、BDD 驗收條件、設計文件 |

## 開發

需要 Rust、Node 20+、Xcode 26（iPhone App）。

```bash
cargo test --workspace          # Rust 測試
(cd ui && npm install && npx vitest run)
(cd relay && npm install && node --test test/*.test.js)
(cd mobile/KunsuCore && swift test)
scripts/build-install.sh        # 建置並安裝到 /Applications，跑 doctor
```

開發流程：文件 → BDD → 先紅的測試 → 實作 → QA，詳見 [`CLAUDE.md`](CLAUDE.md)。
不能被破壞的安全性質列在 [`INVARIANTS.md`](INVARIANTS.md)，每條都有同名測試。

## 安全問題

請寄到 support@kunsu.app，不要開公開 issue。

## 授權

[MIT](LICENSE)
