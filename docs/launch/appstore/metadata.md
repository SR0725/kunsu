# App Store 上架資料（Kunsu iPhone）

送審時照這份填 App Store Connect。截圖在 `screens/`（iPhone 17 Pro Max 模擬器，1320×2868，符合 6.9 吋規格）。

## App 資訊

| 欄位 | 內容 |
|---|---|
| 名稱 | Kunsu — AI 決策收件匣 |
| 副標題（30 字內） | AI 卡住要你決定時叫醒你 |
| Bundle ID | com.rayrealms.kunsu.mobile |
| SKU | kunsu-ios |
| 主要語言 | 繁體中文 |
| 類別 | 開發者工具（次要：生產力） |
| 價格 | 免費，無 App 內購買 |
| 年齡分級 | 4+（問卷全部選「無」） |
| 版權 | 2026 Ray Wu |
| 支援網址 | https://kunsu.app/support |
| 行銷網址 | https://kunsu.app |
| 隱私權政策 | https://kunsu.app/privacy |

## 宣傳文字（170 字內，可隨時改）

Claude Code 和 Codex 需要你拍板時，問題會出現在這裡。躺在床上點一下，AI 就接著做。

## 描述

Kunsu 是 AI Agent 的決策收件匣。

你讓 Claude Code、Codex 跑長任務，它們常常停在一句「要用哪一版？」，然後整晚在等你。Kunsu 把這些需要你決定的問題收在同一個地方，一定帶到你面前；你回答後，答案回到發問的那段對話，AI 接著做。

iPhone App 讓你不在電腦前也能回答：

• 一次一題：打開就只有這次要處理的事，點一下就回答。
• 先問清楚再決定：按「為什麼？」「前因後果？」，軍師讀得到題目的脈絡，答不了就幫你問發問的 AI。
• 5 秒內可以撤回：撤回前不會送到 Mac。
• 動錢一定先核對：金額與參數所見即所簽，批准只對這一次有效。
• 答完可以聊天：問大家在做什麼，或從手機開新任務。

隱私：
• 題目、答案與對話只存在你的 Mac。
• 手機與 Mac 之間端對端加密（X25519、ChaCha20-Poly1305），中轉站只轉送看不懂的密文。
• 雲端只有你的 email 與裝置清單，可以在 App 內刪除帳號。
• 程式碼以 MIT 授權開源。

需要：一台裝了 Kunsu 的 Mac（免費，kunsu.app 下載），以及 Claude Code 或 Codex。沒有 Mac 也可以先用示範資料看看。

## 關鍵字（100 字元內，逗號分隔）

AI,Claude,Codex,agent,決策,通知,收件匣,開發者,coding,自動化,生產力,遠端,審核

## 審查備註（App Review Information）

- 登入：不需要帳號就能審查。登入畫面最下方點「先用示範資料看看」，會進入完整的示範資料（5 個待決題目、回答、撤回、往下拉看歷史紀錄、聊天）。示範資料不會送到任何地方。
- 實際使用需要使用者自己的 Mac 上跑 Kunsu 桌面 App（https://kunsu.app 免費下載），手機用 QR code 與 Mac 配對，之後透過端對端加密的中轉站收發題目。
- Email 登入：輸入 email 後會收到一次性登入連結（15 分鐘內有效），不需要密碼。
- 刪除帳號：設定 › 帳號 › 刪除帳號（Guideline 5.1.1(v)）。
- 相機只用來掃 Mac 上的配對 QR code。
- 聯絡：support@kunsu.app

## App 隱私（營養標示）

| 資料類型 | 收集？ | 用途 | 連結到身分 | 追蹤 |
|---|---|---|---|---|
| 聯絡資訊 › 電子郵件地址 | 是 | App 功能（登入） | 是 | 否 |
| 其他所有類型 | 否 | — | — | — |

題目、答案與對話是端對端加密在使用者自己的裝置之間傳送，開發者無法讀取，依 Apple 定義不屬於「收集」。

## 出口合規

`ITSAppUsesNonExemptEncryption = NO`：只使用作業系統與標準演算法做端對端加密（豁免類別），不需要上傳文件。

## 送審前檢查

- [ ] App Store Connect 建立 App 紀錄（上面的名稱、Bundle ID、SKU）
- [ ] 上傳 build（`scripts/release-ios.sh`）
- [ ] 截圖 `screens/01-light.png`、`02-dark.png`、`03-login.png`
- [ ] 填描述、關鍵字、網址、審查備註、隱私營養標示
- [ ] 送審
