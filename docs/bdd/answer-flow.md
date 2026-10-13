# 作答流程（answer-flow）— BDD 驗收規格

> 依據：docs/CONTRACT.md §2（Answer 結構）、§4（MCP 面永不暴露作答）、§8（acked/三態）、§9（UI/IPC）；
> 設計正本 §4（卡片解剖）、§7（信任三態回執）。
> 不變量對應：INV-7。
> 界外：批准簽章與錢的授權鏈見 money-approval.md；kongko 通道結案見 kongko-integration.md；
> reactor 消化與冪等見 harness-roundtrip.md。

## 概述

作答只能來自四個通道：Tauri IPC（UI，唯一產簽章入口）、`/events/kongko`（發佈即同意）、
Discord 讀回、CLI `kunsu answer`（限使用者本人終端，isatty 檢查）。MCP tools 面**不存在**任何
作答/批准寫入。每題有三態回執 `answered → acknowledged → executed`——「我答了但系統沒反應」
只要發生一次信任歸零，所以失敗必須大聲。

## 行為規格

### 場景 1：MCP 面不存在作答寫入（INV-7）

**Given** agent 持有有效 Bearer token 連上 `/mcp`
**When** 枚舉全部 8 個 tools（`ask_create/ask_wait/ask_answers_since/ask_list/ask_get/ask_update/ask_cancel/ask_supersede`）
**Then** 沒有任何 tool 能寫入 answer、簽章或 approval；嘗試以 `ask_update` 塞 answer 欄位被 400 拒收——token 洩漏最壞情況是建假題，永遠不可能產生假批准

### 場景 2：UI 作答是唯一產簽章入口

**Given** 使用者在蓋板或卡片清單看到一題 choice
**When** 點選一個選項並確認（`answer_question` Tauri IPC command）
**Then** 題目轉 answered：`answer.choice_id` 落盤、`channel = "ui"`、`answered_at` 落當下；approval 類題同時產 HMAC 簽章（見 money-approval.md）；蓋板/卡片原地變身進入執行流視圖

### 場景 3：選項行內編輯觸發參數重算與紅線重檢

**Given** 一題動錢 choice 帶 `params_line`（如「日預算 300 → 360（+20%）」）
**When** 使用者點鉛筆行內編輯選項（如 +20% 改 +10%）
**Then** `params_line` 同步重算顯示新值（所見即所簽）、`edited_params` 記錄編輯後值、param_hash 重生成；重算後重新過紅線檢查——變更額 > NT$2,000 或偏離 > 20% 時自動升 `gate = red` 並鎖死一鍵執行（只能走使用者終端通道）

### 場景 4：自由文字永遠是逃生口

**Given** 任何一題（`allow_free_text` 恆為 true）
**When** 使用者選「改用文字」輸入自由文字回答
**Then** `answer.free_text` 落盤、題目轉 answered；自由文字答案不觸發一鍵執行（無 choice_id 無參數雜湊），由 reactor/agent 讀文字後續處理

### 場景 5：退回重擬計入出題品質

**Given** 使用者認為某題不該問他（資訊不足、問錯層次、根本不是決策）
**When** 點「這題不該問我／退回重擬」
**Then** 題目轉終態（cancelled 語義、留軌跡標記退回原因）、該 source 的出題品質分被扣、落盤 fatigue/quality event；agent 經 `ask_wait`/`ask_answers_since` 能看到退回狀態與理由

### 場景 6：答後同卡原地變身為執行流與收據

**Given** 使用者剛在卡片上作答一題帶 `wake_on_answer` 的黃燈題
**When** 執行流開始
**Then** 同一卡片原地串流顯示 prepare → guard → API 回應 → 落盤清單；終態變收據卡：「✅ 已執行｜decision: …md｜prediction: …md（verify 日期）｜Discord 已記」，各項可點開跳 vault 檔案

### 場景 7：執行失敗絕不靜默

