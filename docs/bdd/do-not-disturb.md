# 勿擾模式（do-not-disturb）— BDD 驗收規格

> 狀態：Current；2026-08-12 完成 Red → Green、installed-app round-trip 與隔離瀏覽器 QA。
> 依據：PRD §6.1、DESIGN「Do Not Disturb」、CONTRACT §3/§4/§9。
> 不變量對應：INV-5、INV-7、INV-17。
> 界外：macOS 系統 Focus 偵測、quiet hours 編輯、問題 snooze、回答與授權語義。

## 背景與範圍

勿擾模式是一個最長 24 小時、可恢復且可稽核的 presentation guard。它讓使用者暫時完全看不到
Kunsu 主動訊息，同時保留 decision inbox 的真相、倒數與安全責任。UI、舊 4 小時快捷鍵及 MCP
共用同一個持久化狀態，不得各自計時。

## 行為規格

### 場景 1：以預設或自訂時長啟用

**Given** 勿擾目前未啟用
**When** 使用者選 1／4／8／24 小時，或輸入 1..1440 分鐘的自訂時長
**Then** server 先持久化 `starts_at/ends_at/actor` 再回 active status；UI 顯示結束時間與剩餘時間，
立即隱藏既有 capsule/overlay，且不發啟用成功的系統通知

### 場景 2：時長驗證與邊界

**Given** 任一 UI、IPC 或 MCP 呼叫者設定勿擾
**When** duration 為 0、負數、非整數或大於 1440 分鐘
**Then** 以可讀 validation error 拒絕，舊狀態不變、不寫部分設定；1440 恰好成功

### 場景 3：完全靜默（INV-17）

**Given** `now < ends_at`
**When** 新題建立、題目進 warned/forced eligibility、批次窗觸發，或 shell reconciliation 執行
**Then** macOS notification、capsule 與 overlay 全部不顯示；main application/menu bar 可由使用者
主動開啟，menu bar 不列題目內容，只顯示待決數與勿擾結束時間

### 場景 4：不改題目生命週期（INV-17）

**Given** 一題在勿擾開始前／期間 pending
**When** 勿擾啟用、延長、縮短、到期、提前解除或 app restart
**Then** 題目的 state 仍可依 wall clock 進 warned/deferred，但 answer/cancel 不被產生，
`cycle_started_at/warn_at/force_at/cycle_number` 位元級不因勿擾操作改變

### 場景 5：到期後恢復

**Given** 勿擾結束時仍有有效題目
**When** 下一次 timer/shell reconciliation 發現 inactive
**Then** 已逾 `force_at` 的最早題在其他 guards 全綠時立即顯示唯一 overlay；未逾時題恢復 capsule
並顯示原倒數；不贈送新 15 分鐘、不補發勿擾期間累積的舊系統通知

### 場景 6：提前結束與冪等

**Given** 勿擾 active 或 inactive
**When** 使用者按「提前結束」或 agent 呼叫 `dnd_clear`
**Then** 狀態持久化為 inactive；重複 clear 仍成功且不改題目；active→inactive 只產生一筆有效變更
audit event，shell 依場景 5 對帳

### 場景 7：restart 與 sleep/wake

**Given** 勿擾 ends_at 已持久化
**When** app 在期間 restart 或 Mac sleep 後於 ends_at 前／後 wake
**Then** 前者保持 active 且不顯示三種主動 surface；後者依 wall clock 視為 inactive 並立即走恢復，
不延長、不重設，也不因 renderer localStorage 遺失而失效

### 場景 8：MCP 可讀、可設、可解除，但不可作答（INV-7）

**Given** agent 持有效 Bearer token
**When** 呼叫 `dnd_get`、`dnd_set {duration_minutes, actor}` 或 `dnd_clear {actor}`
**Then** 回 machine-readable status；set/clear 寫 `dnd_changed` audit，MCP tool 清單變 11；任何回應與
事件都不含 answer receipt，也不能帶 question_id 造成題目狀態轉移

### 場景 9：UI 可用性與可及性

**Given** 使用者只用鍵盤、VoiceOver、reduced-motion 或較長的本地化時間文字
**When** 開啟工具列／設定頁勿擾控制、選時長、遇到錯誤或提前結束
**Then** focus 順序可預測、Escape 關閉 popover、active/ends_at/error 有文字與 aria 狀態；控制與
banner 不造成 toolbar layout shift，動畫停用不影響操作，內容可裁切／換行而不遮住主要動作

