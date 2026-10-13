# 錢的授權鏈（money-approval）— BDD 驗收規格

> 依據：docs/CONTRACT.md §6（授權鏈全文）、§5（/approvals/verify）、§7（錢的結果、人工對帳、重新授權、`money_state`）、§9（回執）；設計正本 §8。
> 不變量對應：INV-1、INV-2、INV-9、INV-10、INV-11、INV-13、INV-15、INV-25。
> 界外：建題時的錢類 schema 拒收見 question-lifecycle.md 場景 2–3；選項編輯紅線重檢見
> answer-flow.md 場景 3。executor.py `--approval-file` 分支由 integration agent 在
> executor 所在的 vault repo 實作、獨立 commit——本檔場景 7–9 是對該端的驗收要求。

## 概述

Kunsu 永不成為第二條動錢通道：app 點擊只產生帶參數雜湊的授權記錄，執行永遠走 executor.py 護欄。
核心設計是**把 prepare 移到點擊當下**（token 新鮮期從「週」壓到「秒」），authorization =
HMAC 簽章的 approval record，效力嚴格限於單筆參數雜湊、單次消耗、15 分鐘 TTL。
一鍵通道只覆蓋 gate=yellow；red 題永不一鍵。

## 行為規格

### 場景 1：建題只帶 prepare 命令、不預先執行

**Given** agent 建一題動錢黃燈題，帶 `money.executor_prepare_cmd`（完整 prepare 命令字串）、`params_line`、`prediction_ref`
**When** 題目入庫直到使用者作答之前
**Then** server 不執行 prepare 命令、不產生任何 executor token——避免 token 先過期（舊系統 TTL 24h vs 積壓週級的死結）

### 場景 2：點批准當下 prepare 並簽章

**Given** 使用者在決策面板選了一題黃燈動錢題的執行型選項（AI 建議），按「批准並執行」
**When** server 處理批准
**Then** 依序發生：(1) 當下 spawn `executor_prepare_cmd`（含 guard 重驗與帳戶漂移 GET 檢查，全用 executor.py 現成邏輯）；(2) 解析 stdout 取新鮮 executor token；(3) 組 approval record `{question_id, executor_token, param_hash, answered_at, nonce}`，其中 `param_hash = sha256(params_line 正規化)`；(4) 以 Keychain（service `me.ray.kunsu.approval-hmac`）的 key 做 HMAC-SHA256 簽章；(5) 落檔 `<vault 根>/projects/meta-ads/data/executor_pending/approvals/<question_id>.json`

### 場景 3：prepare 失敗不產生授權

**Given** 使用者點批准
**When** prepare 命令非零退出或 guard 擋下（如冷卻期未滿、帳戶漂移）
**Then** 不產生 approval record、不落檔；卡片轉紅顯示 prepare 失敗人話原因、題目標 `execution_blocked`；使用者的點擊不會變成懸空授權

### 場景 4：/approvals/verify 首次通過並當場消耗（INV-9）

**Given** 一份剛簽出的合法 approval record
**When** executor 端 `POST /approvals/verify`（Bearer）帶 `{approval_record}`
**Then** 回 `{valid: true, consumed_now: true}`；消耗記錄同時落 DB 與 ledger——驗證走 app 活體 DB，不是純密碼學離線驗證

### 場景 5：同一 record 第二次 verify 必拒（INV-9）

**Given** 場景 4 的 record 已被消耗
**When** 任何人再次以同一 record 呼叫 `/approvals/verify`
**Then** 回 `{valid: false, reason}`——單次消耗，重放無效

### 場景 6：TTL 15 分鐘後 verify 必拒（INV-10）

**Given** 一份未消耗的 approval record，`answered_at` 距今已超過 15 分鐘
**When** `POST /approvals/verify`
**Then** 回 `{valid: false, reason: TTL 過期}`；批准需使用者重新在卡片上點一次（重新 prepare、重新簽）

### 場景 7：param_hash 綁定——批的是那組參數不是那句話

