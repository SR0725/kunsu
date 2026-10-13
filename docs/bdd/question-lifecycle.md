# 題目生命週期（question-lifecycle）— BDD 驗收規格

> 依據：docs/CONTRACT.md §2（資料模型與 server 硬驗證）、§3（狀態機）、§4（ask_create upsert）。
> 不變量對應：INV-1、INV-2、INV-8、INV-11、INV-14。
> 界外：guards 逐情境行為見 forcing-and-guards.md；T0 預算與 per-source 配額見 anti-fatigue.md；
> 作答通道細節見 answer-flow.md。

## 概述

題目由 agent 經 MCP `ask_create` 進入系統，走純函數狀態機
`created → pending → warned → forced → pending ... → answered | expired | cancelled | superseded`，
已回答的題投遞完成（被收割）後標 `routed`；過期、取消、被取代的題保持原終態（歷史與督導要分得出來）。Server 是 schema 的最終守門員（400 拒收、錯誤訊息給 agent 可讀原因）、
app 是 tier 的最終守門員（agent 只給 `requested_tier` 建議值）。所有時限落 SQLite
`next_transition_at` 與 cycle timestamps，server 每秒掃 due rows，重啟從 DB 重建、不靠記憶體 timer。

## 行為規格

### 場景 1：合法建題進入 pending

**Given** Kunsu server 正在運行，agent 持有有效 Bearer token
**When** agent 呼叫 `ask_create`，帶合法 schema：穩定 dedup id（`pa-*` 慣例）、title ≤80 字、question_line ≤24 字決策句、kind/tier/gate 皆在 enum 白名單、evidence ≤3 條且每條有 `source`
**Then** 回傳 `{question_id, state, dedup: "created"}`，題目狀態為 `pending`，`first_proposed` 與 `created_at` 落為當下，`requested_tier` 原樣保存供分級準確率計分

### 場景 2：錢類題缺 prediction_ref 被拒（INV-1）

**Given** agent 建題帶 `money.is_money_op = true`
**When** 該請求缺 `prediction_ref`
**Then** server 回 400 拒收，錯誤訊息以可讀文字說明「錢類題必須帶配對 prediction」；DB 不產生任何題目列

### 場景 3：錢類題過期預設永不 pick_option（INV-2）

**Given** agent 建題帶 `money.is_money_op = true`
**When** `default_action.on_expire == "pick_option"`
**Then** server 回 400 拒收——錢永不因超時自動動；schema 層即擋下，不落 DB

### 場景 4：紅燈題必為多選項（INV-11）

**Given** agent 建題帶 `gate = "red"`
**When** `kind != "choice"`（如 confirm 或 text）
**Then** server 回 400 拒收，理由寫明紅燈必須多選項拍板；`kind = "choice"` 但 options 不在 2..=4 範圍時同樣拒收

### 場景 5：十秒可決 lint 與證據身世硬驗證

**Given** agent 建題
**When** 出現以下任一情況：(a) evidence 任一條缺 `source` 身世標籤；(b) `question_line` 超過 24 字；(c) `kind = "choice"` 而 options 為空；(d) 動錢題（`money.is_money_op`）缺 `params_line`；(e) 任一 enum 欄位不在白名單
**Then** server 一律 400 拒收並標 `malformed`，錯誤訊息逐項指出缺什麼——爛問題在生產端被擋、不落到使用者面前

### 場景 6：同 id upsert 去重不重置任何倒數（INV-8）

**Given** 題目 `pa-x` 已存在且處於 pending，`first_proposed` 為三天前，15 分鐘強制倒數已走到一半
**When** agent 以同 id 再次呼叫 `ask_create`（例如隔天排程重跑）
**Then** 回傳 `{dedup: "touched_existing"}`；只更新 `last_seen` 與證據內容；`first_proposed` 不動（積壓天數誠實）、強制倒數不重置、不發任何新系統通知、`next_transition_at` 不變

### 場景 7：app 是 tier 最終守門員——不合格 T0 程式化降級

**Given** agent 建題 `requested_tier = "t0"`，但不滿足 T0 四條件（動錢或不可逆／證據包完整／真實時限／今日預算未滿）之一
**When** server 收單
**Then** 題目以降級後的生效 `tier`（如 t1）入庫，`requested_tier` 保留原值；降級原因回寫給 agent 可讀；該 source 的分級準確率被記錄計分

### 場景 8：合格問題未答 12 分鐘進入警告

**Given** 一題非 internal 合格問題處於 pending，不限 tier
**When** 建題或新週期開始後 12 分鐘（policy `cycle_min - warn_before_min`）仍未作答
**Then** 狀態轉 `warned`，顯示「3 分鐘後將開啟強制詢問」；tier、每日預算與 guards 都不得阻止 warning

### 場景 9：第 15 分鐘轉蓋板或 deferred

**Given** 題目處於 `warned`
**When** 本輪 `force_at` 到且仍未作答
**Then** guards 全綠時狀態轉 `forced` 並顯示全螢幕 overlay；guard 命中時留在 `warned`/deferred，解除後立即 forced

### 場景 10：關閉未回答介面開始新 15 分鐘週期