### 場景 10：舊 4 小時入口相容

**Given** 使用者按 Cmd+Shift+K、舊 toolbar action 或 menu bar 4 小時入口
**When** shell 呼叫相容 `kill_switch`
**Then** 等價於同一持久化 `dnd_set(240, actor=user)`；不維護第二個 timer，doctor/舊 client 不崩潰

### 場景 11：menu bar 顯示完整狀態與快速設定

**Given** 使用者點 macOS 右上角 Kunsu icon
**When** 勿擾 inactive 或 active
**Then** 清單顯示目前模式、待決數；active 另顯示人話剩餘時間與結束時間且不列題目內容；兩種
狀態皆可直接選 1／2／4／8／12／24 小時，active 可提前結束，「更多時間…」開 app 設定頁；
選 preset 走同一持久化 dnd_set 並立即重建清單

## 驗收清單

- [x] AC-DND-01：1/4/8/24h 與 1..1440m 自訂時長先持久化再生效，active status 完整
- [x] AC-DND-02：0、負數、非整數、>1440 全拒；1440 成功且失敗不改舊狀態
- [x] AC-DND-03：active 期間 notification/capsule/overlay 零顯示，main/menu bar 可主動開且不洩漏題目內容（INV-17）
- [x] AC-DND-04：set/clear/expiry/restart 不改任何題目 cycle/answer/cancel（INV-17）
- [x] AC-DND-05：解除後逾時題立即恢復、未逾時題沿用原倒數，不補發舊通知
- [x] AC-DND-06：clear 冪等，active→inactive audit 唯一且 shell 完成原子對帳
- [x] AC-DND-07：restart/sleep 以持久化 wall-clock ends_at 恢復，不提早解除或延長
- [x] AC-DND-08：MCP 11 tools 含 dnd_get/set/clear，需 auth、可操縱與讀取且絕無 answer 面（INV-7）（2026-10-08 起 12 tools：新增只記錄回饋的 `ask_report`，仍無 answer 面）
- [x] AC-DND-09：keyboard/VoiceOver/reduced-motion/長內容與錯誤狀態 QA 通過
- [x] AC-DND-10：Cmd+Shift+K 與舊 kill_switch 等價 dnd_set(240)，無第二份狀態
- [x] AC-DND-11：menu bar 顯示模式／剩餘時間／結束時間／待決數，提供 1/2/4/8/12/24h、提前結束與更多時間，active 不洩漏題目內容

## QA 證據（2026-08-12）

- Red：`cargo test -p kunsu-server --test integration dnd` 因缺少 `set_dnd/dnd_status/clear_dnd` 編譯失敗；
  UI test 因缺少 `dnd.ts` 失敗。
- Green：`cargo test` 全 workspace 通過；`pnpm test` 83 tests、`tsc --noEmit`、Vite production build 通過。
- Installed app：`scripts/build-install.sh` doctor 全綠，`/describe` 回 11 tools；以安裝版完成
  `dnd_set(1m) → dnd_get(active) → dnd_clear → dnd_get(inactive)`。
- 隔離 UI QA（未使用獨立 Agent）：headed Playwright 驗證 preset、自訂輸入邊界、active 文字/banner、
  提前結束與 Escape 關閉；snapshot 顯示可讀 role/name，截圖確認 toolbar/popover/banner 無遮蔽主要動作。
- Menu bar 補充 QA：`tray::dnd_menu_tests::menu_bar_exposes_complete_dnd_presets_and_human_remaining_time`
  鎖定 1/2/4/8/12/24h 與剩餘時間文案；最終 build-install doctor 再次全綠。

## 測試層級

- Core unit：duration 邊界、active 判斷、GuardState 與題目 cycle 不變量。
- Server integration：SQLite persistence、restart、audit 冪等、MCP auth/schema/read-set-clear。
- Shell/UI unit：三 surface suppression、popover keyboard/aria、狀態文字與 legacy alias。
- End-to-end/manual：active 前已有 overlay、期間新題/批次窗、restart/sleep-wake、到期恢復與 menu bar。
