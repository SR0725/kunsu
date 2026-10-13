# BDD：多使用者第一階段（陌生人的 Mac 也能用、帳號串起 Mac 與手機、使用者之間完全隔離）

> PRD §31.2、§31.3；設計 `docs/design/accounts.md`。2026-10-13 Ray：分階段——先驗證整個產品轉多人沒問題。

## 範圍

本階段只證明「多人」成立：資料位置不再綁作者、Mac 與手機用帳號登入、每台 Mac 用自己的中轉站憑證、不同使用者之間收不到彼此任何東西。
官網、App Store、Google／Apple 登入的正式憑證、推播在下一階段。

## 場景

### 1. 資料位置
**Given** 一台沒有舊資料夾（`paths::LEGACY_VAULT`）的 Mac　**When** 第一次啟動 Kunsu　**Then** 資料放在 `~/Kunsu`（帳本 `~/Kunsu/logs/asks`），不碰任何寫死的個人路徑。
**Given** `~/.kunsu/config.json` 指定了資料夾　**Then** 用那個資料夾。**Given** 環境變數 `KUNSU_VAULT_ROOT`　**Then** 以它為準。
**Given** 作者原本的 Mac（舊資料夾存在、沒有設定檔）　**Then** 繼續用原本的資料夾，不搬資料。

### 2. 選用整合
**Given** 資料夾裡沒有 Meta 廣告的 Discord 設定　**Then** Discord 告警安靜地關閉，Kunsu 其他功能正常。

### 3. Mac 登入帳號
**When** 在主視窗「設定 › 帳號」輸入 email 寄登入連結並點連結　**Then** Kunsu 登入、把這台 Mac 註冊到帳號、拿到自己的中轉站憑證，
之後用它上線（不再需要共用的 `RELAY_MAC_TOKEN`）；登出或在帳號裡移除這台 Mac → 中轉站立刻斷線。

### 4. 手機登入帳號
**When** 手機第一次打開　**Then** 先登入（email 連結；Google、Apple 在下一階段接上正式憑證），登入後看到自己的 Mac 清單，配對仍要掃 QR 並在 Mac 上按允許。
**When** 在手機設定按「刪除帳號」並確認　**Then** 帳號刪除、手機回到登入畫面。

### 5. 使用者之間隔離
**Given** 使用者 A 與 B 各有一台 Mac 與一支手機　**Then** A 的 Mac 憑證不能用 B 的 mac_id 上線；A 的手機送給 B 的 Mac 的封包被 B 的 Mac 拒收；
B 的 Mac 送出的封包只會到 B 的裝置；A 看帳號只看得到自己的裝置。

## 驗收清單

- [ ] AC-MU-01 資料位置：環境變數 > 設定檔 > 舊資料夾（存在時）> `~/Kunsu`；沒有任何寫死個人家目錄（`/Users/<某人>`）的程式碼路徑（Rust 單元測試＋grep 檢查）
- [ ] AC-MU-02 Discord 設定不存在時告警關閉、不出錯（Rust 單元測試）
- [ ] AC-MU-03 Mac 登入、註冊、用自己的憑證上線、移除後斷線（server 整合測試＋實機）
- [ ] AC-MU-04 手機登入、看到自己的 Mac、刪除帳號（Swift 單元測試＋模擬器）
- [ ] AC-MU-05 兩個使用者完全隔離（中轉站＋帳號整合測試，兩組 Mac／手機）
- [ ] AC-MU-06（2026-10-13）從 DMG 裝的 Mac 第一次打開就自己準備好：`~/.kunsu/bin/kunsu`（跟 App 同一版，版本不同就更新）、`policy.toml`
  （沒有才放，不蓋掉使用者的）、playbooks（缺的才補）、外掛放到 `~/.kunsu/plugins/kunsu`（App 管理，每次同步）、從 /Applications 執行時寫開機自動啟動；
  重複執行不改任何已存在的使用者檔案；修復指令改指 `~/.kunsu`，不再指 repo（src-tauri 單元測試）