**Given** 題目仍未回答且 overlay 顯示中
**When** 使用者關閉、按 Esc 或選擇稍後
**Then** dismissal 先落事件，狀態回 `pending`，`cycle_number` 加一，保存新的 12/15 分鐘時間戳並關閉 overlay

### 場景 11：dismissal 次數不限（INV-4）

**Given** 同一題已被關閉任意次數且仍有效未回答
**When** 新一輪 15 分鐘到期且 guards 全綠
**Then** 再次進 `forced`；不得因次數降級、歸檔或停止循環，直到回答或其他合法 terminal condition

### 場景 12：任一通道作答即 answered

**Given** 題目處於任一非終態（pending/warned/forced/legacy snoozed）
**When** 答案從任一合法通道落地（UI IPC / kongko 事件 / Discord 讀回 / CLI isatty）
**Then** 狀態轉 `answered`，記 `answer.channel` 與 `answered_at`；若正在蓋板或警告，浮條/蓋板即刻收掉；終態 append ledger，投遞完成後標 `routed`

### 場景 13：deadline 到期執行出生時宣告的預設

**Given** 題目帶 `deadline_at` 與 `default_action`
**When** deadline 到期仍未作答
**Then** 狀態轉 `expired` 並執行 `default_action` effect：`noop_log` 只落 ledger 留痕；`escalate_discord` 推 Discord 鏡像；`pick_option` 以 `option_id` 記為自動選定（channel = `auto_policy`）——僅非錢題可能走到這裡（錢題在場景 3 已被 schema 擋死）

### 場景 14：T1 題 48 小時未答走預設

**Given** 一題 T1 處於 pending
**When** 48 小時（policy `t1_default_after_h`）未作答
**Then** 依 `default_action` 處理：非錢題執行宣告的預設；錢題歸檔並 Discord 留痕，絕不自動動錢

### 場景 15：T2 題 14 天自動 superseded（INV-14）

**Given** 一題 T2 處於 pending
**When** 距 `first_proposed` 滿 14 天（policy `t2_supersede_after_d`）未處理
**Then** 狀態自動轉 `superseded`；任何題的 pending 壽命 ≤ 14 天、到期必轉終態——「積壓 30 天」在機制上不可能存在

### 場景 16：cancel / supersede 不刪、留軌跡

**Given** 題目處於非終態
**When** agent 呼叫 `ask_cancel` 或 `ask_supersede`
**Then** 狀態轉 `cancelled` / `superseded`，DB 列與 events 轉移史保留、ledger append 終態快照；`ask_get` 仍可查到完整稽核鏈

### 場景 17：重啟後 timer 從 DB 完整重建

**Given** 多題各自帶未來的 cycle timestamps、`next_transition_at` 與 deadline
**When** app 被殺掉後重啟
**Then** server 從 DB 重建所有排程，每秒掃描 due rows；重啟期間已 due 的轉移依 guards 補觸發，不丟失、不重複、不重新贈送 15 分鐘

## 驗收清單

- [ ] AC-QL-01：合法建題回 `{question_id, state, dedup:"created"}` 並進入 pending
- [ ] AC-QL-02：`money.is_money_op` 缺 `prediction_ref` 必 400 拒收（INV-1，測試 `inv_money_requires_prediction`）
- [ ] AC-QL-03：錢類題 `on_expire=pick_option` 必 400 拒收（INV-2，測試 `inv_money_never_auto_expire`）
- [ ] AC-QL-04：`gate=red` 且 `kind!=choice`、或 choice options 不在 2..=4 必 400 拒收（INV-11）
- [ ] AC-QL-05：evidence 缺 source／question_line >24 字／choice 無 options／動錢題無 params_line／非法 enum 全部 400 並標 `malformed`，錯誤訊息可讀
- [ ] AC-QL-06：同 id upsert 只 touch `last_seen`/證據，不重置倒數、不發新通知、`first_proposed` 不動，回 `touched_existing`（INV-8，測試 `inv_upsert_no_reset`）
- [ ] AC-QL-07：不合格 T0 由 app 程式化降級並回寫原因，`requested_tier` 保留供計分
- [ ] AC-QL-08：所有合格題 pending →(12min)→ warned →(15min 且 guards 綠)→ forced 全鏈可觸發
- [ ] AC-QL-09：forced/手動 overlay →未回答 dismissal→ pending + cycle_number++ + 新完整 15min
- [ ] AC-QL-10：dismissal 次數不限，不降級、不歸檔，仍可再次 forced（INV-4，測試 `inv_dismiss_restarts_cycle`）
- [ ] AC-QL-11：四通道任一作答即 answered，蓋板/浮條即刻收掉，收割後 routed；過期、取消、被取代收割後保持原終態
- [ ] AC-QL-12：expired 依 default_action 正確產生 effect；`pick_option` 自動選定記 channel=auto_policy
- [ ] AC-QL-13：T1 48h 未答走宣告預設（錢題只歸檔留痕）；T2 14 天自動 superseded（INV-14，測試 `inv_max_pending_age`）
- [ ] AC-QL-14：cancel/supersede 保留完整軌跡，不刪列
- [ ] AC-QL-15：重啟後 timer 由 DB `next_transition_at` 完整重建，due 轉移補觸發不丟失
- [ ] AC-QL-16：狀態機為 core 純函數（零 I/O），表驅動測試覆蓋全轉移矩陣