**Given** executor 端讀到 approval record 並通過簽章/單次/TTL 驗證
**When** executor 以 prepare token 對應的實際執行參數重算 sha256
**Then** 與 record 內 `param_hash` 不符即拒絕執行——使用者批的永遠是確切參數；選項被編輯過的題，簽出的 hash 對應編輯後的 `params_line`（answer-flow.md 場景 3）

### 場景 8：篡改與偽造必被拒

**Given** 一份 approval record 的任一欄位（question_id/token/param_hash/answered_at/nonce）被修改，或由不持 Keychain key 的程式偽造
**When** `POST /approvals/verify`
**Then** HMAC 驗章失敗回 `{valid: false}`；Keychain key ACL 僅限 app 本體讀取——簽名只可能產生於使用者登入的 Mac 上的一次物理點擊（isatty 閘的密碼學等價物）

### 場景 9：executor 護欄一筆不省

**Given** approval record 全部驗證通過、reactor 跑 `executor.py … commit --approval-file <path>`
**When** commit 執行
**Then** executor 既有護欄原樣執行且獨立於 Kunsu：NT$2,000 紅線、≤20% 偏離、3 天冷卻、TWD 50,000 硬上限；任一護欄擋下即拒絕 fire——就算 app 被整個攻破，這層仍是獨立最後防線；**舊 isatty 通道一行不動**（加法不是替換）

### 場景 10：red 題永不一鍵（INV-11）

**Given** 一題 `gate = red`（建題即紅、或黃燈被編輯升紅）
**When** 使用者在 UI 檢視與作答
**Then** 卡片不顯示一鍵執行按鈕；作答只落 ledger 記錄拍板結果，不 prepare、不簽 approval record；後續執行走使用者本人終端（isatty）或互動 session——一鍵通道只覆蓋 yellow

### 場景 11：單次點擊效力不外溢

**Given** 一筆 approval 已被成功消化執行
**When** 任何 agent 之後消化 answers 或引用該批准
**Then** 該批准效力嚴格限於該筆 param_hash：不構成 session 授權、不構成方向性授權、不成為無人值守排程的預設；一條 approval 只准動一筆，聯想擴權＝違規（MARKETING-OS §3.1 增補條文語義）

### 場景 12：錢的結果以每一次授權追蹤，使用者在回執人工對帳（2026-10-09 QA continuity 第 10 輪）

**Given** 一題黃燈錢題的某一次授權（nonce）已被 executor 消耗（`approvals_consumed`），卻沒有錢鏈結果，或結果是 `EXEC_UNKNOWN`
**When** agent 讀題、重建資料庫、使用者打開回執
**Then** 這題「結果不明」：agent 讀到「結果不明：…等使用者人工對帳」、重新授權被拒、重建列人工對帳；回執有「我核對過：錢已經動了」
「我核對過：錢沒有動」兩鈕（各自再確認一次）；標「沒有動」後可以重新授權，標「已經動了」永不能；帳本有行、重建後一致
（integration：`money_outcomes.rs`；UI：`moneyActions.test.tsx`）

### 場景 13：舊資料不會被誤判結果不明（2026-10-09 錢 QA 第 11 輪，F1）

**Given** migration 0007 之前已 `COMMIT_OK`（或已人工對帳 `RECONCILED_EXECUTED`）的錢題：`approvals_consumed` 有那次的 nonce，
`money_outcomes` 沒有（舊資料庫升級，或用沒有 `money_outcome` 行的舊帳本重建）
**When** Kunsu 啟動，或 `rebuild-db` 重放完帳本
**Then** 那個 nonce 補上同樣開頭的結果，並寫一行帶 `money_outcome`（`backfill: true`）的帳本行；agent 讀到「已執行完成」；重建不列人工對帳、
不發 Discord；補資料冪等（再啟動、再重建都不重複寫帳本）。同一題有兩次以上未記結果的授權不補（較早那次的結果真的不明）。執行紀錄是其他開頭的（例如 8/6 那題的 `EXEC_FAILED: 死信…`）不補，
仍是結果不明、回執有對帳鈕（integration：`qa_money_round11.rs` R11-1／R11-2／R11-3）

