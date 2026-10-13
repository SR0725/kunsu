# 排程督導（routine-check-in）— BDD 驗收規格

> 狀態：**已實作，待 QA**（2026-10-08）。端對端（真的 Claude 排程任務與 Codex automation 各跑一次）需使用者手動驗證。
> 產品依據：`docs/PRD.md` §1.4、§21；視覺依據：`DESIGN.md`「Routines view」。
> 依賴：conversation-continuity.md（答案回到排程那段對話、`ask_report` 回饋）。
> 界外：Kunsu 自建排程或提醒時鐘（紅線）、在 Kunsu 保存目標正本、聊天式來回討論。

## 背景與範圍

使用者想讓 AI 督促自己的長期承諾，例如每天寫日記、每週檢討目標進度。排程本身交給 Claude Routines
（排程任務）與 Codex automations；Kunsu 只負責「排程時間到了，AI 來要你的回應時，你逃不掉」，並保存
每一次的回應與 AI 回饋，讓使用者能回頭看自己有沒有做到。

目標與承諾的正本留在 vault 或排程 prompt；Kunsu 只保存每次 check-in 的題目、回答、AI 回饋與交付狀態，
全部可由 ledger 重建。

## 行為規格

### 場景 1：系統認得這是排程呼叫

**Given** Claude 排程任務或 Codex automation 執行時以 `kunsu ask --routine <id> --routine-name <名稱>
[--every <小時>]` 建題
**When** server 收單
**Then** 題目保存 `source.trigger = scheduled`、`routine_id`、`routine_name`、`routine_kind`（claude_scheduled_task｜
codex_cron｜codex_heartbeat｜other，可由環境推得時自動帶入：Claude 排程任務可推得；Codex 的 cron 與 heartbeat
從環境分不出來，不自動猜，要靠 `--routine-kind`）與 `expected_every_hours`；沒有 routine_id 的題目
`trigger = interactive`。所有建題路徑（含 Kunsu 自己建的題）都套用這個預設；agent 自填的 trigger 優先。
`routine_id` 以 `kunsu-` 開頭保留給 Kunsu 自己的題（doctor 冒煙題、Kongko 啟用題），一律 `interactive`（agent 自填
`scheduled` 也改掉）

### 場景 1b：宣告頻率必須合理

**Given** 建題帶了 `expected_every_hours`
**When** 值不是有限的正數（0、負數、NaN、無限大）
**Then** server 以 400 拒收並說明欄位；CLI 的 `--every` 解析不成正數時（例如 `24h`）、`--every`／`--routine-name`／
`--routine-kind` 沒給值、或沒搭配 `--routine` 時直接報錯、不送出，不會靜默變成「未宣告頻率」；`kunsu ask --json` 也拒絕
不認得的旗標（含 `--every=24` 這種寫法與拼錯的旗標），`--routine kunsu-…`（保留前綴）直接報錯

### 場景 2：check-in 和其他題一樣逃不掉

**Given** 一題排程 check-in（例：「今天的日記：寫下今天最重要的三件事」，kind = text）
**When** 建題後時間經過
**Then** 走同一個 15 分鐘強制循環、三分鐘警告、guards 與勿擾規則；不因為是排程就降級或略過

### 場景 3：每次排程一題、重送不重複

**Given** 同一 routine 同一次執行因重試送了兩次
**When** 兩次都以 `rt-<routine_id>-<YYYYMMDD>[-<HHmm>]` 為 id 建題
**Then** 只有一題（沿用 upsert 不重置規則）；不同日期是不同題

### 場景 4：回答回到那次排程的對話並得到回饋

**Given** 使用者回答了 check-in
**When** 答案依 conversation-continuity 的階梯送回那次排程的對話
**Then** agent 讀到答案、完成它的工作（例如寫入 vault 日記、提出下一步建議），並以 `ask_report` 回傳 AI 回饋；
督導頁該次紀錄顯示回饋全文

### 場景 5：督導頁列出每個排程

**Given** 曾經出現過至少一個排程 check-in：有 routine_id、不是 internal、routine_id 不是 `kunsu-` 保留前綴、
`trigger = scheduled`，而且宣告了名稱（`routine_name`）或頻率（`expected_every_hours`）。只帶 routine_id 的營運排程
決策題（例如每日行銷排程送來的加碼題）不是 check-in，留在待決與歷史，不進督導頁
**When** 使用者在主 app 開啟「督導」
**Then** 只有排程 check-in 出現（doctor 冒煙題、Kongko 啟用題等 Kunsu 自己的題不出現）；每個 routine 一列：名稱、來源圖示（Claude／Codex）、最近一次報到時間、宣告的頻率、最近一次狀態
（待回應／已回應／未回應／退回／已取消），以及近 30 天已回應／未回應／退回次數；依最近報到時間排序。每列是清單中的一個
按鈕（screen reader 聽得到是按鈕），來源圖示有可讀名稱（Claude／Codex）