**Given** 答後執行流的任一步失敗（prepare 失敗、guard 擋下、API 錯誤）
**When** 失敗發生
**Then** 卡片轉紅顯示人話原因；題目回列表標 `execution_blocked`；ledger 記 `execution_result` 失敗內容；絕不靜默吞掉、絕不顯示成功假象

### 場景 8：三態回執時間戳完整

**Given** 一題被作答
**When** 後續消化發生
**Then** 三個時間戳依序落盤且 UI 可見：`answered_at`（作答當下）→ `acked_at`（reactor 起跑或 consumer drain 到即標）→ `executed_at`（執行完成回寫）；`ask_get` 回傳完整三態

### 場景 9：answered 後 24 小時無人處理必告警

**Given** 一題 answered 已滿 24 小時
**When** `acked_at` 仍為空（沒有任何 reactor 或 consumer 碰過）
**Then** app 內該題標大紅，並推 Discord「⚠️ 你答的〈題目〉沒有任何 agent 處理」——失敗要大聲

### 場景 10：CLI 作答通道限使用者本人終端（INV-7）

**Given** `kunsu answer` 在非互動環境被呼叫（stdin/stdout 非 tty，例如被 agent spawn 或管線執行）
**When** isatty 檢查失敗
**Then** 命令拒絕執行並說明「作答限本人終端」；在真實互動終端執行則正常作答，`channel = "cli"`

### 場景 11：Discord 讀回通道作答

**Given** Discord 讀回通道（threads-radar-discord-review bot 模式）已啟用，一題的鏡像訊息在 Discord
**When** 使用者在 Discord 回覆批准/選項
**Then** 題目轉 answered、`channel = "discord"`；與 UI 作答互相可見（單一 queue 多個 view），先答者生效、後到通道看到已 answered 不重複作答

### 場景 12：作答競態只承認一次

**Given** 同一題同時從兩個通道被作答（如 UI 點擊與 Discord 回覆幾乎同時）
**When** 兩個寫入先後抵達
**Then** 只有第一個寫入生效（單一寫者序列化），第二個收到「已 answered」結果；answer 不被覆寫、ledger 只記一次 answered 終態

## 驗收清單

- [ ] AC-AN-01：MCP tools 無任何 answer/approval 寫入面；`ask_update` 帶 answer 欄位被拒（INV-7，測試 `inv_no_answer_via_mcp`）
- [ ] AC-AN-02：UI `answer_question` IPC 是唯一產簽章的作答入口，choice 作答落 `channel=ui` + `answered_at`
- [ ] AC-AN-03：選項編輯後 `params_line` 重算、`edited_params` 落盤、param_hash 重生成
- [ ] AC-AN-04：編輯後金額 > NT$2,000 或偏離 > 20% 自動升 red 並鎖一鍵（與 executor.py 護欄同值）
- [ ] AC-AN-05：所有題皆可自由文字作答；free_text 答案不觸發一鍵執行
- [ ] AC-AN-06：退回重擬轉終態留軌跡、扣 source 出題品質分、落 fatigue/quality event
- [ ] AC-AN-07：答後同卡串流執行流，終態收據卡含 decision/prediction/Discord 三連結
- [ ] AC-AN-08：執行失敗卡片轉紅 + `execution_blocked` + ledger 記錄，無靜默路徑
- [ ] AC-AN-09：`answered_at → acked_at → executed_at` 三態依序落盤且 `ask_get` 可查
- [ ] AC-AN-10：answered 24h 無 acked 觸發 app 大紅 + Discord 告警
- [ ] AC-AN-11：CLI `kunsu answer` 非 isatty 拒絕；isatty 作答 `channel=cli`
- [ ] AC-AN-12：Discord 讀回作答 `channel=discord`，與其他通道單一 queue 互相可見
- [ ] AC-AN-13：同題多通道競態只承認第一個答案，不覆寫、ledger 不重複