### 場景 14：回執依伺服器算好的錢的狀態給按鈕（F2、F3、F9、F10）

**Given** 一題已作答的錢題，shell 送給網頁的題目帶 `money_state`（`running`／`unknown`／`reauthorizable`／`settled`）
**When** 使用者在作答當下的回執、失敗畫面或從歷史紀錄回看
**Then** `unknown` → 兩個對帳鈕，失敗畫面標題「結果不明」、寫「executor 已用掉授權但沒有回報結果，錢可能已經動過，請先到廣告後台核對再對帳」；
`reauthorizable` → 「重新授權並執行」，失敗畫面才寫「修好原因後可以在這裡重新授權」；`running`／`settled` → 沒有按鈕。
8/6 那類 `EXEC_FAILED` 死信卻結果不明的題同樣有對帳鈕。題目更新（回執輪詢、清單更新）時按鈕跟著最新的題目；作答當下收到執行
失敗事件後重新讀題，結果不明就走回執＋對帳鈕。「再確認一次」打開時焦點在「確定」，取消回到原按鈕，送出成功後焦點在「完成／
下一題」。錢那一行的圖示：已執行 ✓、乾跑或沒有執行 –、結果不明 !、還在跑 …，都有文字（UI：`qaMoneyRound11.test.tsx`、
`moneyActions.test.tsx`；shell：`to_ui` 附 `money_state`；瀏覽器 mock 實測焦點與即時更新）

### 場景 15：被擋或被取代的授權事後花不掉（F4，INV-9）

**Given** 一次授權被 Kunsu 記成 `EXEC_BLOCKED`（commit 沒驗證就失敗），或這題之後又簽出了更新的 nonce（重新授權）
**When** 任何人（例如拿到被擋人話狀態的 agent）在 15 分鐘內拿舊 approval 檔打 `/approvals/verify`
**Then** 回 `valid=false`（已有結果／被更新的授權取代），executor 不 fire；Kunsu 記 `EXEC_BLOCKED` 時同時刪掉這次寫的 approval 檔
（best effort）；查消耗與記結果互斥，不會有「Kunsu 記成被擋、executor 卻已驗證」的空檔（integration：`inv_approval_single_use`、
`qa_money_round11.rs` R11-6）

### 場景 16：executor 驗證前當掉不卡死（F5）

**Given** app 在 executor 驗證授權之前（prepare 中、或 commit 還沒打 verify）當掉，錢鏈列卡在 executing
**When** app 重啟
**Then** 這題簽發但沒有結果的 nonce 記 `EXEC_BLOCKED: Kunsu 在 executor 驗證授權之前中斷，錢沒有動`、執行紀錄寫同樣內容、reactor 列
`dead`；Discord 說「錢沒有動，可以在回執重新授權」；回執有「重新授權並執行」且按了放行；agent 讀到被擋，不是「錢鏈還在跑」；
殘留的 commit 子行程晚到也驗證失敗（integration：`qa_money_round11.rs` R11-4）

### 場景 17：錢鏈在跑時不是結果不明（F6）

**Given** commit 正在跑，executor 已驗證（消耗）授權、還沒回報
**When** agent `ask_get`／`ask_wait`，或使用者呼叫人工對帳、重新授權
**Then** agent 讀到「錢鏈還在跑，以 Kunsu 回執為準」；人工對帳與重新授權都被拒；最後 commit 成功只 fire 一次
（integration：`qa_money_round11.rs` R11-5）。錢鏈跑完、同一列接著在原對話追加時，agent 照錢鏈結果讀到「已執行完成」（跟追加訊息一致），
回執的 `money_state` 仍是 `running`、不給按鈕，人工對帳與重新授權照樣被拒（`qa11_agent_reads_the_money_result_while_the_delivery_runs`）

### 場景 18：對帳推翻不了已執行（F7）