### 場景 6：查看過去的回應與 AI 回饋

**Given** 使用者選了一個 routine
**When** 歷史清單載入
**Then** 由新到舊列出每次 check-in：時間、`question_line`、使用者的完整回答、AI 回饋（sanitized Markdown；尚未回報時
顯示「AI 尚未回報」）、交付狀態；過期或取消的也列出並標「未回應」「已取消」，不隱藏；使用者按「這題不該問我」推掉的標「退回」（不是「已取消」），
回答欄顯示「你退回了這題：<原因>」（沒寫原因就是「你退回了這題」）；長內容可展開，
清單可只用鍵盤操作。逾時由 Kunsu 依預設自動選定的不算使用者的回答：回答欄顯示「沒有回答（逾時依預設：<選項>）」，
計入未回應。長內容（含沒有空白的長網址）自動斷行，頁面不出現水平捲動。進入歷史時焦點移到標題，返回列表時
焦點回到原來那一列

### 場景 7：排程可能沒在跑

**Given** routine 宣告了 `expected_every_hours`
**When** 距最近一次報到超過宣告頻率的兩倍
**Then** 督導頁該列標示「排程可能沒在跑：已 N 小時沒報到，請到 Claude／Codex 檢查排程設定」；不發系統
通知、不出現在瀏海或蓋板（不是 actionable question）

### 場景 8：不是聊天介面

**Given** 使用者在督導頁看一筆過去的 check-in
**When** 他想追問或補充
**Then** 頁面沒有回覆框；提供「打開原對話」到 Claude App／Codex App 繼續討論；Kunsu 內一次 check-in 永遠只有
一題、一個回答、一則 AI 回饋

### 場景 9：資料可重建、不搬真相

**Given** SQLite 被刪除
**When** 執行 `kunsu rebuild-db`
**Then** routine 身分、回答與 AI 回饋都能由 ledger＋OPEN.json 還原，督導頁內容一致；Kunsu 不保存目標本身。
排程收割（`ask_answers_since`／`kunsu drain`）前後一致：沒回答的 check-in 收割後仍是「未回應」「退回」或「已取消」，不會變成
「待回應」，30 天統計不變；「退回」靠題目上的 `rejected` 快照，重建後不會變回「已取消」

## 驗收清單

- [ ] AC-RC-01：排程呼叫保存 trigger／routine_id／routine_name／routine_kind／expected_every_hours；其他題（含 Kunsu
  自己建的題、`kunsu-` 保留前綴）為 interactive；agent 自填的 trigger 優先
- [ ] AC-RC-01b：`expected_every_hours` 非有限正數時 server 400；CLI `--every` 解析失敗、旗標缺值或沒搭配 `--routine`
  直接報錯
- [ ] AC-RC-02：check-in 走同一強制循環、guards 與勿擾，不降級
- [ ] AC-RC-03：`rt-<routine>-<日期>[-時間]` 重送 upsert 不重複
- [ ] AC-RC-04：答案回到排程對話，`ask_report` 回饋出現在該次紀錄
- [ ] AC-RC-05：督導頁只列宣告為 check-in 的排程題（scheduled＋名稱或頻率），每 routine 一列，含名稱、圖示（有可讀名稱）、最近報到、頻率、最近狀態與
  30 天統計（已回應／未回應／退回）；列是清單中的按鈕
- [ ] AC-RC-06：歷史由新到舊含時間、題目、回答、AI 回饋、交付狀態；未回應／取消不隱藏；使用者推掉的標「退回」、不算已取消；逾時自動選定不算回答；
  長內容不造成水平捲動；鍵盤可操作，進出歷史時焦點落在標題／原來那列
- [ ] AC-RC-07：超過兩倍頻率未報到只在督導頁標示，不推播、不進瀏海或蓋板
- [ ] AC-RC-08：無回覆框，提供打開原對話；一次 check-in 只有一問一答一回饋
- [ ] AC-RC-09：rebuild-db 與收割前後督導資料一致；不保存目標正本

## 測試對應

- Core／server：source 新欄位驗證與 serde、id 慣例 upsert、ledger 快照含 routine 欄位、rebuild 後一致、
  routine 統計與「可能沒在跑」判斷為純函數（clock 可注入）。
- UI（vitest）：督導頁成員規則、分組、排序、狀態標籤、逾時自動選定、空狀態、AI 回饋 sanitizer、按鈕語意與焦點。
- UI（瀏覽器 QA）：長內容斷行、無水平捲動。
- 端對端：以 Claude 排程任務與 Codex automation 各跑一次 check-in → 回答 → 原對話回饋 → 督導頁顯示。
- agent 端：排程 prompt 範本（寫日記、每週目標檢討）放在 kunsu skill，教 routine 帶 `--routine` 參數。