**Given** 某一次授權的結果已是 `COMMIT_OK`（之後 app 在交付途中當掉，reactor 列 `needs_reconcile`）
**When** 有人呼叫 `resolve_money_outcome`（任一方向）或 `reauthorize_question`
**Then** 人工對帳被拒（沒有結果不明的 nonce）；重新授權被拒（看每一次授權的結果，不只看執行紀錄開頭）；executor 只 fire 過一次；
重啟後走交付中斷回收，不發「請在回執標記」的錢鏈對帳訊息（integration：`qa_money_round11.rs` R11-7）

### 場景 19：prepare 不繼承 Kunsu 的 stdin（F8）

**Given** Kunsu 從終端機啟動（stdin 是終端機或管線）
**When** 錢鏈 spawn prepare
**Then** prepare 的 stdin 是 /dev/null（integration：`qa_money_round11.rs` R11-8）

## 驗收清單

- [ ] AC-MA-01：建題到作答前 prepare 命令零執行、零 token 產生
- [ ] AC-MA-02：點批准當下完成 prepare → token → record → HMAC 簽章 → 落檔五步，record 欄位齊備
- [ ] AC-MA-03：`param_hash = sha256(params_line 正規化)`，編輯後題目簽的是編輯後參數
- [ ] AC-MA-04：prepare 失敗不產生授權檔，卡片轉紅 + `execution_blocked`
- [ ] AC-MA-05：verify 首次 `{valid:true, consumed_now:true}`，消耗記 DB + ledger
- [ ] AC-MA-06：同 record 二次 verify 必 `valid:false`（INV-9，測試 `inv_approval_single_use`）
- [ ] AC-MA-07：`answered_at` +15 分鐘後 verify 必拒（INV-10，測試 `inv_approval_ttl`）
- [ ] AC-MA-08：executor 端 param_hash 重算不符即拒絕
- [ ] AC-MA-09：record 任一欄位篡改或非 Keychain key 簽名驗章必敗；key 僅 app 可讀
- [ ] AC-MA-10：executor 四道護欄（2,000 紅線／20% 偏離／3 天冷卻／50,000 硬上限）在新通道原樣生效
- [ ] AC-MA-11：舊 isatty commit 通道行為與改動前完全一致（diff 驗證：executor.py 舊分支零行變更）
- [ ] AC-MA-12：red 題無一鍵按鈕、作答不產 approval record（INV-11，測試 `inv_red_needs_options`）
- [ ] AC-MA-13：executor.py `--approval-file` 改動獨立 commit、獨立測試（executor 所在的 vault repo）
- [ ] AC-MA-14：錢類題建題層防線齊備——缺 prediction_ref 必拒（INV-1）、過期永不 pick_option（INV-2）

> 2026-10-09 錢 QA 第 11 輪：第 10 輪新加的四條原本編成 AC-MA-12..15，與上面既有的 AC-MA-12..14 撞號，改編成 AC-MA-16..19（既有的不動）。

- [ ] AC-MA-16（2026-10-09 QA continuity 第 10 輪）：非紅燈錢題缺 `executor_prepare_cmd` 建題就拒收（`money_needs_prepare_cmd`），
  不出現按了不會跑的「批准並執行」
- [ ] AC-MA-17：錢的結果以每一次授權（nonce）追蹤；已消耗卻沒有結果的 nonce＝結果不明，重啟恢復時記成 `EXEC_UNKNOWN`，所有讀取面、
  追加訊息與回執一致；交付改寫執行紀錄不影響判斷；`needs_reconcile` 不被覆寫（場景 12）
- [ ] AC-MA-18：使用者在回執標「錢已經動了／錢沒有動」解除結果不明（只有 UI 通道、只收主視窗與蓋板，兩個按鈕都要再確認一次）；
  標「沒有動」後才能重新授權、「已經動了」永不能；帳本有行、重建後一致（場景 12）
- [ ] AC-MA-19：被擋（`EXEC_BLOCKED`）或對帳確認沒動的黃燈錢題，回執有「重新授權並執行」（再確認一次才送出，只收主視窗與蓋板），
  走同一條簽章鏈；已執行、紅燈、安全選項、非 UI 作答的題沒有這個按鈕
- [ ] AC-MA-20（2026-10-09 錢 QA 第 11 輪，F2／F3／F9／F10）：shell 送給網頁的錢題帶伺服器算的 `money_state`，回執、失敗畫面與歷史回看
  一律依它給按鈕與文字（沒有這欄位才退回舊的字串判斷）；8/6 那類 `EXEC_FAILED` 死信卻結果不明的題有對帳鈕；按鈕跟著最新的題目；
  執行失敗事件後重新讀題，結果不明走回執＋對帳鈕；再確認焦點在「確定」、取消回原按鈕、成功後在「完成／下一題」；錢那一行圖示
  ✓／–／!／… 依狀態且附文字（場景 14）
- [ ] AC-MA-21（F1）：migration 0007 之前已 `COMMIT_OK`／`RECONCILED_EXECUTED` 的錢題，啟動與重建後補上同樣開頭的 nonce 結果（帳本有
  `backfill` 行、冪等），不被誤判結果不明、不列人工對帳、不發 Discord；其他開頭、或同一題有兩次以上未記結果的授權，維持結果不明（場景 13）
- [ ] AC-MA-22（F4，INV-9）：`/approvals/verify` 拒絕已有結果的 nonce 與被更新簽發取代的 nonce，判斷與記結果互斥；記 `EXEC_BLOCKED`
  時刪掉 approval 檔（best effort）（場景 15，測試 `inv_approval_single_use`）
- [ ] AC-MA-23（F5）：executor 驗證前當掉 → 重啟後 `EXEC_BLOCKED`（錢沒有動）、reactor 列 `dead`、Discord 說可以重新授權、
  `money_state = reauthorizable`；舊版本留下的同型 `needs_reconcile` 列在啟動補資料時一併轉成這樣（場景 16）
- [ ] AC-MA-24（F6）：錢鏈列 queued／executing、這一輪的錢鏈結果還沒寫入時 agent 讀取面與追加訊息一律「錢鏈還在跑」；錢鏈跑完、同一列
  還在交付時照錢鏈結果說；列結束前人工對帳與重新授權都被拒（場景 17）
- [ ] AC-MA-25（F7）：人工對帳只接受有結果不明 nonce 的題；重新授權看每一次授權的結果，任一 `COMMIT_OK`／`RECONCILED_EXECUTED` 就拒絕；
  錢已執行完成後在交付途中當掉走交付中斷回收（場景 18）
- [ ] AC-MA-26（F8）：prepare 子行程 stdin＝/dev/null（場景 19）

### 實作者自查紀錄（2026-10-09 錢 QA 第 11 輪；不是獨立 QA，上面的勾選留給獨立 QA）

- 自動化：`cargo test --workspace` 全綠（含 `qa_money_round11.rs` 10 條、`money_outcomes.rs` 11 條、`inv_approval_single_use`、shell
  `money_actions_only_from_main_window_and_overlay`／`ui_questions_carry_the_server_money_state`）；UI `pnpm test`、`tsc -b`、`pnpm lint`、`pnpm build` 通過。
- 瀏覽器（vite mock，port 1597，1280×860）：歷史紀錄打開 8/6 型題 → 標題「結果不明」、死信原因、「請先到廣告後台核對再對帳」、兩個對帳鈕；
  鍵盤 Shift+Tab 到「我核對過：錢沒有動」→ Enter → 焦點在「確定」→ Tab、Enter 取消 → 焦點回到原按鈕（第一次實測掉到 `<body>`，改成原按鈕
  只藏起來後通過）→ 再確認送出 → 焦點到「完成」、按鈕即時換成「重新授權並執行」→ 再確認送出 → 按鈕消失、錢鏈跑完後沒有按鈕。
  `kunsu:mock:money-outcome=unknown` 作答：回執先顯示「… 錢：錢鏈還在跑」，執行失敗事件後沒有進「執行沒有完成」，改顯示「! 錢：結果不明…」，
  錢鏈列結束後對帳鈕自己出現；`=blocked`：進失敗畫面、寫「修好原因後可以在這裡重新授權」並有「重新授權並執行」。回看已執行的錢題：「✓ 錢：已執行完成」、沒有按鈕。
